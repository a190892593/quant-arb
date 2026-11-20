//! 统一 Paper Trading 引擎（PRD §5.6、TDD §7）。
//!
//! 与未来 Live 共用同一套机会对象、交易计划格式与执行状态机；
//! 差别只在「腿的落地方式」：DEX 走本地模拟，CEX 走盘口深度模拟。
//!
//! 硬性约束：
//! - CEX 与 DEX 不是同一事务，不存在跨场所原子回滚；
//! - 一切以实际成交数量为准，部分成交必须重算另一条腿；
//! - 订单状态未知不等于未成交，禁止盲目重发；
//! - DEX 回滚也要付 Gas；补偿必须计入本次策略净利。

pub mod scenario;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::cex::depth;
use crate::domain::execution::{
    CexFill, DexTxResult, ExecutionEvent, ExecutionLeg, ExecutionRecord, ExposureRecord,
    UnwindRecord,
};
use crate::domain::ledger::LedgerEntry;
use crate::domain::market::{CostBreakdown, Instrument, ModelLevel, OrderBook};
use crate::domain::money::{bps_of, q8};
use crate::domain::opportunity::{FundingSource, Opportunity};
use crate::domain::plan::{
    fingerprint, plan_hash_input, CexOrderPlan, DexLegPlan, PlanState, TradePlan,
};
use crate::domain::types::{
    AccountKind, Direction, EntryKind, ExecutionState, HedgeOrder, OrderStatus, OrderType, Side,
    StrategyType, VenueKind,
};
use crate::ledger::{LedgerEngine, PostRequest};
use scenario::{Rng, Scenario};

/// Paper 执行参数。
#[derive(Clone, Debug)]
pub struct PaperParams {
    pub ttl_sec: i64,
    pub max_unhedged_usd: Decimal,
    pub hedge_deadline_ms: i64,
    pub price_protection_bps: Decimal,
}

impl Default for PaperParams {
    fn default() -> Self {
        Self {
            ttl_sec: 30,
            // 账户净值 0.5%（5000 基准 = 25 USDC，风控 §7）
            max_unhedged_usd: Decimal::from(25),
            hedge_deadline_ms: 2_000,
            price_protection_bps: Decimal::from(10),
        }
    }
}

/// 执行瞬间的市场快照。
pub struct ExecMarket<'a> {
    pub now: DateTime<Utc>,
    pub head_block: u64,
    pub book: Option<&'a OrderBook>,
    pub instrument: Option<&'a Instrument>,
    pub prices: &'a HashMap<String, Decimal>,
    pub chain_name: String,
    pub cex_venue: String,
    pub dex_venue: String,
}

impl ExecMarket<'_> {
    fn price(&self, asset: &str) -> Decimal {
        self.prices.get(asset).copied().unwrap_or(Decimal::ZERO)
    }
}

/// Paper 引擎。
pub struct PaperEngine {
    pub scenario: Scenario,
    pub params: PaperParams,
    rng: Rng,
    nonce: u64,
}

impl PaperEngine {
    pub fn new(seed: u64, scenario: Scenario, params: PaperParams) -> Self {
        Self {
            scenario,
            params,
            rng: Rng::new(seed),
            nonce: 1,
        }
    }

