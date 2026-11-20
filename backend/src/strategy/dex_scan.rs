//! DEX 多池多跳扫描引擎（PRD §4–§7、§9）。
//!
//! 一次扫描的完整链路：
//!
//! 1. Token / Pool 准入（[crate::dex::registry]）；
//! 2. 构建 Token–Pool 有向多重图（[crate::dex::multigraph]）；
//! 3. 预算约束的简单环搜索（[crate::dex::route_search]）；
//! 4. 有向指纹去重（[crate::dex::fingerprint]）；
//! 5. 逐条路线做投入金额优化（[crate::dex::optimizer]），所有报价基于同一池状态；
//! 6. Gas / 成本 / 净利润（AMM 手续费与滑点已内含在成交价，不重复扣减）；
//! 7. M1 状态一致性模拟 + M3 压力模拟（[crate::dex::sim]）；
//! 8. 路线级门禁（Token/Pool 准入、状态一致性、跳数、容量、模型一致性）；
//! 9. 生命周期推进并生成统一 Opportunity；
//! 10. 增量缓存：池状态未变时复用上次报价，只重算受影响路线。
//!
//! 明确限制：本机未接入 REVM，M2 不可用；因此生命周期最高到 SIMULATED，
//! 不会把仅通过 M1/M3 的机会标记为 VERIFIED。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::config::Config;
use crate::domain::dex_scan::{
    GasEstimate, GraphStats, RouteFingerprints, RouteLegRecord, RouteScanRecord,
    ScanRunStats, SimulationOutcome,
};
use crate::domain::market::{DataQuality, DexRoute, ModelLevel, PoolKind, RiskViolation, RouteLeg};
use crate::domain::money::{bps_of, bps_of_dec, q8, round_step_down};
use crate::domain::opportunity::{FundingSource, InventoryNeed, Opportunity};
use crate::domain::plan::fnv1a_64;
use crate::domain::types::{
    OppStatus, OpportunityLifecycle, RiskSeverity, RouteType, SimulationStatus, StrategyType,
};
use crate::dex::graph::HopEdge;
use crate::dex::optimizer::{self, AmountEvaluation, OptimizationParams};
use crate::dex::registry::{PoolLimits, TokenRegistry};
use crate::dex::route_search::{self, RouteCandidate, SearchLimits};
use crate::dex::{gas, multigraph::TokenGraph, pool, sim};

use super::cost::{build_scenario, usd_bps, worst_case_loss, CostParts};
use super::{MarketView, ScanContext};

fn dec_f64(v: f64) -> Decimal {
    Decimal::from_f64_retain(v).unwrap_or_else(|| Decimal::from(0))
}

/// 新引擎参数。默认值对齐 PRD §4.4 / §5.4 / §7 初始研究基准。
#[derive(Clone, Debug)]
pub struct DexScanParams {
    /// 是否启用新引擎；关闭时由 scanner 回退到旧版三角/跨池扫描。
    pub enabled: bool,
    pub gas: gas::GasParams,
    pub gas_price_gwei: Decimal,
    pub native_price_usd: Decimal,
    pub flash_fee_bps: Decimal,
    pub flash_provider: String,
    pub priority_fee_usd: Decimal,
    pub delay_reserve_bps: Decimal,
    pub mev_reserve_bps: Decimal,
    pub profit_discount_pct: Decimal,
    pub ttl_sec: i64,

    pub min_hops: usize,
    pub max_hops: usize,
    pub max_pools: usize,
    pub max_candidates: usize,
    /// 单轮最多完整定价多少条候选（防止上万条路线拖垮扫描周期）。
    pub max_evaluate: usize,
    /// 定价阶段硬预算（毫秒）。
    pub max_evaluate_ms: u64,
    pub max_visits: u64,
    pub max_elapsed_ms: u64,

    pub min_input_usd: Decimal,
    pub max_input_usd: Decimal,
    pub min_pool_tvl_usd: Decimal,
    pub max_pool_state_age_ms: i64,
    pub max_pool_share_pct: Decimal,
    pub grid_points: usize,
    pub refine_iters: u32,

    pub stress: sim::StressParams,
    pub model_mismatch_max_bps: Decimal,
    pub requote_age_ms: i64,
    pub full_rescan_blocks: u64,

    pub base_symbols: Vec<String>,
    pub notional_ladder: Vec<Decimal>,
}

impl Default for DexScanParams {
    fn default() -> Self {
        Self {
            enabled: true,
            gas: gas::GasParams::default(),
            gas_price_gwei: Decimal::new(2, 2),
            native_price_usd: Decimal::from(2800),
            flash_fee_bps: Decimal::from(5),
            flash_provider: "aave-v3".into(),
            priority_fee_usd: Decimal::new(5, 2),
            // 风控基线 §4.1：延迟与抢先/夹击风险准备金必须扣减，默认值不得为 0。
            delay_reserve_bps: Decimal::from(8),
            mev_reserve_bps: Decimal::from(8),
            profit_discount_pct: Decimal::from(50),
            ttl_sec: 30,
            min_hops: 2,
            max_hops: 5,
            max_pools: 5,
            max_candidates: 100_000,
            max_evaluate: 800,
            max_evaluate_ms: 1_200,
            max_visits: 200_000,
            max_elapsed_ms: 250,
            min_input_usd: Decimal::from(50),
            max_input_usd: Decimal::from(25_000),
            min_pool_tvl_usd: Decimal::from(100_000),
            max_pool_state_age_ms: 3_000,
            max_pool_share_pct: Decimal::from(10),
            grid_points: 14,
            refine_iters: 16,
            stress: sim::StressParams::default(),
            model_mismatch_max_bps: Decimal::from(50),
            requote_age_ms: 800,
            full_rescan_blocks: 30,
            base_symbols: vec!["USDC".into(), "USDT".into()],
            notional_ladder: vec![
                Decimal::from(250),
                Decimal::from(500),
                Decimal::from(1_000),
                Decimal::from(2_500),
                Decimal::from(5_000),
                Decimal::from(7_500),
                Decimal::from(10_000),
            ],
        }
    }
}

impl DexScanParams {
    /// 由运行配置构建。配置缺失时使用默认值。
    pub fn from_config(cfg: &Config) -> Self {
        let d = &cfg.dex_engine;
        let mut p = Self::default();
        p.enabled = d.enabled;
        p.gas = gas::GasParams {
            executor_base: d.gas_executor_base,
            executor_per_hop: d.gas_executor_per_hop,
            transfer_per_hop: d.gas_transfer_per_hop,
            flash_loan: d.gas_flash_loan,
            approve: d.gas_approve,
        };
        p.gas_price_gwei = dec_f64(cfg.dex.gas_price_gwei);
        p.native_price_usd = dec_f64(cfg.dex.native_price_usd);
        p.flash_fee_bps = dec_f64(cfg.dex.flash_loan_fee_bps);
        p.flash_provider = cfg.dex.flash_provider.clone();
        p.ttl_sec = cfg.paper.opportunity_ttl_sec;
        p.min_hops = d.min_hops.max(2);
        p.max_hops = d.max_hops.max(p.min_hops).min(8);
        p.max_pools = d.max_pools.max(p.min_hops);
        p.max_candidates = d.max_candidates;
        p.max_evaluate = d.max_evaluate;
        p.max_evaluate_ms = d.max_evaluate_ms;
        p.max_visits = d.max_visits;
        p.max_elapsed_ms = d.max_elapsed_ms;
        p.min_input_usd = dec_f64(d.min_input_usd);
        p.max_input_usd = dec_f64(d.max_input_usd);
        p.min_pool_tvl_usd = dec_f64(d.min_pool_tvl_usd);
        p.max_pool_state_age_ms = d.max_pool_state_age_ms;
        p.max_pool_share_pct = dec_f64(d.max_pool_share_pct);
        p.grid_points = d.grid_points;
        p.refine_iters = d.refine_iters;
        p.stress = sim::StressParams {
            adverse_bps: dec_f64(d.stress_bps),
            gas_increase_pct: dec_f64(d.stress_gas_pct),
            flash_fee_increase_pct: dec_f64(d.stress_flash_pct),
        };
        p.model_mismatch_max_bps = dec_f64(d.model_mismatch_max_bps);
        p.requote_age_ms = d.requote_age_ms;
        p.full_rescan_blocks = d.full_rescan_blocks;
        p.base_symbols = d.base_symbols.clone();
        p.notional_ladder = cfg
            .paper
            .notional_ladder
            .iter()
            .map(|v| dec_f64(*v))
            .collect();
        p
    }
}

