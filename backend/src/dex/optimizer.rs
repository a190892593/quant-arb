//! 投入金额优化（PRD §5.4）。
//!
//! AMM 输出不是线性的，最优投入不能由固定价差反推。本模块按以下顺序做有界数值搜索：
//! 1. 配置档位（250–10,000 USDC 等）；
//! 2. 对数均匀粗网格（覆盖数量级）；
//! 3. 在最优采样点两侧做黄金分割细化（有界迭代）；
//! 4. 按整数精度重新量化并复算；
//! 5. 输出最小 / 最优 / 最大可盈利投入与完整采样曲线。
//!
//! 明确**不**假定目标函数全局单峰：网格负责发现多个峰，细化只作用于最优峰所在区间；
//! 结果中的 min/max 是采样意义下的边界，受网格分辨率约束，并在 notes 中说明。

use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::BTreeMap;

use crate::domain::dex_scan::{InputOptimization, InputPoint};

/// 优化参数。
#[derive(Clone, Debug)]
pub struct OptimizationParams {
    /// 档位金额（起始 Token 单位），来自配置与风控测试档位。
    pub ladder_amounts: Vec<Decimal>,
    pub min_amount: Decimal,
    pub max_amount: Decimal,
    pub grid_points: usize,
    pub refine_iters: u32,
    /// 数量精度步长（例如 1e-8）；结果全部按该步长向下取整。
    pub quantity_step: Decimal,
}

/// 对某一金额的经济性评估。成本模型由调用方提供（不同路线成本不同）。
#[derive(Clone, Debug)]
pub struct AmountEvaluation {
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    /// 名义金额（USDC），由引擎按起始资产价格给出。
    pub notional_usd: Decimal,
    pub gross_usd: Decimal,
    pub cost_usd: Decimal,
    pub net_usd: Decimal,
    pub gas_usd: Decimal,
    pub price_impact_bps: Decimal,
}

/// 执行优化。eval 返回 None 表示该金额不可报价（例如超出池容量）。
pub fn optimize<F>(params: &OptimizationParams, mut eval: F) -> InputOptimization
where
    F: FnMut(Decimal) -> Option<AmountEvaluation>,
{
    let step = if params.quantity_step > Decimal::ZERO {
        params.quantity_step
    } else {
        Decimal::new(1, 8)
    };
    let min = quantize(params.min_amount.max(step), step).max(step);
    let max = quantize(params.max_amount.max(min), step);
    let mut notes = Vec::new();
    let mut truncated = false;

    // 1) 候选金额：档位 + 对数网格 + 边界。
    let mut amounts: BTreeMap<Decimal, &'static str> = BTreeMap::new();
    for a in &params.ladder_amounts {
        let q = quantize(*a, step);
        if q >= min && q <= max {
            amounts.insert(q, "ladder");
        }
    }
    let grid = params.grid_points.clamp(2, 64);
    for i in 0..grid {
        let t = i as f64 / (grid - 1) as f64;
        let amount = log_lerp(min, max, t);
        let q = quantize(amount, step);
        if q >= min && q <= max {
            amounts.insert(q, "grid");
        }
    }
    amounts.insert(min, "grid");
    amounts.insert(max, "grid");

    // 2) 采样并记录。
    let mut points: Vec<InputPoint> = Vec::with_capacity(amounts.len() + params.refine_iters as usize);
    let mut seen_amounts: BTreeMap<Decimal, usize> = BTreeMap::new();
    for (amount, source) in amounts {
        if let Some(p) = evaluate(&mut eval, amount, source) {
            seen_amounts.insert(amount, points.len());
            points.push(p);
        }
    }

    let mut best = best_of(&points).cloned();
    let mut best_amount = best.as_ref().map(|p| p.amount_in);

    // 3) 在有盈利采样点附近做黄金分割细化。
    if let Some(b) = best_amount {
        let (lo, hi) = bracket(&points, b, min, max);
        if hi > lo {
            let (probes, refined_best) = golden_section(
                &mut eval,
                lo,
                hi,
                step,
                params.refine_iters,
            );
            for p in probes {
                if let Some(&idx) = seen_amounts.get(&p.amount_in) {
                    if p.net_usd > points[idx].net_usd {
                        points[idx] = p;
                    }
                } else {
                    seen_amounts.insert(p.amount_in, points.len());
                    points.push(p);
                }
            }
            if let Some(p) = refined_best {
                if best.as_ref().map(|b| p.net_usd > b.net_usd).unwrap_or(true) {
                    best = Some(p);
                    best_amount = Some(b);
                }
            }
        } else {
            notes.push("最优采样点位于边界，无法进一步细化".to_string());
        }
    }

    // 4) 最终按整数精度复算最优金额。
    if let Some(amount) = best_amount {
        let q = quantize(amount, step);
        if let Some(point) = evaluate(&mut eval, q, "best") {
            if best.as_ref().map(|b| point.net_usd >= b.net_usd).unwrap_or(true) {
                best = Some(point);
            }
        }
    }

    // 5) 汇总边界。
    let mut positive: Vec<&InputPoint> = points.iter().filter(|p| p.net_usd > Decimal::ZERO).collect();
    positive.sort_by(|a, b| a.amount_in.cmp(&b.amount_in));
    let min_profitable = positive.first().map(|p| p.amount_in);
    let max_profitable = positive.last().map(|p| p.amount_in);
    let max_executable = max_profitable.unwrap_or_else(|| best_amount.unwrap_or(min)).min(max);

    if params.refine_iters == 0 {
        notes.push("未启用细化，仅使用档位与网格".to_string());
    }
    if params.grid_points > 64 {
        truncated = true;
        notes.push("网格点超过 64，已截断".to_string());
    }
    if points.is_empty() {
        notes.push("所有候选金额均无法报价".to_string());
    }

    points.sort_by(|a, b| a.amount_in.cmp(&b.amount_in));
    InputOptimization {
        points,
        best,
        min_profitable_amount: min_profitable,
        max_profitable_amount: max_profitable,
        max_executable_amount: max_executable,
        grid_points: grid,
        refine_iters: params.refine_iters as usize,
        truncated,
        notes,
    }
}

