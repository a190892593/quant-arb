//! DEX 报价数学（纯函数，无 I/O、无全局状态）。
//!
//! 设计约定：
//! - 任何函数都**不得 panic**：除零、负值、非法参数一律返回 [`Decimal::ZERO`]，
//!   由上层 [`crate::dex::pool`] 判定拒绝；十进制乘法/除法溢出同样返回 0。
//! - 费率单位统一为 bps（`10_000` = 100%），一律从**输入侧**扣除。
//! - 所有报价输出统一 `round_dp(18)` 后返回，并截断负数（不足最小单位返回 0）。
//! - V3 的金额与价格均以**池内原始单位（raw units）**表达；需要人类可读价格时使用
//!   [`sqrt_price_x96_to_price`] 并传入两个 token 的 decimals。
//!
//! 数值精度边界：
//! - V2 / StableSwap 全程使用 [`Decimal`]（28 位有效数字），StableSwap 的牛顿迭代以
//!   1e-12 相对误差收敛；StableSwap 内部先把余额按最大余额归一化，避免 `D³` 在
//!   大储备（raw units 达 1e18 以上）时溢出 Decimal 的 96 位尾数。
//! - V3 的 `sqrtPriceX96` 是 128 位整数，某些池的原始值超过 `Decimal` 可表示范围
//!   （2^96 本身即大于 `Decimal::MAX`），因此只能经 f64 中转，单次转换相对误差约
//!   1e-16；`u128` 回写同理。
//! - Weighted 池的分数次幂依赖 f64 `powf`，相对误差约 1e-15；量级超过 1e15 时绝对
//!   误差可能达到 1 个最小单位，仅适用于常规规模。
//! - `tick` ↔ `sqrtPriceX96` 使用 f64 `powi/sqrt`，往返误差 < 0.01%（测试覆盖）。

use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::*;

use crate::domain::market::PoolState;
use crate::domain::money::BPS_DENOM;

/// Uniswap V3 定点基数 2^96。
pub const Q96: u128 = 1u128 << 96;

/// V3 最小/最大 tick（与 Uniswap V3 一致）。
const MIN_TICK: i32 = -887_272;
const MAX_TICK: i32 = 887_272;

/// 2^96 的 f64 表示（用于 u128 ↔ 实际 sqrtPrice 的比例换算）。
const Q96_F64: f64 = 79_228_162_514_264_337_593_543_950_336.0;
/// 2^64 的 Decimal 表示（2^64 超出 u64 范围，用 2^32 自乘构造）。
fn two64() -> Decimal {
    let base = Decimal::from(1u64 << 32);
    base * base
}
/// 牛顿迭代与收敛判断使用的相对误差分母。
const CONVERGENCE_DENOM: i64 = 1_000_000_000_000; // 1e12

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 统一收口：截断负数并保留 18 位小数。
fn dq(v: Decimal) -> Decimal {
    let v = if v < Decimal::ZERO { Decimal::ZERO } else { v };
    v.round_dp(18)
}

/// 手续费乘数：`1 - fee_bps/10000`，fee_bps 越界时按 0–10000 截断。
fn fee_multiplier(fee_bps: i64) -> Decimal {
    let f = fee_bps.clamp(0, BPS_DENOM);
    Decimal::from(BPS_DENOM - f) / Decimal::from(BPS_DENOM)
}

/// f64 → Decimal；NaN/Inf/超出表示范围返回 None。
fn dec_from_f64(v: f64) -> Option<Decimal> {
    if !v.is_finite() {
        return None;
    }
    Decimal::from_f64(v)
}

fn dec_to_f64(v: Decimal) -> Option<f64> {
    v.to_f64()
}

/// u128 → Decimal：拆成高/低 64 位，避免 128 位整数超出 Decimal 尾数导致精度丢失。
/// 超过 `Decimal` 表示范围时饱和到 `Decimal::MAX`（现实中不会出现）。
fn u128_to_dec(v: u128) -> Decimal {
    let hi = (v >> 64) as u64;
    let lo = v as u64;
    if hi == 0 {
        return Decimal::from(lo);
    }
    let base = two64();
    Decimal::from(hi)
        .checked_mul(base)
        .and_then(|h| h.checked_add(Decimal::from(lo)))
        .unwrap_or(Decimal::MAX)
}

/// i128 → Decimal：经字符串精确转换（流动性净额可达 1e24 以上）。
fn i128_to_dec(v: i128) -> Decimal {
    match Decimal::from_str(&v.to_string()) {
        Ok(d) => d,
        Err(_) => {
            if v > 0 {
                Decimal::MAX
            } else {
                Decimal::ZERO
            }
        }
    }
}

/// `sqrtPriceX96` → 实际 sqrtPrice（≈ sqrt(token1/token0) 的原始单位比值）。
/// 2^96 超出 Decimal 范围，只能经 f64：相对误差约 1e-16。
fn sqrt_price_dec(sqrt_price_x96: u128) -> Decimal {
    if sqrt_price_x96 == 0 {
        return Decimal::ZERO;
    }
    dec_from_f64(sqrt_price_x96 as f64 / Q96_F64).unwrap_or(Decimal::ZERO)
}

/// 实际 sqrtPrice → `sqrtPriceX96`（截断取整）。
fn sqrt_price_to_x96(sqrt_p: Decimal) -> u128 {
    if sqrt_p <= Decimal::ZERO {
        return 0;
    }
    let f = match dec_to_f64(sqrt_p) {
        Some(v) => v * Q96_F64,
        None => return 0,
    };
    if !f.is_finite() || f <= 0.0 {
        0
    } else if f >= u128::MAX as f64 {
        u128::MAX
    } else {
        f as u128
    }
}

// ---------------------------------------------------------------------------
// Uniswap V2（恒定乘积）
// ---------------------------------------------------------------------------

