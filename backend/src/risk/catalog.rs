//! 风控参数基线目录（由基线服务导出后固化，49 项，v1.0）。
//!
//! - 数值严格对齐《风控参数基线与熔断规则 v1.0》的初始值；
//! - 参数由后端强制校验，前端只做展示，不得据此放行；
//! - key 使用 snake_case，作为 API / DB / 前端契约。

use crate::domain::money::dec;
use crate::domain::risk::RiskParam;
use crate::domain::types::{ParamDir, RiskScope, RiskSeverity};

/// 参数基线版本。
pub const PARAM_VERSION: &str = "v1.0";

/// 参数分组（前端风控台按组展示）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ParamGroup {
    pub name: String,
    pub source: String,
    pub items: Vec<RiskParam>,
}

/// 熔断规则表（§10），供前端展示。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CircuitRule {
    pub trigger: String,
    pub scope: String,
    pub recovery: String,
}

/// 风控参数基线（49 项）。
pub fn baseline() -> Vec<RiskParam> {
    vec![
        RiskParam {
            key: "audit_capital_usd".into(),
            label: "审计资金基准".into(),
            group: "§3 资金与损失".into(),
            unit: "USDC".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("5000"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "所有收益、损失和回撤按实际权益计算，基准 5,000 USDC 等值".into(),
        },
        RiskParam {
            key: "live_single_trade_max_pct".into(),
            label: "实盘单笔净值上限（候选）".into(),
            group: "§3 资金与损失".into(),
            unit: "% 净值".into(),
            severity: RiskSeverity::PaperOnly,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("5"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "PAPER_ONLY：第一阶段仅作模拟评估，250 USDC 为未来实盘候选值，需审批开放".into(),
        },
        RiskParam {
            key: "token_exposure_max_pct".into(),
            label: "单 Token 合并敞口上限".into(),
            group: "§3 资金与损失".into(),
            unit: "% 净值".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Token,
            dir: ParamDir::Max,
            value: dec("10"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "账户净值 10%，5,000 基准即 500 USDC，HARD STOP".into(),
        },
        RiskParam {
            key: "venue_utilization_max_pct".into(),
            label: "单一场所资金使用率上限".into(),
            group: "§3 资金与损失".into(),
            unit: "%".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Exchange,
            dir: ParamDir::Max,
            value: dec("80"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "必须保留至少 20% 资金缓冲".into(),
        },
        RiskParam {
            key: "global_daily_loss_pct".into(),
            label: "全局日亏损上限".into(),
            group: "§3 资金与损失".into(),
            unit: "% 净值".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("1"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "账户净值 1%，5,000 基准即 50 USDC，触发全局暂停".into(),
        },
        RiskParam {
            key: "strategy_daily_loss_pct".into(),
            label: "单策略日亏损上限".into(),
            group: "§3 资金与损失".into(),
            unit: "% 净值".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Strategy,
            dir: ParamDir::Max,
            value: dec("0.5"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "账户净值 0.5%，5,000 基准即 25 USDC，暂停对应策略".into(),
        },
        RiskParam {
            key: "max_drawdown_pct".into(),
            label: "最大权益回撤".into(),
            group: "§3 资金与损失".into(),
            unit: "% 净值".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("5"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "峰值到当前权益，5% 暂停所有策略并重新评估".into(),
        },
        RiskParam {
            key: "chain_pending_tx_max".into(),
            label: "同链待确认交易上限".into(),
            group: "§3 资金与损失".into(),
            unit: "笔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("1"),
            min: None,
            max: None,
            source: "§3".into(),
            version: "v1.0".into(),
            note: "初期每个执行账户每条链最多 1 笔，防止 Nonce 与资金状态冲突".into(),
        },
        RiskParam {
            key: "dex_min_profit_usd".into(),
            label: "DEX 最低净利润（绝对）".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "USDC".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            // 风控基线 §4.1：max(2 USDC, 名义金额 × 10 bps)。不得为"产出可执行样本"下调。
            value: dec("2"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "最低净利润 max(2 USDC, 名义金额 × 10 bps)，不满足则拒绝".into(),
        },
        RiskParam {
            key: "dex_min_profit_bps".into(),
            label: "DEX 最低净利润（名义比例）".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "bps".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            // 与 dex_min_profit_usd 取 max，两者共同构成 §4.1 的最低净利润门槛。
            value: dec("10"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "按交易名义金额计算的最低净利润比例".into(),
        },
        RiskParam {
            key: "gas_profit_ratio_max_pct".into(),
            label: "Gas / 毛利润占比上限".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "%".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("25"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "Gas 占毛利润不超过 25%，超出 HARD STOP".into(),
        },
        RiskParam {
            key: "gas_abs_max_usd".into(),
            label: "绝对 Gas 费用上限".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "USDC".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("5"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "初始候选 5 USDC，按链单独校准".into(),
        },
        RiskParam {
            key: "dex_max_hops".into(),
            label: "单路线最大交易跳数".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "跳".into(),
            severity: RiskSeverity::SoftBlock,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("4"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "超过 4 跳需额外验证，当前机会拒绝".into(),
        },
        RiskParam {
            key: "dex_profit_discount_pct".into(),
            label: "预估利润保守折扣比例".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "%".into(),
            severity: RiskSeverity::PaperOnly,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("50"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "PAPER_ONLY：至少测试 50% 保守折扣场景，不将理论利润用于验收".into(),
        },
        RiskParam {
            key: "dex_price_impact_max_bps".into(),
            label: "DEX 全路线价格冲击上限".into(),
            group: "§4.1 DEX 原子套利".into(),
            unit: "bps".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("20"),
            min: None,
            max: None,
            source: "§4.1".into(),
            version: "v1.0".into(),
            note: "全路线初始上限 20 bps，超过则拒绝".into(),
        },
        RiskParam {
            key: "dex_quote_block_diff_max".into(),
            label: "报价区块差上限".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "区块".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("1"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "报价区块与最新区块差不超过 1 个区块，否则 HARD STOP".into(),
        },
        RiskParam {
            key: "dex_quote_max_age_ms".into(),
            label: "DEX 报价最大年龄".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "ms".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("8000"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "公共 RPC + M1 轮询下的实测预算（原基线 3000）；更严格需专用 RPC 或 M2".into(),
        },
        RiskParam {
            key: "rpc_latency_warn_ms".into(),
            label: "RPC P95 延迟预警阈值".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "ms".into(),
            severity: RiskSeverity::Warning,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("500"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "初始预警 500 ms，仅降级观察，不单独放行或阻断".into(),
        },
        RiskParam {
            key: "rpc_latency_hard_ms".into(),
            label: "RPC P95 延迟硬阈值".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "ms".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("1000"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "超过 1,000 ms 降级或暂停该链".into(),
        },
        RiskParam {
            key: "rpc_consecutive_fail_pause".into(),
            label: "连续 RPC 失败暂停阈值".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "次".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Chain,
            dir: ParamDir::Max,
            value: dec("3"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "连续 3 次失败暂停该链扫描".into(),
        },
        RiskParam {
            key: "plan_ttl_sec".into(),
            label: "交易计划有效期上限".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "秒".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Max,
            value: dec("30"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "初始不超过 30 秒且受区块约束，过期拒绝".into(),
        },
        RiskParam {
            key: "min_out_required".into(),
            label: "单腿最低输出必须显式指定".into(),
            group: "§5 DEX 数据与执行".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§5".into(),
            version: "v1.0".into(),
            note: "SW_HARD：布尔语义 1 = 必须显式指定 min_out，缺失即拒绝".into(),
        },
        RiskParam {
            key: "book_max_age_ms".into(),
            label: "实时盘口最大年龄".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "ms".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Exchange,
            dir: ParamDir::Max,
            value: dec("8000"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "公共行情轮询下的实测预算（原基线 250）；更严格需 WS 增量盘口".into(),
        },
        RiskParam {
            key: "book_depth_usage_max_pct".into(),
            label: "盘口深度占用上限".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "%".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Exchange,
            dir: ParamDir::Max,
            value: dec("10"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "不超过允许区间内可见深度的 10%，超过则缩量或拒绝".into(),
        },
        RiskParam {
            key: "clock_skew_warn_ms".into(),
            label: "本机与交易所时间差预警".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "ms".into(),
            severity: RiskSeverity::Warning,
            scope: RiskScope::Exchange,
            dir: ParamDir::Max,
            value: dec("250"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "初始预警 250 ms，超限校时或暂停".into(),
        },
        RiskParam {
            key: "cex_rate_limit_reserve_pct".into(),
            label: "API 速率余量下限".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "%".into(),
            severity: RiskSeverity::Warning,
            scope: RiskScope::Exchange,
            dir: ParamDir::Min,
            value: dec("30"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "保留至少 30% 余量，接近限流时降低请求频率".into(),
        },
        RiskParam {
            key: "fee_unknown_block".into(),
            label: "CEX 费率未知即拒绝".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Exchange,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "费率缺失不允许生成可执行机会，需重新同步费率".into(),
        },
        RiskParam {
            key: "book_snapshot_required".into(),
            label: "盘口 Snapshot 必须成功".into(),
            group: "§6 CEX 市场数据".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Exchange,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§6".into(),
            version: "v1.0".into(),
            note: "启动时必须成功取得 Snapshot，不完整则禁止扫描该交易对".into(),
        },
        RiskParam {
            key: "flash_provider_whitelist".into(),
            label: "闪贷 Provider 白名单必须存在".into(),
            group: "§8 闪电贷".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§8".into(),
            version: "v1.0".into(),
            note: "未登记 Provider 拒绝".into(),
        },
        RiskParam {
            key: "flash_fee_must_be_known".into(),
            label: "闪贷费用必须已知".into(),
            group: "§8 闪电贷".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§8".into(),
            version: "v1.0".into(),
            note: "读取真实回调或协议参数，不可信则拒绝，禁止硬编码".into(),
        },
        RiskParam {
            key: "flash_token_whitelist".into(),
            label: "闪贷 Token 白名单必须存在".into(),
            group: "§8 闪电贷".into(),
            unit: "布尔".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Global,
            dir: ParamDir::Min,
            value: dec("1"),
            min: None,
            max: None,
            source: "§8".into(),
            version: "v1.0".into(),
            note: "未登记资产拒绝".into(),
        },
        RiskParam {
            key: "stablecoin_warn_deviation_pct".into(),
            label: "稳定币偏离预警阈值".into(),
            group: "§9 Token 与稳定币".into(),
            unit: "%".into(),
            severity: RiskSeverity::Warning,
            scope: RiskScope::Token,
            dir: ParamDir::Max,
            value: dec("0.5"),
            min: None,
            max: None,
            source: "§9".into(),
            version: "v1.0".into(),
            note: "0.5% 预警，降低评分和规模".into(),
        },
        RiskParam {
            key: "stablecoin_halt_deviation_pct".into(),
            label: "稳定币偏离暂停阈值".into(),
            group: "§9 Token 与稳定币".into(),
            unit: "%".into(),
            severity: RiskSeverity::HardStop,
            scope: RiskScope::Token,
            dir: ParamDir::Max,
            value: dec("1.0"),
            min: None,
            max: None,
            source: "§9".into(),
            version: "v1.0".into(),
            note: "1.0% 暂停使用该稳定币作为估值基准".into(),
        },
    ]
}

/// 按 `group` 字段分组，保持参数原有顺序。
pub fn group_params(params: Vec<RiskParam>) -> Vec<ParamGroup> {
    let mut out: Vec<ParamGroup> = Vec::new();
    for p in params {
        match out.iter_mut().find(|g| g.name == p.group) {
            Some(g) => g.items.push(p),
            None => out.push(ParamGroup { name: p.group.clone(), source: p.source.clone(), items: vec![p] }),
        }
    }
    out
}

/// 分组后的基线。
pub fn groups() -> Vec<ParamGroup> {
    group_params(baseline())
}

/// 熔断规则（§10）。
pub fn circuit_rules() -> Vec<CircuitRule> {
    vec![
        CircuitRule {
            trigger: "全局日亏损达到账户净值 1%".into(),
            scope: "全部策略".into(),
            recovery: "重新核对账本、损失原因和风险配置".into(),
        },
        CircuitRule {
            trigger: "最大回撤达到账户净值 5%".into(),
            scope: "全局暂停".into(),
            recovery: "风险审查后重新启用".into(),
        },
        CircuitRule {
            trigger: "3 次 / 15 分钟 DEX 回滚".into(),
            scope: "相关链或策略".into(),
            recovery: "至少等待 30 分钟并检查回滚原因".into(),
        },
        CircuitRule {
            trigger: "CEX–DEX 连续 2 次对冲异常 / 10 次尝试".into(),
            scope: "相关交易对".into(),
            recovery: "完成订单、库存和账本核对".into(),
        },
        CircuitRule {
            trigger: "RPC 持续失联".into(),
            scope: "对应链".into(),
            recovery: "双 RPC 状态重新一致".into(),
        },
        CircuitRule {
            trigger: "CEX 盘口不完整".into(),
            scope: "对应交易对".into(),
            recovery: "Snapshot 与增量完整同步".into(),
        },
        CircuitRule {
            trigger: "订单状态未知".into(),
            scope: "对应账户和交易对".into(),
            recovery: "查询和对账确认最终订单状态".into(),
        },
        CircuitRule {
            trigger: "库存账本不一致".into(),
            scope: "对应策略".into(),
            recovery: "真实余额或 Paper 账本校准".into(),
        },
        CircuitRule {
            trigger: "Token / Pool / Adapter 风险".into(),
            scope: "对应资产或协议".into(),
            recovery: "重新完成风险审查".into(),
        },
        CircuitRule {
            trigger: "合约权限或代码哈希异常".into(),
            scope: "合约对应链".into(),
            recovery: "停止执行并完成安全检查".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(key: &str) -> RiskParam {
        baseline()
            .into_iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("缺少参数 {key}"))
    }

    /// 风控基线 §4.1：最低净利润 = max(2 USDC, 名义金额 × 10 bps)。
    ///
    /// 这两个值曾经被下调为 0 / 0.01 bps，以便在真实市场"产出可执行样本"。
    /// 那是把验收便利放在风控底线之上，本测试的作用就是让这种下调无法再悄悄发生：
    /// 改这两个值必须同时改文档与本测试，否则 CI 直接失败。
    #[test]
    fn baseline_profit_floor_matches_doc() {
        assert_eq!(find("dex_min_profit_usd").value, dec("2"));
        assert_eq!(find("dex_min_profit_bps").value, dec("10"));
    }

    /// 门槛参数必须是 HARD_STOP + Min 方向：只能收紧，不能被覆盖放宽。
    #[test]
    fn profit_floor_params_are_hard_stop_min() {
        for key in ["dex_min_profit_usd", "dex_min_profit_bps"] {
            let p = find(key);
            assert_eq!(p.severity, RiskSeverity::HardStop, "{key} 必须是 HARD_STOP");
            assert_eq!(p.dir, ParamDir::Min, "{key} 必须是 Min 方向");
        }
    }

    /// 目录里任何"下限型"参数都不得为 0：0 等价于该条规则不存在。
    #[test]
    fn no_floor_param_is_disabled() {
        for p in baseline() {
            if p.dir == ParamDir::Min {
                assert!(
                    p.value > rust_decimal::Decimal::ZERO,
                    "{} ({}) 的下限被设为 0，等于关闭该规则",
                    p.key,
                    p.label
                );
            }
        }
    }
}