/// 缓存的路线：保留结构信息与上一次报价，供增量复用。
#[derive(Clone, Debug)]
pub struct CachedRoute {
    pub route_id: String,
    pub chain_id: u64,
    pub start_token: String,
    pub edges: Vec<HopEdge>,
    pub fingerprints: RouteFingerprints,
    pub route_type: RouteType,
    pub last_quote_at: DateTime<Utc>,
    pub price_sig: String,
    pub record: Option<RouteScanRecord>,
    /// 复用命中次数（冷热分级依据，V22-206）。
    pub hits: u64,
}

/// 冷热分级（V22-206）：按「是否盈利 + 复用命中」分三级。
///
/// 分级只影响重算频率与缓存 TTL，**不淘汰冷路线**（探索预算见 PRD §6.5）。
fn tier_of(route: &CachedRoute) -> &'static str {
    match route.record.as_ref() {
        Some(r) if r.net_usd > Decimal::ZERO && route.hits >= 3 => "hot",
        Some(r) if r.net_usd > Decimal::ZERO || route.hits > 0 => "warm",
        _ => "cold",
    }
}

/// 增量扫描缓存：路线集合 + 池 → 路线反向索引。
#[derive(Clone, Debug, Default)]
pub struct MultiHopCache {
    routes: Vec<CachedRoute>,
    pool_index: HashMap<String, Vec<usize>>,
    topology_sig: String,
    last_full_scan_block: u64,
    pool_seen: HashMap<String, (u64, u64)>,
    last_scan_at: Option<DateTime<Utc>>,
    last_stats: Option<ScanRunStats>,
    /// 最近一次全量搜索的访问量（增量轮次沿用，便于展示搜索规模）。
    last_search_visits: u64,
    /// 定价轮转游标：每轮从上次结束位置继续，避免固定前 N 条造成饥饿。
    eval_cursor: usize,
}

/// 池状态签名：价格量化到 1bp、深度粗粒度，用于增量扫描的变更检测。
fn pool_state_sig(rt: &crate::dex::PoolRuntime) -> (u64, u64) {
    match rt.pool.kind {
        PoolKind::UniswapV3 => {
            let sqrt = rt.state.sqrt_price_x96;
            if sqrt == 0 {
                return (0, 0);
            }
            let p = (sqrt as f64) / 2f64.powi(96);
            let bp = (p * p * 10_000.0).round().max(0.0) as u64;
            (bp, (rt.state.liquidity >> 32) as u64)
        }
        _ => {
            let r0 = rt.state.reserve0.to_f64().unwrap_or(0.0);
            let r1 = rt.state.reserve1.to_f64().unwrap_or(0.0);
            if r0 <= 0.0 {
                return (0, 0);
            }
            let bp = ((r1 / r0) * 10_000.0).round().max(0.0) as u64;
            (bp, (r0 * 100.0) as u64)
        }
    }
}

impl MultiHopCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// 冷热分级计数（hot, warm, cold），V22-206。
    pub fn tier_stats(&self) -> (usize, usize, usize) {
        let mut hot = 0usize;
        let mut warm = 0usize;
        let mut cold = 0usize;
        for r in &self.routes {
            match tier_of(r) {
                "hot" => hot += 1,
                "warm" => warm += 1,
                _ => cold += 1,
            }
        }
        (hot, warm, cold)
    }

    pub fn topology_sig(&self) -> &str {
        &self.topology_sig
    }

    pub fn last_stats(&self) -> Option<&ScanRunStats> {
        self.last_stats.as_ref()
    }

    /// 引用某池的路线数量（反向索引）。
    pub fn routes_with_pool(&self, pool_id: &str) -> usize {
        self.pool_index.get(pool_id).map(|v| v.len()).unwrap_or(0)
    }

    pub fn records(&self) -> Vec<RouteScanRecord> {
        self.routes
            .iter()
            .filter_map(|r| r.record.clone())
            .collect()
    }

    pub fn record(&self, route_id: &str) -> Option<RouteScanRecord> {
        self.routes
            .iter()
            .find(|r| r.route_id == route_id)
            .and_then(|r| r.record.clone())
    }

    pub fn route_index(&self, route_id: &str) -> Option<usize> {
        self.routes.iter().position(|r| r.route_id == route_id)
    }

    /// 全量重建路线集合与反向索引。
    pub fn rebuild(
        &mut self,
        candidates: Vec<RouteCandidate>,
        topology_sig: &str,
        block: u64,
        now: DateTime<Utc>,
    ) {
        self.routes = candidates.into_iter().map(|c| {
            CachedRoute {
                route_id: route_id_of(c.chain_id, &c.fingerprints.directed),
                chain_id: c.chain_id,
                start_token: c.start_token,
                edges: c.edges,
                fingerprints: c.fingerprints,
                route_type: c.route_type,
                last_quote_at: now - chrono::Duration::seconds(3600),
                price_sig: String::new(),
                record: None,
                hits: 0,
            }
        }).collect();
        self.pool_index.clear();
        for (idx, r) in self.routes.iter().enumerate() {
            for e in &r.edges {
                self.pool_index.entry(e.pool_id.clone()).or_default().push(idx);
            }
        }
        self.topology_sig = topology_sig.to_string();
        self.last_full_scan_block = block;
    }

    /// 与上次扫描相比发生变化的池（区块号变化或新出现）。
    pub fn changed_pools(&self, view: &MarketView<'_>) -> HashSet<String> {
        let mut out = HashSet::new();
        for rt in view.pools.all() {
            let seen = self.pool_seen.get(&rt.pool.pool_id).copied();
            if seen != Some(pool_state_sig(rt)) {
                out.insert(rt.pool.pool_id.clone());
            }
        }
        out
    }

    /// 记录本轮池状态签名（价格按 1bp 量化，深度按粗粒度），块号变化但价格未动不算变更。
    pub fn note_pools(&mut self, view: &MarketView<'_>, now: DateTime<Utc>) {
        for rt in view.pools.all() {
            self.pool_seen
                .insert(rt.pool.pool_id.clone(), pool_state_sig(rt));
        }
        self.last_scan_at = Some(now);
    }

    fn update_quote(
        &mut self,
        idx: usize,
        record: RouteScanRecord,
        price_sig: String,
        now: DateTime<Utc>,
    ) {
        if let Some(r) = self.routes.get_mut(idx) {
            r.last_quote_at = now;
            r.price_sig = price_sig;
            r.record = Some(record);
        }
    }
}

