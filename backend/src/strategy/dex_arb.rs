//! DEX 原子套利搜索：三角闭环与跨池闭环（PRD §2.2、§2.3、TDD §4.2）。
//!
//! 搜索流程：
//! 1. 枚举同链、不同池、回到起始资产的环；
//! 2. 按真实池状态逐跳精确报价（不拿现货价近似成交价）；
//! 3. 扣除 DEX 手续费、Gas、闪电贷费用、价格冲击、延迟/抢先准备金；
//! 4. 生成统一机会对象，交由风控引擎判定是否可执行。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use crate::domain::market::{DataQuality, DexRoute, ModelLevel, RouteLeg, Token};
use crate::domain::money::{bps_of, bps_of_dec, q8, round_step_down};
use crate::domain::opportunity::{FundingSource, InventoryNeed, Opportunity};
use crate::domain::plan::fingerprint;
use crate::domain::types::{OppStatus, StrategyType};
use crate::dex::{graph, pool};

use super::cost::{build_scenario, usd_bps, worst_case_loss, CostParts};
use super::{DexCostParams, MarketView};

/// 一条经过精确报价的候选路线。
#[derive(Clone, Debug)]
pub struct DexCandidate {
    pub edges: Vec<graph::HopEdge>,
    pub path: pool::PathQuote,
    pub gross_usd: Decimal,
    pub theoretical_edge_bps: Decimal,
    pub costs: CostParts,
    pub net_usd: Decimal,
    pub capacity_usd: Decimal,
}

/// 在给定规模下搜索三角与跨池机会，返回已填充全部成本的机会对象。
pub fn scan(
    view: &MarketView<'_>,
    params: &DexCostParams,
    base_symbols: &[&str],
    notional_usd: Decimal,
    available: &HashMap<String, Decimal>,
) -> Vec<Opportunity> {
    let mut best: HashMap<String, (StrategyType, DexCandidate)> = HashMap::new();

    for base in base_symbols {
        let Some(base_price) = view.prices.get(*base).copied() else {
            continue;
        };
        if base_price <= Decimal::ZERO || notional_usd <= Decimal::ZERO {
            continue;
        }
        let amount_in = q8(notional_usd / base_price);
        if amount_in <= Decimal::ZERO {
            continue;
        }

        let mut cycles: Vec<Vec<graph::HopEdge>> =
            graph::enumerate_triangles(view.pools, base, params.max_hops.max(3));
        cycles.extend(graph::enumerate_cross_pool_cycles(view.pools, base));

        for edges in cycles {
            if edges.len() < 2 {
                continue;
            }
            let strategy = if edges.len() >= 3 {
                StrategyType::DexTriple
            } else {
                StrategyType::DexCross
            };
            let Some(cand) =
                evaluate_cycle(view, params, base, amount_in, base_price, notional_usd, edges)
            else {
                continue;
            };
            let key = cand
                .edges
                .iter()
                .map(|e| format!("{}:{}>{}", e.pool_id, e.token_in, e.token_out))
                .collect::<Vec<_>>()
                .join("|");
            match best.get(&key) {
                Some((_, existing)) if existing.net_usd >= cand.net_usd => {}
                _ => {
                    best.insert(key, (strategy, cand));
                }
            }
        }
    }

    best.into_values()
        .map(|(strategy, cand)| {
            to_opportunity(view, params, &cand, strategy, available)
        })
        .collect()
}

/// 对一条环做精确报价与成本计算。
fn evaluate_cycle(
    view: &MarketView<'_>,
    params: &DexCostParams,
    base: &str,
    amount_in: Decimal,
    base_price: Decimal,
    notional_usd: Decimal,
    edges: Vec<graph::HopEdge>,
) -> Option<DexCandidate> {
    // 池不得重复使用；首尾必须闭环
    let mut seen = HashSet::new();
    for e in &edges {
        if !seen.insert(e.pool_id.clone()) {
            return None;
        }
    }
    if edges.first()?.token_in != base || edges.last()?.token_out != base {
        return None;
    }

    let path = pool::quote_path(view.pools, base, amount_in, &edges).ok()?;
    if path.amount_out <= Decimal::ZERO {
        return None;
    }

    let gross_usd = (path.amount_out - path.amount_in) * base_price;
    let theoretical_edge_bps = theoretical_cycle_bps(view, &edges)?;

    let mut costs = CostParts::new();
    let mut dex_fee_usd = Decimal::ZERO;
    for leg in &path.legs {
        let px = view.prices.get(&leg.token_in).copied().unwrap_or(Decimal::ZERO);
        dex_fee_usd += bps_of(leg.amount_in * px, leg.fee_bps);
    }
    costs.dex_fee = dex_fee_usd;
    costs.gas = gas_usd(params, path.gas_units);
    costs.priority_fee = params.priority_fee_usd;
    costs.flash_fee = bps_of_dec(notional_usd, params.flash_fee_bps);
    costs.price_impact = usd_bps(notional_usd, path.price_impact_bps);
    costs.delay_reserve = usd_bps(notional_usd, params.delay_reserve_bps);
    costs.mev_reserve = usd_bps(notional_usd, params.mev_reserve_bps);

    let net_usd = gross_usd - costs.total();
    let capacity_usd = capacity_for(view, &path, notional_usd);

    Some(DexCandidate {
        edges,
        path,
        gross_usd,
        theoretical_edge_bps,
        costs,
        net_usd,
        capacity_usd,
    })
}

