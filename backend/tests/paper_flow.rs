//! 端到端流程测试：机会 → 计划 → Paper 执行 → 账本核对。
//!
//! 这些测试验证「净利润 = 毛利 − 成本」与「权益变动 = 已实现净额」两条恒等式，
//! 并覆盖 DEX 回滚仍付 Gas 的硬性要求（PRD §2.2、风控 §10）。

use std::collections::HashMap;

use arb_backend::domain::market::{
    CostBreakdown, DataQuality, DexRoute, ModelLevel, ProfitScenario, RiskScore, RouteLeg,
};
use arb_backend::domain::money::dec;
use arb_backend::domain::opportunity::{FundingSource, InventoryNeed, Opportunity};
use arb_backend::domain::types::{
    ExecutionState, OppStatus, OpportunityLifecycle, Side, StrategyType,
};
use arb_backend::ledger::LedgerEngine;
use arb_backend::paper::scenario::Scenario;
use arb_backend::paper::{ExecMarket, PaperEngine, PaperParams};
use rust_decimal::Decimal;

fn prices() -> HashMap<String, Decimal> {
    let mut m = HashMap::new();
    m.insert("USDC".to_string(), Decimal::ONE);
    m.insert("USDT".to_string(), Decimal::ONE);
    m.insert("WETH".to_string(), dec("2800"));
    m.insert("ETH".to_string(), dec("2800"));
    m
}

fn ledger_with_funds() -> LedgerEngine {
    let mut l = LedgerEngine::new(dec("5000"));
    // 给 DEX 钱包与 CEX 现货都准备 USDC/USDT/WETH/ETH
    use arb_backend::domain::types::{AccountKind, EntryKind, VenueKind};
    let mut batch = Vec::new();
    let mut add = |venue: &str, vk: VenueKind, asset: &str, amount: Decimal, kind: AccountKind| {
        let id = l.ensure_account(venue, vk, asset, kind);
        batch.push(arb_backend::ledger::PostRequest {
            account_id: id,
            kind: EntryKind::Deposit,
            asset: asset.to_string(),
            amount,
            price_usd: if asset == "WETH" || asset == "ETH" { dec("2800") } else { Decimal::ONE },
            strategy_type: None,
            opportunity_id: None,
            plan_id: None,
            execution_id: None,
            ref_id: Some("test-seed".into()),
            memo: "测试期初资金".into(),
        });
    };
    add("dex-arb", VenueKind::Dex, "WETH", dec("0.1"), AccountKind::DexWallet);
    add("cex-binance", VenueKind::Cex, "ETH", dec("0.1"), AccountKind::CexSpot);
    add("cex-binance", VenueKind::Cex, "USDC", dec("500"), AccountKind::CexSpot);
    add("dex-arb", VenueKind::Dex, "USDT", dec("500"), AccountKind::DexWallet);
    l.post_batch(batch).expect("期初资金注入失败");
    l
}

fn dex_route(amount_in: Decimal, expected_out: Decimal, min_out: Decimal) -> DexRoute {
    DexRoute {
        route_id: "route_test".into(),
        chain_id: 42161,
        start_token: "USDC".into(),
        end_token: "USDC".into(),
        legs: vec![
            RouteLeg {
                idx: 0,
                protocol_id: "uniswap-v2".into(),
                adapter: "uniswap-v2".into(),
                pool_id: "p1".into(),
                token_in: "USDC".into(),
                token_out: "WETH".into(),
                amount_in,
                expected_out: expected_out / dec("2800"),
                min_out: (expected_out / dec("2800")) * dec("0.995"),
                fee_bps: 5,
                price_impact_bps: dec("3"),
            },
            RouteLeg {
                idx: 1,
                protocol_id: "uniswap-v3".into(),
                adapter: "uniswap-v3".into(),
                pool_id: "p2".into(),
                token_in: "WETH".into(),
                token_out: "USDC".into(),
                amount_in: expected_out / dec("2800"),
                expected_out,
                min_out,
                fee_bps: 5,
                price_impact_bps: dec("3"),
            },
        ],
        amount_in,
        expected_out,
        min_out,
        gas_units: 260_000,
        gas_usd: dec("0.5"),
        price_impact_bps: dec("6"),
        model_level: ModelLevel::M2,
        block_number: 100,
        created_at: chrono::Utc::now(),
    }
}