/// Uniswap V2 exact-in：
/// `amount_out = amount_in*(1-fee)*R_out / (R_in + amount_in*(1-fee))`
///
/// 手续费在输入侧扣除后再进入恒定乘积公式（即标准 V2 的“手续费复利”形式：
/// 每一跳都对**该跳的输入**扣费，多跳路径因此自然复利）。
pub fn v2_amount_out(
    amount_in: Decimal,
    reserve_in: Decimal,
    reserve_out: Decimal,
    fee_bps: i64,
) -> Decimal {
    if amount_in <= Decimal::ZERO || reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let aif = match amount_in.checked_mul(fee_multiplier(fee_bps)) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let num = match aif.checked_mul(reserve_out) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let den = match reserve_in.checked_add(aif) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    if den <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    dq(num / den)
}

/// Uniswap V2 exact-out 反解：返回达到 `amount_out` 所需的最小输入。
///
/// 由 `aif = R_in*out/(R_out-out)`、`in = aif/(1-fee)` 得。
pub fn v2_amount_in(
    amount_out: Decimal,
    reserve_in: Decimal,
    reserve_out: Decimal,
    fee_bps: i64,
) -> Decimal {
    if amount_out <= Decimal::ZERO
        || reserve_in <= Decimal::ZERO
        || reserve_out <= Decimal::ZERO
        || amount_out >= reserve_out
    {
        return Decimal::ZERO;
    }
    let fee = fee_multiplier(fee_bps);
    if fee <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let num = match reserve_in.checked_mul(amount_out) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let rest = reserve_out - amount_out;
    if rest <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let aif = num / rest;
    dq(aif / fee)
}

/// 恒定乘积即期价：`R_out / R_in`（不含手续费与冲击）。
pub fn v2_spot_price(reserve_in: Decimal, reserve_out: Decimal) -> Decimal {
    if reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    dq(reserve_out / reserve_in)
}

// ---------------------------------------------------------------------------
// Uniswap V3
// ---------------------------------------------------------------------------

/// Uniswap V3 单区间 exact-in 报价。
///
/// 记 `aif = amount_in*(1-fee)`、`L = liquidity`、`√P = sqrtPriceX96/2^96`：
/// - `zero_for_one`（卖出 token0，价格下行）：
///   `√P_new = √P·L/(L + aif·√P)`，`amount_out = L·(√P - √P_new)`
/// - 反向（卖出 token1，价格上行）：
///   `√P_new = √P + aif/L`，`amount_out = L·(1/√P - 1/√P_new)`
///
/// 返回 `(amount_out, 新的 sqrtPriceX96)`；参数非法时返回 `(0, 原 sqrtPriceX96)`。
pub fn v3_amount_out_single_range(
    amount_in: Decimal,
    sqrt_price_x96: u128,
    liquidity: u128,
    fee_bps: i64,
    zero_for_one: bool,
) -> (Decimal, u128) {
    if amount_in <= Decimal::ZERO || sqrt_price_x96 == 0 || liquidity == 0 {
        return (Decimal::ZERO, sqrt_price_x96);
    }
    let l = u128_to_dec(liquidity);
    let sqrt_p = sqrt_price_dec(sqrt_price_x96);
    if l <= Decimal::ZERO || sqrt_p <= Decimal::ZERO || l == Decimal::MAX {
        return (Decimal::ZERO, sqrt_price_x96);
    }
    let aif = match amount_in.checked_mul(fee_multiplier(fee_bps)) {
        Some(v) => v,
        None => return (Decimal::ZERO, sqrt_price_x96),
    };
    if aif <= Decimal::ZERO {
        return (Decimal::ZERO, sqrt_price_x96);
    }
    if zero_for_one {
        // denom = L + aif·√P
        let denom = match aif.checked_mul(sqrt_p).and_then(|v| v.checked_add(l)) {
            Some(v) => v,
            None => return (Decimal::ZERO, sqrt_price_x96),
        };
        if denom <= Decimal::ZERO {
            return (Decimal::ZERO, sqrt_price_x96);
        }
        let new_sqrt = match sqrt_p.checked_mul(l).and_then(|v| v.checked_div(denom)) {
            Some(v) => v,
            None => return (Decimal::ZERO, sqrt_price_x96),
        };
        let out = match l.checked_mul(sqrt_p - new_sqrt) {
            Some(v) => v,
            None => return (Decimal::ZERO, sqrt_price_x96),
        };
        (dq(out), sqrt_price_to_x96(new_sqrt))
    } else {
        let new_sqrt = match aif.checked_div(l).and_then(|v| v.checked_add(sqrt_p)) {
            Some(v) => v,
            None => return (Decimal::ZERO, sqrt_price_x96),
        };
        if new_sqrt <= Decimal::ZERO {
            return (Decimal::ZERO, sqrt_price_x96);
        }
        let inv_cur = Decimal::ONE / sqrt_p;
        let inv_new = Decimal::ONE / new_sqrt;
        let out = match l.checked_mul(inv_cur - inv_new) {
            Some(v) => v,
            None => return (Decimal::ZERO, sqrt_price_x96),
        };
        (dq(out), sqrt_price_to_x96(new_sqrt))
    }
}