    /// 由机会生成可执行计划（Paper 与未来 Live 共用格式）。
    pub fn build_plan(&mut self, opp: &Opportunity, mkt: &ExecMarket<'_>) -> TradePlan {
        let now = mkt.now;
        self.nonce += 1;
        let deadline = now + Duration::seconds(self.params.ttl_sec);

        let dex_leg = opp.dex_route.as_ref().map(|r| DexLegPlan {
            route_id: r.route_id.clone(),
            chain_id: r.chain_id,
            amount_in: r.amount_in,
            min_out: r.min_out,
            gas_limit: r.gas_units.max(120_000),
            max_gas_price_gwei: Decimal::new(5, 1),
            max_block_number: r.block_number + 3,
            legs: r.legs.clone(),
            route_hash: fingerprint(&format!("{}{}", r.route_id, r.amount_in)),
        });

        let cex_leg = match (opp.cex_exchange.clone(), opp.cex_symbol.clone()) {
            (Some(exchange), Some(symbol)) => {
                let side = match opp.direction {
                    Some(Direction::DexBuyCexSell) => Side::Sell,
                    Some(Direction::CexBuyDexSell) => Side::Buy,
                    None => Side::Sell,
                };
                let qty = opp.target_qty;
                let reference = mkt
                    .book
                    .and_then(|b| match side {
                        Side::Buy => b.best_ask(),
                        Side::Sell => b.best_bid(),
                    })
                    .unwrap_or(Decimal::ONE);
                let limit = match side {
                    Side::Buy => {
                        reference
                            * (Decimal::ONE
                                + self.params.price_protection_bps / Decimal::from(10_000))
                    }
                    Side::Sell => {
                        reference
                            * (Decimal::ONE
                                - self.params.price_protection_bps / Decimal::from(10_000))
                    }
                };
                let base = mkt
                    .instrument
                    .map(|i| i.base.clone())
                    .unwrap_or_else(|| symbol.trim_end_matches("USDT").to_string());
                Some(CexOrderPlan {
                    exchange,
                    symbol,
                    base,
                    quote: "USDT".into(),
                    side,
                    qty,
                    limit_price: limit,
                    reference_price: reference,
                    max_slippage_bps: self.params.price_protection_bps,
                    order_type: OrderType::Ioc,
                    allow_partial: true,
                    client_order_id: format!("cx_{}", crate::domain::new_id("o").replace("o_", "")),
                })
            }
            _ => None,
        };

        let mut plan = TradePlan {
            plan_id: crate::domain::new_id("plan"),
            opportunity_id: opp.opportunity_id.clone(),
            strategy_type: opp.strategy_type,
            chain_id: opp.chain_id,
            direction: opp.direction,
            hedge_order: opp.hedge_order,
            target_notional: opp.target_notional,
            dex_leg,
            cex_leg,
            funding: opp.funding.clone(),
            min_profit_usd: Decimal::ZERO,
            expected_net_profit_usd: opp.conservative_net_profit,
            nonce: self.nonce,
            deadline,
            created_at: now,
            state: PlanState::Approved,
            plan_hash: String::new(),
            signer: "paper-simulator".into(),
            signature: "local-simulated".into(),
        };
        plan.plan_hash = fingerprint(&plan_hash_input(&plan));
        plan
    }

    /// 执行一次计划，返回完整执行记录（同时写入账本）。
    pub fn execute(
        &mut self,
        opp: &Opportunity,
        plan: &TradePlan,
        ledger: &mut LedgerEngine,
        mkt: &ExecMarket<'_>,
    ) -> ExecutionRecord {
        let mut rec = new_record(opp, plan, mkt);
        if plan.is_expired(mkt.now) {
            rec.transition(ExecutionState::Expired, "计划已过期，未提交任何腿", "WARN");
            return rec;
        }
        rec.transition(ExecutionState::Validated, "计划校验通过（Paper）", "INFO");

        // 只保留 DEX 原子执行路径（CEX–DEX 非原子策略已移除）。
        self.execute_dex_atomic(opp, plan, ledger, mkt, &mut rec);
        rec.updated_at = Utc::now();
        rec.recompute_pnl();
        rec
    }

    // ------------------------------------------------------------------
    // DEX 原子执行
    // ------------------------------------------------------------------

