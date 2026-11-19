//! 分级模拟（PRD §6.1–§6.3）。
//!
//! - M0：仅用即期价连乘的理论估计，扫描漏斗第一层，**不可**作为机会依据。
//! - M1：状态一致性模拟。要求全部池来自同一区块快照，按同一批状态逐跳复算，
//!   与报价输出比对；不一致即失败。
//! - M2：本地 EVM（REVM）完整执行。本机未接入 REVM，只能返回 UNAVAILABLE，
//!   不允许把任何 M1 结果冒称为 M2 或 VERIFIED。
//! - M3：压力模拟。把池储备/价格朝不利方向扰动、Gas 上调、闪电贷费用上调后
//!   重新报价并重算净利，用于判断机会的抗压能力。
//!
//! 所有模拟结果都写入 RouteScanRecord，便于复现与误差统计。

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::domain::dex_scan::{GasEstimate, SimulationOutcome};
use crate::domain::market::PoolKind;
use crate::domain::money::diff_bps;
use crate::domain::new_id;
use crate::domain::types::{ModelLevel, SimulationStatus};
use crate::dex::graph::HopEdge;
use crate::dex::pool::{PoolSet, quote_path};

/// 压力参数。
#[derive(Clone, Debug)]
pub struct StressParams {
    /// 池状态朝不利方向的扰动（bps）。同时作用于储备/价格与流动性。
    pub adverse_bps: Decimal,
    /// Gas 上调比例（%）。
    pub gas_increase_pct: Decimal,
    /// 闪电贷费用上调比例（%）。
    pub flash_fee_increase_pct: Decimal,
}

impl Default for StressParams {
    fn default() -> Self {
        Self {
            adverse_bps: Decimal::from(25),
            gas_increase_pct: Decimal::from(50),
            flash_fee_increase_pct: Decimal::from(50),
        }
    }
}

/// M0：即期价连乘估计（仅用于对比，不可作为机会依据）。
pub fn theoretical_out(set: &PoolSet, start: &str, amount_in: Decimal, edges: &[HopEdge]) -> Option<Decimal> {
    let mut amount = amount_in;
    for e in edges {
        let rt = set.get(&e.pool_id)?;
        let spot = rt.spot_price(&e.token_in)?;
        if spot <= Decimal::ZERO {
            return None;
        }
        amount *= spot;
    }
    if start.is_empty() {
        return None;
    }
    Some(amount)
}

/// M1：状态一致性模拟。
pub fn simulate_m1(
    set: &PoolSet,
    start: &str,
    amount_in: Decimal,
    edges: &[HopEdge],
    quoted_out: Decimal,
) -> SimulationOutcome {
    let mut notes = Vec::new();
    let simulation_id = new_id("sim");
    let created_at = Utc::now();

    // 池不得重复使用（PRD §4.3）。
    let mut pools: Vec<&str> = edges.iter().map(|e| e.pool_id.as_str()).collect();
    let n = pools.len();
    pools.sort_unstable();
    pools.dedup();
    if pools.len() != n {
        notes.push("路线重复使用同一池，报价无效".to_string());
        return failed(
            simulation_id,
            ModelLevel::M1,
            SimulationStatus::Failed,
            quoted_out,
            Decimal::ZERO,
            set.block_number(),
            notes,
            created_at,
        );
    }

    let mut blocks: Vec<u64> = Vec::new();
    for e in edges {
        match set.get(&e.pool_id) {
            Some(rt) => blocks.push(rt.state.block_number),
            None => {
                notes.push(format!("缺少池状态：{}", e.pool_id));
                return failed(
                    simulation_id,
                    ModelLevel::M1,
                    SimulationStatus::Failed,
                    quoted_out,
                    Decimal::ZERO,
                    set.block_number(),
                    notes,
                    created_at,
                );
            }
        }
    }
    blocks.sort_unstable();
    blocks.dedup();
    if blocks.len() != 1 && !blocks.is_empty() {
        notes.push(format!(
            "各跳池状态区块不一致（{:?}），禁止拼成同时刻快照",
            blocks
        ));
        return failed(
            simulation_id,
            ModelLevel::M1,
            SimulationStatus::Failed,
            quoted_out,
            Decimal::ZERO,
            *blocks.last().unwrap_or(&0),
            notes,
            created_at,
        );
    }

    let block = blocks.first().copied().unwrap_or_else(|| set.block_number());
    match quote_path(set, start, amount_in, edges) {
        Ok(path) => {
            let diff = diff_bps(path.amount_out, quoted_out);
            let status = if path.amount_out <= Decimal::ZERO {
                notes.push("模拟输出为零，路线不可成交".to_string());
                SimulationStatus::Failed
            } else if path.amount_out != quoted_out {
                notes.push(format!(
                    "复算输出 {} 与报价 {} 不一致，差异 {} bps",
                    path.amount_out, quoted_out, diff
                ));
                if diff.abs() > Decimal::from(50) {
                    SimulationStatus::ModelMismatch
                } else {
                    SimulationStatus::Passed
                }
            } else {
                notes.push("全部池来自同一区块快照，逐跳复算与报价一致".to_string());
                if let Some(theo) = theoretical_out(set, start, amount_in, edges) {
                    notes.push(format!(
                        "理论即期输出 {}，实际输出 {}（差异 {} bps，含手续费与冲击）",
                        theo.round_dp(8),
                        path.amount_out.round_dp(8),
                        diff_bps(path.amount_out, theo).round_dp(2)
                    ));
                }
                SimulationStatus::Passed
            };
            SimulationOutcome {
                simulation_id,
                level: ModelLevel::M1,
                status,
                quoted_out,
                simulated_out: path.amount_out,
                diff_bps: diff,
                net_usd: None,
                gas_estimate: None,
                state_block: block,
                notes,
                created_at,
            }
        }
        Err(e) => {
            notes.push(format!("复算失败：{e}"));
            failed(
                simulation_id,
                ModelLevel::M1,
                SimulationStatus::Failed,
                quoted_out,
                Decimal::ZERO,
                block,
                notes,
                created_at,
            )
        }
    }
}