/// 跨 tick 的 V3 exact-in 报价：从 `state.tick` 出发，按 `state.ticks` 的
/// `liquidity_net` 逐段推进，最多处理 200 个 tick。
///
/// 语义：
/// - 手续费取 `state.fee_bps`，一次性从输入中扣除后再按段分割（与逐段扣费等价）。
/// - `zero_for_one` 时价格下行，穿越 **低于** 当前 tick 的边界（由高到低），
///   每穿越一次 `L -= liquidity_net`；反向则价格上行、`L += liquidity_net`。
/// - 流动性耗尽（`L <= 0`）或 tick 用尽时停止，返回**已成交部分**，不返回错误。
/// - `state.ticks` 为空视为“全部流动性集中在现价”，即单区间恒定 L。
///
/// 返回 `(amount_out, ticks_crossed)`。
pub fn v3_quote_with_ticks(
    state: &PoolState,
    zero_for_one: bool,
    amount_in: Decimal,
) -> (Decimal, usize) {
    if amount_in <= Decimal::ZERO || state.liquidity == 0 || state.sqrt_price_x96 == 0 {
        return (Decimal::ZERO, 0);
    }
    let mut remaining = match amount_in.checked_mul(fee_multiplier(state.fee_bps)) {
        Some(v) => v,
        None => return (Decimal::ZERO, 0),
    };
    if remaining <= Decimal::ZERO {
        return (Decimal::ZERO, 0);
    }
    let mut l = u128_to_dec(state.liquidity);
    let mut sqrt_p = sqrt_price_dec(state.sqrt_price_x96);
    if l <= Decimal::ZERO || l == Decimal::MAX || sqrt_p <= Decimal::ZERO {
        return (Decimal::ZERO, 0);
    }

    // 候选 tick：按穿越方向排序（下行为降序、上行为升序）。
    let mut candidates: Vec<i32> = state
        .ticks
        .iter()
        .map(|t| t.tick_index)
        .filter(|t| {
            if zero_for_one {
                *t < state.tick
            } else {
                *t > state.tick
            }
        })
        .collect();
    candidates.sort_unstable();
    if zero_for_one {
        candidates.reverse();
    }

    let mut out = Decimal::ZERO;
    let mut crossed = 0usize;

    for tick in candidates.into_iter() {
        if crossed >= 200 {
            break;
        }
        let net = state
            .ticks
            .iter()
            .find(|t| t.tick_index == tick)
            .map(|t| t.liquidity_net)
            .unwrap_or(0);
        let net_dec = i128_to_dec(net);
        let target = sqrt_price_dec(tick_to_sqrt_price_x96(tick));
        if target <= Decimal::ZERO {
            break;
        }

        if zero_for_one {
            if target >= sqrt_p {
                // 该 tick 不在当前价之下（或与现价重合），停止推进。
                break;
            }
            let inv_cur = Decimal::ONE / sqrt_p;
            let inv_target = Decimal::ONE / target;
            let need = match l.checked_mul(inv_target - inv_cur) {
                Some(v) => v,
                None => break,
            };
            if need <= remaining {
                let seg_out = match l.checked_mul(sqrt_p - target) {
                    Some(v) => v,
                    None => break,
                };
                out += seg_out;
                remaining -= need;
                sqrt_p = target;
                crossed += 1;
                l -= net_dec;
                if l <= Decimal::ZERO {
                    return (dq(out), crossed);
                }
            } else {
                let inv_new = match remaining
                    .checked_div(l)
                    .and_then(|v| v.checked_add(inv_cur))
                {
                    Some(v) => v,
                    None => return (dq(out), crossed),
                };
                if inv_new <= Decimal::ZERO {
                    return (dq(out), crossed);
                }
                let new_sqrt = Decimal::ONE / inv_new;
                if let Some(seg_out) = l.checked_mul(sqrt_p - new_sqrt) {
                    out += seg_out;
                }
                return (dq(out), crossed);
            }
        } else {
            if target <= sqrt_p {
                break;
            }
            let need = match l.checked_mul(target - sqrt_p) {
                Some(v) => v,
                None => break,
            };
            if need <= remaining {
                let inv_cur = Decimal::ONE / sqrt_p;
                let inv_target = Decimal::ONE / target;
                let seg_out = match l.checked_mul(inv_cur - inv_target) {
                    Some(v) => v,
                    None => break,
                };
                out += seg_out;
                remaining -= need;
                sqrt_p = target;
                crossed += 1;
                l += net_dec;
                if l <= Decimal::ZERO {
                    return (dq(out), crossed);
                }
            } else {
                let new_sqrt = match remaining.checked_div(l).and_then(|v| v.checked_add(sqrt_p)) {
                    Some(v) => v,
                    None => return (dq(out), crossed),
                };
                if new_sqrt <= Decimal::ZERO {
                    return (dq(out), crossed);
                }
                let inv_cur = Decimal::ONE / sqrt_p;
                let inv_new = Decimal::ONE / new_sqrt;
                if let Some(seg_out) = l.checked_mul(inv_cur - inv_new) {
                    out += seg_out;
                }
                return (dq(out), crossed);
            }
        }
    }

    // 结尾：仍在最后一个区间内成交剩余输入。
    if remaining > Decimal::ZERO && l > Decimal::ZERO && sqrt_p > Decimal::ZERO {
        if zero_for_one {
            let inv_cur = Decimal::ONE / sqrt_p;
            if let Some(inv_new) = remaining
                .checked_div(l)
                .and_then(|v| v.checked_add(inv_cur))
            {
                if inv_new > Decimal::ZERO {
                    let new_sqrt = Decimal::ONE / inv_new;
                    if let Some(seg_out) = l.checked_mul(sqrt_p - new_sqrt) {
                        out += seg_out;
                    }
                }
            }
        } else if let Some(new_sqrt) = remaining.checked_div(l).and_then(|v| v.checked_add(sqrt_p))
        {
            if new_sqrt > Decimal::ZERO {
                let inv_cur = Decimal::ONE / sqrt_p;
                let inv_new = Decimal::ONE / new_sqrt;
                if let Some(seg_out) = l.checked_mul(inv_cur - inv_new) {
                    out += seg_out;
                }
            }
        }
    }

    (dq(out), crossed)
}

/// `tick` → `sqrtPriceX96`，`sqrtP = sqrt(1.0001^tick) · 2^96`。
///
/// 使用 f64 `powi/sqrt` 近似后截断为 u128；tick 越界时截断到 ±887272，
/// 溢出 u128 时饱和到 `u128::MAX`。往返误差见测试（< 0.01%）。
pub fn tick_to_sqrt_price_x96(tick: i32) -> u128 {
    let t = tick.clamp(MIN_TICK, MAX_TICK);
    let price = 1.0001f64.powi(t);
    if !price.is_finite() || price <= 0.0 {
        return 0;
    }
    let sp = price.sqrt() * Q96_F64;
    if !sp.is_finite() || sp <= 0.0 {
        0
    } else if sp >= u128::MAX as f64 {
        u128::MAX
    } else {
        sp as u128
    }
}

