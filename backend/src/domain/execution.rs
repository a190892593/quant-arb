//! 执行记录：一次机会从计划到结算的完整轨迹（TDD §7、PRD §5.6）。
//!
//! 关键约束：
//! - 请求数量 ≠ 成交数量，一律以实际成交为准。
//! - 部分成交、回滚、状态未知、补偿都必须留痕。
//! - 未对冲敞口单独计量，并记录持续时间。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::market::{CostBreakdown, DataQuality, ModelLevel};
use super::types::{
    Direction, ExecutionState, HedgeOrder, OrderStatus, OrderType, Side, StrategyType,
};
/// 单腿（CEX 或 DEX）的执行快照。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionLeg {
    /// `CEX` / `DEX`
    pub venue_kind: String,
    pub venue: String,
    pub symbol: String,
    pub side: Side,
    pub qty_requested: Decimal,
    pub qty_filled: Decimal,
    pub avg_price: Decimal,
    pub fee_amount: Decimal,
    pub fee_asset: String,
    pub submitted_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub status: OrderStatus,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
}

/// CEX 模拟成交明细。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CexFill {
    pub fill_id: String,
    pub order_id: String,
    pub client_order_id: String,
    pub exchange: String,
    pub symbol: String,
    pub side: Side,
    pub price: Decimal,
    pub qty: Decimal,
    pub fee: Decimal,
    pub fee_asset: String,
    pub ts: DateTime<Utc>,
    pub is_compensation: bool,
}

/// DEX 链上模拟结果（第一阶段本地模拟，不广播）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DexTxResult {
    pub tx_hash: String,
    pub block_number: u64,
    pub gas_used: u64,
    pub gas_price_gwei: Decimal,
    pub gas_usd: Decimal,
    pub success: bool,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub min_out: Decimal,
    pub revert_reason: Option<String>,
    pub simulated_at: DateTime<Utc>,
}

/// 未对冲敞口记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExposureRecord {
    pub asset: String,
    /// 正数为多头、负数为空头。
    pub qty: Decimal,
    pub usd_value: Decimal,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    /// 峰值敞口（USD），用于「最大未对冲敞口」统计。
    pub peak_usd: Decimal,
}

impl ExposureRecord {
    /// 敞口持续时间（毫秒）。
    pub fn duration_ms(&self, now: DateTime<Utc>) -> i64 {
        (self.closed_at.unwrap_or(now) - self.opened_at).num_milliseconds()
    }
}

/// 补偿（反向平仓）记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnwindRecord {
    pub unwind_id: String,
    pub reason: String,
    pub venue: String,
    pub symbol: String,
    pub side: Side,
    pub qty: Decimal,
    pub avg_price: Decimal,
    pub fee: Decimal,
    /// 补偿造成的净损失（正数表示亏损）。
    pub loss_usd: Decimal,
    pub attempts: u32,
    pub status: OrderStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// 状态机迁移事件。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub ts: DateTime<Utc>,
    pub from: ExecutionState,
    pub to: ExecutionState,
    pub message: String,
    /// `INFO` / `WARN` / `ERROR`
    pub level: String,
}

/// 一次完整执行记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub execution_id: String,
    pub plan_id: String,
    pub opportunity_id: String,
    pub strategy_type: StrategyType,
    pub chain_id: u64,
    pub chain_name: String,
    pub state: ExecutionState,
    pub direction: Option<Direction>,
    pub hedge_order: Option<HedgeOrder>,

    pub symbol: String,
    pub target_notional: Decimal,
    pub required_dex_inventory: Decimal,
    pub required_cex_inventory: Decimal,

    pub legs: Vec<ExecutionLeg>,
    pub fills: Vec<CexFill>,
    pub dex_tx: Option<DexTxResult>,

    pub qty_requested: Decimal,
    pub qty_filled: Decimal,
    pub qty_hedged: Decimal,

    pub exposure: Option<ExposureRecord>,
    pub max_exposure_usd: Decimal,
    pub exposure_ms: i64,

    pub gross_pnl: Decimal,
    pub costs: CostBreakdown,
    pub net_pnl: Decimal,
    pub gas_paid_usd: Decimal,
    pub compensation_cost: Decimal,

    pub unwind: Option<UnwindRecord>,
    pub events: Vec<ExecutionEvent>,

    pub data_quality: DataQuality,
    pub model_level: ModelLevel,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub notes: Vec<String>,
}

impl ExecutionRecord {
    /// 成交率：实际成交 / 请求数量。
    pub fn fill_ratio(&self) -> Decimal {
        if self.qty_requested.is_zero() {
            return Decimal::ZERO;
        }
        self.qty_filled / self.qty_requested
    }

    /// 是否需要对账（已成交但有敞口或补偿）。
    pub fn needs_reconciliation(&self) -> bool {
        self.exposure.is_some() || self.unwind.is_some() || self.state.is_exception()
    }

    /// 记录一次状态迁移。
    pub fn transition(&mut self, to: ExecutionState, message: impl Into<String>, level: &str) {
        let from = self.state;
        self.state = to;
        self.updated_at = Utc::now();
        self.events.push(ExecutionEvent {
            ts: Utc::now(),
            from,
            to,
            message: message.into(),
            level: level.to_string(),
        });
    }

    /// 重新计算成本合计与净利（合计由明细求和，不写死）。
    pub fn recompute_pnl(&mut self) {
        let total_cost = self.costs.total();
        self.net_pnl = self.gross_pnl - total_cost;
        self.gas_paid_usd = self.costs.get("gas");
        self.compensation_cost = self
            .costs
            .get("hedge_compensation")
            .max(Decimal::ZERO);
    }
}

/// 执行列表用的轻量视图。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub execution_id: String,
    pub plan_id: String,
    pub strategy_type: StrategyType,
    pub symbol: String,
    pub chain_name: String,
    pub state: ExecutionState,
    pub hedge_order: Option<HedgeOrder>,
    pub qty_requested: Decimal,
    pub qty_filled: Decimal,
    pub net_pnl: Decimal,
    pub exposure_usd: Decimal,
    pub exposure_ms: i64,
    pub created_at: DateTime<Utc>,
}

impl From<&ExecutionRecord> for ExecutionSummary {
    fn from(e: &ExecutionRecord) -> Self {
        Self {
            execution_id: e.execution_id.clone(),
            plan_id: e.plan_id.clone(),
            strategy_type: e.strategy_type,
            symbol: e.symbol.clone(),
            chain_name: e.chain_name.clone(),
            state: e.state,
            hedge_order: e.hedge_order,
            qty_requested: e.qty_requested,
            qty_filled: e.qty_filled,
            net_pnl: e.net_pnl,
            exposure_usd: e
                .exposure
                .as_ref()
                .map(|x| x.usd_value.abs())
                .unwrap_or(Decimal::ZERO),
            exposure_ms: e.exposure_ms,
            created_at: e.created_at,
        }
    }
}

/// CEX 订单（含状态未知场景）。第一阶段仅模拟。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperOrder {
    pub order_id: String,
    pub client_order_id: String,
    pub exchange: String,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub qty: Decimal,
    pub limit_price: Decimal,
    pub filled_qty: Decimal,
    pub avg_price: Decimal,
    pub fee: Decimal,
    pub status: OrderStatus,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 状态未知查询窗口（风控 §7：初始 2 秒）。
    pub unknown_since: Option<DateTime<Utc>>,
}
