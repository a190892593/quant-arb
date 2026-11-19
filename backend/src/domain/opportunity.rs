//! 标准机会对象（TDD §6.1）与四层盈亏推演（PRD §5.5）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::market::{
    CostBreakdown, DataQuality, DexRoute, ModelLevel, ProfitScenario, RiskScore, RiskViolation,
};
use super::types::{Direction, HedgeOrder, OppStatus, StrategyType};

/// 资金方式。闪电贷只用于同链原子策略；CEX–DEX 使用预置库存。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FundingSource {
    OwnCapital,
    FlashLoan {
        provider: String,
        token: String,
        amount: Decimal,
        /// 费用以 bps 表示时同时给出实际费用金额，避免硬编码固定比例。
        fee_bps: Decimal,
        fee_amount: Decimal,
    },
}

impl FundingSource {
    pub fn label(&self) -> &'static str {
        match self {
            FundingSource::OwnCapital => "自有资金",
            FundingSource::FlashLoan { .. } => "闪电贷",
        }
    }

    pub fn is_flash(&self) -> bool {
        matches!(self, FundingSource::FlashLoan { .. })
    }
}

/// 库存需求：机会在任一场所需要的资产数量。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryNeed {
    pub venue: String,
    pub asset: String,
    pub required: Decimal,
    pub available: Decimal,
}

impl InventoryNeed {
    pub fn is_sufficient(&self) -> bool {
        self.available >= self.required
    }

    pub fn shortfall(&self) -> Decimal {
        (self.required - self.available).max(Decimal::ZERO)
    }
}

/// 统一机会对象。DEX 原子与 CEX–DEX 对冲共用同一结构，只是字段填充不同。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Opportunity {
    pub opportunity_id: String,
    pub strategy_type: StrategyType,
    pub chain_id: u64,
    pub chain_name: String,

    // ---- DEX 腿 ----
    pub dex_route: Option<DexRoute>,

    // ---- CEX 腿 ----
    pub cex_exchange: Option<String>,
    pub cex_symbol: Option<String>,
    pub direction: Option<Direction>,

    /// 目标名义金额（USDC）。
    pub target_notional: Decimal,
    /// 目标基础资产数量（CEX 腿与 DEX 腿按此数量对齐）。
    pub target_qty: Decimal,
    pub required_dex_inventory: Vec<InventoryNeed>,
    pub required_cex_inventory: Vec<InventoryNeed>,

    /// 理论价差（bps），仅用于对比，不作为验收依据。
    pub theoretical_edge_bps: Decimal,
    pub executable_gross_profit: Decimal,
    pub total_cost: Decimal,
    pub conservative_net_profit: Decimal,
    pub stress_net_profit: Decimal,
    pub worst_case_loss: Decimal,

    pub costs: CostBreakdown,
    pub scenario: ProfitScenario,
    pub risk_score: RiskScore,

    pub data_quality: DataQuality,
    pub model_level: ModelLevel,
    /// 模拟可信度说明，例如「M2 深度成交 + 竞争模型」。
    pub confidence: String,

    pub quote_timestamp: DateTime<Utc>,
    pub quote_block_number: u64,
    pub book_age_ms: i64,
    pub quote_age_ms: i64,

    pub status: OppStatus,
    /// 被风控拒绝的具体原因（PRD §5.4）。
    pub rejections: Vec<RiskViolation>,

    /// 在当前深度/容量下可用的最大名义金额。
    pub capacity_usd: Decimal,
    pub funding: FundingSource,

    pub hedge_order: Option<HedgeOrder>,
    pub detected_at: DateTime<Utc>,
    /// 计划有效期（风控 §5：初始不超过 30 秒且受区块约束）。
    pub expires_at: DateTime<Utc>,
}

impl Opportunity {
    /// 是否在有效期内。
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now < self.expires_at
    }

    /// 是否有可执行路径（状态与时效同时满足）。
    pub fn is_tradable(&self, now: DateTime<Utc>) -> bool {
        self.status == OppStatus::Actionable && self.is_fresh(now)
    }

    /// 净利润率（bps，按目标名义）。
    pub fn net_bps(&self) -> Decimal {
        if self.target_notional.is_zero() {
            Decimal::ZERO
        } else {
            self.conservative_net_profit / self.target_notional * Decimal::from(10_000)
        }
    }
}
