//! DEX 多池多跳扫描的审计型数据结构（PRD §4–§7、§9）。
//!
//! 与 [crate::domain::opportunity::Opportunity] 的分工：
//! - Opportunity 是执行与风控的输入，字段服务于 Paper 执行；
//! - RouteScanRecord 是扫描与审计的记录，保存指纹、逐腿状态、投入优化曲线、
//!   M1/M3 模拟结果与拒绝原因，可由 API / 数据库复现。
//!
//! 全部金额使用 Decimal，禁止浮点；时间统一 UTC。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::market::{PoolKind, RiskViolation};
use super::types::{
    ModelLevel, OpportunityLifecycle, RouteType, SimulationStatus, TokenStatus,
};

/// 路线指纹集合（PRD §4.6）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFingerprints {
    /// 有向指纹：池序列 + 方向。同方向、同池序列视为同一机会。
    pub directed: String,
    /// 无方向指纹：忽略起点旋转与整体方向，用于识别镜像路线。
    pub undirected: String,
    /// 池序列指纹。
    pub pool_sequence: String,
    /// Token 序列指纹。
    pub token_sequence: String,
    /// 起始资产指纹。
    pub start_asset: String,
}

/// Gas 估算分项（PRD §5.6）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GasComponent {
    pub key: String,
    pub label: String,
    pub units: u64,
}

/// Gas 估算结果。必须记录价格、原生资产估值、区块与时间，便于复算。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GasEstimate {
    pub units: u64,
    pub gas_price_gwei: Decimal,
    pub native_price_usd: Decimal,
    pub usd: Decimal,
    pub components: Vec<GasComponent>,
    pub block_number: u64,
    pub estimated_at: DateTime<Utc>,
    pub note: String,
}

/// 单次模拟结果（M1 / M3）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimulationOutcome {
    pub simulation_id: String,
    pub level: ModelLevel,
    pub status: SimulationStatus,
    /// 报价输出（未扰动）。
    pub quoted_out: Decimal,
    /// 模拟输出（同等状态复算，或压力扰动后的输出）。
    pub simulated_out: Decimal,
    /// (simulated - quoted) / quoted * 10000。
    pub diff_bps: Decimal,
    /// 模拟时的净利评估（成本模型由引擎提供，可为空）。
    pub net_usd: Option<Decimal>,
    pub gas_estimate: Option<GasEstimate>,
    pub state_block: u64,
    pub notes: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl SimulationOutcome {
    /// 该等级是否允许进入可执行机会。
    pub fn is_passed(&self) -> bool {
        self.status == SimulationStatus::Passed
    }
}

/// 某一投入金额下的完整经济性评估点。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputPoint {
    pub amount_in: Decimal,
    pub amount_in_usd: Decimal,
    pub amount_out: Decimal,
    pub gross_usd: Decimal,
    /// 显式成本合计（Gas、闪电贷、优先费、延迟/MEV 准备金等）。
    /// AMM 手续费与价格冲击已含在 amount_out 中，不在此重复扣除。
    pub cost_usd: Decimal,
    pub net_usd: Decimal,
    pub net_bps: Decimal,
    pub gas_usd: Decimal,
    pub gas_share_pct: Decimal,
    pub price_impact_bps: Decimal,
    /// ladder / grid / refine / best。
    pub source: String,
}

/// 投入金额优化结果（PRD §5.4）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputOptimization {
    pub points: Vec<InputPoint>,
    pub best: Option<InputPoint>,
    /// 采样点中最小可盈利投入（不是解析解，受网格分辨率约束）。
    pub min_profitable_amount: Option<Decimal>,
    /// 采样点中最大可盈利投入。
    pub max_profitable_amount: Option<Decimal>,
    /// 受池容量与配置上限约束的最大可执行投入。
    pub max_executable_amount: Decimal,
    pub grid_points: usize,
    pub refine_iters: usize,
    /// 是否因预算/边界截断。
    pub truncated: bool,
    pub notes: Vec<String>,
}

/// 路线中单跳的可审计记录（PRD §7.4 / §14.4）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteLegRecord {
    pub idx: usize,
    pub pool_id: String,
    pub protocol_id: String,
    pub amm: PoolKind,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub fee_bps: i64,
    pub price_impact_bps: Decimal,
    pub gas_units: u64,
    pub pool_tvl_usd: Decimal,
    pub block_number: u64,
    pub state_age_ms: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SizeQuote {
    pub notional_usd: Decimal,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub gross_usd: Decimal,
    pub total_cost_usd: Decimal,
    pub net_usd: Decimal,
    pub net_bps: Decimal,
}

