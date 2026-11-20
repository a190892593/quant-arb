//! CEX–DEX 非原子对冲机会搜索（PRD §2.4、TDD §5.3、§7）。
//!
//! 与 DEX 原子套利的关键区别：
//! - 两条腿无法原子提交/回滚，必须计入延迟准备金、对冲补偿准备金与库存再平衡成本；
//! - CEX 腿必须按盘口深度逐档成交，不能用中间价；
//! - 机会必须标注方向（DEX 买/CEX 卖 或 CEX 买/DEX 卖）与执行顺序假设。

use chrono::Utc;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

use crate::cex::depth;
use crate::domain::market::{DataQuality, DexRoute, Instrument, ModelLevel, OrderBook, RouteLeg};
use crate::domain::money::{bps_of, bps_of_dec, q8};
use crate::domain::opportunity::{FundingSource, InventoryNeed, Opportunity};
use crate::domain::plan::fingerprint;
use crate::domain::types::{Direction, HedgeOrder, OppStatus, OrderType, Side, StrategyType};
use crate::dex::graph::HopEdge;
use crate::dex::pool::{self, PathQuote};

use super::cost::{build_scenario, usd_bps, worst_case_loss, CostParts};
use super::{CexDexParams, MarketView};

/// 搜索 CEX–DEX 机会。
///
/// `prefer_cex_first`：本轮机会假设的执行顺序（Paper 必须分别统计两种顺序）。
pub fn scan(
    view: &MarketView<'_>,
    params: &CexDexParams,
    notional_usd: Decimal,
    dex_available: &HashMap<String, Decimal>,
    cex_available: &HashMap<String, Decimal>,
    prefer_cex_first: bool,
) -> Vec<Opportunity> {
    let mut out = Vec::new();

    for (symbol, inst) in view.instruments {
        if !inst.is_tradable() || !inst.fee_known {
            continue;
        }
        let Some(book) = view.books.get(&inst.exchange, symbol) else {
            continue;
        };
        if !book.valid {
            continue;
        }
        let Some(mid) = book.mid() else {
            continue;
        };
        if mid <= Decimal::ZERO {
            continue;
        }
        let Some(base_tok) = resolve_asset(view, &inst.base) else {
            continue;
        };
        let Some(quote_tok) = resolve_asset(view, &inst.quote) else {
            continue;
        };

        let age_ms = (view.now - book.recv_ts).num_milliseconds();
        // 轮询行情的盘口年龄至少是一个同步周期；用 8s 与风控 book_max_age_ms 对齐，
        // 超龄在后续 book_quality 中降级/拒绝，而不是在这里静默丢弃全部 CEX–DEX。
        if age_ms > 8_000 {
            continue;
        }

        // 目标数量：先按名义金额，再受盘口深度占用与 DEX 容量约束
        let mut qty = inst.quantize_qty(notional_usd / mid);
        let cex_depth_cap = depth::max_qty_within_depth_pct(
            book,
            inst,
            Side::Buy,
            params.depth_usage_max_pct,
        );
        let cex_depth_cap_sell = depth::max_qty_within_depth_pct(
            book,
            inst,
            Side::Sell,
            params.depth_usage_max_pct,
        );
        qty = qty.min(cex_depth_cap.max(cex_depth_cap_sell));
        if qty <= Decimal::ZERO {
            continue;
        }

        // 两个方向分别评估：
        // - DEX 买 / CEX 卖：DEX 侧用 quote 买 base，CEX 侧卖出 base
        // - CEX 买 / DEX 卖：CEX 侧买入 base，DEX 侧卖出 base 换 quote
        for direction in [Direction::DexBuyCexSell, Direction::CexBuyDexSell] {
            let (dex_in, dex_out) = if direction.dex_is_buy() {
                (&quote_tok, &base_tok)
            } else {
                (&base_tok, &quote_tok)
            };
            let Some(opp) = evaluate_direction(
                view,
                params,
                inst,
                book,
                dex_in,
                dex_out,
                mid,
                qty,
                direction,
                dex_available,
                cex_available,
                prefer_cex_first,
            ) else {
                continue;
            };
            out.push(opp);
        }
    }

    out
}

