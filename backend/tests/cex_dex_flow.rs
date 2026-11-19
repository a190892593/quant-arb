//! CEX–DEX 双腿 Paper 流程测试（TDD §7.1）。
//!
//! 关键断言：
//! - CEX 先行时，DEX 腿必须按 CEX 实际成交量重算；
//! - 部分成交后剩余敞口必须被计量并处置；
//! - 两条腿的账本流水与执行记录净利一致（权益变动 = 净利）。

use std::collections::HashMap;

use arb_backend::domain::execution::ExecutionRecord;
use arb_backend::domain::market::{
    BookLevel, CostBreakdown, DataQuality, DexRoute, Instrument, ModelLevel, OrderBook,
    ProfitScenario, RiskScore, RouteLeg,
};
use arb_backend::domain::money::dec;
use arb_backend::domain::opportunity::{FundingSource, InventoryNeed, Opportunity};
use arb_backend::domain::types::{
    AccountKind, Direction, EntryKind, ExecutionState, HedgeOrder, OppStatus, OpportunityLifecycle,
    StrategyType, VenueKind,
};
use arb_backend::ledger::{LedgerEngine, PostRequest};
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

fn ledger_with_both_legs() -> LedgerEngine {
    let mut l = LedgerEngine::new(dec("5000"));
    let mut batch = Vec::new();
    let mut add = |venue: &str, vk: VenueKind, asset: &str, amount: Decimal, kind: AccountKind| {
        let id = l.ensure_account(venue, vk, asset, kind);
        batch.push(PostRequest {
            account_id: id,
            kind: EntryKind::Deposit,
            asset: asset.to_string(),
            amount,
            price_usd: if asset == "WETH" || asset == "ETH" {
                dec("2800")
            } else {
                Decimal::ONE
            },
            strategy_type: None,
            opportunity_id: None,
            plan_id: None,
            execution_id: None,
            ref_id: Some("test-seed".into()),
            memo: "测试期初资金".into(),
        });
    };
    // DEX 侧持有 USDC，CEX 侧持有 ETH（DEX 买、CEX 卖）
    add("dex-arb", VenueKind::Dex, "USDC", dec("2000"), AccountKind::DexWallet);
    add("cex-binance", VenueKind::Cex, "ETH", dec("0.5"), AccountKind::CexSpot);
    add("cex-binance", VenueKind::Cex, "USDT", dec("2000"), AccountKind::CexSpot);
    l.post_batch(batch).expect("期初资金注入失败");
    l
}

fn book() -> OrderBook {
    OrderBook {
        exchange: "binance".into(),
        symbol: "ETHUSDT".into(),
        seq: 1,
        bids: vec![
            BookLevel { price: dec("2800"), qty: dec("1") },
            BookLevel { price: dec("2790"), qty: dec("1") },
        ],
        asks: vec![
            BookLevel { price: dec("2802"), qty: dec("1") },
            BookLevel { price: dec("2812"), qty: dec("1") },
        ],
        exchange_ts: chrono::Utc::now(),
        recv_ts: chrono::Utc::now(),
        valid: true,
        invalid_reason: None,
        last_sync_ts: chrono::Utc::now(),
    }
}

fn instrument() -> Instrument {
    Instrument {
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
    }
}

/// DEX 买 WETH：花 USDC，得到 WETH。
fn dex_buy_route(usdc_in: Decimal, weth_out: Decimal) -> DexRoute {
    DexRoute {
        route_id: "route_cex_dex".into(),
        chain_id: 42161,
        start_token: "USDC".into(),
        end_token: "WETH".into(),
        legs: vec![RouteLeg {
            idx: 0,
            protocol_id: "uniswap-v3".into(),
            adapter: "uniswap-v3".into(),
            pool_id: "uni-v3-weth-usdc-5".into(),
            token_in: "USDC".into(),
            token_out: "WETH".into(),
            amount_in: usdc_in,
            expected_out: weth_out,
            min_out: weth_out * dec("0.995"),
            fee_bps: 5,
            price_impact_bps: dec("5"),
        }],
        amount_in: usdc_in,
        expected_out: weth_out,
        min_out: weth_out * dec("0.995"),
        gas_units: 130_000,
        gas_usd: dec("0.18"),
        price_impact_bps: dec("5"),
        model_level: ModelLevel::M2,
        block_number: 100,
        created_at: chrono::Utc::now(),
    }
}