/// 理论价差（bps）：每跳即期价连乘与 1 的偏离。
fn theoretical_cycle_bps(view: &MarketView<'_>, edges: &[graph::HopEdge]) -> Option<Decimal> {
    let mut product = Decimal::ONE;
    for e in edges {
        let rt = view.pools.get(&e.pool_id)?;
        let spot = rt.spot_price(&e.token_in)?;
        if spot <= Decimal::ZERO {
            return None;
        }
        product *= spot;
    }
    Some((product - Decimal::ONE) * Decimal::from(10_000))
}

/// 容量：受每跳池子可承受规模的限制（取最紧的一跳）。
fn capacity_for(view: &MarketView<'_>, path: &pool::PathQuote, target: Decimal) -> Decimal {
    let mut cap = target * Decimal::from(4);
    for leg in &path.legs {
        if let Some(rt) = view.pools.get(&leg.pool_id) {
            let px_in = view.prices.get(&leg.token_in).copied().unwrap_or(Decimal::ONE);
            // 以较大一侧储备的 10% 作为单笔可承受上限，避免把池子打穿
            let reserve_usd = rt.state.reserve0.max(rt.state.reserve1) * px_in;
            let leg_cap = reserve_usd * Decimal::new(1, 1);
            if leg_cap > Decimal::ZERO && leg_cap < cap {
                cap = leg_cap;
            }
        }
    }
    cap.max(Decimal::ZERO)
}

fn gas_usd(params: &DexCostParams, gas_units: u64) -> Decimal {
    // gas_units × gas_price_gwei × 1e-9 × native_price
    Decimal::from(gas_units) * params.gas_price_gwei * Decimal::new(1, 9) * params.native_price_usd
}

/// 候选 → 统一机会对象。
pub fn to_opportunity(
    view: &MarketView<'_>,
    params: &DexCostParams,
    cand: &DexCandidate,
    strategy: StrategyType,
    available: &HashMap<String, Decimal>,
) -> Opportunity {
    let now = view.now;
    let start = cand.edges.first().map(|e| e.token_in.clone()).unwrap_or_default();
    let end = cand.edges.last().map(|e| e.token_out.clone()).unwrap_or_default();

    // 每跳最低输出必须显式指定：按 50bps buffer 保护
    let buffer = Decimal::ONE + Decimal::new(5, 3);
    let mut legs = Vec::with_capacity(cand.path.legs.len());
    for (i, l) in cand.path.legs.iter().enumerate() {
        let pool = view.pools.get(&l.pool_id);
        legs.push(RouteLeg {
            idx: i,
            protocol_id: pool.map(|p| p.pool.protocol_id.clone()).unwrap_or_default(),
            adapter: pool.map(|p| p.pool.protocol_id.clone()).unwrap_or_default(),
            pool_id: l.pool_id.clone(),
            token_in: l.token_in.clone(),
            token_out: l.token_out.clone(),
            amount_in: l.amount_in,
            expected_out: l.amount_out,
            min_out: q8(l.amount_out / buffer),
            fee_bps: l.fee_bps,
            price_impact_bps: l.price_impact_bps,
        });
    }

    let route_hash = fingerprint(
        &legs.iter()
            .map(|l| {
                format!(
                    "{}:{}:{}:{}:{}",
                    l.pool_id, l.token_in, l.token_out, l.amount_in, l.min_out
                )
            })
            .collect::<Vec<_>>()
            .join("|"),
    );

    let start_price = view.prices.get(&start).copied().unwrap_or(Decimal::ZERO);
    let notional_usd = cand.path.amount_in * start_price;
    let available_qty = available.get(&start).copied().unwrap_or(Decimal::ZERO);

    let funding = if available_qty >= cand.path.amount_in {
        FundingSource::OwnCapital
    } else {
        FundingSource::FlashLoan {
            provider: params.flash_provider.clone(),
            token: start.clone(),
            amount: q8(cand.path.amount_in),
            fee_bps: params.flash_fee_bps,
            fee_amount: bps_of_dec(notional_usd, params.flash_fee_bps),
        }
    };

    let scenario = build_scenario(
        cand.theoretical_edge_bps / Decimal::from(10_000) * notional_usd,
        cand.gross_usd,
        &cand.costs,
        params.profit_discount_pct,
        worst_case_loss(cand.gross_usd, &cand.costs, params.adverse_bps, notional_usd),
    );

    let route = DexRoute {
        route_id: format!("route_{route_hash}"),
        chain_id: view.chain_id,
        start_token: start.clone(),
        end_token: end.clone(),
        legs,
        amount_in: cand.path.amount_in,
        expected_out: cand.path.amount_out,
        min_out: q8(cand.path.amount_out / buffer),
        gas_units: cand.path.gas_units,
        gas_usd: cand.costs.gas,
        price_impact_bps: cand.path.price_impact_bps,
        model_level: ModelLevel::M2,
        block_number: cand.path.block_number,
        created_at: now,
    };

    let mut opp = Opportunity {
        opportunity_id: crate::domain::new_id("opp"),
        strategy_type: strategy,
        chain_id: view.chain_id,
        chain_name: view.chain_name.clone(),
        dex_route: Some(route),
        cex_exchange: None,
        cex_symbol: None,
        direction: None,
        target_notional: notional_usd,
        target_qty: cand.path.amount_in,
        required_dex_inventory: vec![InventoryNeed {
            venue: format!("dex-{}", view.chain_name.to_lowercase()),
            asset: start.clone(),
            required: cand.path.amount_in,
            available: available_qty,
        }],
        required_cex_inventory: vec![],
        theoretical_edge_bps: cand.theoretical_edge_bps,
        executable_gross_profit: cand.gross_usd,
        total_cost: cand.costs.total(),
        conservative_net_profit: cand.gross_usd - cand.costs.total(),
        stress_net_profit: scenario.stress_net_usd,
        worst_case_loss: scenario.worst_case_loss_usd,
        costs: cand.costs.breakdown(),
        scenario,
        risk_score: crate::domain::market::RiskScore::from_components(vec![]),
        data_quality: route_quality(view, cand),
        model_level: ModelLevel::M2,
        confidence: "M2 精确池状态 + 逐跳最低输出".into(),
        quote_timestamp: now,
        quote_block_number: cand.path.block_number,
        book_age_ms: 0,
        quote_age_ms: 0,
        status: OppStatus::Watch,
        rejections: vec![],
        capacity_usd: cand.capacity_usd,
        funding,
        hedge_order: None,
        detected_at: now,
        expires_at: now + chrono::Duration::seconds(params.ttl_sec),
    };

    if opp.data_quality.rank() >= DataQuality::Low.rank() {
        opp.status = OppStatus::Rejected;
        opp.confidence = "池状态过期或验证等级不足".into();
    }
    opp
}

