//! 期望收益（EV）决策器（PRD v2.2 §12.3–§12.4，任务 V22-605）。
//!
//! 只做研究与模拟：输入全部来自历史统计或显式假设，本模块**不发送真实交易**。
//!
//! ```text
//! 期望净收益 = P(成功)×成功净利 − P(失败)×失败成本 − 额外竞价成本
//! ```
//!
//! 关键约束（PRD §12.3）：
//! - `P(成功)` 必须来自历史统计或标注为假设，缺失即 `INSUFFICIENT_DATA`；
//! - 失败成本包含回滚 Gas，不得省略；
//! - 高竞争机会不因毛利润高就自动参与。

use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const COMPETE: &str = "COMPETE";
pub const WATCH: &str = "WATCH";
pub const SKIP: &str = "SKIP";
pub const INSUFFICIENT_DATA: &str = "INSUFFICIENT_DATA";

/// 竞价模拟输入。
#[derive(Clone, Debug, Deserialize)]
pub struct BidInput {
    /// 历史成功概率（0–1）。缺失表示数据不足。
    pub p_success: Option<f64>,
    /// 历史失败概率（0–1）。缺失表示数据不足。
    pub p_fail: Option<f64>,
    /// 成功时的净利（USDC）。
    pub success_net: Decimal,
    /// 失败时的成本（USDC，含回滚 Gas）。
    pub failure_cost: Decimal,
    /// 额外竞价成本（USDC）。
    pub extra_bid_cost: Decimal,
    /// 最低期望净收益门槛（USDC）。
    pub min_expected: Decimal,
    /// 样本量；0 视为不足。
    pub samples: u64,
    /// 竞争等级（LOW / MEDIUM / HIGH）；缺省按 MEDIUM 处理（V23-601）。
    #[serde(default)]
    pub tier: Option<String>,
    /// `P` 是否来自真实历史统计（false 表示只是假设）。
    #[serde(default)]
    pub p_from_history: bool,
}

/// 竞价决策结果。
#[derive(Clone, Debug, Serialize)]
pub struct BidDecision {
    pub decision: String,
    pub expected_net: Option<Decimal>,
    pub p_success: Option<f64>,
    pub p_fail: Option<f64>,
    /// `history` / `assumption` / `missing`
    pub p_source: String,
    /// 实际采用的竞争等级（LOW / MEDIUM / HIGH）。
    pub tier: String,
    pub reason: String,
}

/// 三档竞价策略（V23-601）：不同竞争等级使用不同的竞价成本与参与门槛。
#[derive(Clone, Copy, Debug)]
pub struct TierPolicy {
    /// 竞价成本倍数（高竞争更贵）。
    pub bid_cost_factor: f64,
    /// 参与门槛倍数（高竞争要求更高 EV）。
    pub min_expected_factor: f64,
}

/// 按竞争等级取策略；未知等级按 MEDIUM 处理。
pub fn policy_for(tier: &str) -> TierPolicy {
    match tier.to_ascii_uppercase().as_str() {
        "LOW" => TierPolicy {
            bid_cost_factor: 1.0,
            min_expected_factor: 1.0,
        },
        "HIGH" => TierPolicy {
            bid_cost_factor: 2.5,
            min_expected_factor: 2.0,
        },
        _ => TierPolicy {
            bid_cost_factor: 1.5,
            min_expected_factor: 1.2,
        },
    }
}