fn route_id_of(chain_id: u64, directed: &str) -> String {
    format!("route_{chain_id}_{directed}")
}

/// 一次扫描的输出。
#[derive(Clone, Debug)]
pub struct DexScanOutput {
    pub routes: Vec<RouteScanRecord>,
    pub opportunities: Vec<Opportunity>,
    pub stats: ScanRunStats,
    pub graph: GraphStats,
}

/// 单条路线的完整定价结果。
struct PricedAmount {
    eval: AmountEvaluation,
    path: pool::PathQuote,
    costs: CostParts,
    gas: GasEstimate,
    flash: bool,
}

/// 执行一次扫描（含增量复用）。
pub fn scan(
    view: &MarketView<'_>,
    params: &DexScanParams,
    ctx: &ScanContext,
    cache: &mut MultiHopCache,
) -> DexScanOutput {
    let started = Instant::now();
    let now = view.now;
    let mut stats = ScanRunStats {
        run_id: crate::domain::new_id("scan"),
        started_at: Some(now),
        min_hops: params.min_hops,
        max_hops: params.max_hops,
        model_level: ModelLevel::M1,
        ..Default::default()
    };

    let token_registry = TokenRegistry::build(view.tokens);
    let pool_limits = PoolLimits {
        min_tvl_usd: params.min_pool_tvl_usd,
        max_state_age_ms: params.max_pool_state_age_ms,
        min_hop_liquidity_usd: params.min_pool_tvl_usd / Decimal::from(5),
    };
    let graph = TokenGraph::build(view.pools, &token_registry, &pool_limits, now);
    let graph_stats = graph.stats().clone();
    stats.pools = graph_stats.pools;
    stats.eligible_pools = graph_stats.eligible_pools;
    stats.tokens = graph_stats.tokens;
    stats.approved_tokens = graph_stats.approved_tokens;
    stats.edges = graph_stats.edges;

    let bases: Vec<String> = params
        .base_symbols
        .iter()
        .filter(|s| token_registry.start_ok(s) && graph.has_token(s))
        .cloned()
        .collect();
    stats.base_tokens = bases.clone();
    if bases.is_empty() {
        stats.notes.push("没有通过准入的起始资产，交易图为空".to_string());
        cache.note_pools(view, now);
        cache.last_stats = Some(stats.clone());
        return DexScanOutput {
            routes: Vec::new(),
            opportunities: Vec::new(),
            stats,
            graph: graph_stats,
        };
    }

    let topology_changed = cache.topology_sig != graph.signature();
    let block_gap = view.head_block.saturating_sub(cache.last_full_scan_block);
    let full = topology_changed
        || cache.routes.is_empty()
        || block_gap >= params.full_rescan_blocks.max(1);

    if full {
        let limits = SearchLimits {
            min_hops: params.min_hops,
            max_hops: params.max_hops,
            max_pools: params.max_pools,
            max_candidates: params.max_candidates,
            max_visits: params.max_visits,
            max_elapsed_ms: params.max_elapsed_ms,
        };
        let outcome = route_search::search(&graph, &bases, &limits);
        stats.candidates = outcome.routes.len();
        stats.visits = outcome.stats.visits;
        stats.truncated = outcome.stats.truncated;
        stats.reason = if topology_changed {
            "拓扑变化：重建路线集合".into()
        } else if cache.routes.is_empty() {
            "首次扫描：全量路线枚举".into()
        } else {
            format!("周期性全量重建（每 {} 区块）", params.full_rescan_blocks)
        };
        cache.rebuild(outcome.routes, graph.signature(), view.head_block, now);
        cache.last_search_visits = outcome.stats.visits;
    } else {
        stats.candidates = cache.routes.len();
        stats.visits = cache.last_search_visits;
        stats.reason = "增量扫描：复用路线集合".into();
    }
    stats.full_rescan = full;

    // 受影响路线：所涉任一池状态变化、报价过期、价格变化或从未报价。
    let changed = if full {
        HashSet::new()
    } else {
        cache.changed_pools(view)
    };
    let mut to_quote: Vec<usize> = Vec::new();
    for idx in 0..cache.routes.len() {
        // 冷热分级只调整"多久重算一次"：热路线更快重算，冷路线更慢但**不淘汰**。
        let (touched, stale, price_changed, no_record) = {
            let route = &cache.routes[idx];
            let effective_age = match tier_of(route) {
                "hot" => (params.requote_age_ms / 2).max(200),
                "cold" => params.requote_age_ms.saturating_mul(2),
                _ => params.requote_age_ms,
            };
            (
                route.edges.iter().any(|e| changed.contains(&e.pool_id)),
                (now - route.last_quote_at).num_milliseconds() > effective_age,
                route.price_sig != price_signature(view, &route.edges),
                route.record.is_none(),
            )
        };
        if full || touched || stale || price_changed || no_record {
            to_quote.push(idx);
        } else {
            stats.reused += 1;
            cache.routes[idx].hits = cache.routes[idx].hits.saturating_add(1);
        }
    }
    // V22-204：探索预算——未探索路线（hits=0）优先进入本轮定价，避免只扫旧路线。
    to_quote.sort_by_key(|i| cache.routes[*i].hits);
    stats.requoted = to_quote.len();

    let mut routes_out: Vec<RouteScanRecord> = Vec::new();
    let mut opportunities: Vec<Opportunity> = Vec::new();

    // 定价阶段也需要硬预算：候选可能上万条，不能只给枚举加 250ms 超时。
    let eval_started = Instant::now();
    let eval_budget = std::time::Duration::from_millis(params.max_evaluate_ms.max(50));
    let mut evaluated = 0usize;
    let mut attempted: HashSet<usize> = HashSet::new();
    let total = to_quote.len();
    let start_at = if total == 0 { 0 } else { cache.eval_cursor % total };
    for step in 0..total {
        if evaluated >= params.max_evaluate {
            stats.truncated = true;
            stats
                .notes
                .push(format!("定价数量达上限 {}，其余留待下一轮", params.max_evaluate));
            break;
        }
        if eval_started.elapsed() >= eval_budget {
            stats.truncated = true;
            stats.notes.push(format!(
                "定价预算 {}ms 用尽，已评估 {} 条",
                params.max_evaluate_ms, evaluated
            ));
            break;
        }
        let idx = to_quote[(start_at + step) % total];
        evaluated += 1;
        attempted.insert(idx);
        let cached = cache.routes[idx].clone();
        match evaluate_route(view, params, ctx, &token_registry, &cached) {
            Some((record, opp)) => {
                stats.profitable += usize::from(record.net_usd > Decimal::ZERO);
                if record.lifecycle == OpportunityLifecycle::Rejected {
                    stats.rejected_by_gate += 1;
                }
                cache.update_quote(
                    idx,
                    record.clone(),
                    price_signature(view, &cached.edges),
                    now,
                );
                routes_out.push(record);
                opportunities.push(opp);
            }
            None => {
                stats.rejected_by_gate += 1;
                stats.notes.push(format!("路线 {} 无法报价，已跳过", cached.route_id));
            }
        }
    }

    // 复用上一次报价：只更新年龄与来源，不重新报价。
    cache.eval_cursor = if total == 0 { 0 } else { (start_at + evaluated) % total };
    for (idx, route) in cache.routes.iter().enumerate() {
        if attempted.contains(&idx) {
            continue;
        }
        if let Some(mut record) = route.record.clone() {
            record.source = "incremental".into();
            record.quote_age_ms = record
                .quote_age_ms
                .max((now - route.last_quote_at).num_milliseconds());
            // 池状态签名未变时，复用报价视为当前区块有效，避免被 block_diff 误拒。
            record.block_number = view.head_block;
            if record.quote_age_ms > params.ttl_sec * 1_000 {
                record.lifecycle = OpportunityLifecycle::Expired;
            }
            match to_opportunity(view, params, ctx, &record) {
                Some(opp) => opportunities.push(opp),
                None => {
                    // 复用路线拿不到机会：多半是起始 Token 无价格或腿为空，记一条诊断便于定位
                    stats.notes.push(format!(
                        "复用路线 {} 未生成机会：start={} price={} legs={}",
                        record.route_id,
                        record.start_token,
                        view.prices.get(&record.start_token).copied().unwrap_or(Decimal::ZERO),
                        record.legs.len()
                    ));
                }
            }
            routes_out.push(record);
        }
    }

    routes_out.sort_by(|a, b| b.net_usd.cmp(&a.net_usd));
    opportunities.sort_by(|a, b| b.conservative_net_profit.cmp(&a.conservative_net_profit));

    cache.note_pools(view, now);
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    stats.started_at = Some(now);
    cache.last_stats = Some(stats.clone());

    DexScanOutput {
        routes: routes_out,
        opportunities,
        stats,
        graph: graph_stats,
    }
}