    fn execute_dex_atomic(
        &mut self,
        opp: &Opportunity,
        plan: &TradePlan,
        ledger: &mut LedgerEngine,
        mkt: &ExecMarket<'_>,
        rec: &mut ExecutionRecord,
    ) {
        let Some(route) = opp.dex_route.as_ref() else {
            rec.transition(ExecutionState::Rejected, "缺少 DEX 路线", "ERROR");
            return;
        };
        let start = route.start_token.clone();
        let end = route.end_token.clone();
        let amount_in = route.amount_in;
        let px_start = mkt.price(&start);

        rec.transition(ExecutionState::Planned, "已生成链上计划（本地模拟，不广播）", "INFO");

        // ---- 资金准备 ----
        let ctx = PostCtx::new(opp, plan, rec);
        let mut flash_fee_usd = Decimal::ZERO;
        let mut pending: Vec<PostRequest> = Vec::new();

        match &opp.funding {
            FundingSource::FlashLoan {
                amount,
                fee_amount,
                token,
                ..
            } => {
                if token != &start {
                    rec.transition(ExecutionState::Rejected, "闪电贷资产与起始资产不一致", "ERROR");
                    return;
                }
                flash_fee_usd = *fee_amount * mkt.price(&start);
                let wallet = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &start, AccountKind::DexWallet);
                pending.push(ctx.req(wallet, &start, *amount, EntryKind::FlashBorrow, mkt, "闪电贷借款到账"));
            }
            FundingSource::OwnCapital => {
                let wallet = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &start, AccountKind::DexWallet);
                let bal = ledger.balance(&wallet);
                if bal < amount_in {
                    rec.notes.push(format!(
                        "自有资金不足：需要 {amount_in} {start}，可用 {bal}"
                    ));
                    rec.transition(ExecutionState::Rejected, "自有资金不足，未提交链上交易", "WARN");
                    return;
                }
                pending.push(ctx.req(wallet, &start, -amount_in, EntryKind::Trade, mkt, "DEX 交易输入"));
            }
        }

        // ---- 本地 EVM 模拟 ----
        rec.transition(ExecutionState::DexSubmitted, "本地 EVM 模拟已提交", "INFO");
        let latency = self
            .rng
            .range_u64(self.scenario.dex_latency_ms.0, self.scenario.dex_latency_ms.1);
        let slip_bps = dec_from_f64(self.rng.range(
            self.scenario.dex_slippage_bps.0,
            self.scenario.dex_slippage_bps.1,
        ));
        let out = q8(route.expected_out * (Decimal::ONE - slip_bps / Decimal::from(10_000)));
        let gas_usd = route.gas_usd;
        let reverted =
            self.rng.chance(self.scenario.dex_revert_probability) || out < route.min_out;

        if reverted {
            let reason = if out < route.min_out {
                format!("最低输出不满足：模拟 {out} < 要求 {}", route.min_out)
            } else {
                "模拟被抢先/夹击，Pool 状态变化导致回滚".to_string()
            };
            book_gas(ledger, opp, plan, rec, mkt, gas_usd, "失败交易 Gas");
            rec.dex_tx = Some(DexTxResult {
                tx_hash: fake_hash(&mut self.rng),
                block_number: mkt.head_block,
                gas_used: route.gas_units,
                gas_price_gwei: Decimal::new(2, 2),
                gas_usd,
                success: false,
                amount_in,
                amount_out: Decimal::ZERO,
                min_out: route.min_out,
                revert_reason: Some(reason.clone()),
                simulated_at: mkt.now + Duration::milliseconds(latency as i64),
            });
            rec.legs.push(dex_leg(start.clone(), amount_in, Decimal::ZERO, amount_in, Decimal::ZERO, OrderStatus::Rejected, mkt, latency));
            rec.costs.push("gas", "Gas（失败交易）", gas_usd);
            rec.gross_pnl = Decimal::ZERO;
            rec.transition(ExecutionState::DexReverted, reason, "ERROR");
            return;
        }

        // ---- 成功：按链上顺序写账 ----
        if let FundingSource::FlashLoan { amount, fee_amount, .. } = &opp.funding {
            let wallet = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &start, AccountKind::DexWallet);
            pending.push(ctx.req(wallet, &start, -amount_in, EntryKind::Trade, mkt, "DEX 交易输入（闪电贷资金）"));
            let _ = amount;
            let wallet_repay = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &start, AccountKind::DexWallet);
            let wallet_out = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &end, AccountKind::DexWallet);
            pending.push(ctx.req(wallet_out, &end, out, EntryKind::Trade, mkt, "DEX 交易输出"));
            pending.push(ctx.req(wallet_repay, &start, -(*amount + *fee_amount), EntryKind::FlashRepay, mkt, "偿还闪电贷本金与费用"));
        } else {
            let wallet_out = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, &end, AccountKind::DexWallet);
            pending.push(ctx.req(wallet_out, &end, out, EntryKind::Trade, mkt, "DEX 交易输出"));
        }
        let gas_acct = ensure(ledger, "chain", VenueKind::Dex, "USDC", AccountKind::GasExpense);
        let wallet_usdc = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, "USDC", AccountKind::DexWallet);
        pending.push(ctx.req(wallet_usdc, "USDC", -gas_usd, EntryKind::Gas, mkt, "交易 Gas"));
        pending.push(ctx.req(gas_acct, "USDC", gas_usd, EntryKind::Gas, mkt, "交易 Gas 记入成本"));
        post_batch(ledger, pending, rec);

        let gross = out * mkt.price(&end) - amount_in * px_start;
        rec.dex_tx = Some(DexTxResult {
            tx_hash: fake_hash(&mut self.rng),
            block_number: mkt.head_block,
            gas_used: route.gas_units,
            gas_price_gwei: Decimal::new(2, 2),
            gas_usd,
            success: true,
            amount_in,
            amount_out: out,
            min_out: route.min_out,
            revert_reason: None,
            simulated_at: mkt.now + Duration::milliseconds(latency as i64),
        });
        rec.legs.push(dex_leg(start.clone(), amount_in, out, amount_in, amount_in / out, OrderStatus::Filled, mkt, latency));

        rec.costs.push("dex_fee", "DEX 手续费（已含在成交价中）", opp.costs.get("dex_fee"));
        rec.costs.push("gas", "Gas", gas_usd);
        if !flash_fee_usd.is_zero() {
            rec.costs.push("flash_fee", "闪电贷费用", flash_fee_usd);
        }
        rec.gross_pnl = gross;
        rec.qty_requested = amount_in;
        rec.qty_filled = amount_in;
        mirror_realized_pnl(ledger, opp, plan, rec, mkt, gross);
        rec.transition(ExecutionState::Hedged, "原子闭环完成并已核对", "INFO");
        rec.transition(ExecutionState::Reconciled, "账本已记账", "INFO");
    }

    // ------------------------------------------------------------------
}