fn clamp01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// 计算期望净收益并给出决策。
pub fn decide(input: &BidInput) -> BidDecision {
    let tier = input
        .tier
        .as_deref()
        .unwrap_or("MEDIUM")
        .to_ascii_uppercase();
    let policy = policy_for(&tier);
    let bid_cost = (input.extra_bid_cost
        * Decimal::from_f64_retain(policy.bid_cost_factor).unwrap_or(Decimal::ONE))
    .round_dp(6);
    let min_expected = (input.min_expected
        * Decimal::from_f64_retain(policy.min_expected_factor).unwrap_or(Decimal::ONE))
    .round_dp(6);
    let p_source = if input.p_from_history {
        "history"
    } else {
        "assumption"
    };
    let (Some(ps_raw), Some(pf_raw)) = (input.p_success, input.p_fail) else {
        return BidDecision {
            decision: INSUFFICIENT_DATA.into(),
            expected_net: None,
            p_success: input.p_success,
            p_fail: input.p_fail,
            p_source: "missing".into(),
            tier: tier.clone(),
            reason: "缺少 P(成功)/P(失败)：必须先有历史统计或显式假设".into(),
        };
    };
    if input.samples == 0 {
        return BidDecision {
            decision: INSUFFICIENT_DATA.into(),
            expected_net: None,
            p_success: Some(ps_raw),
            p_fail: Some(pf_raw),
            p_source: p_source.into(),
            tier: tier.clone(),
            reason: "样本量为 0：不外推，按 PRD §14.5 给 INSUFFICIENT_DATA".into(),
        };
    }
    let ps = clamp01(ps_raw);
    let pf = clamp01(pf_raw);
    if ps + pf > 1.0 + 1e-6 {
        return BidDecision {
            decision: INSUFFICIENT_DATA.into(),
            expected_net: None,
            p_success: Some(ps),
            p_fail: Some(pf),
            p_source: p_source.into(),
            tier: tier.clone(),
            reason: "P(成功)+P(失败) > 1，输入不自洽".into(),
        };
    }
    let ps_dec = Decimal::from_f64_retain(ps).unwrap_or(Decimal::ZERO);
    let pf_dec = Decimal::from_f64_retain(pf).unwrap_or(Decimal::ZERO);
    // f64 概率转 Decimal 会带二进制尾差，统一量化到 6 位小数（USDC 展示口径）。
    let expected = (ps_dec * input.success_net
        - pf_dec * input.failure_cost
        - bid_cost)
        .round_dp(6);

    let (decision, reason) = if expected <= Decimal::ZERO {
        (
            SKIP,
            format!("期望净收益 {expected} ≤ 0：参与即负期望"),
        )
    } else if expected >= min_expected {
        (
            COMPETE,
            format!("期望净收益 {expected} ≥ 门槛 {min_expected}（{tier} 档）"),
        )
    } else {
        (
            WATCH,
            format!("期望净收益 {expected} > 0 但低于门槛 {min_expected}（{tier} 档）"),
        )
    };
    BidDecision {
        decision: decision.into(),
        expected_net: Some(expected),
        p_success: Some(ps),
        p_fail: Some(pf),
        p_source: p_source.into(),
        tier,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> BidInput {
        BidInput {
            p_success: Some(0.6),
            p_fail: Some(0.4),
            success_net: Decimal::from(20),
            failure_cost: Decimal::from(3),
            extra_bid_cost: Decimal::from(1),
            min_expected: Decimal::from(5),
            samples: 100,
            p_from_history: true,
            // 既有测试按低竞争档（成本/门槛系数 1.0）验证 EV 公式本身。
            tier: Some("LOW".into()),
        }
    }

    #[test]
    fn missing_probability_is_insufficient() {
        let mut i = base();
        i.p_success = None;
        assert_eq!(decide(&i).decision, INSUFFICIENT_DATA);
    }

    #[test]
    fn zero_samples_is_insufficient() {
        let mut i = base();
        i.samples = 0;
        assert_eq!(decide(&i).decision, INSUFFICIENT_DATA);
    }

    #[test]
    fn negative_ev_skips() {
        let mut i = base();
        i.success_net = Decimal::from(1);
        // 0.6*1 - 0.4*3 - 1 = -1.6
        let d = decide(&i);
        assert_eq!(d.decision, SKIP);
    }

    #[test]
    fn above_threshold_competes() {
        let d = decide(&base());
        // 0.6*20 - 0.4*3 - 1 = 12 - 1.2 - 1 = 9.8 ≥ 5
        assert_eq!(d.decision, COMPETE);
        assert_eq!(d.expected_net, Some(Decimal::new(98, 1)));
    }

    #[test]
    fn between_zero_and_threshold_watches() {
        let mut i = base();
        i.min_expected = Decimal::from(50);
        assert_eq!(decide(&i).decision, WATCH);
    }

    #[test]
    fn inconsistent_probabilities_are_rejected() {
        let mut i = base();
        i.p_success = Some(0.9);
        i.p_fail = Some(0.9);
        assert_eq!(decide(&i).decision, INSUFFICIENT_DATA);
    }

    #[test]
    fn low_tier_competes_where_high_tier_watches() {
        // 基础 EV = 0.6*20 − 0.4*3 = 10.8
        // LOW：成本 1 → 9.8，门槛 5 → COMPETE
        // HIGH：成本 2.5 → 8.3，门槛 5×2=10 → WATCH
        let mut i = base();
        i.tier = Some("LOW".into());
        assert_eq!(decide(&i).decision, COMPETE);
        i.tier = Some("HIGH".into());
        let high = decide(&i);
        assert_eq!(high.decision, WATCH);
        assert_eq!(high.tier, "HIGH");
    }

    #[test]
    fn missing_tier_defaults_to_medium() {
        let mut i = base();
        i.tier = None;
        assert_eq!(decide(&i).tier, "MEDIUM");
    }
}