/// 对单条缓存路线重新报价、优化、模拟并生成记录与机会。
fn evaluate_route(
    view: &MarketView<'_>,
    params: &DexScanParams,
    ctx: &ScanContext,
    registry: &TokenRegistry,
    cached: &CachedRoute,
) -> Option<(RouteScanRecord, Opportunity)> {
    let now = view.now;
    let start = cached.start_token.clone();
    let start_price = view.prices.get(&start).copied().unwrap_or(Decimal::ZERO);
    if start_price <= Decimal::ZERO {
        tracing::warn!(route = %cached.route_id, start = %start, "报价失败：缺少起始 Token 价格");
        return None;
    }

    let capacity_usd = capacity_usd(view, &cached.edges, params.max_pool_share_pct);
    let max_usd = capacity_usd.min(params.max_input_usd);
    let min_amount = round_step_down(
        params.min_input_usd / start_price,
        Decimal::new(1, 8),
    );
    let max_amount = round_step_down(max_usd / start_price, Decimal::new(1, 8));
    if max_amount <= Decimal::ZERO || max_amount < min_amount {
        tracing::warn!(route = %cached.route_id, capacity = %capacity_usd, max_amount = %max_amount, min_amount = %min_amount, "报价失败：容量/金额边界");
        for e in &cached.edges {
            if let Some(rt) = view.pools.get(&e.pool_id) {
                let zero_for_one = e.token_in == rt.pool.token0;
                let reserve_in = if zero_for_one { rt.state.reserve0 } else { rt.state.reserve1 };
                let price_in = view.prices.get(&e.token_in).copied().unwrap_or(Decimal::ONE);
                tracing::warn!(pool = %e.pool_id, token = %e.token_in, reserve = %reserve_in, price = %price_in, value = %(reserve_in * price_in), "容量来源");
            }
        }
        return None;
    }

    let ladder: Vec<Decimal> = params
        .notional_ladder
        .iter()
        .map(|usd| round_step_down(*usd / start_price, Decimal::new(1, 8)))
        .filter(|a| *a >= min_amount && *a <= max_amount)
        .collect();

    let opt_params = OptimizationParams {
        ladder_amounts: ladder,
        min_amount,
        max_amount,
        grid_points: params.grid_points,
        refine_iters: params.refine_iters,
        quantity_step: Decimal::new(1, 8),
    };

    let optimization = optimizer::optimize(&opt_params, |amount| {
        price_amount(view, params, ctx, &start, start_price, &cached.edges, amount)
            .map(|p| p.eval)
    });

    // 选定价点：优先最优盈利点；否则取净利最高的采样点（用于展示拒绝原因）。
    let chosen_amount = optimization
        .best
        .as_ref()
        .map(|p| p.amount_in)
        .or_else(|| {
            optimization
                .points
                .iter()
                .max_by(|a, b| a.net_usd.cmp(&b.net_usd))
                .map(|p| p.amount_in)
        })?;
    let priced = match price_amount(
        view,
        params,
        ctx,
        &start,
        start_price,
        &cached.edges,
        chosen_amount,
    ) {
        Some(p) => p,
        None => {
            tracing::warn!(route = %cached.route_id, amount = %chosen_amount, "报价失败：price_amount 返回空");
            return None;
        }
    };

    let block = priced.path.block_number;
    let quoted_out = priced.path.amount_out;
    let m1 = sim::simulate_m1(view.pools, &start, chosen_amount, &cached.edges, quoted_out);
    let stress = {
        let flash = priced.flash;
        let notional = priced.eval.notional_usd;
        let fee_bps = params.flash_fee_bps;
        let base_gas = priced.gas.clone();
        let priority = params.priority_fee_usd;
        let delay = params.delay_reserve_bps;
        let mev = params.mev_reserve_bps;
        let flash_multiplier =
            Decimal::ONE + params.stress.flash_fee_increase_pct / Decimal::from(100);
        sim::simulate_m3(
            view.pools,
            &start,
            chosen_amount,
            &cached.edges,
            quoted_out,
            &params.stress,
            Some(&base_gas),
            move |out, gas_est| {
                let gas_usd = gas_est.map(|g| g.usd).unwrap_or(base_gas.usd);
                let mut c = CostParts::new();
                c.gas = gas_usd;
                c.priority_fee = priority;
                if flash {
                    c.flash_fee = bps_of_dec(notional, fee_bps) * flash_multiplier;
                }
                c.delay_reserve = usd_bps(notional, delay);
                c.mev_reserve = usd_bps(notional, mev);
                (out - chosen_amount) * (notional / chosen_amount) - c.total()
            },
        )
    };

    let theoretical_edge_bps = sim::theoretical_out(view.pools, &start, chosen_amount, &cached.edges)
        .map(|theo| (theo / chosen_amount - Decimal::ONE) * Decimal::from(10_000))
        .unwrap_or(Decimal::ZERO);

    let embedded_fee = embedded_fee_usd(view, &priced.path);
    let quality = route_quality(view, &cached.edges);

    let mut record = RouteScanRecord {
        route_id: cached.route_id.clone(),
        chain_id: view.chain_id,
        start_token: start.clone(),
        route_type: cached.route_type,
        fingerprints: cached.fingerprints.clone(),
        token_path: token_path(&start, &cached.edges),
        pool_path: cached.edges.iter().map(|e| e.pool_id.clone()).collect(),
        protocols: cached
            .edges
            .iter()
            .filter_map(|e| view.pools.get(&e.pool_id).map(|p| p.pool.protocol_id.clone()))
            .collect(),
        amms: cached
            .edges
            .iter()
            .map(|e| e.kind.as_str().to_string())
            .collect(),
        hops: cached.edges.len(),
        legs: leg_records(view, &priced.path, now),
        amount_in: chosen_amount,
        amount_out: quoted_out,
        notional_usd: priced.eval.notional_usd,
        gross_usd: priced.eval.gross_usd,
        total_cost_usd: priced.costs.total(),
        net_usd: priced.eval.net_usd,
        net_bps: if priced.eval.notional_usd > Decimal::ZERO {
            priced.eval.net_usd / priced.eval.notional_usd * Decimal::from(10_000)
        } else {
            Decimal::ZERO
        },
        embedded_fee_usd: embedded_fee,
        gas: Some(priced.gas.clone()),
        flash_funded: priced.flash,
        optimization: Some(optimization),
        simulation: Some(m1.clone()),
        stress: Some(stress.clone()),
        lifecycle: OpportunityLifecycle::Quoted,
        rejections: Vec::new(),
        block_number: block,
        quote_age_ms: oldest_pool_age_ms(view, &cached.edges, now),
        detected_at: now,
        expires_at: now + chrono::Duration::seconds(params.ttl_sec),
        model_level: ModelLevel::M1,
        opportunity_id: None,
        capacity_usd: max_usd,
        size_quotes: Vec::new(),
        source: "full".into(),
    };

    // 固定资金档位逐档报价（用于对比不同规模下的手续费/冲击/净利）。
    let mut size_quotes = Vec::new();
    for usd in &params.notional_ladder {
        let amount = round_step_down(*usd / start_price, Decimal::new(1, 8));
        if amount <= Decimal::ZERO || amount < min_amount || amount > max_amount {
            continue;
        }
        let Some(p) = price_amount(view, params, ctx, &start, start_price, &cached.edges, amount)
        else {
            continue;
        };
        size_quotes.push(crate::domain::dex_scan::SizeQuote {
            notional_usd: p.eval.notional_usd,
            amount_in: amount,
            amount_out: p.path.amount_out,
            gross_usd: p.eval.gross_usd,
            total_cost_usd: p.costs.total(),
            net_usd: p.eval.net_usd,
            net_bps: if p.eval.notional_usd > Decimal::ZERO {
                p.eval.net_usd / p.eval.notional_usd * Decimal::from(10_000)
            } else {
                Decimal::ZERO
            },
        });
    }
    record.size_quotes = size_quotes;

    record.rejections = gate_route(
        view,
        params,
        registry,
        &record,
        &m1,
        &stress,
        theoretical_edge_bps,
        quality,
    );
    record.advance_lifecycle();

    let opp = to_opportunity(view, params, ctx, &record)?;
    record.opportunity_id = Some(opp.opportunity_id.clone());
    Some((record, opp))
}