/// CEX 腿模拟结果摘要。
struct CexFillSummary {
    filled: Decimal,
    notional: Decimal,
    avg_price: Decimal,
    unknown: bool,
}

impl CexFillSummary {
    fn rejected(unknown: bool) -> Self {
        Self {
            filled: Decimal::ZERO,
            notional: Decimal::ZERO,
            avg_price: Decimal::ZERO,
            unknown,
        }
    }
}

fn inst_taker(i: &Instrument) -> i64 {
    if i.taker_bps.is_zero() {
        10
    } else {
        i.taker_bps.round().to_i64().unwrap_or(10)
    }
}

fn dec_from_f64(v: f64) -> Decimal {
    Decimal::from_f64_retain(v).unwrap_or(Decimal::ZERO)
}

/// 记账上下文：统一附带策略/机会/计划/执行关联（自持所有权，避免借用冲突）。
#[derive(Clone)]
struct PostCtx {
    strategy: StrategyType,
    opportunity_id: String,
    plan_id: String,
    execution_id: String,
}

impl PostCtx {
    fn new(opp: &Opportunity, plan: &TradePlan, rec: &ExecutionRecord) -> Self {
        Self {
            strategy: opp.strategy_type,
            opportunity_id: opp.opportunity_id.clone(),
            plan_id: plan.plan_id.clone(),
            execution_id: rec.execution_id.clone(),
        }
    }

    fn req(
        &self,
        account_id: String,
        asset: &str,
        amount: Decimal,
        kind: EntryKind,
        mkt: &ExecMarket<'_>,
        memo: &str,
    ) -> PostRequest {
        PostRequest {
            account_id,
            kind,
            asset: asset.to_string(),
            amount: q8(amount),
            price_usd: mkt.price(asset),
            strategy_type: Some(self.strategy),
            opportunity_id: Some(self.opportunity_id.clone()),
            plan_id: Some(self.plan_id.clone()),
            execution_id: Some(self.execution_id.clone()),
            ref_id: None,
            memo: memo.to_string(),
        }
    }
}

fn ensure(
    ledger: &mut LedgerEngine,
    venue: &str,
    vk: VenueKind,
    asset: &str,
    kind: AccountKind,
) -> String {
    ledger.ensure_account(venue, vk, asset, kind)
}