fn route_quality(view: &MarketView<'_>, cand: &DexCandidate) -> DataQuality {
    let mut worst = DataQuality::Verified;
    for e in &cand.edges {
        let Some(rt) = view.pools.get(&e.pool_id) else {
            return DataQuality::Invalid;
        };
        if !rt.pool.verified || !rt.pool.execution_allowed {
            return DataQuality::Invalid;
        }
        let age = rt.state.age_ms(view.now);
        let q = if age > 3_000 {
            DataQuality::Invalid
        } else if age > 1_000 {
            DataQuality::Low
        } else if age > 300 {
            DataQuality::Medium
        } else {
            DataQuality::High
        };
        if q.rank() > worst.rank() {
            worst = q;
        }
    }
    if cand.path.block_number + 1 < view.head_block {
        return DataQuality::Low;
    }
    worst
}

/// 按 Token 精度截断数量。
pub fn quantize_amount(amount: Decimal, token: &Token) -> Decimal {
    let step = Decimal::new(1, token.decimals.min(18));
    round_step_down(amount, step)
}

/// 两个时间戳是否落在同一扫描窗口内。
pub fn same_scan(a: DateTime<Utc>, b: DateTime<Utc>, window_ms: i64) -> bool {
    (a - b).num_milliseconds().abs() <= window_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gas_cost_matches_manual() {
        let p = DexCostParams {
            gas_price_gwei: Decimal::new(2, 2), // 0.02 gwei
            native_price_usd: Decimal::from(2800),
            ..DexCostParams::default()
        };
        // 390000 gas × 0.02 gwei = 0.0000078 ETH × 2800 = 0.02184 USD
        assert_eq!(gas_usd(&p, 390_000).round_dp(6), Decimal::new(2184, 5));
    }

    #[test]
    fn quantize_respects_decimals() {
        let token = Token {
            symbol: "USDC".into(),
            chain_id: 1,
            address: "0x0".into(),
            decimals: 2,
            standard: "ERC20".into(),
            code_hash: "0x1".into(),
            risk_tags: vec![],
            allowed_start: true,
            allowed_hop: true,
            allowed_flash: true,
            special_transfer: false,
            enabled: true,
        };
        assert_eq!(quantize_amount(Decimal::new(123456, 3), &token), Decimal::new(12345, 2));
    }
}