/// 对给定金额做完整定价（报价 + Gas + 现金成本）。
fn price_amount(
    view: &MarketView<'_>,
    params: &DexScanParams,
    ctx: &ScanContext,
    start: &str,
    start_price: Decimal,
    edges: &[HopEdge],
    amount: Decimal,
) -> Option<PricedAmount> {
    if amount <= Decimal::ZERO {
        return None;
    }
    let path = pool::quote_path(view.pools, start, amount, edges).ok()?;
    if path.amount_out <= Decimal::ZERO {
        return None;
    }
    let notional = amount * start_price;
    let gross = (path.amount_out - amount) * start_price;
    let available = ctx.dex_available.get(start).copied().unwrap_or(Decimal::ZERO);
    let flash = available < amount;

    let kinds: Vec<_> = edges
        .iter()
        .map(|e| e.kind)
        .collect();
    let gas = gas::estimate(
        &kinds,
        flash,
        0,
        &params.gas,
        params.gas_price_gwei,
        params.native_price_usd,
        path.block_number,
    );

    let mut costs = CostParts::new();
    costs.gas = gas.usd;
    costs.priority_fee = params.priority_fee_usd;
    if flash {
        costs.flash_fee = bps_of_dec(notional, params.flash_fee_bps);
    }
    costs.delay_reserve = usd_bps(notional, params.delay_reserve_bps);
    costs.mev_reserve = usd_bps(notional, params.mev_reserve_bps);

    let total = costs.total();
    let eval = AmountEvaluation {
        amount_in: amount,
        amount_out: path.amount_out,
        notional_usd: notional,
        gross_usd: gross,
        cost_usd: total,
        net_usd: gross - total,
        gas_usd: gas.usd,
        price_impact_bps: path.price_impact_bps,
    };
    Some(PricedAmount {
        eval,
        path,
        costs,
        gas,
        flash,
    })
}

/// 路线容量：受最紧一跳「单侧储备价值 × 单池占比上限」约束。
fn capacity_usd(view: &MarketView<'_>, edges: &[HopEdge], share_pct: Decimal) -> Decimal {
    let share = (share_pct / Decimal::from(100)).clamp(Decimal::ZERO, Decimal::ONE);
    let mut cap = Decimal::MAX;
    for e in edges {
        let Some(rt) = view.pools.get(&e.pool_id) else {
            return Decimal::ZERO;
        };
        let zero_for_one = e.token_in == rt.pool.token0;
        let reserve_in = if zero_for_one {
            rt.state.reserve0
        } else {
            rt.state.reserve1
        };
        let price_in = view.prices.get(&e.token_in).copied().unwrap_or(Decimal::ONE);
        let value = reserve_in * price_in;
        if value <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        cap = cap.min(value * share);
    }
    if cap == Decimal::MAX {
        Decimal::ZERO
    } else {
        cap
    }
}

fn leg_records(view: &MarketView<'_>, path: &pool::PathQuote, now: DateTime<Utc>) -> Vec<RouteLegRecord> {
    path.legs
        .iter()
        .enumerate()
        .map(|(idx, leg)| {
            let rt = view.pools.get(&leg.pool_id);
            RouteLegRecord {
                idx,
                pool_id: leg.pool_id.clone(),
                protocol_id: rt.map(|p| p.pool.protocol_id.clone()).unwrap_or_default(),
                amm: rt.map(|p| p.pool.kind).unwrap_or(crate::domain::market::PoolKind::Unknown),
                token_in: leg.token_in.clone(),
                token_out: leg.token_out.clone(),
                amount_in: leg.amount_in,
                amount_out: leg.amount_out,
                fee_bps: leg.fee_bps,
                price_impact_bps: leg.price_impact_bps,
                gas_units: rt.map(|p| p.gas_units()).unwrap_or(0),
                pool_tvl_usd: rt.map(|p| p.pool.tvl_usd).unwrap_or(Decimal::ZERO),
                block_number: rt.map(|p| p.state.block_number).unwrap_or(0),
                state_age_ms: rt.map(|p| p.state.age_ms(now)).unwrap_or(i64::MAX),
            }
        })
        .collect()
}

fn embedded_fee_usd(view: &MarketView<'_>, path: &pool::PathQuote) -> Decimal {
    path.legs
        .iter()
        .map(|leg| {
            let price = view.prices.get(&leg.token_in).copied().unwrap_or(Decimal::ZERO);
            bps_of(leg.amount_in * price, leg.fee_bps)
        })
        .sum()
}

fn token_path(start: &str, edges: &[HopEdge]) -> Vec<String> {
    let mut out = vec![start.to_string()];
    for e in edges {
        out.push(e.token_out.clone());
    }
    out
}

fn oldest_pool_age_ms(view: &MarketView<'_>, edges: &[HopEdge], now: DateTime<Utc>) -> i64 {
    edges
        .iter()
        .filter_map(|e| view.pools.get(&e.pool_id))
        .map(|p| p.state.age_ms(now))
        .max()
        .unwrap_or(0)
}