/// 记录一笔 CEX 腿的成交与手续费（卖出扣资产、买入扣计价币）。
#[allow(clippy::too_many_arguments)]
fn post_cex_leg(
    ctx: &PostCtx,
    ledger: &mut LedgerEngine,
    mkt: &ExecMarket<'_>,
    instrument: &Instrument,
    side: Side,
    filled: Decimal,
    notional: Decimal,
    fee: Decimal,
    batch: &mut Vec<PostRequest>,
) {
    let base_acct = ensure(ledger, &mkt.cex_venue, VenueKind::Cex, &instrument.base, AccountKind::CexSpot);
    let quote_acct = ensure(ledger, &mkt.cex_venue, VenueKind::Cex, &instrument.quote, AccountKind::CexSpot);
    let fee_acct = ensure(ledger, &mkt.cex_venue, VenueKind::Cex, &instrument.quote, AccountKind::CexFee);
    match side {
        Side::Sell => {
            batch.push(ctx.req(base_acct, &instrument.base, -filled, EntryKind::Trade, mkt, "CEX 卖出"));
            batch.push(ctx.req(quote_acct, &instrument.quote, notional - fee, EntryKind::Trade, mkt, "CEX 卖出回款（扣费后）"));
        }
        Side::Buy => {
            batch.push(ctx.req(quote_acct, &instrument.quote, -(notional + fee), EntryKind::Trade, mkt, "CEX 买入支出（含费）"));
            batch.push(ctx.req(base_acct, &instrument.base, filled, EntryKind::Trade, mkt, "CEX 买入"));
        }
    }
    batch.push(ctx.req(fee_acct, &instrument.quote, fee, EntryKind::Fee, mkt, "CEX 手续费"));
}

fn book_gas(
    ledger: &mut LedgerEngine,
    opp: &Opportunity,
    plan: &TradePlan,
    rec: &mut ExecutionRecord,
    mkt: &ExecMarket<'_>,
    gas_usd: Decimal,
    memo: &str,
) {
    let ctx = PostCtx::new(opp, plan, rec);
    let gas_acct = ensure(ledger, "chain", VenueKind::Dex, "USDC", AccountKind::GasExpense);
    let wallet = ensure(ledger, &mkt.dex_venue, VenueKind::Dex, "USDC", AccountKind::DexWallet);
    post_batch(
        ledger,
        vec![
            ctx.req(wallet, "USDC", -gas_usd, EntryKind::Gas, mkt, memo),
            ctx.req(gas_acct, "USDC", gas_usd, EntryKind::Gas, mkt, memo),
        ],
        rec,
    );
}

/// 把毛已实现盈亏镜像到 REALIZED_PNL 统计账户（不改变权益）。
fn mirror_realized_pnl(
    ledger: &mut LedgerEngine,
    opp: &Opportunity,
    plan: &TradePlan,
    rec: &mut ExecutionRecord,
    mkt: &ExecMarket<'_>,
    gross: Decimal,
) {
    if gross.is_zero() {
        return;
    }
    let acct = ledger.ensure_account("global", VenueKind::Clearing, "USDC", AccountKind::RealizedPnl);
    let ctx = PostCtx::new(opp, plan, rec);
    post_batch(
        ledger,
        vec![ctx.req(acct, "USDC", gross, EntryKind::Pnl, mkt, "已实现盈亏（毛）")],
        rec,
    );
}