fn evaluate<F>(eval: &mut F, amount: Decimal, source: &'static str) -> Option<InputPoint>
where
    F: FnMut(Decimal) -> Option<AmountEvaluation>,
{
    if amount <= Decimal::ZERO {
        return None;
    }
    let e = eval(amount)?;
    let amount_in_usd = e.notional_usd;
    let net_bps = if amount_in_usd > Decimal::ZERO {
        e.net_usd / amount_in_usd * Decimal::from(10_000)
    } else {
        Decimal::ZERO
    };
    let gas_share = if amount_in_usd > Decimal::ZERO {
        e.gas_usd / amount_in_usd * Decimal::from(100)
    } else {
        Decimal::ZERO
    };
    Some(InputPoint {
        amount_in: e.amount_in,
        amount_in_usd,
        amount_out: e.amount_out,
        gross_usd: e.gross_usd,
        cost_usd: e.cost_usd,
        net_usd: e.net_usd,
        net_bps,
        gas_usd: e.gas_usd,
        gas_share_pct: gas_share,
        price_impact_bps: e.price_impact_bps,
        source: source.to_string(),
    })
}

fn best_of(points: &[InputPoint]) -> Option<&InputPoint> {
    points
        .iter()
        .filter(|p| p.net_usd > Decimal::ZERO)
        .max_by(|a, b| a.net_usd.cmp(&b.net_usd))
}

/// 最优采样点左右相邻点构成的细化区间（比例兜底）。
fn bracket(points: &[InputPoint], best: Decimal, min: Decimal, max: Decimal) -> (Decimal, Decimal) {
    let mut lower = min;
    let mut upper = max;
    for p in points {
        if p.amount_in < best {
            lower = lower.max(p.amount_in);
        } else if p.amount_in > best && p.amount_in < upper {
            upper = p.amount_in;
        }
    }
    if upper <= lower {
        lower = (best / Decimal::from(2)).max(min);
        upper = (best * Decimal::from(2)).min(max);
    }
    (lower, upper)
}

fn golden_section<F>(
    eval: &mut F,
    lo: Decimal,
    hi: Decimal,
    step: Decimal,
    iters: u32,
) -> (Vec<InputPoint>, Option<InputPoint>)
where
    F: FnMut(Decimal) -> Option<AmountEvaluation>,
{
    let ratio = Decimal::new(618_033_988_749_895, 15); // 0.618033988749895
    let mut a = lo;
    let mut b = hi;
    let mut out: Vec<InputPoint> = Vec::new();
    let mut best: Option<InputPoint> = None;
    let mut seen: BTreeMap<Decimal, ()> = BTreeMap::new();

    for _ in 0..iters {
        if b <= a {
            break;
        }
        let span = b - a;
        let c = b - span * ratio;
        let d = a + span * ratio;
        let pc = eval_point(eval, c, step);
        let pd = eval_point(eval, d, step);
        for p in [pc.clone(), pd.clone()].into_iter().flatten() {
            if seen.insert(p.amount_in, ()).is_none() {
                if best.as_ref().map(|b| p.net_usd > b.net_usd).unwrap_or(true) {
                    best = Some(p.clone());
                }
                out.push(p);
            }
        }
        match (pc, pd) {
            (Some(cp), Some(dp)) => {
                if cp.net_usd > dp.net_usd {
                    b = d;
                } else {
                    a = c;
                }
            }
            (Some(_), None) => b = d,
            (None, Some(_)) => a = c,
            (None, None) => break,
        }
    }

    (out, best)
}

fn eval_point<F>(eval: &mut F, amount: Decimal, step: Decimal) -> Option<InputPoint>
where
    F: FnMut(Decimal) -> Option<AmountEvaluation>,
{
    let q = quantize(amount, step);
    if q <= Decimal::ZERO {
        return None;
    }
    evaluate(eval, q, "refine")
}

fn quantize(v: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return v;
    }
    (v / step).floor() * step
}