fn price_signature(view: &MarketView<'_>, edges: &[HopEdge]) -> String {
    let mut tokens: Vec<&str> = edges.iter().map(|e| e.token_out.as_str()).collect();
    if let Some(first) = edges.first() {
        tokens.push(first.token_in.as_str());
    }
    tokens.sort_unstable();
    tokens.dedup();
    let mut s = String::new();
    for t in tokens {
        let p = view.prices.get(t).copied().unwrap_or(Decimal::ZERO);
        s.push_str(&format!("{t}={p};"));
    }
    format!("{:016x}", fnv1a_64(&s))
}

fn route_quality(view: &MarketView<'_>, edges: &[HopEdge]) -> DataQuality {
    let mut worst = DataQuality::High;
    let mut blocks = HashSet::new();
    for e in edges {
        let Some(rt) = view.pools.get(&e.pool_id) else {
            return DataQuality::Invalid;
        };
        if !rt.pool.verified || !rt.pool.execution_allowed || !rt.pool.kind.execution_allowed() {
            return DataQuality::Invalid;
        }
        blocks.insert(rt.state.block_number);
        // 未接 REVM/M2：实时快照最高只到 B，A 仅保留给 M2 验证。
        let q = DataQuality::from_age(rt.state.age_ms(view.now), 8_000);
        if q.rank() > worst.rank() {
            worst = q;
        }
    }
    if blocks.len() > 1 {
        return DataQuality::Invalid;
    }
    if blocks.iter().copied().next().unwrap_or(0) + 1 < view.head_block {
        worst = worst.max_rank(DataQuality::Low);
    }
    worst
}

trait MaxRank {
    fn max_rank(self, other: DataQuality) -> DataQuality;
}

impl MaxRank for DataQuality {
    fn max_rank(self, other: DataQuality) -> DataQuality {
        if other.rank() > self.rank() { other } else { self }
    }
}

/// 路线级门禁（PRD §8.3、§12）。返回含 WARNING 的违规列表；
/// 只有非 WARNING 项会阻断可执行机会。
fn gate_route(
    view: &MarketView<'_>,
    params: &DexScanParams,
    registry: &TokenRegistry,
    record: &RouteScanRecord,
    m1: &SimulationOutcome,
    stress: &SimulationOutcome,
    theoretical_edge_bps: Decimal,
    quality: DataQuality,
) -> Vec<RiskViolation> {
    let mut out = Vec::new();
    let hard = RiskSeverity::HardStop;

    for token in &record.token_path {
        if !registry.hop_ok(token) {
            out.push(RiskViolation {
                key: "token_admission".into(),
                label: "Token 准入".into(),
                severity: hard,
                limit: Decimal::ONE,
                actual: Decimal::ZERO,
                unit: "bool".into(),
                message: format!("Token {token} 未通过准入（{}）", registry.status(token)),
            });
        }
    }

    for leg in &record.legs {
        let Some(rt) = view.pools.get(&leg.pool_id) else {
            out.push(RiskViolation {
                key: "pool_missing".into(),
                label: "池状态缺失".into(),
                severity: hard,
                limit: Decimal::ONE,
                actual: Decimal::ZERO,
                unit: "bool".into(),
                message: format!("池 {} 状态缺失", leg.pool_id),
            });
            continue;
        };
        if !rt.pool.verified || !rt.pool.execution_allowed || !rt.pool.kind.execution_allowed() {
            out.push(RiskViolation {
                key: "pool_admission".into(),
                label: "池准入".into(),
                severity: hard,
                limit: Decimal::ONE,
                actual: Decimal::ZERO,
                unit: "bool".into(),
                message: format!("池 {} 未通过执行准入", leg.pool_id),
            });
        }
    }

    match m1.status {
        SimulationStatus::Passed => {}
        SimulationStatus::ModelMismatch => out.push(RiskViolation {
            key: "model_mismatch".into(),
            label: "报价与模拟不一致".into(),
            severity: hard,
            limit: params.model_mismatch_max_bps,
            actual: m1.diff_bps.abs(),
            unit: "bps".into(),
            message: format!(
                "M1 复算差异 {} bps 超过阈值 {} bps",
                m1.diff_bps.abs(),
                params.model_mismatch_max_bps
            ),
        }),
        SimulationStatus::Failed => out.push(RiskViolation {
            key: "simulation_failed".into(),
            label: "M1 状态一致性模拟失败".into(),
            severity: hard,
            limit: Decimal::ONE,
            actual: Decimal::ZERO,
            unit: "bool".into(),
            message: m1.notes.join("；"),
        }),
        SimulationStatus::Unavailable => out.push(RiskViolation {
            key: "simulation_unavailable".into(),
            label: "模拟不可用".into(),
            severity: hard,
            limit: Decimal::ONE,
            actual: Decimal::ZERO,
            unit: "bool".into(),
            message: "M1 模拟不可用，禁止生成可执行机会".into(),
        }),
    }

    if record.hops > params.max_hops {
        out.push(RiskViolation {
            key: "route_hops_max".into(),
            label: "路线最大跳数".into(),
            severity: hard,
            limit: Decimal::from(params.max_hops as u64),
            actual: Decimal::from(record.hops as u64),
            unit: "跳".into(),
            message: format!("路线 {} 跳超过上限 {}", record.hops, params.max_hops),
        });
    }

    if record.notional_usd > record.capacity_usd {
        out.push(RiskViolation {
            key: "route_capacity".into(),
            label: "路线可执行容量".into(),
            severity: hard,
            limit: record.capacity_usd,
            actual: record.notional_usd,
            unit: "USDC".into(),
            message: format!(
                "名义 {} USDC 超过路线容量 {} USDC",
                record.notional_usd, record.capacity_usd
            ),
        });
    }

    if quality.rank() >= DataQuality::Low.rank() {
        out.push(RiskViolation {
            key: "state_snapshot".into(),
            label: "状态快照质量".into(),
            severity: hard,
            limit: Decimal::ONE,
            actual: Decimal::from(quality.rank() as u64),
            unit: "等级".into(),
            message: format!("路线状态质量 {quality}，禁止生成可执行机会"),
        });
    }

    if let Some(net) = stress.net_usd {
        if net <= Decimal::ZERO {
            out.push(RiskViolation {
                key: "stress_negative".into(),
                label: "压力模拟净利".into(),
                severity: RiskSeverity::Warning,
                limit: Decimal::ZERO,
                actual: net,
                unit: "USDC".into(),
                message: format!(
                    "M3 压力模拟净利 {} USDC，机会不抗压（仅警告）",
                    net.round_dp(4)
                ),
            });
        }
    }

    if record.net_bps <= Decimal::ZERO {
        out.push(RiskViolation {
            key: "route_not_profitable".into(),
            label: "路线净利润".into(),
            severity: RiskSeverity::Warning,
            limit: Decimal::ZERO,
            actual: record.net_bps,
            unit: "bps".into(),
            message: format!(
                "最优投入下净利 {} bps，仅作为观察项",
                record.net_bps.round_dp(2)
            ),
        });
    }

    // 理论价差远大于实际净利时记录误差，但不阻断（AMM 费用与冲击本来就会吃掉价差）。
    if theoretical_edge_bps > Decimal::ZERO && record.net_bps < theoretical_edge_bps / Decimal::from(2) {
        out.push(RiskViolation {
            key: "theoretical_gap".into(),
            label: "理论价差与实际净利差距".into(),
            severity: RiskSeverity::Warning,
            limit: theoretical_edge_bps / Decimal::from(2),
            actual: record.net_bps,
            unit: "bps".into(),
            message: format!(
                "理论价差 {} bps，实际净利 {} bps（含手续费与冲击）",
                theoretical_edge_bps.round_dp(2),
                record.net_bps.round_dp(2)
            ),
        });
    }

    out
}