fn post_batch(ledger: &mut LedgerEngine, batch: Vec<PostRequest>, rec: &mut ExecutionRecord) {
    // 零金额流水没有会计意义，账本也会拒绝，因此在提交前过滤。
    let batch: Vec<PostRequest> = batch
        .into_iter()
        .filter(|r| !r.amount.is_zero() || r.kind == EntryKind::Adjustment)
        .collect();
    if batch.is_empty() {
        return;
    }
    match ledger.post_batch(batch) {
        Ok(_) => {}
        Err(e) => {
            rec.notes.push(format!("账本记账失败：{e}"));
            rec.transition(
                ExecutionState::ManualRecoveryRequired,
                format!("记账失败，需要人工核对：{e}"),
                "ERROR",
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dex_leg(
    symbol: String,
    qty_requested: Decimal,
    qty_out: Decimal,
    qty_in: Decimal,
    avg_price: Decimal,
    status: OrderStatus,
    mkt: &ExecMarket<'_>,
    latency_ms: u64,
) -> ExecutionLeg {
    let confirmed = Some(mkt.now + Duration::milliseconds(latency_ms as i64));
    let avg = if avg_price.is_zero() && !qty_out.is_zero() {
        qty_in / qty_out
    } else {
        avg_price
    };
    ExecutionLeg {
        venue_kind: "DEX".into(),
        venue: mkt.dex_venue.clone(),
        symbol,
        side: Side::Buy,
        qty_requested,
        qty_filled: qty_out,
        avg_price: avg,
        fee_amount: Decimal::ZERO,
        fee_asset: "USDC".into(),
        submitted_at: Some(mkt.now),
        confirmed_at: confirmed,
        status,
        tx_hash: None,
        block_number: Some(mkt.head_block),
    }
}

fn new_record(opp: &Opportunity, plan: &TradePlan, mkt: &ExecMarket<'_>) -> ExecutionRecord {
    let symbol = opp
        .cex_symbol
        .clone()
        .or_else(|| opp.dex_route.as_ref().map(|r| r.start_token.clone()))
        .unwrap_or_default();
    ExecutionRecord {
        execution_id: crate::domain::new_id("exec"),
        plan_id: plan.plan_id.clone(),
        opportunity_id: opp.opportunity_id.clone(),
        strategy_type: opp.strategy_type,
        chain_id: opp.chain_id,
        chain_name: opp.chain_name.clone(),
        state: ExecutionState::Detected,
        direction: opp.direction,
        hedge_order: opp.hedge_order,
        symbol,
        target_notional: opp.target_notional,
        required_dex_inventory: opp.required_dex_inventory.iter().map(|i| i.required).sum(),
        required_cex_inventory: opp.required_cex_inventory.iter().map(|i| i.required).sum(),
        legs: vec![],
        fills: vec![],
        dex_tx: None,
        qty_requested: opp.target_qty,
        qty_filled: Decimal::ZERO,
        qty_hedged: Decimal::ZERO,
        exposure: None,
        max_exposure_usd: Decimal::ZERO,
        exposure_ms: 0,
        gross_pnl: Decimal::ZERO,
        costs: CostBreakdown::new(),
        net_pnl: Decimal::ZERO,
        gas_paid_usd: Decimal::ZERO,
        compensation_cost: Decimal::ZERO,
        unwind: None,
        events: vec![ExecutionEvent {
            ts: mkt.now,
            from: ExecutionState::Detected,
            to: ExecutionState::Detected,
            message: "机会进入执行队列".into(),
            level: "INFO".into(),
        }],
        data_quality: opp.data_quality,
        model_level: ModelLevel::M2,
        created_at: mkt.now,
        updated_at: mkt.now,
        notes: vec![],
    }
}

fn fake_hash(rng: &mut Rng) -> String {
    format!("0x{:016x}{:016x}", rng.next_u64(), rng.next_u64())
}

/// 把执行记录中的成本明细表达为「净利 = 毛利 − 成本」。
pub fn describe_pnl(rec: &ExecutionRecord) -> String {
    format!(
        "毛利 {} − 成本 {} = 净利 {}",
        rec.gross_pnl.round_dp(4),
        rec.costs.total().round_dp(4),
        rec.net_pnl.round_dp(4)
    )
}

/// 供上层判断：执行记录是否需要人工介入。
pub fn needs_manual_recovery(rec: &ExecutionRecord) -> bool {
    matches!(
        rec.state,
        ExecutionState::ManualRecoveryRequired | ExecutionState::UnwindFailed
    ) || rec.notes.iter().any(|n| n.contains("账本记账失败"))
}

/// 执行结果对账：检查是否存在必须人工处理的记账异常。
pub fn reconcile_execution(rec: &ExecutionRecord, _entries: &[LedgerEntry]) -> bool {
    !needs_manual_recovery(rec)
}

/// 用于展示的默认价格表（资产 → USDC 价格）。
pub fn default_prices(prices: &HashMap<String, Decimal>) -> Vec<(String, Decimal)> {
    prices.iter().map(|(k, v)| (k.clone(), *v)).collect()
}