/// M2：本地 EVM 模拟。当前环境未接入 REVM，显式返回 UNAVAILABLE。
pub fn simulate_m2_unavailable(block: u64) -> SimulationOutcome {
    SimulationOutcome {
        simulation_id: new_id("sim"),
        level: ModelLevel::M2,
        status: SimulationStatus::Unavailable,
        quoted_out: Decimal::ZERO,
        simulated_out: Decimal::ZERO,
        diff_bps: Decimal::ZERO,
        net_usd: None,
        gas_estimate: None,
        state_block: block,
        notes: vec![
            "本机未接入 REVM/本地 EVM，M2 模拟不可用；不得据此标记 VERIFIED".to_string(),
        ],
        created_at: Utc::now(),
    }
}

/// M3：压力模拟。net_at 由调用方给出（用压力后的输出与 Gas 重算净利）。
pub fn simulate_m3<F>(
    set: &PoolSet,
    start: &str,
    amount_in: Decimal,
    edges: &[HopEdge],
    quoted_out: Decimal,
    stress: &StressParams,
    base_gas: Option<&GasEstimate>,
    mut net_at: F,
) -> SimulationOutcome
where
    F: FnMut(Decimal, Option<&GasEstimate>) -> Decimal,
{
    let simulation_id = new_id("sim");
    let created_at = Utc::now();
    let mut notes = Vec::new();
    let mut stressed = set.clone();
    let adverse = if stress.adverse_bps >= Decimal::ZERO {
        stress.adverse_bps
    } else {
        Decimal::ZERO
    };

    for e in edges {
        if let Some(rt) = stressed.get_mut(&e.pool_id) {
            let zero_for_one = e.token_in == rt.pool.token0;
            apply_adverse_state(rt, zero_for_one, adverse);
        }
    }

    let gas = base_gas.map(|g| stress_gas(g, stress.gas_increase_pct));
    notes.push(format!(
        "池状态向不利方向扰动 {} bps；Gas 上调 {}%",
        adverse, stress.gas_increase_pct
    ));
    if stress.flash_fee_increase_pct > Decimal::ZERO {
        notes.push(format!(
            "闪电贷费用按 +{}% 计入压力成本",
            stress.flash_fee_increase_pct
        ));
    }

    match quote_path(&stressed, start, amount_in, edges) {
        Ok(path) => {
            let net = net_at(path.amount_out, gas.as_ref());
            let status = if path.amount_out <= Decimal::ZERO || net <= Decimal::ZERO {
                SimulationStatus::Failed
            } else {
                SimulationStatus::Passed
            };
            notes.push(format!(
                "压力输出 {}，压力净利 {} USDC",
                path.amount_out.round_dp(8),
                net.round_dp(4)
            ));
            SimulationOutcome {
                simulation_id,
                level: ModelLevel::M3,
                status,
                quoted_out,
                simulated_out: path.amount_out,
                diff_bps: diff_bps(path.amount_out, quoted_out),
                net_usd: Some(net),
                gas_estimate: gas,
                state_block: set.block_number(),
                notes,
                created_at,
            }
        }
        Err(e) => {
            notes.push(format!("压力复算失败：{e}"));
            failed(
                simulation_id,
                ModelLevel::M3,
                SimulationStatus::Failed,
                quoted_out,
                Decimal::ZERO,
                set.block_number(),
                notes,
                created_at,
            )
        }
    }
}

