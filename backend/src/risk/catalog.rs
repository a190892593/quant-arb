//! 风控参数基线目录（v1.0）。
//!
//! 所有数值严格对齐《风控参数基线与熔断规则 v1.0》的初始值。
//! 参数由后端强制校验；前端只做展示，不得据此放行。
//!
//! - 参数 key 使用英文 snake_case，作为 API / DB / 前端契约，不得随意改名。
//! - `severity` 按文档语义：HARD_STOP / SOFT_BLOCK / WARNING / PAPER_ONLY。
//! - `dir` 表示该参数是上限（Max，越小越安全）还是下限（Min，越大越安全）。

use crate::domain::money::dec;
use crate::domain::risk::RiskParam;
use crate::domain::types::{ParamDir, RiskScope, RiskSeverity};

/// 参数基线版本。
pub const PARAM_VERSION: &str = "v1.0";

// ---- 参数分组（与文档 § 对应，前端风控台按组展示） ----
pub const GROUP_FUNDS: &str = "§3 资金与损失";
pub const GROUP_DEX_PROFIT: &str = "§4.1 DEX 原子套利";
pub const GROUP_CEX_DEX_PROFIT: &str = "§4.2 CEX–DEX 对冲";
pub const GROUP_DEX_DATA: &str = "§5 DEX 数据与执行";
pub const GROUP_CEX_DATA: &str = "§6 CEX 市场数据";
pub const GROUP_HEDGE: &str = "§7 双腿对冲执行";
pub const GROUP_FLASH: &str = "§8 闪电贷";
pub const GROUP_TOKEN: &str = "§9 Token 与稳定币";

/// 参数分组（前端风控台按组展示）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ParamGroup {
    pub name: String,
    pub source: String, // 例如 "§3"
    pub items: Vec<RiskParam>,
}

/// 熔断规则表（§10），供前端展示。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CircuitRule {
    pub trigger: String,
    pub scope: String,
    pub recovery: String,
}

#[allow(clippy::too_many_arguments)]
fn mk(
    key: &str,
    label: &str,
    group: &str,
    unit: &str,
    severity: RiskSeverity,
    scope: RiskScope,
    dir: ParamDir,
    value: &str,
    min: Option<&str>,
    max: Option<&str>,
    source: &str,
    note: &str,
) -> RiskParam {
    RiskParam {
        key: key.to_string(),
        label: label.to_string(),
        group: group.to_string(),
        unit: unit.to_string(),
        severity,
        scope,
        dir,
        value: dec(value),
        min: min.map(dec),
        max: max.map(dec),
        source: source.to_string(),
        version: PARAM_VERSION.to_string(),
        note: note.to_string(),
    }
}

/// 上限型参数（Max）：值越小越严格，允许区间 [0, +∞)。
#[allow(clippy::too_many_arguments)]
fn cap(
    key: &str,
    label: &str,
    group: &str,
    unit: &str,
    severity: RiskSeverity,
    scope: RiskScope,
    value: &str,
    source: &str,
    note: &str,
) -> RiskParam {
    mk(
        key,
        label,
        group,
        unit,
        severity,
        scope,
        ParamDir::Max,
        value,
        Some("0"),
        None,
        source,
        note,
    )
}

/// 下限型参数（Min）：值越大越严格。
#[allow(clippy::too_many_arguments)]
fn floor(
    key: &str,
    label: &str,
    group: &str,
    unit: &str,
    severity: RiskSeverity,
    scope: RiskScope,
    value: &str,
    source: &str,
    note: &str,
) -> RiskParam {
    mk(
        key,
        label,
        group,
        unit,
        severity,
        scope,
        ParamDir::Min,
        value,
        None,
        None,
        source,
        note,
    )
}