/// 记录 → 统一 Opportunity（供风控、Paper 与前端复用）。
fn to_opportunity(
    view: &MarketView<'_>,
    params: &DexScanParams,
    ctx: &ScanContext,
    record: &RouteScanRecord,
) -> Option<Opportunity> {
    let start = &record.start_token;
    let start_price = view.prices.get(start).copied().unwrap_or(Decimal::ZERO);
    if start_price <= Decimal::ZERO {
        return None;
    }
    if record.legs.is_empty() {
        return None;
    }
    let available = ctx.dex_available.get(start).copied().unwrap_or(Decimal::ZERO);
    let flash = record.flash_funded;

    let buffer = Decimal::ONE + Decimal::new(5, 3);
    let mut legs = Vec::with_capacity(record.legs.len());
    for leg in &record.legs {
        legs.push(RouteLeg {
            idx: leg.idx,
            protocol_id: leg.protocol_id.clone(),
            adapter: leg.protocol_id.clone(),
            pool_id: leg.pool_id.clone(),
            token_in: leg.token_in.clone(),
            token_out: leg.token_out.clone(),
            amount_in: leg.amount_in,
            expected_out: leg.amount_out,
            min_out: q8(leg.amount_out / buffer),
            fee_bps: leg.fee_bps,
            price_impact_bps: leg.price_impact_bps,
        });
    }

    let gas_usd = record.gas.as_ref().map(|g| g.usd).unwrap_or(Decimal::ZERO);
    let mut costs = CostParts::new();
    costs.gas = gas_usd;
    costs.priority_fee = params.priority_fee_usd;
    if flash {
        costs.flash_fee = bps_of_dec(record.notional_usd, params.flash_fee_bps);
    }
    costs.delay_reserve = usd_bps(record.notional_usd, params.delay_reserve_bps);
    costs.mev_reserve = usd_bps(record.notional_usd, params.mev_reserve_bps);

    let theoretical_bps = record
        .legs
        .iter()
        .map(|l| {
            view.pools
                .get(&l.pool_id)
                .and_then(|p| p.spot_price(&l.token_in))
                .unwrap_or(Decimal::ZERO)
        })
        .try_fold(Decimal::ONE, |acc, spot| {
            if spot > Decimal::ZERO { Some(acc * spot) } else { None }
        })
        .map(|product| (product - Decimal::ONE) * Decimal::from(10_000))
        .unwrap_or(Decimal::ZERO);

    let scenario = build_scenario(
        theoretical_bps / Decimal::from(10_000) * record.notional_usd,
        record.gross_usd,
        &costs,
        params.profit_discount_pct,
        worst_case_loss(
            record.gross_usd,
            &costs,
            params.stress.adverse_bps,
            record.notional_usd,
        ),
    );
    let stress_net = record
        .stress
        .as_ref()
        .and_then(|s| s.net_usd)
        .unwrap_or(scenario.stress_net_usd);

    let funding = if flash {
        FundingSource::FlashLoan {
            provider: params.flash_provider.clone(),
            token: start.clone(),
            amount: q8(record.amount_in),
            fee_bps: params.flash_fee_bps,
            fee_amount: bps_of_dec(record.notional_usd, params.flash_fee_bps),
        }
    } else {
        FundingSource::OwnCapital
    };

    let route = DexRoute {
        route_id: record.route_id.clone(),
        chain_id: view.chain_id,
        start_token: start.clone(),
        end_token: start.clone(),
        legs,
        amount_in: record.amount_in,
        expected_out: record.amount_out,
        min_out: q8(record.amount_out / buffer),
        gas_units: record.gas.as_ref().map(|g| g.units).unwrap_or(0),
        gas_usd,
        price_impact_bps: record
            .legs
            .iter()
            .map(|l| l.price_impact_bps)
            .max()
            .unwrap_or(Decimal::ZERO),
        model_level: ModelLevel::M1,
        block_number: record.block_number,
        created_at: record.detected_at,
    };

    let strategy = if record.hops >= 3 {
        StrategyType::DexTriple
    } else {
        StrategyType::DexCross
    };
    // 稳定机会 ID：同一策略 + 路线 + 名义金额跨周期复用，避免每轮生成新记录。
    let id_seed = format!(
        "{}|{}|{}",
        strategy.as_str(),
        route.route_id,
        record.notional_usd
    );
    let opportunity_id = format!("opp_{:016x}", crate::domain::plan::fnv1a_64(&id_seed));

    Some(Opportunity {
        opportunity_id,
        strategy_type: strategy,
        chain_id: view.chain_id,
        chain_name: view.chain_name.clone(),
        dex_route: Some(route),
        cex_exchange: None,
        cex_symbol: None,
        direction: None,
        m2: None,
        target_notional: record.notional_usd,
        target_qty: record.amount_in,
        required_dex_inventory: vec![InventoryNeed {
            venue: format!("dex-{}", view.chain_name.to_lowercase()),
            asset: start.clone(),
            required: record.amount_in,
            available,
        }],
        required_cex_inventory: vec![],
        theoretical_edge_bps: theoretical_bps,
        executable_gross_profit: record.gross_usd,
        total_cost: record.total_cost_usd,
        conservative_net_profit: record.net_usd,
        stress_net_profit: stress_net,
        worst_case_loss: scenario.worst_case_loss_usd,
        costs: costs.breakdown(),
        scenario,
        risk_score: crate::domain::market::RiskScore::from_components(vec![]),
        data_quality: route_quality(view, &hop_edges(record)),
        model_level: ModelLevel::M1,
        confidence: format!(
            "M1 状态一致性 + M3 压力（{}）；指纹 {}；容量 {} USDC；AMM 手续费已内含 {} USDC",
            record.stress.as_ref().map(|s| s.status.as_str()).unwrap_or("N/A"),
            record.fingerprints.directed,
            record.capacity_usd.round_dp(2),
            record.embedded_fee_usd.round_dp(4)
        ),
        quote_timestamp: record.detected_at,
        quote_block_number: record.block_number,
        book_age_ms: 0,
        quote_age_ms: record.quote_age_ms,
        status: OppStatus::Watch,
        lifecycle: record.lifecycle,
        route_fingerprint: Some(record.fingerprints.directed.clone()),
        rejections: record.rejections.clone(),
        capacity_usd: record.capacity_usd,
        funding,
        hedge_order: None,
        detected_at: record.detected_at,
        expires_at: record.expires_at,
    })
}