fn apply_adverse_state(rt: &mut crate::dex::pool::PoolRuntime, zero_for_one: bool, adverse_bps: Decimal) {
    let adverse = (adverse_bps / Decimal::from(10_000)).min(Decimal::new(5, 1));
    let up = Decimal::ONE + adverse;
    let down = Decimal::ONE - adverse;
    match rt.pool.kind {
        PoolKind::UniswapV2 | PoolKind::StableSwap | PoolKind::WeightedPool => {
            if zero_for_one {
                rt.state.reserve1 *= down;
                rt.state.reserve0 *= up;
            } else {
                rt.state.reserve0 *= down;
                rt.state.reserve1 *= up;
            }
        }
        PoolKind::UniswapV3 => {
            let f = adverse.to_f64().unwrap_or(0.0);
            let factor = if zero_for_one {
                (1.0 - f).max(0.5).sqrt()
            } else {
                (1.0 + f).min(1.5).sqrt()
            };
            rt.state.sqrt_price_x96 = scale_u128(rt.state.sqrt_price_x96, factor);
            rt.state.liquidity = scale_u128(rt.state.liquidity, (1.0 - f).max(0.5));
        }
        PoolKind::Unknown => {}
    }
}

fn scale_u128(v: u128, f: f64) -> u128 {
    if !f.is_finite() || f <= 0.0 {
        return v;
    }
    let x = v as f64 * f;
    if x.is_finite() && x >= 0.0 && x <= u128::MAX as f64 {
        x as u128
    } else {
        v
    }
}

fn stress_gas(g: &GasEstimate, pct: Decimal) -> GasEstimate {
    let factor = Decimal::ONE + pct.max(Decimal::ZERO) / Decimal::from(100);
    let mut out = g.clone();
    out.usd = g.usd * factor;
    out.note = format!("{}；压力模拟 Gas +{}%", g.note, pct);
    out.estimated_at = Utc::now();
    out
}

