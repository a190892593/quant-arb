//! 市场数据模型：Token、池、链、CEX 合约、订单簿、报价、路线、成本与评分。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::money::q6;
pub use super::types::{ChainStage, DataQuality, ModelLevel, PoolKind, RiskSeverity, Side};

/// 链注册表条目（TDD §3.1）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chain {
    pub chain_id: u64,
    pub chain_name: String,
    pub native_token: String,
    pub rpc_primary: String,
    pub rpc_secondary: String,
    pub websocket_endpoint: Option<String>,
    pub block_time_ms: u64,
    pub finality_model: String,
    pub gas_model: String,
    pub stage: ChainStage,
    pub data_quality: DataQuality,
    pub adapter_version: String,
    pub enabled: bool,
}

impl Chain {
    /// 报价最大年龄：`min(2 × 平均区块时间, 3s)`（风控 §5）。
    pub fn quote_max_age_ms(&self) -> u64 {
        (self.block_time_ms * 2).min(3_000)
    }
}

/// DEX 协议注册条目（TDD §4.1）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DexProtocol {
    pub protocol_id: String,
    pub protocol_version: String,
    pub kind: PoolKind,
    pub chain_id: u64,
    pub factory: String,
    pub router: String,
    pub adapter: String,
    pub enabled: bool,
    pub verified: bool,
    pub audit_note: String,
}

/// Token 注册表条目（合约 §2.3）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Token {
    pub symbol: String,
    pub chain_id: u64,
    pub address: String,
    pub decimals: u32,
    pub standard: String,
    pub code_hash: String,
    pub risk_tags: Vec<String>,
    pub allowed_start: bool,
    pub allowed_hop: bool,
    pub allowed_flash: bool,
    pub special_transfer: bool,
    pub enabled: bool,
}

impl Token {
    /// 只有常规 ERC-20 且未被标记特殊行为时才允许自动执行。
    pub fn execution_allowed(&self) -> bool {
        self.enabled && !self.special_transfer && self.standard == "ERC20"
    }
}

/// 池注册表条目（PRD §5.2）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolRef {
    pub pool_id: String,
    pub chain_id: u64,
    pub protocol_id: String,
    pub kind: PoolKind,
    pub address: String,
    pub token0: String,
    pub token1: String,
    pub fee_bps: i64,
    pub tick_spacing: Option<i32>,
    pub tvl_usd: Decimal,
    pub verified: bool,
    pub execution_allowed: bool,
    pub added_at: DateTime<Utc>,
}

/// V3 单 tick 流动性。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TickLiquidity {
    pub tick_index: i32,
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
}

/// 池状态快照。所有报价必须绑定 `block_number` 与 `updated_at`。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolState {
    pub pool_id: String,
    pub block_number: u64,
    pub block_hash: String,
    pub updated_at: DateTime<Utc>,
    /// V2 / StableSwap / Weighted 使用储备量；V3 亦保留用于展示 TVL。
    pub reserve0: Decimal,
    pub reserve1: Decimal,
    /// V3 价格（Q96 定点）与当前活跃流动性。
    pub sqrt_price_x96: u128,
    pub liquidity: u128,
    pub tick: i32,
    pub ticks: Vec<TickLiquidity>,
    /// StableSwap 放大系数；Weighted 池权重。
    pub amplification: Decimal,
    pub weight0: Decimal,
    pub weight1: Decimal,
    pub fee_bps: i64,
}

impl PoolState {
    /// 状态年龄（毫秒）。
    pub fn age_ms(&self, now: DateTime<Utc>) -> i64 {
        (now - self.updated_at).num_milliseconds()
    }
}

/// CEX 合约交易规则与费率（TDD §5.1、风控 §6）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instrument {
    pub exchange: String,
    pub symbol: String,
    pub base: String,
    pub quote: String,
    /// `TRADING` / `BREAK` / `HALT` / `UNKNOWN`
    pub status: String,
    pub min_qty: Decimal,
    pub qty_step: Decimal,
    pub min_notional: Decimal,
    pub price_tick: Decimal,
    pub maker_bps: Decimal,
    pub taker_bps: Decimal,
    /// 费率是否已同步；未知费率不允许生成可执行机会。
    pub fee_known: bool,
    pub updated_at: DateTime<Utc>,
}

impl Instrument {
    /// 按数量精度向下取整。
    pub fn quantize_qty(&self, qty: Decimal) -> Decimal {
        super::money::round_step_down(qty, self.qty_step)
    }