fn hop_edges(record: &RouteScanRecord) -> Vec<HopEdge> {
    record
        .legs
        .iter()
        .map(|l| HopEdge {
            pool_id: l.pool_id.clone(),
            token_in: l.token_in.clone(),
            token_out: l.token_out.clone(),
            kind: l.amm,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cex::BookEngine;
    use crate::domain::market::{Instrument, PoolKind, PoolRef, PoolState, Token, TokenStatus};
    use crate::domain::money::dec;
    use crate::domain::risk::ControlState;
    use crate::dex::math;

    /// V22-308：七档资金必须逐档独立，不允许插值或合并。
    #[test]
    fn notional_ladder_has_seven_independent_tiers() {
        let p = DexScanParams::default();
        assert_eq!(p.notional_ladder.len(), 7, "七档：250/500/1000/2500/5000/7500/10000");
        let distinct: std::collections::BTreeSet<String> =
            p.notional_ladder.iter().map(|d| d.to_string()).collect();
        assert_eq!(distinct.len(), 7, "七档不得重复或插值合并");
    }

    /// 准备金不得为 0：为 0 等于把风险成本从净利里删掉。
    #[test]
    fn default_params_keep_risk_reserves() {
        let p = DexScanParams::default();
        assert_eq!(p.delay_reserve_bps, dec("8"));
        assert_eq!(p.mev_reserve_bps, dec("8"));
        assert_eq!(p.profit_discount_pct, dec("50"));
    }

    fn token(symbol: &str) -> Token {
        Token {
            symbol: symbol.into(),
            chain_id: 1,
            address: format!("0x{symbol}"),
            decimals: 6,
            standard: "ERC20".into(),
            code_hash: "0x1".into(),
            status: TokenStatus::Approved,
            risk_tags: vec![],
            allowed_start: true,
            allowed_hop: true,
            allowed_flash: true,
            special_transfer: false,
            enabled: true,
        }
    }

    fn add_pool(set: &mut pool::PoolSet, id: &str, t0: &str, t1: &str, r0: &str, r1: &str) {
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
                decimals0: 0,
                decimals1: 0,
                fee_bps: 30,
                tick_spacing: None,
                tvl_usd: dec("5000000"),
                verified: true,
                execution_allowed: true,
                added_at: now,
            },
            PoolState {
                pool_id: id.into(),
                block_number: 100,
                block_hash: "0xb".into(),
                updated_at: now,
                reserve0: dec(r0),
                reserve1: dec(r1),
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

    fn tokens() -> HashMap<String, Token> {
        let mut m = HashMap::new();
        for s in ["USDC", "WETH"] {
            m.insert(s.to_string(), token(s));
        }
        m
    }

    fn prices() -> HashMap<String, Decimal> {
        let mut m = HashMap::new();
        m.insert("USDC".into(), Decimal::ONE);
        m.insert("WETH".into(), dec("2800"));
        m
    }

    fn ctx(dex_available: HashMap<String, Decimal>) -> ScanContext {
        ScanContext {
            equity_usd: dec("5000"),
            peak_equity_usd: dec("5000"),
            account_daily_loss_usd: Decimal::ZERO,
            strategy_daily_loss: HashMap::new(),
            unrealized_pnl_usd: Decimal::ZERO,
            controls: ControlState::new(),
            head_block: 100,
            dex_available,
            cex_available: HashMap::new(),
            fill_rate: HashMap::new(),
        }
    }

    fn available() -> HashMap<String, Decimal> {
        let mut m = HashMap::new();
        m.insert("USDC".to_string(), dec("5000"));
        m
    }

    fn two_pool_market() -> pool::PoolSet {
        let mut pools = pool::PoolSet::new(1);
        add_pool(&mut pools, "p1", "USDC", "WETH", "1000000", "364");
        add_pool(&mut pools, "p2", "USDC", "WETH", "1000000", "370");
        pools
    }

    fn view<'a>(
        pools: &'a pool::PoolSet,
        tokens: &'a HashMap<String, Token>,
        prices: &'a HashMap<String, Decimal>,
        books: &'a BookEngine,
        instruments: &'a HashMap<String, Instrument>,
    ) -> MarketView<'a> {
        MarketView {
            now: Utc::now(),
            chain_id: 1,
            chain_name: "Arbitrum".into(),
            head_block: 100,
            pools,
            tokens,
            prices,
            books,
            instruments,
        }
    }

    #[test]
    fn scan_finds_cross_pool_route_and_builds_record() {
        let pools = two_pool_market();
        let tokens = tokens();
        let prices = prices();
        let books = BookEngine::new();
        let instruments: HashMap<String, Instrument> = HashMap::new();
        let v = view(&pools, &tokens, &prices, &books, &instruments);
        let mut cache = MultiHopCache::new();
        let out = scan(&v, &DexScanParams::default(), &ctx(available()), &mut cache);
        assert!(!out.routes.is_empty(), "应发现 2 跳闭环");
        assert_eq!(out.routes.len(), cache.route_count());
        let r = &out.routes[0];
        assert_eq!(r.route_type, RouteType::CrossPool);
        assert_eq!(r.hops, 2);
        assert_eq!(r.fingerprints.directed.len(), 16);
        assert!(r.optimization.is_some());
        let sim = r.simulation.as_ref().expect("M1");
        assert_eq!(sim.level, ModelLevel::M1);
        assert_eq!(sim.status, SimulationStatus::Passed);
        assert!(r.stress.is_some());
        assert_eq!(r.model_level, ModelLevel::M1);
        assert!(out.stats.edges >= 4);
        assert!(out.stats.visits > 0);
        assert!(!out.opportunities.is_empty());
        let opp = &out.opportunities[0];
        assert!(opp.route_fingerprint.is_some());
        assert_eq!(opp.model_level, ModelLevel::M1);
        assert!(!opp.dex_route.as_ref().unwrap().legs.is_empty());
    }

    #[test]
    fn incremental_scan_reuses_when_nothing_changes() {
        let pools = two_pool_market();
        let tokens = tokens();
        let prices = prices();
        let books = BookEngine::new();
        let instruments: HashMap<String, Instrument> = HashMap::new();
        let v = view(&pools, &tokens, &prices, &books, &instruments);
        let mut cache = MultiHopCache::new();
        let first = scan(&v, &DexScanParams::default(), &ctx(available()), &mut cache);
        assert!(first.stats.requoted > 0);
        let second = scan(&v, &DexScanParams::default(), &ctx(available()), &mut cache);
        assert_eq!(second.stats.requoted, 0, "池状态与价格未变，不应重新报价");
        assert_eq!(second.stats.reused, second.stats.candidates);
        assert!(second.routes.iter().all(|r| r.source == "incremental"));
        assert_eq!(second.routes.len(), first.routes.len());
    }

    #[test]
    fn capacity_respects_share_limit() {
        let pools = two_pool_market();
        let tokens = tokens();
        let prices = prices();
        let books = BookEngine::new();
        let instruments: HashMap<String, Instrument> = HashMap::new();
        let v = view(&pools, &tokens, &prices, &books, &instruments);
        let edges = vec![HopEdge {
            pool_id: "p1".into(),
            token_in: "USDC".into(),
            token_out: "WETH".into(),
            kind: PoolKind::UniswapV2,
        }];
        assert_eq!(capacity_usd(&v, &edges, Decimal::from(10)), dec("100000"));
    }

    #[test]
    fn unapproved_token_never_appears_in_routes() {
        let pools = two_pool_market();
        let mut tokens = tokens();
        tokens.get_mut("WETH").unwrap().status = TokenStatus::Unknown;
        let prices = prices();
        let books = BookEngine::new();
        let instruments: HashMap<String, Instrument> = HashMap::new();
        let v = view(&pools, &tokens, &prices, &books, &instruments);
        let mut cache = MultiHopCache::new();
        let out = scan(&v, &DexScanParams::default(), &ctx(available()), &mut cache);
        assert!(out.routes.is_empty());
        assert!(out.opportunities.is_empty());
        assert_eq!(out.stats.edges, 0);
    }
}