/// 布尔型硬约束：1 = 必须成立，0 = 不成立（始终拒绝）。
fn flag(
    key: &str,
    label: &str,
    group: &str,
    scope: RiskScope,
    source: &str,
    note: &str,
) -> RiskParam {
    mk(
        key,
        label,
        group,
        "布尔",
        RiskSeverity::HardStop,
        scope,
        ParamDir::Min,
        "1",
        Some("1"),
        Some("1"),
        source,
        note,
    )
}

/// 参数基线（v1.0）。所有数值来自《风控参数基线与熔断规则 v1.0》。
pub fn baseline() -> Vec<RiskParam> {
    vec![
        // ================= §3 资金与损失 =================
        mk(
            "audit_capital_usd",
            "审计资金基准",
            GROUP_FUNDS,
            "USDC",
            RiskSeverity::HardStop,
            RiskScope::Global,
            ParamDir::Min,
            "5000",
            None,
            None,
            "§3",
            "所有收益、损失和回撤按实际权益计算，基准 5,000 USDC 等值",
        ),
        cap(
            "live_single_trade_max_pct",
            "实盘单笔净值上限（候选）",
            GROUP_FUNDS,
            "% 净值",
            RiskSeverity::PaperOnly,
            RiskScope::Global,
            "5",
            "§3",
            "PAPER_ONLY：第一阶段仅作模拟评估，250 USDC 为未来实盘候选值，需审批开放",
        ),
        cap(
            "token_exposure_max_pct",
            "单 Token 合并敞口上限",
            GROUP_FUNDS,
            "% 净值",
            RiskSeverity::HardStop,
            RiskScope::Token,
            "10",
            "§3",
            "账户净值 10%，5,000 基准即 500 USDC，HARD STOP",
        ),
        cap(
            "venue_utilization_max_pct",
            "单一场所资金使用率上限",
            GROUP_FUNDS,
            "%",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "80",
            "§3",
            "必须保留至少 20% 资金缓冲",
        ),
        cap(
            "global_daily_loss_pct",
            "全局日亏损上限",
            GROUP_FUNDS,
            "% 净值",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "1",
            "§3",
            "账户净值 1%，5,000 基准即 50 USDC，触发全局暂停",
        ),
        cap(
            "strategy_daily_loss_pct",
            "单策略日亏损上限",
            GROUP_FUNDS,
            "% 净值",
            RiskSeverity::HardStop,
            RiskScope::Strategy,
            "0.5",
            "§3",
            "账户净值 0.5%，5,000 基准即 25 USDC，暂停对应策略",
        ),
        cap(
            "max_drawdown_pct",
            "最大权益回撤",
            GROUP_FUNDS,
            "% 净值",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "5",
            "§3",
            "峰值到当前权益，5% 暂停所有策略并重新评估",
        ),
        cap(
            "chain_pending_tx_max",
            "同链待确认交易上限",
            GROUP_FUNDS,
            "笔",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "1",
            "§3",
            "初期每个执行账户每条链最多 1 笔，防止 Nonce 与资金状态冲突",
        ),
        // ================= §4.1 DEX 原子套利 =================
        floor(
            "dex_min_profit_usd",
            "DEX 最低净利润（绝对）",
            GROUP_DEX_PROFIT,
            "USDC",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "2",
            "§4.1",
            "最低净利润 max(2 USDC, 名义金额 × 10 bps)，不满足则拒绝",
        ),
        floor(
            "dex_min_profit_bps",
            "DEX 最低净利润（名义比例）",
            GROUP_DEX_PROFIT,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "10",
            "§4.1",
            "按交易名义金额计算的最低净利润比例",
        ),
        cap(
            "gas_profit_ratio_max_pct",
            "Gas / 毛利润占比上限",
            GROUP_DEX_PROFIT,
            "%",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "25",
            "§4.1",
            "Gas 占毛利润不超过 25%，超出 HARD STOP",
        ),
        cap(
            "gas_abs_max_usd",
            "绝对 Gas 费用上限",
            GROUP_DEX_PROFIT,
            "USDC",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "5",
            "§4.1",
            "初始候选 5 USDC，按链单独校准",
        ),
        cap(
            "dex_max_hops",
            "单路线最大交易跳数",
            GROUP_DEX_PROFIT,
            "跳",
            RiskSeverity::SoftBlock,
            RiskScope::Global,
            "4",
            "§4.1",
            "超过 4 跳需额外验证，当前机会拒绝",
        ),
        floor(
            "dex_profit_discount_pct",
            "预估利润保守折扣比例",
            GROUP_DEX_PROFIT,
            "%",
            RiskSeverity::PaperOnly,
            RiskScope::Global,
            "50",
            "§4.1",
            "PAPER_ONLY：至少测试 50% 保守折扣场景，不将理论利润用于验收",
        ),
        cap(
            "dex_price_impact_max_bps",
            "DEX 全路线价格冲击上限",
            GROUP_DEX_PROFIT,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "20",
            "§4.1",
            "全路线初始上限 20 bps，超过则拒绝",
        ),
        // ================= §4.2 CEX–DEX 对冲 =================
        floor(
            "cex_dex_min_profit_usd",
            "CEX–DEX 最低净利润（绝对）",
            GROUP_CEX_DEX_PROFIT,
            "USDC",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "5",
            "§4.2",
            "最低保守净利润 max(5 USDC, 名义金额 × 30 bps)",
        ),
        floor(
            "cex_dex_min_profit_bps",
            "CEX–DEX 最低净利润（名义比例）",
            GROUP_CEX_DEX_PROFIT,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "30",
            "§4.2",
            "按交易名义金额计算的最低保守净利润比例",
        ),
        floor(
            "cex_depth_multiple",
            "CEX 可见深度倍数要求",
            GROUP_CEX_DEX_PROFIT,
            "倍",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "10",
            "§4.2",
            "允许价格区间内可见深度至少为订单金额的 10 倍",
        ),
        cap(
            "cex_depth_usage_max_pct",
            "CEX 盘口深度占用上限",
            GROUP_CEX_DEX_PROFIT,
            "%",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "10",
            "§4.2",
            "超过可见深度 10% 则拒绝或缩量",
        ),
        cap(
            "cex_price_protection_bps",
            "CEX 买卖保护价偏离上限",
            GROUP_CEX_DEX_PROFIT,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "10",
            "§4.2",
            "距参考 Bid/Ask 不超过 10 bps，超出则不下模拟订单",
        ),
        cap(
            "cex_dex_price_impact_max_bps",
            "DEX 价格冲击上限（CEX–DEX）",
            GROUP_CEX_DEX_PROFIT,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "20",
            "§4.2",
            "全路线初始上限 20 bps，超过则拒绝",
        ),
        floor(
            "cex_dex_discount_pct",
            "预估收益保守折扣比例",
            GROUP_CEX_DEX_PROFIT,
            "%",
            RiskSeverity::PaperOnly,
            RiskScope::Global,
            "50",
            "§4.2",
            "PAPER_ONLY：必须测试 50% 保守折扣场景，仅作压力分析",
        ),
        // ================= §5 DEX 数据与执行 =================
        cap(
            "dex_quote_block_diff_max",
            "报价区块差上限",
            GROUP_DEX_DATA,
            "区块",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "1",
            "§5",
            "报价区块与最新区块差不超过 1 个区块，否则 HARD STOP",
        ),
        cap(
            "dex_quote_max_age_ms",
            "DEX 报价最大年龄",
            GROUP_DEX_DATA,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "3000",
            "§5",
            "min(2 × 链平均区块时间, 3 秒)，超时须重新报价",
        ),
        cap(
            "rpc_latency_warn_ms",
            "RPC P95 延迟预警阈值",
            GROUP_DEX_DATA,
            "ms",
            RiskSeverity::Warning,
            RiskScope::Chain,
            "500",
            "§5",
            "初始预警 500 ms，仅降级观察，不单独放行或阻断",
        ),
        cap(
            "rpc_latency_hard_ms",
            "RPC P95 延迟硬阈值",
            GROUP_DEX_DATA,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "1000",
            "§5",
            "超过 1,000 ms 降级或暂停该链",
        ),
        cap(
            "rpc_consecutive_fail_pause",
            "连续 RPC 失败暂停阈值",
            GROUP_DEX_DATA,
            "次",
            RiskSeverity::HardStop,
            RiskScope::Chain,
            "3",
            "§5",
            "连续 3 次失败暂停该链扫描",
        ),
        cap(
            "plan_ttl_sec",
            "交易计划有效期上限",
            GROUP_DEX_DATA,
            "秒",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "30",
            "§5",
            "初始不超过 30 秒且受区块约束，过期拒绝",
        ),
        flag(
            "min_out_required",
            "单腿最低输出必须显式指定",
            GROUP_DEX_DATA,
            RiskScope::Global,
            "§5",
            "SW_HARD：布尔语义 1 = 必须显式指定 min_out，缺失即拒绝",
        ),
        // ================= §6 CEX 市场数据 =================
        cap(
            "book_max_age_ms",
            "实时盘口最大年龄",
            GROUP_CEX_DATA,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "250",
            "§6",
            "超过 250 ms 拒绝新机会",
        ),
        cap(
            "book_depth_usage_max_pct",
            "盘口深度占用上限",
            GROUP_CEX_DATA,
            "%",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "10",
            "§6",
            "不超过允许区间内可见深度的 10%，超过则缩量或拒绝",
        ),
        cap(
            "clock_skew_warn_ms",
            "本机与交易所时间差预警",
            GROUP_CEX_DATA,
            "ms",
            RiskSeverity::Warning,
            RiskScope::Exchange,
            "250",
            "§6",
            "初始预警 250 ms，超限校时或暂停",
        ),
        floor(
            "cex_rate_limit_reserve_pct",
            "API 速率余量下限",
            GROUP_CEX_DATA,
            "%",
            RiskSeverity::Warning,
            RiskScope::Exchange,
            "30",
            "§6",
            "保留至少 30% 余量，接近限流时降低请求频率",
        ),
        flag(
            "fee_unknown_block",
            "CEX 费率未知即拒绝",
            GROUP_CEX_DATA,
            RiskScope::Exchange,
            "§6",
            "费率缺失不允许生成可执行机会，需重新同步费率",
        ),
        flag(
            "book_snapshot_required",
            "盘口 Snapshot 必须成功",
            GROUP_CEX_DATA,
            RiskScope::Exchange,
            "§6",
            "启动时必须成功取得 Snapshot，不完整则禁止扫描该交易对",
        ),
        // ================= §7 双腿对冲执行 =================
        cap(
            "unhedged_exposure_max_pct",
            "未对冲敞口上限",
            GROUP_HEDGE,
            "% 净值",
            RiskSeverity::HardStop,
            RiskScope::Global,
            "0.5",
            "§7",
            "账户净值 0.5%，5,000 基准即 25 USDC，HARD STOP 并触发补偿",
        ),
        cap(
            "max_unhedged_ms",
            "最大未对冲时间",
            GROUP_HEDGE,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "2000",
            "§7",
            "超过 2 秒触发补偿并暂停对应交易对",
        ),
        cap(
            "hedge_deadline_ms",
            "对冲完成时限",
            GROUP_HEDGE,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "2000",
            "§7",
            "第一条腿完成后 2 秒内必须完成对冲",
        ),
        cap(
            "order_timeout_ms",
            "订单请求响应超时",
            GROUP_HEDGE,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "1000",
            "§7",
            "初始 1 秒，超时标记状态未知，不盲目重发",
        ),
        cap(
            "order_unknown_window_ms",
            "订单状态未知查询窗口",
            GROUP_HEDGE,
            "ms",
            RiskSeverity::HardStop,
            RiskScope::Exchange,
            "2000",
            "§7",
            "先通过订单 ID 查询和对账，确认最终状态",
        ),
        cap(
            "requote_max",
            "自动重新报价次数上限",
            GROUP_HEDGE,
            "次",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "1",
            "§7",
            "最多重新报价 1 次，再失败则拒绝",
        ),
        cap(
            "max_price_deviation_bps",
            "最大价格偏离",
            GROUP_HEDGE,
            "bps",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "10",
            "§7",
            "超限拒绝或停止",
        ),
        cap(
            "hedge_anomaly_limit",
            "连续对冲异常次数上限",
            GROUP_HEDGE,
            "次",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "2",
            "§7",
            "最近 10 次尝试中 2 次对冲异常即暂停该交易对",
        ),
        cap(
            "hedge_anomaly_window",
            "连续对冲异常统计窗口",
            GROUP_HEDGE,
            "次尝试",
            RiskSeverity::HardStop,
            RiskScope::Symbol,
            "10",
            "§7",
            "以最近 10 次尝试为统计窗口",
        ),
        // ================= §8 闪电贷 =================
        flag(
            "flash_provider_whitelist",
            "闪贷 Provider 白名单必须存在",
            GROUP_FLASH,
            RiskScope::Global,
            "§8",
            "未登记 Provider 拒绝",
        ),
        flag(
            "flash_fee_must_be_known",
            "闪贷费用必须已知",
            GROUP_FLASH,
            RiskScope::Global,
            "§8",
            "读取真实回调或协议参数，不可信则拒绝，禁止硬编码",
        ),
        flag(
            "flash_token_whitelist",
            "闪贷 Token 白名单必须存在",
            GROUP_FLASH,
            RiskScope::Global,
            "§8",
            "未登记资产拒绝",
        ),
        // ================= §9 Token 与稳定币 =================
        cap(
            "stablecoin_warn_deviation_pct",
            "稳定币偏离预警阈值",
            GROUP_TOKEN,
            "%",
            RiskSeverity::Warning,
            RiskScope::Token,
            "0.5",
            "§9",
            "0.5% 预警，降低评分和规模",
        ),
        cap(
            "stablecoin_halt_deviation_pct",
            "稳定币偏离暂停阈值",
            GROUP_TOKEN,
            "%",
            RiskSeverity::HardStop,
            RiskScope::Token,
            "1.0",
            "§9",
            "1.0% 暂停使用该稳定币作为估值基准",
        ),
    ]
}