/// `sqrtPriceX96` → 人类可读价格 `token1 per token0`（考虑两边 decimals）。
///
/// `price = (sqrtPriceX96/2^96)^2 · 10^(dec0-dec1)`。
/// 超出 Decimal 表示范围时返回 0（该状态本身不可用于报价）。
pub fn sqrt_price_x96_to_price(sqrt_price_x96: u128, dec0: u32, dec1: u32) -> Decimal {
    if sqrt_price_x96 == 0 {
        return Decimal::ZERO;
    }
    let s = sqrt_price_x96 as f64 / Q96_F64;
    let price_raw = s * s;
    let exp = (dec0 as i64 - dec1 as i64).clamp(-600, 600) as i32;
    let scale = 10f64.powi(exp);
    match dec_from_f64(price_raw * scale) {
        Some(v) if v > Decimal::ZERO => dq(v),
        _ => Decimal::ZERO,
    }
}

/// `sqrtPriceX96` → 最近的 tick（[`tick_to_sqrt_price_x96`] 的近似逆运算）。
///
/// 使用 f64 对数计算后四舍五入，仅用于校验与展示，不参与报价。
pub fn sqrt_price_x96_to_tick(sqrt_price_x96: u128) -> i32 {
    if sqrt_price_x96 == 0 {
        return MIN_TICK;
    }
    let s = sqrt_price_x96 as f64 / Q96_F64;
    let price = s * s;
    if !price.is_finite() || price <= 0.0 {
        return MIN_TICK;
    }
    let t = price.ln() / 1.0001f64.ln();
    if !t.is_finite() {
        return MAX_TICK;
    }
    (t.round() as i64).clamp(MIN_TICK as i64, MAX_TICK as i64) as i32
}

// ---------------------------------------------------------------------------
// StableSwap（Curve，n = 2）
// ---------------------------------------------------------------------------

/// 归一化后的 D 迭代（余额已缩放到 ≤ 1，避免 `D³` 溢出 Decimal）。
fn stableswap_d_normalized(x0: Decimal, x1: Decimal, amplification: Decimal) -> Decimal {
    let n = Decimal::from(2);
    if x0 <= Decimal::ZERO || x1 <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let s = match x0.checked_add(x1) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    if s <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    // Curve 的 A 必须 ≥ 1；非法/过小值按 1 处理。
    let amp = if amplification < Decimal::ONE {
        Decimal::ONE
    } else {
        amplification
    };
    let ann = match amp.checked_mul(n).and_then(|v| v.checked_mul(n)) {
        Some(v) => v, // Ann = A·n^n
        None => return Decimal::ZERO,
    };
    if ann <= Decimal::ONE {
        return Decimal::ZERO;
    }

    let mut d = s;
    for _ in 0..255 {
        // D_P = D · Π(D/(x_i·n))
        let d_p = d
            .checked_mul(d)
            .and_then(|v| v.checked_div(x0 * n))
            .and_then(|v| v.checked_mul(d))
            .and_then(|v| v.checked_div(x1 * n))
            .unwrap_or(Decimal::ZERO);
        if d_p <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let d_prev = d;
        let num = match ann
            .checked_mul(s)
            .and_then(|v| d_p.checked_mul(n).and_then(|w| w.checked_add(v)))
            .and_then(|v| v.checked_mul(d))
        {
            Some(v) => v,
            None => return Decimal::ZERO,
        };
        let den = match ann
            .checked_sub(Decimal::ONE)
            .and_then(|v| v.checked_mul(d))
            .and_then(|v| {
                d_p.checked_mul(n + Decimal::ONE)
                    .and_then(|w| w.checked_add(v))
            }) {
            Some(v) => v,
            None => return Decimal::ZERO,
        };
        if den <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        d = num / den;
        if d <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let tol = d / Decimal::from(CONVERGENCE_DENOM);
        if (d - d_prev).abs() <= tol {
            break;
        }
    }
    d
}

/// StableSwap 不变量 D（n = 2，Curve 定义：`Ann = A·n^n`）。
///
/// 求解 `Ann·Σx + D = Ann·D + D^(n+1)/(n^n·Πx)`，牛顿迭代最多 255 次，
/// 相对误差 1e-12 收敛。余额为 0 或参数非法时返回 0。
///
/// 内部先按最大余额归一化（D 是一次齐次函数），避免大储备下 `D³` 溢出。
pub fn stableswap_d(balances: [Decimal; 2], amplification: Decimal) -> Decimal {
    let [x0, x1] = balances;
    if x0 <= Decimal::ZERO || x1 <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let m = x0.max(x1);
    if m <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let d_norm = stableswap_d_normalized(x0 / m, x1 / m, amplification);
    if d_norm <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    match d_norm.checked_mul(m) {
        Some(v) => dq(v),
        None => Decimal::ZERO,
    }
}

/// 归一化坐标下的 get_y：给定输入侧新余额 `x_new`、输出侧余额 `x_other`、
/// 不变量 `d` 与 `ann`，反解输出侧的新余额 y。
fn stable_get_y(x_new: Decimal, x_other: Decimal, d: Decimal, ann: Decimal) -> Option<Decimal> {
    let n = Decimal::from(2);
    if d <= Decimal::ZERO
        || ann <= Decimal::ONE
        || x_new <= Decimal::ZERO
        || x_other <= Decimal::ZERO
    {
        return None;
    }
    // c = D · D/(x_new·n) · D/(Ann·n)，仅输入侧（输出侧 i == j 被跳过）
    let mut c = d;
    c = c.checked_mul(d)?.checked_div(x_new * n)?;
    c = c.checked_mul(d)?.checked_div(ann * n)?;
    let b = x_new.checked_add(d.checked_div(ann)?)?;

    let mut y = d;
    for _ in 0..255 {
        let den = n.checked_mul(y)?.checked_add(b)?.checked_sub(d)?;
        if den <= Decimal::ZERO {
            return None;
        }
        let y_prev = y;
        y = y.checked_mul(y)?.checked_add(c)?.checked_div(den)?;
        if y <= Decimal::ZERO {
            return None;
        }
        if (y - y_prev).abs() <= y / Decimal::from(CONVERGENCE_DENOM) {
            break;
        }
    }
    Some(y)
}

