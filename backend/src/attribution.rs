//! 失败归因（PRD v2.2 §11.3，任务 V22-105 / V22-307）。
//!
//! 12 类归因只回答"这一步为什么没成"，用于决定下一轮优化方向（速度 / 数据 / 模拟 / 竞价）。
//! 分类只基于**已有事实**（拒绝 key、执行状态、revert 原因），不做猜测；无法归类时给 UNKNOWN。

pub const DISCOVERY_TOO_SLOW: &str = "DISCOVERY_TOO_SLOW";
pub const QUOTE_TOO_SLOW: &str = "QUOTE_TOO_SLOW";
pub const M2_TOO_SLOW: &str = "M2_TOO_SLOW";
pub const STALE_STATE: &str = "STALE_STATE";
pub const COMPETITOR_FILLED_FIRST: &str = "COMPETITOR_FILLED_FIRST";
pub const GAS_COST_TOO_HIGH: &str = "GAS_COST_TOO_HIGH";
pub const PRIORITY_FEE_TOO_HIGH: &str = "PRIORITY_FEE_TOO_HIGH";
pub const PROFIT_BELOW_THRESHOLD: &str = "PROFIT_BELOW_THRESHOLD";
pub const LIQUIDITY_INSUFFICIENT: &str = "LIQUIDITY_INSUFFICIENT";
pub const SIMULATION_REVERTED: &str = "SIMULATION_REVERTED";
pub const DATA_INCOMPLETE: &str = "DATA_INCOMPLETE";
pub const UNKNOWN: &str = "UNKNOWN";

/// 全部 12 类（用于报告与前端枚举）。
pub const ALL: [&str; 12] = [
    DISCOVERY_TOO_SLOW,
    QUOTE_TOO_SLOW,
    M2_TOO_SLOW,
    STALE_STATE,
    COMPETITOR_FILLED_FIRST,
    GAS_COST_TOO_HIGH,
    PRIORITY_FEE_TOO_HIGH,
    PROFIT_BELOW_THRESHOLD,
    LIQUIDITY_INSUFFICIENT,
    SIMULATION_REVERTED,
    DATA_INCOMPLETE,
    UNKNOWN,
];

/// 机会拒绝原因 key → 归因类别。
pub fn classify_rejection(key: &str) -> &'static str {
    let k = key.to_ascii_lowercase();
    if k.contains("min_profit") {
        return PROFIT_BELOW_THRESHOLD;
    }
    if k.contains("priority") {
        return PRIORITY_FEE_TOO_HIGH;
    }
    if k.contains("gas") {
        return GAS_COST_TOO_HIGH;
    }
    if k.contains("m2") {
        return SIMULATION_REVERTED;
    }
    if k.contains("quote") && k.contains("age") {
        return QUOTE_TOO_SLOW;
    }
    if k.contains("age") || k.contains("stale") || k.contains("block") || k.contains("state") {
        return STALE_STATE;
    }
    if k.contains("liquidity") || k.contains("capacity") || k.contains("depth") {
        return LIQUIDITY_INSUFFICIENT;
    }
    if k.contains("data_quality") || k.contains("missing") {
        return DATA_INCOMPLETE;
    }
    UNKNOWN
}

/// 执行状态 + revert 原因 → 归因类别。
pub fn classify_execution(state: &str, revert: Option<&str>) -> &'static str {
    let s = state.to_ascii_lowercase();
    let r = revert.unwrap_or("").to_ascii_lowercase();
    if s.contains("revert") {
        if r.contains("slippage")
            || r.contains("too little")
            || r.contains("insufficient output")
            || r.contains("insufficient balance")
            || r.contains("transfer_from")
        {
            return LIQUIDITY_INSUFFICIENT;
        }
        return SIMULATION_REVERTED;
    }
    if s.contains("expired") {
        return STALE_STATE;
    }
    if s.contains("insufficient") {
        return LIQUIDITY_INSUFFICIENT;
    }
    UNKNOWN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profit_rejection_maps_to_threshold() {
        assert_eq!(
            classify_rejection("min_profit"),
            PROFIT_BELOW_THRESHOLD
        );
    }

    /// 报价年龄属于「报价太慢」，与「状态过期」是不同归因（PRD §11.3）。
    #[test]
    fn quote_age_maps_to_quote_too_slow() {
        assert_eq!(classify_rejection("quote_age"), QUOTE_TOO_SLOW);
    }

    #[test]
    fn stale_state_keys_map_to_stale() {
        for key in ["state_snapshot", "pool_state_age", "quote_block_diff"] {
            assert_eq!(classify_rejection(key), STALE_STATE, "{key}");
        }
    }

    #[test]
    fn revert_with_slippage_maps_to_liquidity() {
        assert_eq!(
            classify_execution("DEX_REVERTED", Some("Too little received")),
            LIQUIDITY_INSUFFICIENT
        );
        assert_eq!(
            classify_execution("DEX_REVERTED", Some("panic: overflow")),
            SIMULATION_REVERTED
        );
    }

    #[test]
    fn unknown_when_no_fact() {
        assert_eq!(classify_execution("FILLED", None), UNKNOWN);
        assert_eq!(classify_rejection("some_new_rule"), UNKNOWN);
    }

    #[test]
    fn all_has_twelve_distinct_classes() {
        let mut v = ALL.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), 12);
    }
}
