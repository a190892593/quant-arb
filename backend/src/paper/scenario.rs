//! Paper 撮合场景参数：把「现实中会出问题的地方」显式建模。
//!
//! 这些概率不是装饰：每个分支都会产出真实的执行状态、成本与账本流水。
//! 默认值来自 PRD/TDD 对第一阶段的要求（部分成交、回滚、状态未知、补偿失败）。

use serde::{Deserialize, Serialize};

/// 确定性 RNG（SplitMix64）。测试与演示都需要可复现。
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [0,1) 均匀分布。
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// 概率判定。
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && self.next_f64() < p
    }

    /// 区间内均匀取值。
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_f64() * (hi - lo)
    }

    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo + 1))
    }
}

/// 模拟场景参数（概率与区间）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    /// DEX 交易失败（最低输出不满足 / 被抢跑）的概率。
    pub dex_revert_probability: f64,
    /// CEX IOC 部分成交概率。
    pub partial_fill_probability: f64,
    /// 部分成交的成交比例区间。
    pub partial_fill_band: (f64, f64),
    /// 订单响应超时（状态未知）概率。
    pub timeout_probability: f64,
    /// 状态未知最终确认为「未成交」的概率（其余为已成交）。
    pub unknown_resolve_cancel_probability: f64,
    /// 补偿单再次失败的概率。
    pub unwind_fail_probability: f64,
    /// DEX 实际成交相对报价的滑点区间（bps）。
    pub dex_slippage_bps: (f64, f64),
    /// CEX 腿确认延迟区间（毫秒）。
    pub cex_latency_ms: (u64, u64),
    /// DEX 腿确认延迟区间（毫秒）。
    pub dex_latency_ms: (u64, u64),
    /// 无机会时是否允许空转（仅用于演示）。
    pub allow_idle: bool,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            dex_revert_probability: 0.06,
            partial_fill_probability: 0.12,
            partial_fill_band: (0.35, 0.85),
            timeout_probability: 0.04,
            unknown_resolve_cancel_probability: 0.4,
            unwind_fail_probability: 0.04,
            dex_slippage_bps: (0.0, 8.0),
            cex_latency_ms: (40, 180),
            dex_latency_ms: (900, 2_600),
            allow_idle: true,
        }
    }
}

impl Scenario {
    /// 乐观场景：全部成交、无回滚（用于回归测试正常路径）。
    pub fn optimistic() -> Self {
        Self {
            dex_revert_probability: 0.0,
            partial_fill_probability: 0.0,
            partial_fill_band: (1.0, 1.0),
            timeout_probability: 0.0,
            unknown_resolve_cancel_probability: 0.0,
            unwind_fail_probability: 0.0,
            dex_slippage_bps: (0.0, 0.0),
            cex_latency_ms: (30, 60),
            dex_latency_ms: (800, 1_200),
            allow_idle: true,
        }
    }

    /// 压力场景：高回滚、高部分成交（审计 M3 压力模型）。
    pub fn stress() -> Self {
        Self {
            dex_revert_probability: 0.25,
            partial_fill_probability: 0.35,
            partial_fill_band: (0.2, 0.6),
            timeout_probability: 0.12,
            unknown_resolve_cancel_probability: 0.5,
            unwind_fail_probability: 0.15,
            dex_slippage_bps: (5.0, 30.0),
            cex_latency_ms: (150, 600),
            dex_latency_ms: (1_500, 4_000),
            allow_idle: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::new(43);
        assert_ne!(Rng::new(42).next_u64(), c.next_u64());
    }

    #[test]
    fn chance_bounds() {
        let mut r = Rng::new(7);
        assert!(!r.chance(0.0));
        assert!(r.chance(1.0));
        for _ in 0..1000 {
            let v = r.next_f64();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn optimistic_never_reverts() {
        let mut r = Rng::new(1);
        let s = Scenario::optimistic();
        for _ in 0..10_000 {
            assert!(!r.chance(s.dex_revert_probability));
            assert!(!r.chance(s.partial_fill_probability));
        }
    }

    #[test]
    fn range_within_bounds() {
        let mut r = Rng::new(9);
        for _ in 0..100 {
            let v = r.range(10.0, 20.0);
            assert!((10.0..20.0).contains(&v));
        }
        assert_eq!(r.range(5.0, 5.0), 5.0);
    }
}