/// StableSwap exact-in 报价（Curve get_y 反解）。
///
/// 手续费从输入侧扣除（`aif = amount_in·(1-fee)`），再以 `x_in + aif` 反解输出侧
/// 余额；平价附近滑点极小（见测试）。参数非法、余额为 0、解出无效值时返回 0。
pub fn stableswap_amount_out(
    amount_in: Decimal,
    reserve_in: Decimal,
    reserve_out: Decimal,
    amplification: Decimal,
    fee_bps: i64,
) -> Decimal {
    if amount_in <= Decimal::ZERO || reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let aif = match amount_in.checked_mul(fee_multiplier(fee_bps)) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    if aif <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let m = reserve_in.max(reserve_out);
    if m <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let x_in_n = reserve_in / m;
    let x_out_n = reserve_out / m;
    let d = stableswap_d_normalized(x_in_n, x_out_n, amplification);
    if d <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let amp = if amplification < Decimal::ONE {
        Decimal::ONE
    } else {
        amplification
    };
    let ann = match amp
        .checked_mul(Decimal::from(2))
        .and_then(|v| v.checked_mul(Decimal::from(2)))
    {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let x_new = match reserve_in.checked_add(aif).and_then(|v| v.checked_div(m)) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let y = match stable_get_y(x_new, x_out_n, d, ann) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let out_n = x_out_n - y;
    if out_n <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    match out_n.checked_mul(m) {
        Some(v) => dq(v),
        None => Decimal::ZERO,
    }
}

// ---------------------------------------------------------------------------
// Balancer Weighted Pool
// ---------------------------------------------------------------------------

/// Balancer 加权池 exact-in：
/// `out = R_out · (1 - (R_in/(R_in + in_after_fee))^(w_in/w_out))`
///
/// 幂运算经 f64 `powf` 完成后再转 Decimal（相对误差约 1e-15，超过 1e15 量级时
/// 绝对误差会放大，见模块注释）。参数非法时返回 0。
pub fn weighted_amount_out(
    amount_in: Decimal,
    reserve_in: Decimal,
    reserve_out: Decimal,
    weight_in: Decimal,
    weight_out: Decimal,
    fee_bps: i64,
) -> Decimal {
    if amount_in <= Decimal::ZERO
        || reserve_in <= Decimal::ZERO
        || reserve_out <= Decimal::ZERO
        || weight_in <= Decimal::ZERO
        || weight_out <= Decimal::ZERO
    {
        return Decimal::ZERO;
    }
    let aif = match amount_in.checked_mul(fee_multiplier(fee_bps)) {
        Some(v) => v,
        None => return Decimal::ZERO,
    };
    let (ri, ro, ai, wi, wo) = match (
        dec_to_f64(reserve_in),
        dec_to_f64(reserve_out),
        dec_to_f64(aif),
        dec_to_f64(weight_in),
        dec_to_f64(weight_out),
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e)) => (a, b, c, d, e),
        _ => return Decimal::ZERO,
    };
    let denom = ri + ai;
    if denom <= 0.0 || !denom.is_finite() {
        return Decimal::ZERO;
    }
    let ratio = ri / denom;
    if !(ratio.is_finite() && ratio > 0.0) {
        return Decimal::ZERO;
    }
    let exp = wi / wo;
    if !exp.is_finite() {
        return Decimal::ZERO;
    }
    let out = ro * (1.0 - ratio.powf(exp));
    match dec_from_f64(out) {
        Some(v) if v > Decimal::ZERO => dq(v),
        _ => Decimal::ZERO,
    }
}

// ---------------------------------------------------------------------------
// 价格冲击
// ---------------------------------------------------------------------------