    /// 按价格精度与买卖方向取整（买单价向上、卖单价向下，保证不越过保护价）。
    pub fn quantize_price(&self, price: Decimal, side: Side) -> Decimal {
        match side {
            Side::Buy => super::money::round_step_down(price, self.price_tick),
            Side::Sell => super::money::round_step_down(price, self.price_tick),
        }
    }

    /// 最小可下单数量：同时满足 min_qty 与 min_notional。
    pub fn min_tradable_qty(&self, price: Decimal) -> Decimal {
        let by_notional = if price.is_zero() {
            self.min_qty
        } else {
            (self.min_notional / price).max(self.min_qty)
        };
        super::money::round_step_nearest(
            by_notional.max(self.min_qty),
            self.qty_step,
        )
    }

    /// 状态是否允许下单。
    pub fn is_tradable(&self) -> bool {
        self.status == "TRADING"
    }
}

/// 盘口档位。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: Decimal,
    pub qty: Decimal,
}

/// 盘口快照（完整、带序号与双时间戳）。只有完整且未过期的盘口才能生成机会。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderBook {
    pub exchange: String,
    pub symbol: String,
    pub seq: u64,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub exchange_ts: DateTime<Utc>,
    pub recv_ts: DateTime<Utc>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
    pub last_sync_ts: DateTime<Utc>,
}

impl OrderBook {
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.first().map(|l| l.price)
    }

    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first().map(|l| l.price)
    }

    pub fn mid(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((a + b) / Decimal::from(2)),
            _ => None,
        }
    }

    pub fn spread_bps(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) if !b.is_zero() => {
                Some((a - b) / b * Decimal::from(10_000))
            }
            _ => None,
        }
    }

    /// 数据年龄（毫秒），以本地接收时间为准。
    pub fn age_ms(&self, now: DateTime<Utc>) -> i64 {
        (now - self.recv_ts).num_milliseconds()
    }

    /// 可见深度（报价币计价）。
    pub fn depth_usd(&self, side: Side) -> Decimal {
        let levels = match side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        };
        levels.iter().map(|l| l.price * l.qty).sum()
    }
}

/// 盘口健康状态（风控 §6）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookHealth {
    pub exchange: String,
    pub symbol: String,
    pub valid: bool,
    pub seq: u64,
    pub age_ms: i64,
    pub spread_bps: Option<Decimal>,
    pub depth_bid_usd: Decimal,
    pub depth_ask_usd: Decimal,
    pub synced_at: DateTime<Utc>,
    pub reason: Option<String>,
}

/// 统一报价（DEX 单跳 / CEX 盘口深度成交共用）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quote {
    pub venue: String,
    pub symbol: String,
    pub side: Side,
    /// 输入数量（CEX 为 base 数量，DEX 为 token_in 数量）。
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    /// 平均成交价（quote/base）。
    pub average_price: Decimal,
    /// 最差档位成交价，用于价格保护。
    pub worst_price: Decimal,
    /// 相对盘口/现货的冲击（bps）。
    pub price_impact_bps: Decimal,
    pub fee_amount: Decimal,
    pub filled: bool,
    pub levels_consumed: usize,
    pub quoted_at: DateTime<Utc>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
}

impl Quote {
    /// 无效报价：任何下游计算必须拒绝，而不是按 0 成交。
    pub fn invalid(venue: &str, symbol: &str, side: Side, reason: &str) -> Quote {
        Quote {
            venue: venue.to_string(),
            symbol: symbol.to_string(),
            side,
            amount_in: Decimal::ZERO,
            amount_out: Decimal::ZERO,
            average_price: Decimal::ZERO,
            worst_price: Decimal::ZERO,
            price_impact_bps: Decimal::ZERO,
            fee_amount: Decimal::ZERO,
            filled: false,
            levels_consumed: 0,
            quoted_at: Utc::now(),
            valid: false,
            invalid_reason: Some(reason.to_string()),
        }
    }
}

/// 路线交易腿（TDD §4.2）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteLeg {
    pub idx: usize,
    pub protocol_id: String,
    pub adapter: String,
    pub pool_id: String,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: Decimal,
    pub expected_out: Decimal,
    pub min_out: Decimal,
    pub fee_bps: i64,
    pub price_impact_bps: Decimal,
}

/// 同链原子路线。只有同链路线可以进入链上 Executor。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DexRoute {
    pub route_id: String,
    pub chain_id: u64,
    pub start_token: String,
    pub end_token: String,
    pub legs: Vec<RouteLeg>,
    pub amount_in: Decimal,
    pub expected_out: Decimal,
    pub min_out: Decimal,
    pub gas_units: u64,
    pub gas_usd: Decimal,
    pub price_impact_bps: Decimal,
    pub model_level: ModelLevel,
    pub block_number: u64,
    pub created_at: DateTime<Utc>,
}