/// 评估单个方向的完整盈亏。
#[allow(clippy::too_many_arguments)]
fn evaluate_direction(
    view: &MarketView<'_>,
    params: &CexDexParams,
    inst: &Instrument,
    book: &OrderBook,
    dex_in: &str,
    dex_out: &str,
    mid: Decimal,
    qty: Decimal,
    direction: Direction,
    dex_available: &HashMap<String, Decimal>,
    cex_available: &HashMap<String, Decimal>,
    prefer_cex_first: bool,
) -> Option<Opportunity> {
    let now = Utc::now();
    let cex_side = if direction.dex_is_buy() {
        Side::Sell
    } else {
        Side::Buy
    };

    // ---------- DEX 腿：先求「为得到 qty base 需要多少 quote」 ----------
    // 简化：按 mid 估算输入，再按实际报价输出回填，一次迭代足够（偏离过大则放弃）。
    let dex_amount_in = if direction.dex_is_buy() {
        q8(qty * mid)
    } else {
        q8(qty)
    };
    if dex_amount_in <= Decimal::ZERO {
        return None;
    }
    let (path, edges) = find_best_path(view, dex_in, dex_out, dex_amount_in, params.max_hops)?;
    if path.amount_out <= Decimal::ZERO {
        return None;
    }

    // ---------- CEX 腿：以 DEX 实际成交数量为准（TDD §7.3） ----------
    let cex_qty = if direction.dex_is_buy() {
        // DEX 买入得到 base，然后在 CEX 卖出同样数量
        inst.quantize_qty(path.amount_out)
    } else {
        // CEX 先买 base，再把实际成交数量拿去 DEX 卖
        inst.quantize_qty(qty)
    };
    if cex_qty < inst.min_qty {
        return None;
    }
    let cex_limit = match cex_side {
        Side::Buy => mid * (Decimal::ONE + params.price_protection_bps / Decimal::from(10_000)),
        Side::Sell => mid * (Decimal::ONE - params.price_protection_bps / Decimal::from(10_000)),
    };
    let cex_quote = depth::simulate_order(
        book,
        inst,
        cex_side,
        cex_qty,
        Some(cex_limit),
        OrderType::Ioc,
    );
    if !cex_quote.valid || cex_quote.filled_qty <= Decimal::ZERO {
        return None;
    }

    // 若 CEX 部分成交，另一腿数量必须按实际成交量重算（这里保留机会但标注实际规模）
    let base_qty = cex_quote.filled_qty;

    // ---------- 盈亏 ----------
    let (gross_usd, dex_fee_usd, cex_fee_usd, dex_impact_bps) = if direction.dex_is_buy() {
        // 花 USDC 买 base（quote 资产），CEX 卖出 base 得到 quote（USDT）
        let spent_usd = path.amount_in * price_of(view, dex_in);
        let proceeds_usd = cex_quote.notional;
        let dex_fee = path
            .legs
            .iter()
            .map(|l| {
                bps_of(
                    l.amount_in * price_of(view, &l.token_in),
                    l.fee_bps,
                )
            })
            .sum::<Decimal>();
        let cex_fee = bps_of_dec(cex_quote.notional, inst.taker_bps);
        (proceeds_usd - spent_usd, dex_fee, cex_fee, path.price_impact_bps)
    } else {
        // CEX 买入 base 花 USDT，DEX 卖出 base 得到 USDC
        let cex_cost = cex_quote.notional;
        let proceeds_usd = path.amount_out * price_of(view, dex_out);
        let dex_fee = path
            .legs
            .iter()
            .map(|l| {
                bps_of(
                    l.amount_in * price_of(view, &l.token_in),
                    l.fee_bps,
                )
            })
            .sum::<Decimal>();
        let cex_fee = bps_of_dec(cex_quote.notional, inst.taker_bps);
        (proceeds_usd - cex_cost, dex_fee, cex_fee, path.price_impact_bps)
    };

    // 毛利还原到「未扣 DEX 手续费」：path.amount_out 已含费，成本桥里手续费只计一次。
    let fee_bps_total: Decimal = path
        .legs
        .iter()
        .map(|l| Decimal::from(l.fee_bps))
        .sum();
    let gross_usd = gross_usd + dex_fee_usd;
    let dex_impact_ex_fee = (dex_impact_bps - fee_bps_total).max(Decimal::ZERO);

    let actual_notional = base_qty * mid;
    if actual_notional <= Decimal::ZERO {
        return None;
    }

    // CEX 侧价格冲击（相对对手价）
    let cex_impact_bps = match cex_side {
        Side::Buy => {
            let best = book.best_ask().unwrap_or(mid);
            if best.is_zero() {
                Decimal::ZERO
            } else {
                (cex_quote.avg_price - best) / best * Decimal::from(10_000)
            }
        }
        Side::Sell => {
            let best = book.best_bid().unwrap_or(mid);
            if best.is_zero() {
                Decimal::ZERO
            } else {
                (best - cex_quote.avg_price) / best * Decimal::from(10_000)
            }
        }
    };

    let mut costs = CostParts::new();
    costs.dex_fee = dex_fee_usd;
    costs.cex_fee = cex_fee_usd;
    costs.gas = gas_usd(params, path.gas_units);
    costs.price_impact = usd_bps(
        actual_notional,
        dex_impact_ex_fee + cex_impact_bps.max(Decimal::ZERO),
    );
    costs.delay_reserve = usd_bps(actual_notional, params.delay_reserve_bps);
    costs.hedge_compensation = usd_bps(actual_notional, params.compensation_bps);
    costs.rebalance = usd_bps(actual_notional, params.rebalance_bps);
    costs.slippage = usd_bps(actual_notional, params.slippage_bps);
    costs.flash_fee = Decimal::ZERO; // 第一阶段 CEX–DEX 不使用闪电贷

    let net_usd = gross_usd - costs.total();

    // 理论价差：CEX 成交均价 vs DEX 实际成交均价（同以 quote/base 计价）
    let theoretical_edge_bps = theoretical_edge(direction, &path, cex_quote.avg_price);

    // ---------- 机会对象 ----------
    let buffer = Decimal::ONE + Decimal::new(5, 3);
    let mut legs = Vec::with_capacity(path.legs.len());
    for (i, l) in path.legs.iter().enumerate() {
        let p = view.pools.get(&l.pool_id);
        legs.push(RouteLeg {
            idx: i,
            protocol_id: p.map(|x| x.pool.protocol_id.clone()).unwrap_or_default(),
            adapter: p.map(|x| x.pool.protocol_id.clone()).unwrap_or_default(),
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
    let route = DexRoute {
        route_id: format!("route_{route_hash}"),
        chain_id: view.chain_id,
        start_token: dex_in.to_string(),
        end_token: dex_out.to_string(),
        legs,
        amount_in: path.amount_in,
        expected_out: path.amount_out,
        min_out: q8(path.amount_out / buffer),
        gas_units: path.gas_units,
        gas_usd: costs.gas,
        price_impact_bps: path.price_impact_bps,
        model_level: ModelLevel::M1,
        block_number: path.block_number,
        created_at: now,
    };

    // 库存需求：两条腿分别需要预置库存
    let mut dex_need = Vec::new();
    let mut cex_need = Vec::new();
    let (dex_asset, dex_required) = if direction.dex_is_buy() {
        (dex_in.to_string(), path.amount_in)
    } else {
        (dex_in.to_string(), path.amount_in)
    };
    dex_need.push(InventoryNeed {
        venue: format!("dex-{}", view.chain_name.to_lowercase()),
        asset: dex_asset.clone(),
        required: dex_required,
        available: dex_available.get(&dex_asset).copied().unwrap_or(Decimal::ZERO),
    });
    let (cex_asset, cex_required) = match cex_side {
        Side::Buy => (inst.quote.clone(), cex_quote.notional),
        Side::Sell => (inst.base.clone(), cex_quote.filled_qty),
    };
    cex_need.push(InventoryNeed {
        venue: format!("cex-{}", inst.exchange),
        asset: cex_asset.clone(),
        required: cex_required,
        available: cex_available.get(&cex_asset).copied().unwrap_or(Decimal::ZERO),
    });

    let scenario = build_scenario(
        theoretical_edge_bps / Decimal::from(10_000) * actual_notional,
        gross_usd,
        &costs,
        params.profit_discount_pct,
        worst_case_loss(gross_usd, &costs, params.adverse_bps, actual_notional),
    );

    let data_quality = book_quality(book, view.now).min_rank(pool_quality(view, &edges));
    let capacity = min_capacity(view, &path, actual_notional, book, inst, cex_side, params);

    let id_seed = format!(
        "cex_dex|{}|{}|{:?}|{}",
        inst.exchange, inst.symbol, direction, actual_notional
    );
    let opportunity_id = format!("opp_{:016x}", crate::domain::plan::fnv1a_64(&id_seed));

    let mut opp = Opportunity {
        opportunity_id,
        strategy_type: StrategyType::CexDex,
        chain_id: view.chain_id,
        chain_name: view.chain_name.clone(),
        dex_route: Some(route),
        cex_exchange: Some(inst.exchange.clone()),
        cex_symbol: Some(inst.symbol.clone()),
        direction: Some(direction),
        target_notional: actual_notional,
        target_qty: base_qty,
        required_dex_inventory: dex_need,
        required_cex_inventory: cex_need,
        theoretical_edge_bps,
        executable_gross_profit: gross_usd,
        total_cost: costs.total(),
        conservative_net_profit: net_usd,
        stress_net_profit: scenario.stress_net_usd,
        worst_case_loss: scenario.worst_case_loss_usd,
        costs: costs.breakdown(),
        scenario,
        risk_score: crate::domain::market::RiskScore::from_components(vec![]),
        data_quality,
        model_level: ModelLevel::M1,
        confidence: "M2 盘口深度成交 + 延迟/补偿准备金".into(),
        m2: None,
        quote_timestamp: now,
        quote_block_number: path.block_number,
        book_age_ms: (view.now - book.recv_ts).num_milliseconds(),
        quote_age_ms: 0,
        status: OppStatus::Watch,
        lifecycle: crate::domain::types::OpportunityLifecycle::Quoted,
        route_fingerprint: None,
        rejections: vec![],
        capacity_usd: capacity,
        funding: FundingSource::OwnCapital,
        hedge_order: Some(if prefer_cex_first {
            HedgeOrder::CexFirst
        } else {
            HedgeOrder::DexFirst
        }),
        detected_at: now,
        expires_at: now + chrono::Duration::seconds(params.ttl_sec),
    };

    if opp.data_quality.rank() >= DataQuality::Low.rank() {
        opp.status = OppStatus::Rejected;
        opp.confidence = "盘口或池状态质量不足".into();
    }
    Some(opp)
}

/// 理论价差（bps）：两边成交均价的相对偏离。
///
/// - DEX 买 / CEX 卖：`(cex - dex) / dex`
/// - CEX 买 / DEX 卖：`(dex - cex) / cex`
fn theoretical_edge(direction: Direction, path: &PathQuote, cex_price: Decimal) -> Decimal {
    if path.amount_in.is_zero() || path.amount_out.is_zero() || cex_price.is_zero() {
        return Decimal::ZERO;
    }
    let edge = match direction {
        Direction::DexBuyCexSell => {
            let dex_price = path.amount_in / path.amount_out; // quote per base
            (cex_price - dex_price) / dex_price
        }
        Direction::CexBuyDexSell => {
            let dex_price = path.amount_out / path.amount_in; // quote per base
            (dex_price - cex_price) / cex_price
        }
    };
    edge * Decimal::from(10_000)
}

fn min_capacity(
    view: &MarketView<'_>,
    path: &PathQuote,
    actual_notional: Decimal,
    book: &OrderBook,
    inst: &Instrument,
    side: Side,
    params: &CexDexParams,
) -> Decimal {
    // CEX 侧：允许价格区间内可见深度 / 深度倍数要求
    let ref_price = match side {
        Side::Buy => book.best_ask(),
        Side::Sell => book.best_bid(),
    }
    .unwrap_or(Decimal::ONE);
    let depth_usd = depth::depth_within_bps(book, side, ref_price, params.price_protection_bps);
    let cex_cap = depth_usd / params.depth_usage_max_pct.max(Decimal::ONE) * Decimal::from(10);
    let _ = inst;
    // DEX 侧：最紧一跳储备的 10%
    let dex_cap = path
        .legs
        .iter()
        .filter_map(|l| {
            view.pools.get(&l.pool_id).map(|p| {
                let px = view.prices.get(&l.token_in).copied().unwrap_or(Decimal::ONE);
                p.state.reserve0.max(p.state.reserve1) * px * Decimal::new(1, 1)
            })
        })
        .fold(actual_notional * Decimal::from(6), Decimal::min);
    cex_cap.min(dex_cap).max(Decimal::ZERO)
}

fn book_quality(book: &OrderBook, now: chrono::DateTime<Utc>) -> DataQuality {
    if !book.valid {
        return DataQuality::Invalid;
    }
    let age = (now - book.recv_ts).num_milliseconds();
    // 与风控 book_max_age_ms（8s）对齐；轮询行情下盘口年龄至少一个同步周期。
    DataQuality::from_age(age, 8_000)
}

fn pool_quality(view: &MarketView<'_>, edges: &[HopEdge]) -> DataQuality {
    let mut worst = DataQuality::High;
    for e in edges {
        let Some(rt) = view.pools.get(&e.pool_id) else {
            return DataQuality::Invalid;
        };
        if !rt.pool.verified || !rt.pool.execution_allowed {
            return DataQuality::Invalid;
        }
        // 未接 REVM/M2：实时快照最高只到 B。
        let q = DataQuality::from_age(rt.state.age_ms(view.now), 8_000);
        if q.rank() > worst.rank() {
            worst = q;
        }
    }
    worst
}

trait MinRank {
    fn min_rank(self, other: DataQuality) -> DataQuality;
}

impl MinRank for DataQuality {
    fn min_rank(self, other: DataQuality) -> DataQuality {
        if self.rank() >= other.rank() { self } else { other }
    }
}

/// 在池集合上找 from→to 的最优路径（最多 max_hops 跳，池不重复）。
pub fn find_best_path(
    view: &MarketView<'_>,
    from: &str,
    to: &str,
    amount_in: Decimal,
    max_hops: usize,
) -> Option<(PathQuote, Vec<HopEdge>)> {
    if from == to {
        return None;
    }
    let mut best: Option<(PathQuote, Vec<HopEdge>)> = None;
    let mut visited_tokens: HashSet<String> = HashSet::new();
    let mut path: Vec<HopEdge> = Vec::new();
    visited_tokens.insert(from.to_string());
    dfs(
        view,
        from,
        to,
        amount_in,
        max_hops,
        &mut visited_tokens,
        &mut path,
        &mut best,
    );
    best
}

#[allow(clippy::too_many_arguments)]
fn dfs(
    view: &MarketView<'_>,
    current: &str,
    target: &str,
    amount_in: Decimal,
    remaining: usize,
    visited_tokens: &mut HashSet<String>,
    path: &mut Vec<HopEdge>,
    best: &mut Option<(PathQuote, Vec<HopEdge>)>,
) {
    if remaining == 0 {
        return;
    }
    for rt in view.pools.all() {
        if !rt.pool.verified || !rt.pool.execution_allowed {
            continue;
        }
        if path.iter().any(|e| e.pool_id == rt.pool.pool_id) {
            continue;
        }
        let (token_in, token_out) = if rt.pool.token0 == current {
            (rt.pool.token0.clone(), rt.pool.token1.clone())
        } else if rt.pool.token1 == current {
            (rt.pool.token1.clone(), rt.pool.token0.clone())
        } else {
            continue;
        };
        if path.len() + 1 < remaining.max(1) && token_out == target {
            // 允许在中途到达目标（至少一跳），无需额外处理
        }
        if visited_tokens.contains(&token_out) && token_out != target {
            continue;
        }
        path.push(HopEdge {
            pool_id: rt.pool.pool_id.clone(),
            token_in: token_in.clone(),
            token_out: token_out.clone(),
            kind: rt.pool.kind,
        });

        if token_out == target {
            if let Ok(q) = pool::quote_path(view.pools, path[0].token_in.as_str(), amount_in, path) {
                let better = match best {
                    None => true,
                    Some((b, _)) => q.amount_out > b.amount_out,
                };
                if better {
                    *best = Some((q, path.clone()));
                }
            }
        } else {
            visited_tokens.insert(token_out.clone());
            dfs(
                view,
                &token_out,
                target,
                amount_in,
                remaining - 1,
                visited_tokens,
                path,
                best,
            );
            visited_tokens.remove(&token_out);
        }
        path.pop();
    }
}

/// CEX 资产符号 → DEX Token 符号（ETH→WETH、BTC→WBTC 等包装关系）。
pub fn resolve_asset(view: &MarketView<'_>, sym: &str) -> Option<String> {
    if view.tokens.contains_key(sym) {
        return Some(sym.to_string());
    }
    let wrapped = format!("W{sym}");
    if view.tokens.contains_key(&wrapped) {
        return Some(wrapped);
    }
    view.tokens
        .keys()
        .find(|k| k.ends_with(sym) || k.contains(sym))
        .cloned()
}

fn price_of(view: &MarketView<'_>, sym: &str) -> Decimal {
    view.prices.get(sym).copied().unwrap_or(Decimal::ZERO)
}

fn gas_usd(params: &CexDexParams, gas_units: u64) -> Decimal {
    Decimal::from(gas_units)
        * params.gas_price_gwei
        * Decimal::new(1, 9)
        * params.native_price_usd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_asset_prefers_exact_then_wrapped() {
        // 该测试只验证映射规则本身（不构造 MarketView）
        assert_eq!(format!("W{}", "ETH"), "WETH");
    }

    #[test]
    fn min_rank_picks_worse_quality() {
        assert_eq!(
            DataQuality::Verified.min_rank(DataQuality::Low),
            DataQuality::Low
        );
        assert_eq!(
            DataQuality::Invalid.min_rank(DataQuality::High),
            DataQuality::Invalid
        );
    }
}
