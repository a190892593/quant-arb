//! 绩效与报表模型（PRD §7 Paper Trading 验收、风控 §11 统计要求）。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::types::StrategyType;

/// 权益曲线点。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquityPoint {
    pub ts: DateTime<Utc>,
    /// 当日/当点总权益（USDC）。
    pub equity: Decimal,
    /// 累计已实现 + 未实现盈亏。
    pub pnl: Decimal,
    /// 从历史峰值回撤（金额，正数表示回撤）。
    pub drawdown: Decimal,
    /// 回撤百分比。
    pub drawdown_pct: Decimal,
}

/// 单策略绩效。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategyPerformance {
    pub strategy_type: StrategyType,
    pub label: String,
    pub trades: u64,
    pub wins: u64,
    pub losses: u64,
    pub win_rate: Decimal,
    pub gross_profit: Decimal,
    pub total_cost: Decimal,
    pub net_profit: Decimal,
    pub avg_net_profit: Decimal,
    pub max_drawdown: Decimal,
    /// 成交率（CEX 实际成交 / 请求数量）。
    pub fill_rate: Decimal,
    /// DEX 回滚率。
    pub revert_rate: Decimal,
    /// 平均未对冲敞口时间（毫秒）。
    pub avg_exposure_ms: i64,
    /// 容量利用率：实际成交名义 / 可用资金。
    pub capital_utilization: Decimal,
}

/// 净利润分布直方图。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DistributionBin {
    pub label: String,
    pub from: Decimal,
    pub to: Decimal,
    pub count: u64,
}

/// 关键指标（带单位与说明）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Kpi {
    pub key: String,
    pub label: String,
    pub value: Decimal,
    pub unit: String,
    pub note: String,
    /// 相对上一周期的变化。
    pub delta: Option<Decimal>,
}

/// 汇总绩效报告（`GET /paper/performance`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub generated_at: DateTime<Utc>,
    pub start_equity: Decimal,
    pub equity: Decimal,
    pub realized_pnl: Decimal,
    pub unrealized_pnl: Decimal,
    pub total_fees: Decimal,
    pub total_gas: Decimal,
    pub total_compensation: Decimal,
    pub total_rebalance: Decimal,
    pub net_pnl: Decimal,
    pub total_return_pct: Decimal,
    pub max_drawdown: Decimal,
    pub max_drawdown_usd: Decimal,
    pub sharpe: Decimal,
    pub profit_factor: Decimal,
    pub trades: u64,
    pub wins: u64,
    pub losses: u64,
    pub win_rate: Decimal,
    pub avg_hold_ms: i64,
    pub fill_rate: Decimal,
    pub revert_rate: Decimal,
    pub capital_utilization: Decimal,
    pub series: Vec<EquityPoint>,
    pub by_strategy: Vec<StrategyPerformance>,
    pub distribution: Vec<DistributionBin>,
    pub kpis: Vec<Kpi>,
}

impl PerformanceReport {
    pub fn empty(equity: Decimal) -> Self {
        Self {
            generated_at: Utc::now(),
            start_equity: equity,
            equity,
            realized_pnl: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
            total_fees: Decimal::ZERO,
            total_gas: Decimal::ZERO,
            total_compensation: Decimal::ZERO,
            total_rebalance: Decimal::ZERO,
            net_pnl: Decimal::ZERO,
            total_return_pct: Decimal::ZERO,
            max_drawdown: Decimal::ZERO,
            max_drawdown_usd: Decimal::ZERO,
            sharpe: Decimal::ZERO,
            profit_factor: Decimal::ZERO,
            trades: 0,
            wins: 0,
            losses: 0,
            win_rate: Decimal::ZERO,
            avg_hold_ms: 0,
            fill_rate: Decimal::ZERO,
            revert_rate: Decimal::ZERO,
            capital_utilization: Decimal::ZERO,
            series: vec![],
            by_strategy: vec![],
            distribution: vec![],
            kpis: vec![],
        }
    }
}

/// 扫描器运行状态（`GET /scanner/status`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScannerStatus {
    pub state: String,
    pub started_at: Option<DateTime<Utc>>,
    pub uptime_ms: u64,
    pub quotes: u64,
    pub detected: u64,
    pub actionable: u64,
    pub rejected: u64,
    pub watch: u64,
    pub expired: u64,
    pub executed: u64,
    pub chains: u64,
    pub protocols: u64,
    pub pools: u64,
    pub scan_interval_ms: u64,
    pub last_scan_ms: u64,
    // ---- 多跳引擎（PRD §9.2）----
    #[serde(default)]
    pub engine_routes: u64,
    #[serde(default)]
    pub engine_candidates: u64,
    #[serde(default)]
    pub engine_requoted: u64,
    #[serde(default)]
    pub engine_reused: u64,
    #[serde(default)]
    pub engine_edges: u64,
    #[serde(default)]
    pub engine_tokens: u64,
    #[serde(default)]
    pub engine_visits: u64,
    #[serde(default)]
    pub engine_truncated: bool,
    #[serde(default)]
    pub engine_full_rescan: bool,
}

impl Default for ScannerStatus {
    fn default() -> Self {
        Self {
            state: "STOPPED".into(),
            started_at: None,
            uptime_ms: 0,
            quotes: 0,
            detected: 0,
            actionable: 0,
            rejected: 0,
            watch: 0,
            expired: 0,
            executed: 0,
            chains: 0,
            protocols: 0,
            pools: 0,
            scan_interval_ms: 0,
            last_scan_ms: 0,
            engine_routes: 0,
            engine_candidates: 0,
            engine_requoted: 0,
            engine_reused: 0,
            engine_edges: 0,
            engine_tokens: 0,
            engine_visits: 0,
            engine_truncated: false,
            engine_full_rescan: false,
        }
    }
}