/// 价格冲击（bps）：`|exec - spot| / spot · 10000`。
///
/// 注意：`exec` 已包含手续费，因此该指标是“手续费 + 滑点”的综合偏离度
/// （与 [`crate::domain::market::Quote::price_impact_bps`] 语义一致）。
/// `spot <= 0` 时返回 0。
pub fn price_impact_bps(spot: Decimal, exec: Decimal) -> Decimal {
    if spot <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let diff = (exec - spot).abs();
    match diff.checked_div(spot) {
        Some(v) => dq(v * Decimal::from(BPS_DENOM)),
        None => Decimal::ZERO,
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::TickLiquidity;
    use crate::domain::money::dec;
    use chrono::Utc;

    fn rel_err(a: Decimal, b: Decimal) -> Decimal {
        if b.is_zero() {
            return if a.is_zero() {
                Decimal::ZERO
            } else {
                Decimal::MAX
            };
        }
        ((a - b) / b).abs()
    }

    fn assert_close(a: Decimal, b: Decimal, tol: &str, what: &str) {
        let e = rel_err(a, b);
        assert!(e < dec(tol), "{what}: a={a}, b={b}, rel_err={e} >= {tol}");
    }

    fn v3_state(
        sqrt_price_x96: u128,
        liquidity: u128,
        tick: i32,
        ticks: Vec<TickLiquidity>,
        fee_bps: i64,
    ) -> PoolState {
        PoolState {
            pool_id: "p".to_string(),
            block_number: 1,
            block_hash: String::new(),
            updated_at: Utc::now(),
            reserve0: Decimal::ZERO,
            reserve1: Decimal::ZERO,
            sqrt_price_x96,
            liquidity,
            tick,
            ticks,
            amplification: Decimal::ZERO,
            weight0: Decimal::ZERO,
            weight1: Decimal::ZERO,
            fee_bps,
        }
    }

    // ---- V2 -------------------------------------------------------------

    #[test]
    fn v2_known_value_with_fee() {
        // R_in = 1e6, R_out = 2e6, in = 1000, fee = 30bps
        // aif = 997, out = 997*2e6/(1e6+997) = 1992.01396188...
        let out = v2_amount_out(dec("1000"), dec("1000000"), dec("2000000"), 30);
        assert_close(out, dec("1992.0139619"), "0.0000001", "V2 含手续费手算值");
        // 手续费使输出严格小于无手续费结果
        let out0 = v2_amount_out(dec("1000"), dec("1000000"), dec("2000000"), 0);
        assert!(out < out0, "手续费必须减少输出: {out} vs {out0}");
    }

    #[test]
    fn v2_zero_fee_matches_formula() {
        // 10 * 2000 / (1000 + 10) = 19.8019801980198...
        let out = v2_amount_out(dec("10"), dec("1000"), dec("2000"), 0);
        assert_close(
            out,
            dec("19.80198019801980198"),
            "0.000000001",
            "V2 零手续费",
        );
    }

    #[test]
    fn v2_exact_out_is_inverse_of_exact_in() {
        let (r_in, r_out) = (dec("1000000"), dec("2000000"));

        // 已知输出 → 反解输入 → 再正向报价，应回到原输出
        // 该池价格 2:1，因此 500 输出只需约 250.8 输入（含 30bps 手续费）
        let want = dec("500");
        let need = v2_amount_in(want, r_in, r_out, 30);
        // 精确值：1e6*500/(2e6-500)/(1-0.003) = 250.81496051...
        assert_close(need, dec("250.8149605"), "0.000001", "V2 exact-out 反解");
        let back = v2_amount_out(need, r_in, r_out, 30);
        assert!(
            rel_err(back, want) < dec("0.000001"),
            "exact-out/exact-in 不互逆: {back} vs {want}"
        );

        // 已知输入 → 正向报价 → 反解，应回到原输入
        let amount_in = dec("1000");
        let got = v2_amount_out(amount_in, r_in, r_out, 30);
        let need2 = v2_amount_in(got, r_in, r_out, 30);
        assert!(
            rel_err(need2, amount_in) < dec("0.000001"),
            "exact-in/exact-out 不互逆: {need2} vs {amount_in}"
        );
    }

    #[test]
    fn v2_spot_and_guards() {
        assert_eq!(v2_spot_price(dec("1000"), dec("2500")), dec("2.5"));
        assert_eq!(v2_spot_price(Decimal::ZERO, dec("1")), Decimal::ZERO);
        // 除以零 / 负值 / 超额输出一律返回 0，不 panic
        assert_eq!(
            v2_amount_out(dec("1"), Decimal::ZERO, dec("100"), 30),
            Decimal::ZERO
        );
        assert_eq!(
            v2_amount_out(dec("1"), dec("100"), Decimal::ZERO, 30),
            Decimal::ZERO
        );
        assert_eq!(
            v2_amount_out(dec("-1"), dec("100"), dec("100"), 30),
            Decimal::ZERO
        );
        assert_eq!(
            v2_amount_in(dec("2000"), dec("1000"), dec("1000"), 30),
            Decimal::ZERO,
            "输出超出储备时必须返回 0"
        );
        // 100% 手续费 → 输出 0
        assert_eq!(
            v2_amount_out(dec("10"), dec("1000"), dec("1000"), 10_000),
            Decimal::ZERO
        );
    }

    #[test]
    fn v2_multi_hop_fee_compounds() {
        // 两跳各扣 30bps；储备放大到 1e9 使价格冲击 (~0.1bps/跳) 可忽略，
        // 从而损失应≈ 手续费复利 60bps 而非简单的 60bps 线性相加之外的滑点。
        let mid = v2_amount_out(dec("1000"), dec("1000000000"), dec("1000000000"), 30);
        let end = v2_amount_out(mid, dec("1000000000"), dec("1000000000"), 30);
        let total_fee = dec("1000") - end;
        let bps = total_fee / dec("1000") * Decimal::from(10_000);
        assert!(
            bps > dec("58") && bps < dec("62"),
            "两跳手续费应累计约 60bps，实际 {bps}"
        );
    }

    // ---- V3 -------------------------------------------------------------

    #[test]
    fn v3_single_range_fee_is_applied() {
        let sqrt_p = Q96; // 价格 1
        let l: u128 = 1_000_000_000_000_000_000_000_000; // 1e24
        let amount = dec("1000000000000000000"); // 1e18

        let (out_fee, new_sqrt_fee) = v3_amount_out_single_range(amount, sqrt_p, l, 30, true);
        let (out_zero, _) = v3_amount_out_single_range(amount, sqrt_p, l, 0, true);

        assert!(out_fee > Decimal::ZERO && out_fee < out_zero);
        // 手续费只从输入侧扣除：output 比例 ≈ 0.997（数量远小于 L 时曲率可忽略）
        let ratio = out_fee / out_zero;
        assert_close(ratio, dec("0.997"), "0.0001", "V3 单区间手续费比例");
        // zero_for_one 后价格必须下降
        assert!(new_sqrt_fee < sqrt_p, "zero_for_one 后 sqrtPrice 必须下降");
    }

    #[test]
    fn v3_price_moves_in_correct_direction() {
        let l: u128 = 1_000_000_000_000_000_000;
        let amount = dec("1000000000000000");
        let (out_down, sp_down) = v3_amount_out_single_range(amount, Q96, l, 30, true);
        let (out_up, sp_up) = v3_amount_out_single_range(amount, Q96, l, 30, false);
        assert!(out_down > Decimal::ZERO && out_up > Decimal::ZERO);
        assert!(sp_down < Q96, "卖出 token0 价格下行");
        assert!(sp_up > Q96, "卖出 token1 价格上行");
        // 对称流动性 + 对称价格 → 两个方向的输出应接近（差异仅来自曲率）
        assert_close(out_down, out_up, "0.01", "对称方向输出量级");
    }

    #[test]
    fn v3_boundaries_do_not_panic() {
        // 零流动性 / 零价格 / 零输入
        assert_eq!(
            v3_amount_out_single_range(dec("1"), 0, 1_000, 30, true),
            (Decimal::ZERO, 0)
        );
        assert_eq!(
            v3_amount_out_single_range(dec("1"), Q96, 0, 30, true),
            (Decimal::ZERO, Q96)
        );
        assert_eq!(
            v3_amount_out_single_range(Decimal::ZERO, Q96, 1_000, 30, true),
            (Decimal::ZERO, Q96)
        );
        // 极端 sqrtPrice / 流动性不得 panic
        let _ = v3_amount_out_single_range(dec("1"), u128::MAX, u128::MAX, 30, false);
        let _ = v3_amount_out_single_range(dec("1"), u128::MAX, u128::MAX, 30, true);
        let _ = v3_amount_out_single_range(dec("1"), 1, 1, 30, true);
        // 价格换算的溢出边界
        assert_eq!(sqrt_price_x96_to_price(0, 18, 6), Decimal::ZERO);
        assert_eq!(sqrt_price_x96_to_price(u128::MAX, 30, 0), Decimal::ZERO);
        let _ = sqrt_price_x96_to_price(u128::MAX, 0, 0);
        // tick 溢出截断
        assert!(tick_to_sqrt_price_x96(i32::MIN) > 0);
        assert!(tick_to_sqrt_price_x96(i32::MAX) > 0);
    }

    #[test]
    fn tick_and_price_roundtrip_within_0_01_percent() {
        for tick in [-200_000, -60_000, -1, 0, 1, 50_000, 200_000] {
            let sp = tick_to_sqrt_price_x96(tick);
            assert!(sp > 0, "tick {tick} 的 sqrtPrice 应 > 0");
            let price = sqrt_price_x96_to_price(sp, 0, 0);
            let expected = 1.0001f64.powi(tick);
            let got = price.to_f64().unwrap();
            let err = ((got - expected) / expected).abs();
            assert!(
                err < 1e-4,
                "tick {tick} 往返误差 {err} 超过 0.01%（price={price}, expected={expected}）"
            );
            // 反向：sqrtPrice → tick
            assert_eq!(sqrt_price_x96_to_tick(sp), tick, "tick {tick} 反解不一致");
        }
    }

    #[test]
    fn v3_quote_with_ticks_is_monotonic_and_crosses() {
        let liquidity: u128 = 1_000_000_000_000_000_000; // 1e18
        let ticks = vec![
            TickLiquidity {
                tick_index: -1000,
                liquidity_net: 500_000_000_000_000_000, // +5e17
                liquidity_gross: 500_000_000_000_000_000,
            },
            TickLiquidity {
                tick_index: -3000,
                liquidity_net: -1_000_000_000_000_000_000, // -1e18
                liquidity_gross: 1_000_000_000_000_000_000,
            },
        ];
        let state = v3_state(Q96, liquidity, 0, ticks, 30);

        let small = dec("10000000000000000"); // 1e16
        let mid = dec("100000000000000000"); // 1e17
        let big = dec("10000000000000000000"); // 1e19

        let (o_small, c_small) = v3_quote_with_ticks(&state, true, small);
        let (o_mid, c_mid) = v3_quote_with_ticks(&state, true, mid);
        let (o_big, c_big) = v3_quote_with_ticks(&state, true, big);

        assert!(o_small > Decimal::ZERO && o_mid > Decimal::ZERO && o_big > Decimal::ZERO);
        assert!(o_small < o_mid && o_mid < o_big, "输出必须随输入单调递增");
        assert_eq!(c_small, 0, "小额不应跨越 tick");
        assert!(c_mid >= 1, "1e17 应至少跨越一个 tick");
        assert!(c_big >= c_mid, "更大输入应跨越不少于中等输入的 tick 数");
        assert!(c_big <= 200, "最多处理 200 个 tick");
        // 每跳输出必然小于扣费后的输入
        assert!(o_big < dec("10000000000000000000"));
    }

    #[test]
    fn v3_liquidity_exhaustion_returns_partial() {
        // 在 -1000 处净额为 +L：下行穿越后流动性归零 → 必须停止并返回已成交部分
        let liquidity: u128 = 1_000_000_000_000_000_000;
        let ticks = vec![TickLiquidity {
            tick_index: -1000,
            liquidity_net: liquidity as i128,
            liquidity_gross: liquidity,
        }];
        let state = v3_state(Q96, liquidity, 0, ticks, 30);

        let (out, crossed) = v3_quote_with_ticks(&state, true, dec("100000000000000000000"));
        assert!(out > Decimal::ZERO, "流动性耗尽前必须成交一部分");
        assert_eq!(crossed, 1, "应只穿越 1 个 tick 后流动性归零");
        // 全部流动性只存在于 tick 0 到 -1000 之间：输出上限 = L*(1 - 1.0001^(-500))
        // ≈ 1e18 * 0.04877 = 4.877e16；耗尽后不得继续输出。
        let max_out = dec("49000000000000000"); // 4.9e16
        assert!(
            out < max_out,
            "流动性耗尽后不应继续输出: {out} >= {max_out}"
        );
        assert!(
            out > dec("48000000000000000"),
            "应成交该区间几乎全部可用流动性: {out}"
        );
    }

    #[test]
    fn v3_empty_ticks_is_single_range() {
        let state = v3_state(Q96, 1_000_000_000_000_000_000, 0, vec![], 30);
        let (out, crossed) = v3_quote_with_ticks(&state, true, dec("1000000000000000"));
        let (direct, _) = v3_amount_out_single_range(
            dec("1000000000000000"),
            Q96,
            1_000_000_000_000_000_000,
            30,
            true,
        );
        assert_eq!(crossed, 0);
        assert_eq!(out, direct, "无 tick 时必须退化为单区间公式");
    }

    // ---- StableSwap -----------------------------------------------------

    #[test]
    fn stableswap_d_balanced_and_imbalanced() {
        let d = stableswap_d([dec("1000"), dec("1000")], dec("100"));
        assert_close(d, dec("2000"), "0.0000001", "平价池 D 应等于总余额");
        // 不均衡时 D 略小于总余额（恒定和近似）
        let d2 = stableswap_d([dec("1000"), dec("500")], dec("100"));
        assert!(d2 < dec("1500") && d2 > dec("1490"), "D(1000,500) = {d2}");
        // 非法输入
        assert_eq!(
            stableswap_d([Decimal::ZERO, dec("1")], dec("100")),
            Decimal::ZERO
        );
        assert_eq!(
            stableswap_d([dec("1"), dec("1")], Decimal::ZERO),
            dec("2"),
            "A 非法时按 1 处理；平价池 D 恒等于总余额"
        );
    }

    #[test]
    fn stableswap_near_parity_loss_below_1bps_beyond_fee() {
        // 1,000,000 USDC / 1,000,000 USDT，A=100，fee=4bps，换 1000
        let out = stableswap_amount_out(dec("1000"), dec("1000000"), dec("1000000"), dec("100"), 4);
        let after_fee = dec("999.6");
        assert!(out > Decimal::ZERO);
        assert!(out < after_fee, "输出必须小于扣费后的名义金额: {out}");
        let loss = after_fee - out;
        let loss_bps = loss / dec("1000") * Decimal::from(10_000);
        assert!(
            loss_bps < dec("1"),
            "手续费之外的滑点必须 < 1bps，实际 {loss_bps}bps（out={out}）"
        );
        // 手续费本身必须被扣除
        assert!(out < dec("1000"), "手续费必须计入");
    }

    #[test]
    fn stableswap_extreme_imbalance_below_linear() {
        // 1e6 : 1e4 极端不均衡，卖出充裕侧 1000 → 输出远低于 1:1 线性比例
        let out = stableswap_amount_out(dec("1000"), dec("1000000"), dec("10000"), dec("100"), 4);
        assert!(out > Decimal::ZERO, "不均衡池仍应给出正输出: {out}");
        assert!(
            out < dec("999.6"),
            "极端不均衡时输出必须低于 1:1 线性比例（含手续费上界）: {out}"
        );
        assert!(out < dec("500"), "极端不均衡必须产生显著滑点: {out}");
        // 与平价池同一输入比较：不均衡池输出更少
        let balanced =
            stableswap_amount_out(dec("1000"), dec("1000000"), dec("1000000"), dec("100"), 4);
        assert!(
            out < balanced,
            "不均衡池输出应低于平价池: {out} vs {balanced}"
        );
    }

    #[test]
    fn stableswap_guards() {
        assert_eq!(
            stableswap_amount_out(dec("1000"), Decimal::ZERO, dec("1000"), dec("100"), 4),
            Decimal::ZERO
        );
        assert_eq!(
            stableswap_amount_out(dec("0"), dec("1000"), dec("1000"), dec("100"), 4),
            Decimal::ZERO
        );
        // 100% 手续费 → 0
        assert_eq!(
            stableswap_amount_out(dec("1000"), dec("1e6"), dec("1e6"), dec("100"), 10_000),
            Decimal::ZERO
        );
        // A 非法（0）按 1 处理，仍不得 panic
        let out = stableswap_amount_out(dec("10"), dec("1e6"), dec("1e6"), Decimal::ZERO, 0);
        assert!(out > Decimal::ZERO);
    }

    // ---- Weighted -------------------------------------------------------

    #[test]
    fn weighted_50_50_degrades_to_v2() {
        let (r_in, r_out) = (dec("1000000"), dec("1000000"));
        let amount = dec("1000");
        let w = dec("0.5");
        for fee in [0i64, 30] {
            let v2 = v2_amount_out(amount, r_in, r_out, fee);
            let wp = weighted_amount_out(amount, r_in, r_out, w, w, fee);
            assert_close(wp, v2, "0.000001", "50/50 权重应退化为 V2");
        }
    }

    #[test]
    fn weighted_80_20_differs_and_is_positive() {
        let out = weighted_amount_out(
            dec("1000"),
            dec("1000000"),
            dec("1000000"),
            dec("0.8"),
            dec("0.2"),
            30,
        );
        assert!(out > Decimal::ZERO);
        // 现货价 = (R_out/w_out)·(w_in/R_in) = 4（储备相等、权重 80/20）
        // 因此 1 单位高权重资产应换出约 4 单位低权重资产，明显多于 V2 的 1:1。
        let v2 = v2_amount_out(dec("1000"), dec("1000000"), dec("1000000"), 30);
        assert!(out > v2, "卖出高权重资产应得到更多输出: {out} vs {v2}");
        let exec = out / dec("1000");
        assert!(
            exec > dec("3.9") && exec < dec("4.0"),
            "80/20 权重下成交均价应接近 4: {exec}"
        );
        // 反向：买入低权重资产（w_in/w_out = 0.25）输出显著更少
        let rev = weighted_amount_out(
            dec("1000"),
            dec("1000000"),
            dec("1000000"),
            dec("0.2"),
            dec("0.8"),
            30,
        );
        assert!(rev > Decimal::ZERO && rev < v2, "反向权重输出应更少: {rev}");
        // 权重非法 → 0
        assert_eq!(
            weighted_amount_out(
                dec("1000"),
                dec("1000000"),
                dec("1000000"),
                Decimal::ZERO,
                dec("0.5"),
                30
            ),
            Decimal::ZERO
        );
    }

    #[test]
    fn weighted_guards() {
        assert_eq!(
            weighted_amount_out(
                dec("1"),
                Decimal::ZERO,
                dec("1"),
                dec("0.5"),
                dec("0.5"),
                30
            ),
            Decimal::ZERO
        );
        assert_eq!(
            weighted_amount_out(dec("0"), dec("1"), dec("1"), dec("0.5"), dec("0.5"), 30),
            Decimal::ZERO
        );
    }

    // ---- 价格冲击 -------------------------------------------------------

    #[test]
    fn price_impact_math() {
        assert_eq!(price_impact_bps(dec("100"), dec("99.9")), dec("10"));
        assert_eq!(price_impact_bps(dec("100"), dec("100.1")), dec("10"));
        assert_eq!(price_impact_bps(Decimal::ZERO, dec("100")), Decimal::ZERO);
        // 手续费带来的偏离也被计入（与 exec/spot 定义一致）
        assert_eq!(price_impact_bps(dec("1"), dec("0.997")), dec("30"));
    }
}
