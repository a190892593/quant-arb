//! 统一成本模型（TDD §6.2）。
//!
//! 净利润 = 实际回款价值 − 实际投入价值 − DEX 费用 − CEX 费用 − Gas − 闪电贷费用
//!          − Builder/优先费 − 滑点 − 对冲补偿成本 − 库存再平衡成本 − 其他执行成本
//!
//! 所有成本项以 USDC 计价，明细逐项可加总；合计由明细求和得出，不写死（DESIGN.md §8）。

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::domain::market::{CostBreakdown, CostItem, ProfitScenario};

/// 一笔交易的成本输入（全部为 USDC 计价的正数成本）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CostParts {
    pub dex_fee: Decimal,
    pub cex_fee: Decimal,
    pub gas: Decimal,
    pub priority_fee: Decimal,
    pub flash_fee: Decimal,
    pub price_impact: Decimal,
    pub slippage: Decimal,
    pub delay_reserve: Decimal,
    pub hedge_compensation: Decimal,
    pub rebalance: Decimal,
    pub mev_reserve: Decimal,
    pub other: Decimal,
}

impl CostParts {
    pub fn new() -> Self {
        Self::default()
    }

    /// 明细求和。
    pub fn total(&self) -> Decimal {
        self.dex_fee
            + self.cex_fee
            + self.gas
            + self.priority_fee
            + self.flash_fee
            + self.price_impact
            + self.slippage
            + self.delay_reserve
            + self.hedge_compensation
            + self.rebalance
            + self.mev_reserve
            + self.other
    }

    /// 转为可展示的成本明细，key 与前端成本构成一一对应。
    pub fn breakdown(&self) -> CostBreakdown {
        let mut b = CostBreakdown::new();
        b.push("dex_fee", "DEX 手续费", self.dex_fee);
        b.push("cex_fee", "CEX 手续费", self.cex_fee);
        b.push("gas", "Gas", self.gas);
        b.push("priority_fee", "优先费/Builder", self.priority_fee);
        b.push("flash_fee", "闪电贷费用", self.flash_fee);
        b.push("price_impact", "价格冲击", self.price_impact);
        b.push("slippage", "滑点", self.slippage);
        b.push("delay_reserve", "延迟风险准备金", self.delay_reserve);
        b.push(
            "hedge_compensation",
            "对冲补偿准备金",
            self.hedge_compensation,
        );
        b.push("rebalance", "库存再平衡", self.rebalance);
        b.push("mev_reserve", "抢先/夹击准备金", self.mev_reserve);
        b.push("other", "其他执行成本", self.other);
        b
    }
}

/// 由明细反查 key 金额（用于净利重算）。
pub fn cost_of(breakdown: &CostBreakdown, key: &str) -> Decimal {
    breakdown.get(key)
}

/// 成本明细的稳定排序（按金额降序），用于前端表格。
pub fn sorted_items(breakdown: &CostBreakdown) -> Vec<CostItem> {
    let mut items = breakdown.items.clone();
    items.sort_by(|a, b| b.amount.cmp(&a.amount));
    items
}

/// 构造四层盈亏推演。
///
/// - `theoretical_usd`：理论价差（M0），仅用于对比；
/// - `gross_usd`：按真实池状态/盘口深度计算的可成交毛利；
/// - `costs`：全部成本；
/// - `discount_pct`：保守折扣（风控要求至少测试 50%）；
/// - `worst_case_loss_usd`：最坏情景损失（正数）。
pub fn build_scenario(
    theoretical_usd: Decimal,
    gross_usd: Decimal,
    costs: &CostParts,
    discount_pct: Decimal,
    worst_case_loss_usd: Decimal,
) -> ProfitScenario {
    let net = gross_usd - costs.total();
    let discount = (Decimal::from(100) - discount_pct) / Decimal::from(100);
    // 压力模型：毛利按折扣后计算，成本按 100% 计入（成本不会因为行情变差而减少）
    let stress = gross_usd * discount - costs.total();
    ProfitScenario {
        theoretical_usd: q(theoretical_usd),
        executable_gross_usd: q(gross_usd),
        total_cost_usd: q(costs.total()),
        conservative_net_usd: q(net),
        stress_net_usd: q(stress),
        worst_case_loss_usd: q(worst_case_loss_usd),
    }
}