fn opportunity(route: DexRoute) -> Opportunity {
    Opportunity {
        opportunity_id: "opp_test".into(),
        strategy_type: StrategyType::DexTriple,
        chain_id: 42161,
        chain_name: "Arbitrum".into(),
        dex_route: Some(route.clone()),
        cex_exchange: None,
        cex_symbol: None,
        direction: None,
        target_notional: route.amount_in,
        target_qty: route.amount_in,
        required_dex_inventory: vec![InventoryNeed {
            venue: "dex-arb".into(),
            asset: "USDC".into(),
            required: route.amount_in,
            available: dec("5000"),
        }],
        required_cex_inventory: vec![],
        theoretical_edge_bps: dec("20"),
        executable_gross_profit: route.expected_out - route.amount_in,
        total_cost: dec("0.5"),
        conservative_net_profit: route.expected_out - route.amount_in - dec("0.5"),
        stress_net_profit: Decimal::ZERO,
        worst_case_loss: dec("0.5"),
        costs: CostBreakdown::new(),
        scenario: ProfitScenario::zero(),
        risk_score: RiskScore::from_components(vec![]),
        data_quality: DataQuality::Verified,
        model_level: ModelLevel::M2,
        confidence: "test".into(),
        quote_timestamp: chrono::Utc::now(),
        quote_block_number: 100,
        book_age_ms: 0,
        quote_age_ms: 0,
        status: OppStatus::Actionable,
        lifecycle: OpportunityLifecycle::Simulated,
        route_fingerprint: None,
        rejections: vec![],
        capacity_usd: dec("5000"),
        funding: FundingSource::OwnCapital,
        hedge_order: None,
        detected_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(30),
    }
}

fn exec_market<'a>(
    prices: &'a HashMap<String, Decimal>,
    now: chrono::DateTime<chrono::Utc>,
) -> ExecMarket<'a> {
    ExecMarket {
        now,
        head_block: 100,
        book: None,
        instrument: None,
        prices,
        chain_name: "Arbitrum".into(),
        cex_venue: "cex-binance".into(),
        dex_venue: "dex-arb".into(),
    }
}

#[test]
fn dex_atomic_success_updates_ledger_and_pnl() {
    let prices = prices();
    let mut ledger = ledger_with_funds();
    let start_equity = ledger.total_equity();
    let start_usdc = {
        let acct = ledger
            .find_account("dex-arb", "USDC", arb_backend::domain::types::AccountKind::DexWallet)
            .expect("DEX 钱包账户");
        ledger.balance(&acct.account_id)
    };

    // 1000 USDC → 1010 USDC（毛利 10 USDC），Gas 0.5
    let route = dex_route(dec("1000"), dec("1010"), dec("1005"));
    let opp = opportunity(route);
    let mut engine = PaperEngine::new(1, Scenario::optimistic(), PaperParams::default());
    let now = chrono::Utc::now();
    let mkt = exec_market(&prices, now);
    let plan = engine.build_plan(&opp, &mkt);
    let rec = engine.execute(&opp, &plan, &mut ledger, &mkt);

    assert_eq!(rec.state, ExecutionState::Reconciled, "事件: {:?}", rec.events);
    assert_eq!(rec.gross_pnl, dec("10"));
    assert_eq!(rec.net_pnl, dec("9.5"), "净利应为毛利 − Gas");
    assert!(rec.dex_tx.as_ref().unwrap().success);

    let end_usdc = {
        let acct = ledger
            .find_account("dex-arb", "USDC", arb_backend::domain::types::AccountKind::DexWallet)
            .unwrap();
        ledger.balance(&acct.account_id)
    };
    assert_eq!(end_usdc - start_usdc, dec("9.5"), "钱包变动应等于净利");

    // 权益变动 = 已实现毛盈亏 − Gas
    let equity_delta = ledger.total_equity() - start_equity;
    assert_eq!(equity_delta, dec("9.5"));
    assert_eq!(ledger.realized_pnl(), dec("10"));
    assert_eq!(ledger.total_gas(), dec("0.5"));
}