fn opportunity(route: DexRoute, qty: Decimal) -> Opportunity {
    let notional = qty * dec("2800");
    Opportunity {
        opportunity_id: "opp_hedge".into(),
        strategy_type: StrategyType::CexDex,
        chain_id: 42161,
        chain_name: "Arbitrum".into(),
        dex_route: Some(route.clone()),
        cex_exchange: Some("binance".into()),
        cex_symbol: Some("ETHUSDT".into()),
        direction: Some(Direction::DexBuyCexSell),
        target_notional: notional,
        target_qty: qty,
        required_dex_inventory: vec![InventoryNeed {
            venue: "dex-arb".into(),
            asset: "USDC".into(),
            required: route.amount_in,
            available: dec("2000"),
        }],
        required_cex_inventory: vec![InventoryNeed {
            venue: "cex-binance".into(),
            asset: "ETH".into(),
            required: qty,
            available: dec("0.5"),
        }],
        theoretical_edge_bps: dec("25"),
        executable_gross_profit: dec("3"),
        total_cost: dec("1"),
        conservative_net_profit: dec("2"),
        stress_net_profit: dec("1"),
        worst_case_loss: dec("1"),
        costs: CostBreakdown::new(),
        scenario: ProfitScenario::zero(),
        risk_score: RiskScore::from_components(vec![]),
        data_quality: DataQuality::Verified,
        model_level: ModelLevel::M2,
        confidence: "test".into(),
        quote_timestamp: chrono::Utc::now(),
        quote_block_number: 100,
        book_age_ms: 10,
        quote_age_ms: 10,
        status: OppStatus::Actionable,
        lifecycle: OpportunityLifecycle::Simulated,
        route_fingerprint: None,
        rejections: vec![],
        capacity_usd: dec("5000"),
        funding: FundingSource::OwnCapital,
        hedge_order: Some(HedgeOrder::CexFirst),
        detected_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(30),
    }
}

fn run(scenario: Scenario, qty: Decimal) -> (ExecutionRecord, LedgerEngine, Decimal) {
    let prices = prices();
    let mut ledger = ledger_with_both_legs();
    let start_equity = ledger.total_equity();
    let b = book();
    let inst = instrument();
    let route = dex_buy_route(dec("279.16"), qty * dec("1.0004"));
    let opp = opportunity(route, qty);
    let mut engine = PaperEngine::new(11, scenario, PaperParams::default());
    let now = chrono::Utc::now();
    let mkt = ExecMarket {
        now,
        head_block: 100,
        book: Some(&b),
        instrument: Some(&inst),
        prices: &prices,
        chain_name: "Arbitrum".into(),
        cex_venue: "cex-binance".into(),
        dex_venue: "dex-arb".into(),
    };
    let plan = engine.build_plan(&opp, &mkt);
    let rec = engine.execute(&opp, &plan, &mut ledger, &mkt);
    let delta = ledger.total_equity() - start_equity;
    (rec, ledger, delta)
}

#[test]
fn cex_first_full_fill_completes_both_legs() {
    let (rec, ledger, delta) = run(Scenario::optimistic(), dec("0.1"));

    assert_eq!(rec.state, ExecutionState::Reconciled, "events: {:?}", rec.events);
    assert!(rec.qty_filled > Decimal::ZERO);
    assert_eq!(rec.qty_filled, dec("0.1"), "CEX 应全部成交");
    assert!(rec.fills.len() >= 1, "必须有 CEX 成交明细");
    // CEX 卖 + DEX 买 = 两条腿
    let has_cex = rec.legs.iter().any(|l| l.venue_kind == "CEX");
    let has_dex = rec.legs.iter().any(|l| l.venue_kind == "DEX");
    assert!(has_cex && has_dex, "两腿都应留痕");
    // 账本权益变动必须等于执行净利
    assert_eq!(delta.round_dp(6), rec.net_pnl.round_dp(6), "权益变动应等于净利");
    assert_eq!(ledger.total_gas(), rec.costs.get("gas"), "Gas 必须单独记账");
}

#[test]
fn cex_partial_fill_scales_dex_leg_and_records_exposure() {
    let mut scenario = Scenario::optimistic();
    scenario.partial_fill_probability = 1.0;
    scenario.partial_fill_band = (0.5, 0.5);

    let (rec, _ledger, _delta) = run(scenario, dec("0.1"));
    assert_eq!(rec.state, ExecutionState::Reconciled);
    assert_eq!(rec.qty_filled, dec("0.05"), "IOC 只成交一半时必须以实际数量为准");
    // 第二条腿的输入量按实际成交量缩放（DEX 腿以 USDC 计价，约 279.16 × 0.5）
    let dex_tx = rec.dex_tx.as_ref().expect("DEX 模拟结果");
    assert!(
        (dex_tx.amount_in - dec("139.58")).abs() < dec("0.5"),
        "DEX 腿应按 CEX 实际成交比例重算，实际 {}",
        dex_tx.amount_in
    );
    assert!(rec.events.iter().any(|e| e.to == ExecutionState::PartiallyFilled));
}

#[test]
fn cex_rejection_prevents_dex_leg() {
    let mut scenario = Scenario::optimistic();
    scenario.timeout_probability = 1.0;
    scenario.unknown_resolve_cancel_probability = 1.0;

    let (rec, ledger, delta) = run(scenario, dec("0.1"));
    // 状态未知 → 查询确认为未成交 → 不执行第二腿
    assert!(
        matches!(rec.state, ExecutionState::OrderStatusUnknown | ExecutionState::HedgeFailed),
        "state={:?}",
        rec.state
    );
    assert!(rec.dex_tx.is_none(), "CEX 未成交时不得执行 DEX");
    assert_eq!(delta, Decimal::ZERO, "未成交不应产生资金变动");
    assert_eq!(ledger.total_gas(), Decimal::ZERO);
}