/// 一条被扫描、报价与模拟过的路线。该结构可直接落库并复现计算过程。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteScanRecord {
    /// 稳定 ID：route_ + 有向指纹。同一路线每次扫描复用同一 ID。
    pub route_id: String,
    pub chain_id: u64,
    pub start_token: String,
    pub route_type: RouteType,
    pub fingerprints: RouteFingerprints,
    pub token_path: Vec<String>,
    pub pool_path: Vec<String>,
    pub protocols: Vec<String>,
    pub amms: Vec<String>,
    pub hops: usize,
    pub legs: Vec<RouteLegRecord>,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub notional_usd: Decimal,
    /// 毛利润（已扣除 AMM 手续费与滑点）。
    pub gross_usd: Decimal,
    /// 显式成本合计（不含已内含在成交价里的 AMM 费用）。
    pub total_cost_usd: Decimal,
    pub net_usd: Decimal,
    pub net_bps: Decimal,
    /// AMM 手续费的信息性估值（不重复扣减，仅用于解释成交价损耗）。
    pub embedded_fee_usd: Decimal,
    pub gas: Option<GasEstimate>,
    /// 该次报价是否按闪电贷资金计价（影响费用与 Gas 分项）。
    pub flash_funded: bool,
    pub optimization: Option<InputOptimization>,
    /// M1：状态一致性模拟。
    pub simulation: Option<SimulationOutcome>,
    /// M3：压力模拟。
    pub stress: Option<SimulationOutcome>,
    pub lifecycle: OpportunityLifecycle,
    pub rejections: Vec<RiskViolation>,
    pub block_number: u64,
    pub quote_age_ms: i64,
    pub detected_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub model_level: ModelLevel,
    pub opportunity_id: Option<String>,
    pub capacity_usd: Decimal,
    /// 固定资金档位逐档报价（250/500/1k/2.5k/5k/7.5k/10k）。
    #[serde(default)]
    pub size_quotes: Vec<SizeQuote>,
    /// full：本轮全量重算；incremental：复用缓存（池状态未变）。
    pub source: String,
}

impl RouteScanRecord {
    pub fn is_profitable(&self) -> bool {
        self.net_usd > Decimal::ZERO
    }

    /// 生命周期推进：只有在 M1 通过后才允许 SIMULATED。
    pub fn advance_lifecycle(&mut self) {
        if self
            .rejections
            .iter()
            .any(|r| r.severity != super::types::RiskSeverity::Warning)
        {
            self.lifecycle = OpportunityLifecycle::Rejected;
            return;
        }
        if !self.is_profitable() {
            self.lifecycle = OpportunityLifecycle::Quoted;
            return;
        }
        match &self.simulation {
            Some(sim) if sim.is_passed() => {
                self.lifecycle = OpportunityLifecycle::Simulated;
            }
            Some(_) => {
                self.lifecycle = OpportunityLifecycle::Failed;
            }
            None => {
                self.lifecycle = OpportunityLifecycle::ProfitableEstimate;
            }
        }
    }
}

/// 一次扫描运行的统计（PRD §4.5 / §9.2）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanRunStats {
    pub run_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub elapsed_ms: u64,
    pub full_rescan: bool,
    pub reason: String,
    pub pools: usize,
    pub eligible_pools: usize,
    pub tokens: usize,
    pub approved_tokens: usize,
    pub edges: usize,
    pub candidates: usize,
    pub requoted: usize,
    pub reused: usize,
    pub visits: u64,
    pub pruned_liquidity: usize,
    pub pruned_risk: usize,
    pub truncated: bool,
    pub rejected_by_gate: usize,
    pub profitable: usize,
    pub base_tokens: Vec<String>,
    pub min_hops: usize,
    pub max_hops: usize,
    pub model_level: ModelLevel,
    pub notes: Vec<String>,
}

impl Default for ScanRunStats {
    fn default() -> Self {
        Self {
            run_id: String::new(),
            started_at: None,
            elapsed_ms: 0,
            full_rescan: true,
            reason: "init".into(),
            pools: 0,
            eligible_pools: 0,
            tokens: 0,
            approved_tokens: 0,
            edges: 0,
            candidates: 0,
            requoted: 0,
            reused: 0,
            visits: 0,
            pruned_liquidity: 0,
            pruned_risk: 0,
            truncated: false,
            rejected_by_gate: 0,
            profitable: 0,
            base_tokens: Vec::new(),
            min_hops: 2,
            max_hops: 5,
            model_level: ModelLevel::M1,
            notes: Vec::new(),
        }
    }
}

/// Token 准入判定（PRD §3.3 / §8.2）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenAdmission {
    pub symbol: String,
    pub status: TokenStatus,
    pub allowed_start: bool,
    pub allowed_hop: bool,
    pub allowed_flash: bool,
    /// 不满足准入的具体原因。
    pub reasons: Vec<String>,
}

impl TokenAdmission {
    pub fn hop_ok(&self) -> bool {
        self.status.is_approved() && self.allowed_hop
    }

    pub fn start_ok(&self) -> bool {
        self.status.is_approved() && self.allowed_start
    }
}

/// 池准入判定（PRD §8.1）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolAdmission {
    pub pool_id: String,
    pub eligible: bool,
    pub tvl_usd: Decimal,
    pub reasons: Vec<String>,
}

/// 图层统计。前端「路由搜索中心」直接展示。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphStats {
    pub chain_id: u64,
    pub tokens: usize,
    pub approved_tokens: usize,
    pub pools: usize,
    pub eligible_pools: usize,
    pub edges: usize,
    pub rejected_pools: usize,
    pub built_at: Option<DateTime<Utc>>,
    pub signature: String,
}