/// 在 [min, max] 上按对数均匀插值（金额跨度通常 1–3 个数量级）。
fn log_lerp(min: Decimal, max: Decimal, t: f64) -> Decimal {
    let lo = min.to_f64().unwrap_or(1.0).max(1e-12);
    let hi = max.to_f64().unwrap_or(lo).max(lo);
    let v = (lo.ln() + (hi.ln() - lo.ln()) * t).exp();
    Decimal::from_f64(v).unwrap_or(min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    /// 合成经济性：5% 价差减去二次冲击，解析最优 = 2500。
    fn eval(amount: Decimal) -> Option<AmountEvaluation> {
        let out = amount * dec("1.05") - amount * amount / Decimal::from(100_000);
        if out <= Decimal::ZERO {
            return None;
        }
        let gross = out - amount;
        let gross_usd = gross * dec("1");
        let cost = dec("0.5");
        Some(AmountEvaluation {
            amount_in: amount,
            amount_out: out,
            notional_usd: amount,
            gross_usd,
            cost_usd: cost,
            net_usd: gross_usd - cost,
            gas_usd: cost,
            price_impact_bps: Decimal::ZERO,
        })
    }

    fn params() -> OptimizationParams {
        OptimizationParams {
            ladder_amounts: vec![dec("250"), dec("500"), dec("1000"), dec("2500")],
            min_amount: dec("100"),
            max_amount: dec("10000"),
            grid_points: 16,
            refine_iters: 20,
            quantity_step: dec("0.0001"),
        }
    }

    #[test]
    fn finds_near_analytic_optimum() {
        let r = optimize(&params(), eval);
        let best = r.best.expect("应有最优金额");
        assert!(best.amount_in > dec("2300") && best.amount_in < dec("2700"), "{best:?}");
        assert!(best.net_usd > Decimal::ZERO);
        assert!(r.min_profitable_amount.is_some());
        assert!(r.max_profitable_amount.is_some());
        assert!(r.max_executable_amount <= dec("10000"));
        assert!(!r.points.is_empty());
        // 采样曲线按金额升序
        assert!(r.points.windows(2).all(|w| w[0].amount_in <= w[1].amount_in));
        // 量化步长
        for p in &r.points {
            assert_eq!(p.amount_in, quantize(p.amount_in, dec("0.0001")));
        }
    }

    #[test]
    fn respects_max_bound() {
        let mut p = params();
        p.max_amount = dec("600");
        let r = optimize(&p, eval);
        let best = r.best.unwrap();
        assert!(best.amount_in <= dec("600"));
        assert!(best.amount_in > dec("400"));
    }

    #[test]
    fn no_profit_yields_no_best() {
        let p = params();
        let r = optimize(&p, |amount| {
            Some(AmountEvaluation {
                amount_in: amount,
                amount_out: amount,
                notional_usd: amount,
                gross_usd: Decimal::ZERO,
                cost_usd: dec("1"),
                net_usd: dec("-1"),
                gas_usd: dec("1"),
                price_impact_bps: Decimal::ZERO,
            })
        });
        assert!(r.best.is_none());
        assert!(r.min_profitable_amount.is_none());
        assert!(r.max_profitable_amount.is_none());
        assert!(!r.points.is_empty());
    }

    #[test]
    fn unquotable_amounts_are_skipped() {
        let p = params();
        let r = optimize(&p, |amount| if amount > dec("3000") { None } else { eval(amount) });
        assert!(r.points.iter().all(|p| p.amount_in <= dec("3000")));
        assert!(r.best.is_some());
    }

    #[test]
    fn deterministic() {
        let a = optimize(&params(), eval);
        let b = optimize(&params(), eval);
        assert_eq!(a.points.len(), b.points.len());
        assert_eq!(
            a.best.as_ref().map(|p| p.amount_in),
            b.best.as_ref().map(|p| p.amount_in)
        );
    }

    #[test]
    fn notes_explain_method() {
        let mut p = params();
        p.refine_iters = 0;
        let r = optimize(&p, eval);
        assert!(r.notes.iter().any(|n| n.contains("未启用细化")));
    }

    #[test]
    fn log_lerp_hits_bounds() {
        assert_eq!(log_lerp(dec("1"), dec("1000"), 0.0), dec("1"));
        let end = log_lerp(dec("1"), dec("1000"), 1.0);
        assert!(end > dec("999") && end < dec("1001"));
        let mid = log_lerp(dec("1"), dec("1000"), 0.5);
        assert!(mid > dec("30") && mid < dec("32"));
    }
}