/// 按 §3–§9 分组。
pub fn groups() -> Vec<ParamGroup> {
    group_params(baseline())
}

/// 将任意参数集合按 `group` 字段分组，保持参数原有顺序。
pub fn group_params(params: Vec<RiskParam>) -> Vec<ParamGroup> {
    let mut out: Vec<ParamGroup> = Vec::new();
    for p in params {
        match out.iter_mut().find(|g| g.name == p.group) {
            Some(g) => g.items.push(p),
            None => out.push(ParamGroup {
                name: p.group.clone(),
                source: p.source.clone(),
                items: vec![p],
            }),
        }
    }
    out
}

/// 熔断规则表（§10），供前端展示。
pub fn circuit_rules() -> Vec<CircuitRule> {
    let r = |trigger: &str, scope: &str, recovery: &str| CircuitRule {
        trigger: trigger.to_string(),
        scope: scope.to_string(),
        recovery: recovery.to_string(),
    };
    vec![
        r(
            "全局日亏损达到账户净值 1%",
            "全部策略",
            "重新核对账本、损失原因和风险配置",
        ),
        r(
            "最大回撤达到账户净值 5%",
            "全局暂停",
            "风险审查后重新启用",
        ),
        r(
            "3 次 / 15 分钟 DEX 回滚",
            "相关链或策略",
            "至少等待 30 分钟并检查回滚原因",
        ),
        r(
            "CEX–DEX 连续 2 次对冲异常 / 10 次尝试",
            "相关交易对",
            "完成订单、库存和账本核对",
        ),
        r(
            "RPC 持续失联",
            "对应链",
            "双 RPC 状态重新一致",
        ),
        r(
            "CEX 盘口不完整",
            "对应交易对",
            "Snapshot 与增量完整同步",
        ),
        r(
            "订单状态未知",
            "对应账户和交易对",
            "查询和对账确认最终订单状态",
        ),
        r(
            "库存账本不一致",
            "对应策略",
            "真实余额或 Paper 账本校准",
        ),
        r(
            "Token / Pool / Adapter 风险",
            "对应资产或协议",
            "重新完成风险审查",
        ),
        r(
            "合约权限或代码哈希异常",
            "合约对应链",
            "停止执行并完成安全检查",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::collections::BTreeSet;

    #[test]
    fn baseline_has_enough_params() {
        let params = baseline();
        assert!(
            params.len() >= 40,
            "参数目录至少 40 条，实际 {}",
            params.len()
        );
    }

    #[test]
    fn keys_are_unique() {
        let params = baseline();
        let mut keys = BTreeSet::new();
        for p in &params {
            assert!(keys.insert(p.key.clone()), "参数 key 重复: {}", p.key);
        }
    }

    #[test]
    fn all_values_positive_and_metadata_complete() {
        for p in baseline() {
            assert!(p.value > Decimal::ZERO, "{} 的值必须 > 0", p.key);
            assert!(!p.label.is_empty(), "{} 缺少 label", p.key);
            assert!(!p.group.is_empty(), "{} 缺少 group", p.key);
            assert!(!p.unit.is_empty(), "{} 缺少 unit", p.key);
            assert!(!p.source.is_empty(), "{} 缺少 source", p.key);
            assert!(!p.note.is_empty(), "{} 缺少 note", p.key);
            assert_eq!(p.version, PARAM_VERSION);
        }
    }

    #[test]
    fn required_baseline_values_match_doc() {
        let find = |key: &str| {
            baseline()
                .into_iter()
                .find(|p| p.key == key)
                .unwrap_or_else(|| panic!("缺少参数 {key}"))
        };
        assert_eq!(find("dex_min_profit_usd").value, dec("2"));
        assert_eq!(find("cex_dex_min_profit_bps").value, dec("30"));
        assert_eq!(find("audit_capital_usd").value, dec("5000"));
        assert_eq!(find("live_single_trade_max_pct").value, dec("5"));
        assert_eq!(find("global_daily_loss_pct").value, dec("1"));
        assert_eq!(find("strategy_daily_loss_pct").value, dec("0.5"));
        assert_eq!(find("max_drawdown_pct").value, dec("5"));
        assert_eq!(find("gas_abs_max_usd").value, dec("5"));
        assert_eq!(find("cex_depth_multiple").value, dec("10"));
        assert_eq!(find("stablecoin_halt_deviation_pct").value, dec("1.0"));
        assert_eq!(
            find("live_single_trade_max_pct").severity,
            RiskSeverity::PaperOnly
        );
        assert_eq!(
            find("rpc_latency_warn_ms").severity,
            RiskSeverity::Warning
        );
    }

    #[test]
    fn groups_cover_all_params() {
        let groups = groups();
        let total: usize = groups.iter().map(|g| g.items.len()).sum();
        assert_eq!(total, baseline().len());
        assert!(groups.len() >= 8, "至少 8 个分组，实际 {}", groups.len());
        let names: BTreeSet<_> = groups.iter().map(|g| g.source.clone()).collect();
        for s in ["§3", "§4.1", "§4.2", "§5", "§6", "§7", "§8", "§9"] {
            assert!(names.contains(s), "缺少分组 {s}");
        }
    }

    #[test]
    fn circuit_rules_cover_section_10() {
        let rules = circuit_rules();
        assert!(rules.len() >= 10);
        assert!(rules.iter().all(|r| !r.trigger.is_empty()));
        assert!(rules.iter().all(|r| !r.scope.is_empty()));
        assert!(rules.iter().all(|r| !r.recovery.is_empty()));
    }
}