/// 最坏情景损失估算：价格按 `adverse_bps` 反向、成本全额、部分成交后被迫补偿。
pub fn worst_case_loss(
    gross_usd: Decimal,
    costs: &CostParts,
    adverse_bps: Decimal,
    notional: Decimal,
) -> Decimal {
    let adverse = notional * adverse_bps / Decimal::from(10_000);
    // 最坏情况下：毛利归零，承担全部成本 + 逆向价格变动 + 补偿
    (costs.total() + adverse).max(Decimal::ZERO)
}

/// 按 bps 计算 USD 金额。
pub fn usd_bps(notional: Decimal, bps: Decimal) -> Decimal {
    notional * bps / Decimal::from(10_000)
}

fn q(v: Decimal) -> Decimal {
    v.round_dp(6)
}

/// 成本占比（bps，相对名义金额）。
pub fn cost_bps(costs: &CostParts, notional: Decimal) -> Decimal {
    if notional.is_zero() {
        return Decimal::ZERO;
    }
    costs.total() / notional * Decimal::from(10_000)
}

/// 成本按 key 汇总（用于绩效页面「收益来自哪里」）。
pub fn aggregate_costs(breakdowns: &[CostBreakdown]) -> BTreeMap<String, Decimal> {
    let mut map: BTreeMap<String, Decimal> = BTreeMap::new();
    for b in breakdowns {
        for item in &b.items {
            *map.entry(item.key.clone()).or_insert(Decimal::ZERO) += item.amount;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    fn parts() -> CostParts {
        CostParts {
            dex_fee: dec("1.2"),
            cex_fee: dec("0.8"),
            gas: dec("0.35"),
            flash_fee: dec("0.5"),
            delay_reserve: dec("0.6"),
            hedge_compensation: dec("0.4"),
            ..Default::default()
        }
    }

    #[test]
    fn total_equals_sum_of_items() {
        let p = parts();
        let b = p.breakdown();
        assert_eq!(b.total(), p.total());
        assert_eq!(p.total(), dec("3.85"));
    }

    #[test]
    fn scenario_is_conservative() {
        let p = parts();
        let s = build_scenario(dec("12"), dec("10"), &p, dec("50"), dec("6"));
        assert_eq!(s.executable_gross_usd, dec("10"));
        assert_eq!(s.total_cost_usd, dec("3.85"));
        assert_eq!(s.conservative_net_usd, dec("6.15"));
        // 压力模型下毛利减半、成本不变
        assert_eq!(s.stress_net_usd, dec("1.15"));
        assert!(s.stress_net_usd < s.conservative_net_usd);
    }

    #[test]
    fn stress_can_be_negative() {
        let p = parts();
        let s = build_scenario(dec("5"), dec("4"), &p, dec("50"), dec("8"));
        assert!(s.stress_net_usd < Decimal::ZERO);
        assert_eq!(s.worst_case_loss_usd, dec("8"));
    }

    #[test]
    fn worst_case_includes_adverse_move() {
        let p = parts();
        let loss = worst_case_loss(dec("10"), &p, dec("20"), dec("1000"));
        // 3.85 成本 + 1000*20bps=2 逆向 = 5.85
        assert_eq!(loss, dec("5.85"));
    }

    #[test]
    fn aggregate_by_key() {
        let p = parts();
        let map = aggregate_costs(&[p.breakdown(), p.breakdown()]);
        assert_eq!(map.get("gas"), Some(&dec("0.7")));
        assert_eq!(map.get("flash_fee"), Some(&dec("1.0")));
    }
}