impl DexRoute {
    /// 路线是否闭环（最终资产 == 起始资产）。
    pub fn is_closed_loop(&self) -> bool {
        self.start_token == self.end_token
    }

    pub fn hops(&self) -> usize {
        self.legs.len()
    }
}

/// 成本明细项。`amount` 为 USDC 计价的正成本；补偿/返还使用负数并在 label 说明。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostItem {
    pub key: String,
    pub label: String,
    pub amount: Decimal,
    pub note: Option<String>,
}

/// 成本构成。合计由明细求和得出，不写死（DESIGN.md §8）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub items: Vec<CostItem>,
}

impl CostBreakdown {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, key: &str, label: &str, amount: Decimal) {
        if amount.is_zero() {
            return;
        }
        self.items.push(CostItem {
            key: key.to_string(),
            label: label.to_string(),
            amount: q6(amount),
            note: None,
        });
    }

    pub fn push_note(&mut self, key: &str, label: &str, amount: Decimal, note: &str) {
        if amount.is_zero() {
            return;
        }
        self.items.push(CostItem {
            key: key.to_string(),
            label: label.to_string(),
            amount: q6(amount),
            note: Some(note.to_string()),
        });
    }

    pub fn total(&self) -> Decimal {
        self.items.iter().map(|i| i.amount).sum()
    }

    pub fn get(&self, key: &str) -> Decimal {
        self.items
            .iter()
            .filter(|i| i.key == key)
            .map(|i| i.amount)
            .sum()
    }
}

/// 四层盈亏推演（PRD §5.5）。必须同时提供，禁止只展示理论值。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfitScenario {
    /// 理论价差（M0）。
    pub theoretical_usd: Decimal,
    /// 按真实池状态/盘口深度计算的可成交毛利。
    pub executable_gross_usd: Decimal,
    pub total_cost_usd: Decimal,
    /// 计入全部成本的净利润。
    pub conservative_net_usd: Decimal,
    /// 压力模型（50% 折扣 + 延迟/部分成交/回滚）。
    pub stress_net_usd: Decimal,
    /// 最坏情景损失（正数表示损失金额）。
    pub worst_case_loss_usd: Decimal,
}

impl ProfitScenario {
    pub fn zero() -> Self {
        Self {
            theoretical_usd: Decimal::ZERO,
            executable_gross_usd: Decimal::ZERO,
            total_cost_usd: Decimal::ZERO,
            conservative_net_usd: Decimal::ZERO,
            stress_net_usd: Decimal::ZERO,
            worst_case_loss_usd: Decimal::ZERO,
        }
    }
}

/// 评分构成项（TDD §6.3）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreComponent {
    pub key: String,
    pub label: String,
    pub points: Decimal,
    pub max: Decimal,
    pub note: Option<String>,
}

/// 机会评分（0–100）。评分不能替代硬性风控拦截。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiskScore {
    pub total: Decimal,
    pub grade: String,
    pub components: Vec<ScoreComponent>,
}

impl RiskScore {
    pub fn from_components(components: Vec<ScoreComponent>) -> Self {
        let total: Decimal = components.iter().map(|c| c.points).sum();
        let grade = if total >= Decimal::from(80) {
            "A"
        } else if total >= Decimal::from(65) {
            "B"
        } else if total >= Decimal::from(50) {
            "C"
        } else if total >= Decimal::from(35) {
            "D"
        } else {
            "E"
        };
        Self {
            total: q6(total),
            grade: grade.to_string(),
            components,
        }
    }
}

/// 一条被风控拒绝的记录：必须包含规则、阈值、实际值与等级。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiskViolation {
    pub key: String,
    pub label: String,
    pub severity: RiskSeverity,
    pub limit: Decimal,
    pub actual: Decimal,
    pub unit: String,
    pub message: String,
}

/// 风控判定结果。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RiskDecision {
    pub allowed: bool,
    pub violations: Vec<RiskViolation>,
    pub warnings: Vec<RiskViolation>,
    pub checked_at: DateTime<Utc>,
    pub param_version: String,
}

impl RiskDecision {
    pub fn reject(mut self, v: RiskViolation) -> Self {
        self.allowed = false;
        if v.severity == RiskSeverity::Warning {
            self.warnings.push(v);
        } else {
            self.violations.push(v);
        }
        self
    }

    /// 拒绝原因摘要，写入机会对象供前端展示。
    pub fn reasons(&self) -> Vec<String> {
        self.violations
            .iter()
            .map(|v| format!("[{}] {}: 阈值 {} 实际 {}", v.severity, v.label, v.limit, v.actual))
            .collect()
    }
}
