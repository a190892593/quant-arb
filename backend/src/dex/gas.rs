//! Gas 估算（PRD §5.6）。
//!
//! 不使用「固定次数 × 固定费用」。按路线逐项累计：
//! - 每跳 AMM 交换（按 AMM 类型区分）；
//! - 每跳 Token 转账；
//! - Executor 基础开销与每跳逻辑；
//! - 闪电贷借款/还款；
//! - Approve / Permit。
//!
//! 估算值必须带 Gas 价格、原生资产价格、区块号与时间戳，便于事后复算误差。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::dex_scan::{GasComponent, GasEstimate};
use crate::domain::market::PoolKind;

/// Gas 参数（单位：gas units）。
#[derive(Clone, Debug)]
pub struct GasParams {
    pub executor_base: u64,
    pub executor_per_hop: u64,
    pub transfer_per_hop: u64,
    pub flash_loan: u64,
    pub approve: u64,
}

impl Default for GasParams {
    fn default() -> Self {
        Self {
            executor_base: 45_000,
            executor_per_hop: 12_000,
            transfer_per_hop: 30_000,
            flash_loan: 90_000,
            approve: 50_000,
        }
    }
}

/// 单次交换的 AMM Gas 基线（与 [crate::dex::pool::PoolRuntime::gas_units] 同源）。
pub fn amm_swap_units(kind: PoolKind) -> u64 {
    match kind {
        PoolKind::UniswapV2 => 90_000,
        PoolKind::UniswapV3 => 130_000,
        PoolKind::StableSwap => 150_000,
        PoolKind::WeightedPool => 180_000,
        PoolKind::Unknown => 0,
    }
}

/// 估算一条路线的 Gas。approvals 为需要单独授权且无 permit 的 Token 数。
pub fn estimate(
    kinds: &[PoolKind],
    flash: bool,
    approvals: usize,
    params: &GasParams,
    gas_price_gwei: Decimal,
    native_price_usd: Decimal,
    block_number: u64,
) -> GasEstimate {
    let mut components = Vec::new();
    let mut push = |key: &str, label: &str, units: u64| {
        if units > 0 {
            components.push(GasComponent {
                key: key.to_string(),
                label: label.to_string(),
                units,
            });
        }
    };

    let mut amm = 0u64;
    for kind in kinds {
        amm = amm.saturating_add(amm_swap_units(*kind));
    }
    push("amm_swap", "AMM 交换", amm);
    push("executor_base", "Executor 基础开销", params.executor_base);
    push(
        "executor_hops",
        "Executor 逐跳逻辑",
        params.executor_per_hop.saturating_mul(kinds.len() as u64),
    );
    push(
        "token_transfer",
        "Token 转账",
        params.transfer_per_hop.saturating_mul(kinds.len() as u64),
    );
    if flash {
        push("flash_loan", "闪电贷借款/还款", params.flash_loan);
    }
    push(
        "approve",
        "Approve/Permit",
        params.approve.saturating_mul(approvals as u64),
    );

    let units: u64 = components.iter().map(|c| c.units).sum();
    let usd = gas_usd(units, gas_price_gwei, native_price_usd);
    let estimated_at = Utc::now();

    GasEstimate {
        units,
        gas_price_gwei,
        native_price_usd,
        usd,
        components,
        block_number,
        estimated_at,
        note: format!(
            "按 {} 跳 AMM 类型估算；Gas 价格 {gas_price_gwei} gwei，原生资产 {native_price_usd} USDC",
            kinds.len()
        ),
    }
}

/// gas units → USDC。
pub fn gas_usd(units: u64, gas_price_gwei: Decimal, native_price_usd: Decimal) -> Decimal {
    Decimal::from(units) * gas_price_gwei * Decimal::new(1, 9) * native_price_usd
}

/// 估算误差（bps）：用于对比历史估算与实际 Gas。
pub fn estimate_error_bps(estimated: Decimal, actual: Decimal) -> Decimal {
    if actual.is_zero() {
        Decimal::ZERO
    } else {
        (estimated - actual) / actual * Decimal::from(10_000)
    }
}

/// 带时间戳的说明文本。
pub fn describe(estimate: &GasEstimate, at: DateTime<Utc>) -> String {
    format!(
        "{} gas · {} USDC · block {} · {}",
        estimate.units, estimate.usd, estimate.block_number, at
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    #[test]
    fn components_sum_to_units_and_usd_formula() {
        let p = GasParams::default();
        let gas = estimate(
            &[PoolKind::UniswapV2, PoolKind::UniswapV3],
            false,
            0,
            &p,
            dec("0.02"),
            dec("2800"),
            100,
        );
        let sum: u64 = gas.components.iter().map(|c| c.units).sum();
        assert_eq!(sum, gas.units);
        // 2 跳：amm 220k + base 45k + hops 24k + transfers 60k = 349k
        assert_eq!(gas.units, 349_000);
        assert_eq!(gas.usd, dec("0.019544"));
        assert!(!gas.components.iter().any(|c| c.key == "flash_loan"));
        assert_eq!(gas.block_number, 100);
        assert!(gas.note.contains("2 跳"));
    }

    #[test]
    fn flash_and_approvals_add_units() {
        let p = GasParams::default();
        let base = estimate(&[PoolKind::UniswapV2], false, 0, &p, dec("1"), dec("3000"), 1);
        let full = estimate(&[PoolKind::UniswapV2], true, 2, &p, dec("1"), dec("3000"), 1);
        assert_eq!(full.units, base.units + p.flash_loan + 2 * p.approve);
        assert!(full.usd > base.usd);
    }

    #[test]
    fn unknown_amm_contributes_no_swap_gas() {
        let p = GasParams::default();
        let gas = estimate(&[PoolKind::Unknown], false, 0, &p, dec("0.02"), dec("2800"), 1);
        assert_eq!(amm_swap_units(PoolKind::Unknown), 0);
        assert!(!gas.components.iter().any(|c| c.key == "amm_swap"));
        assert_eq!(
            gas.units,
            p.executor_base + p.executor_per_hop + p.transfer_per_hop
        );
    }

    #[test]
    fn error_bps_is_signed() {
        assert_eq!(estimate_error_bps(dec("110"), dec("100")), dec("1000"));
        assert_eq!(estimate_error_bps(dec("90"), dec("100")), dec("-1000"));
        assert_eq!(estimate_error_bps(dec("90"), Decimal::ZERO), Decimal::ZERO);
    }
}