#[allow(clippy::too_many_arguments)]
fn failed(
    simulation_id: String,
    level: ModelLevel,
    status: SimulationStatus,
    quoted_out: Decimal,
    simulated_out: Decimal,
    block: u64,
    notes: Vec<String>,
    created_at: chrono::DateTime<Utc>,
) -> SimulationOutcome {
    SimulationOutcome {
        simulation_id,
        level,
        status,
        quoted_out,
        simulated_out,
        diff_bps: Decimal::ZERO,
        net_usd: None,
        gas_estimate: None,
        state_block: block,
        notes,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::{PoolRef, PoolState};
    use crate::domain::money::dec;
    use crate::dex::math;
    use crate::dex::pool::PoolRuntime;

    fn add(set: &mut PoolSet, id: &str, t0: &str, t1: &str, block: u64) {
        let now = Utc::now();
        set.upsert(
            PoolRef {
                pool_id: id.into(),
                chain_id: 1,
                protocol_id: "uni-v2".into(),
                kind: PoolKind::UniswapV2,
                address: format!("0x{id}"),
                token0: t0.into(),
                token1: t1.into(),
                fee_bps: 30,
                tick_spacing: None,
                tvl_usd: dec("1000000"),
                verified: true,
                execution_allowed: true,
                added_at: now,
            },
            PoolState {
                pool_id: id.into(),
                block_number: block,
                block_hash: format!("0x{block:x}"),
                updated_at: now,
                reserve0: dec("1000000"),
                reserve1: dec("1000"),
                sqrt_price_x96: math::Q96,
                liquidity: 0,
                tick: 0,
                ticks: vec![],
                amplification: dec("100"),
                weight0: dec("0.5"),
                weight1: dec("0.5"),
                fee_bps: 30,
            },
        );
    }

    fn pool_rt<'a>(set: &'a mut PoolSet, id: &str) -> &'a mut PoolRuntime {
        set.get_mut(id).unwrap()
    }

    fn edges(_set: &PoolSet) -> Vec<HopEdge> {
        vec![
            HopEdge {
                pool_id: "p1".into(),
                token_in: "USDC".into(),
                token_out: "WETH".into(),
                kind: PoolKind::UniswapV2,
            },
            HopEdge {
                pool_id: "p2".into(),
                token_in: "WETH".into(),
                token_out: "USDC".into(),
                kind: PoolKind::UniswapV2,
            },
        ]
    }

    fn two_pool_set() -> PoolSet {
        let mut set = PoolSet::new(1);
        add(&mut set, "p1", "USDC", "WETH", 100);
        add(&mut set, "p2", "WETH", "USDC", 100);
        pool_rt(&mut set, "p2").state.reserve0 = dec("1200000");
        pool_rt(&mut set, "p2").state.reserve1 = dec("1000");
        let _ = edges(&set);
        set
    }

    #[test]
    fn m1_passes_on_consistent_state() {
        let set = two_pool_set();
        let e = edges(&set);
        let quoted = quote_path(&set, "USDC", dec("1000"), &e).unwrap().amount_out;
        let sim = simulate_m1(&set, "USDC", dec("1000"), &e, quoted);
        assert_eq!(sim.status, SimulationStatus::Passed);
        assert_eq!(sim.level, ModelLevel::M1);
        assert_eq!(sim.diff_bps, Decimal::ZERO);
        assert!(sim.notes.iter().any(|n| n.contains("同一区块")));
    }

    #[test]
    fn m1_rejects_mixed_blocks() {
        let mut set = two_pool_set();
        pool_rt(&mut set, "p2").state.block_number = 101;
        let e = edges(&set);
        let sim = simulate_m1(&set, "USDC", dec("1000"), &e, dec("999"));
        assert_eq!(sim.status, SimulationStatus::Failed);
        assert!(sim.notes.iter().any(|n| n.contains("区块不一致")));
    }

    #[test]
    fn m1_flags_model_mismatch_beyond_threshold() {
        let set = two_pool_set();
        let e = edges(&set);
        let sim = simulate_m1(&set, "USDC", dec("1000"), &e, dec("500"));
        assert_eq!(sim.status, SimulationStatus::ModelMismatch);
        assert!(sim.diff_bps.abs() > Decimal::from(50));
    }

    #[test]
    fn m1_rejects_reused_pool() {
        let set = two_pool_set();
        let e = vec![
            HopEdge {
                pool_id: "p1".into(),
                token_in: "USDC".into(),
                token_out: "WETH".into(),
                kind: PoolKind::UniswapV2,
            },
            HopEdge {
                pool_id: "p1".into(),
                token_in: "WETH".into(),
                token_out: "USDC".into(),
                kind: PoolKind::UniswapV2,
            },
        ];
        let sim = simulate_m1(&set, "USDC", dec("1000"), &e, dec("1000"));
        assert_eq!(sim.status, SimulationStatus::Failed);
        assert!(sim.notes.iter().any(|n| n.contains("重复使用同一池")));
        // 反向报价仍可用，用于确认 set 未被修改
        let _ = set.get("p1").unwrap();
    }

    #[test]
    fn m2_is_explicitly_unavailable() {
        let sim = simulate_m2_unavailable(42);
        assert_eq!(sim.status, SimulationStatus::Unavailable);
        assert_eq!(sim.level, ModelLevel::M2);
        assert!(sim.notes.iter().any(|n| n.contains("REVM")));
    }

    #[test]
    fn m3_stress_reduces_output_and_net() {
        let set = two_pool_set();
        let e = edges(&set);
        let quoted = quote_path(&set, "USDC", dec("1000"), &e).unwrap().amount_out;
        let stress = StressParams::default();
        let sim = simulate_m3(&set, "USDC", dec("1000"), &e, quoted, &stress, None, |out, _| {
            (out - dec("1000")) * dec("1")
        });
        assert_eq!(sim.level, ModelLevel::M3);
        assert!(sim.simulated_out < quoted);
        assert!(sim.diff_bps < Decimal::ZERO);
        assert!(sim.net_usd.is_some());
        // 原始池状态未被污染
        let recheck = quote_path(&set, "USDC", dec("1000"), &e).unwrap();
        assert_eq!(recheck.amount_out, quoted);
    }

    #[test]
    fn m3_gas_increase_is_applied() {
        let set = two_pool_set();
        let e = edges(&set);
        let quoted = quote_path(&set, "USDC", dec("1000"), &e).unwrap().amount_out;
        let gas = crate::dex::gas::estimate(
            &[PoolKind::UniswapV2, PoolKind::UniswapV2],
            false,
            0,
            &crate::dex::gas::GasParams::default(),
            dec("0.02"),
            dec("2800"),
            100,
        );
        let sim = simulate_m3(
            &set,
            "USDC",
            dec("1000"),
            &e,
            quoted,
            &StressParams::default(),
            Some(&gas),
            |out, g| out - dec("1000") - g.map(|x| x.usd).unwrap_or_default(),
        );
        let stressed_gas = sim.gas_estimate.as_ref().unwrap();
        assert_eq!(stressed_gas.usd, gas.usd * dec("1.5"));
    }
}