#[test]
fn dex_revert_still_pays_gas() {
    let prices = prices();
    let mut ledger = ledger_with_funds();
    let start_equity = ledger.total_equity();

    let route = dex_route(dec("1000"), dec("1010"), dec("1005"));
    let opp = opportunity(route);
    // 必定回滚的场景
    let mut scenario = Scenario::optimistic();
    scenario.dex_revert_probability = 1.0;
    let mut engine = PaperEngine::new(2, scenario, PaperParams::default());
    let now = chrono::Utc::now();
    let mkt = exec_market(&prices, now);
    let plan = engine.build_plan(&opp, &mkt);
    let rec = engine.execute(&opp, &plan, &mut ledger, &mkt);

    assert_eq!(rec.state, ExecutionState::DexReverted);
    assert!(rec.dex_tx.as_ref().map(|t| !t.success).unwrap_or(false));
    assert_eq!(rec.gross_pnl, Decimal::ZERO);
    assert_eq!(rec.net_pnl, dec("-0.5"), "回滚不等于零成本：必须计入 Gas");
    assert_eq!(ledger.total_gas(), dec("0.5"));
    assert_eq!(ledger.total_equity(), start_equity - dec("0.5"));
}

#[test]
fn insufficient_inventory_is_rejected_without_posting() {
    let prices = prices();
    let mut ledger = LedgerEngine::new(dec("100")); // 只有 50 USDC 在 DEX 钱包
    let start_equity = ledger.total_equity();

    let route = dex_route(dec("1000"), dec("1010"), dec("1005"));
    let opp = opportunity(route);
    let mut engine = PaperEngine::new(3, Scenario::optimistic(), PaperParams::default());
    let now = chrono::Utc::now();
    let mkt = exec_market(&prices, now);
    let plan = engine.build_plan(&opp, &mkt);
    let rec = engine.execute(&opp, &plan, &mut ledger, &mkt);

    assert_eq!(rec.state, ExecutionState::Rejected);
    assert!(rec.notes.iter().any(|n| n.contains("自有资金不足")));
    assert_eq!(ledger.total_equity(), start_equity, "被拒绝的机会不得产生资金变动");
}

#[test]
fn plan_hash_is_unique_per_plan_and_expires() {
    let prices = prices();
    let opp = opportunity(dex_route(dec("1000"), dec("1010"), dec("1005")));
    let mut engine = PaperEngine::new(4, Scenario::optimistic(), PaperParams::default());
    let now = chrono::Utc::now();
    let mkt = exec_market(&prices, now);
    let p1 = engine.build_plan(&opp, &mkt);
    let p2 = engine.build_plan(&opp, &mkt);
    assert_ne!(p1.plan_hash, p2.plan_hash, "每份计划必须有独立指纹与 nonce");
    assert_ne!(p1.nonce, p2.nonce);

    // 过期计划不得执行
    let expired = arb_backend::domain::plan::TradePlan {
        deadline: now - chrono::Duration::seconds(5),
        ..p1
    };
    let mut ledger = ledger_with_funds();
    let rec = engine.execute(&opp, &expired, &mut ledger, &mkt);
    assert_eq!(rec.state, ExecutionState::Expired);
}

#[test]
fn cex_leg_sells_into_bid_depth() {
    // 该测试只验证盘口方向与深度：卖出必须吃 bid 档位
    use arb_backend::cex::depth;
    use arb_backend::domain::market::{BookLevel, Instrument, OrderBook};
    use arb_backend::domain::types::OrderType;

    let book = OrderBook {
        exchange: "binance".into(),
        symbol: "ETHUSDT".into(),
        seq: 1,
        bids: vec![
            BookLevel { price: dec("2800"), qty: dec("1") },
            BookLevel { price: dec("2790"), qty: dec("1") },
        ],
        asks: vec![BookLevel { price: dec("2802"), qty: dec("1") }],
        exchange_ts: chrono::Utc::now(),
        recv_ts: chrono::Utc::now(),
        valid: true,
        invalid_reason: None,
        last_sync_ts: chrono::Utc::now(),
    };
    let inst = Instrument {
        exchange: "binance".into(),
        symbol: "ETHUSDT".into(),
        base: "ETH".into(),
        quote: "USDT".into(),
        status: "TRADING".into(),
        min_qty: dec("0.0001"),
        qty_step: dec("0.0001"),
        min_notional: dec("5"),
        price_tick: dec("0.01"),
        maker_bps: dec("8"),
        taker_bps: dec("10"),
        fee_known: true,
        updated_at: chrono::Utc::now(),
    };
    let q = depth::simulate_order(&book, &inst, Side::Sell, dec("1.5"), None, OrderType::Ioc);
    // 1.0 @2800 + 0.5 @2790 = 4195
    assert_eq!(q.filled_qty, dec("1.5"));
    assert_eq!(q.notional, dec("4195"));
    assert_eq!(q.avg_price.round_dp(4), dec("2796.6667"));
    assert_eq!(q.fee, dec("4.195"));
}
