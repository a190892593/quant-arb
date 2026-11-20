//! 预算约束下的简单闭环搜索（PRD §4.3–§4.5）。
//!
//! 算法：从起始 Token 出发做有界深度优先搜索（DFS），只允许简单环——
//! 同一池不得复用、中间 Token 不得重复，回到起始 Token 即闭合。
//!
//! 预算：访问次数、候选路线数、时间、深度、池数五重限制；任一命中即停止并标记
//! truncated。搜索不调用报价模型，因此可以把候选数做到很大；报价与模拟由上层逐条执行。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::domain::dex_scan::RouteFingerprints;
use crate::domain::types::RouteType;
use crate::dex::fingerprint;
use crate::dex::graph::HopEdge;

use super::multigraph::{GraphEdge, TokenGraph};

/// 搜索预算（PRD §4.4）。
#[derive(Clone, Debug)]
pub struct SearchLimits {
    pub min_hops: usize,
    pub max_hops: usize,
    pub max_pools: usize,
    pub max_candidates: usize,
    pub max_visits: u64,
    pub max_elapsed_ms: u64,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            min_hops: 2,
            max_hops: 5,
            max_pools: 5,
            max_candidates: 100_000,
            max_visits: 200_000,
            max_elapsed_ms: 250,
        }
    }
}

/// 一条结构合法的候选闭环。此时尚未报价。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteCandidate {
    pub chain_id: u64,
    pub start_token: String,
    pub edges: Vec<HopEdge>,
    pub fingerprints: RouteFingerprints,
    pub route_type: RouteType,
}

impl RouteCandidate {
    pub fn hops(&self) -> usize {
        self.edges.len()
    }
}

/// 搜索统计（写入 ScanRunStats，供前端「候选 → 剪枝 → 精确模拟」漏斗展示）。
#[derive(Clone, Debug, Default)]
pub struct SearchStats {
    pub visits: u64,
    pub candidates: usize,
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub max_depth_reached: usize,
    pub pruned_pool_reuse: usize,
    pub pruned_token_reuse: usize,
    pub pruned_budget: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SearchOutcome {
    pub routes: Vec<RouteCandidate>,
    pub stats: SearchStats,
}

/// 从多个起始资产搜索闭环；预算在各起始资产之间共享。
pub fn search(
    graph: &TokenGraph,
    start_tokens: &[String],
    limits: &SearchLimits,
) -> SearchOutcome {
    let started = Instant::now();
    let deadline = Duration::from_millis(limits.max_elapsed_ms.max(1));
    let mut out = SearchOutcome::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut visits: u64 = 0;
    let mut truncated = false;

    for start in start_tokens {
        if out.routes.len() >= limits.max_candidates {
            truncated = true;
            break;
        }
        if !graph.has_token(start) {
            continue;
        }
        let mut path: Vec<HopEdge> = Vec::new();
        let mut used_pools: Vec<String> = Vec::new();
        let mut used_tokens: Vec<String> = vec![start.clone()];
        let mut state = WalkState {
            visits: &mut visits,
            truncated: &mut truncated,
            started: &started,
            deadline,
        };
        dfs(
            graph,
            start,
            start,
            0,
            limits,
            &mut state,
            &mut path,
            &mut used_pools,
            &mut used_tokens,
            &mut out.routes,
            &mut seen,
            &mut out.stats,
        );
        if *state.truncated && started.elapsed() >= deadline {
            break;
        }
    }

    out.stats.visits = visits;
    out.stats.candidates = out.routes.len();
    out.stats.truncated = truncated;
    out.stats.elapsed_ms = started.elapsed().as_millis() as u64;
    out
}

struct WalkState<'a> {
    visits: &'a mut u64,
    truncated: &'a mut bool,
    started: &'a Instant,
    deadline: Duration,
}

#[allow(clippy::too_many_arguments)]
fn dfs(
    graph: &TokenGraph,
    start: &str,
    current: &str,
    depth: usize,
    limits: &SearchLimits,
    state: &mut WalkState<'_>,
    path: &mut Vec<HopEdge>,
    used_pools: &mut Vec<String>,
    used_tokens: &mut Vec<String>,
    routes: &mut Vec<RouteCandidate>,
    seen: &mut HashSet<String>,
    stats: &mut SearchStats,
) {
    if *state.truncated {
        return;
    }
    if depth >= limits.max_hops || used_pools.len() >= limits.max_pools {
        return;
    }
    if *state.visits >= limits.max_visits
        || routes.len() >= limits.max_candidates
        || state.started.elapsed() >= state.deadline
    {
        *state.truncated = true;
        stats.pruned_budget += 1;
        return;
    }

    let edges = graph.edges_from(current).to_vec();
    for edge in edges {
        *state.visits += 1;
        stats.max_depth_reached = stats.max_depth_reached.max(depth + 1);
        if *state.visits >= limits.max_visits
            || routes.len() >= limits.max_candidates
            || state.started.elapsed() >= state.deadline
        {
            *state.truncated = true;
            stats.pruned_budget += 1;
            return;
        }
        if used_pools.iter().any(|p| p == &edge.pool_id) {
            stats.pruned_pool_reuse += 1;
            continue;
        }

        let next = edge.token_out.as_str();
        let closes = next == start;

        if closes {
            let hops = depth + 1;
            if hops < limits.min_hops.max(2) {
                continue;
            }
            let hop = edge.to_hop();
            let mut cycle: Vec<HopEdge> = path.clone();
            cycle.push(hop);
            let fp = fingerprint::fingerprints(start, &cycle);
            if !seen.insert(fp.directed.clone()) {
                continue;
            }
            routes.push(RouteCandidate {
                chain_id: graph.chain_id(),
                start_token: start.to_string(),
                route_type: RouteType::of_hops(cycle.len()),
                edges: cycle,
                fingerprints: fp,
            });
            continue;
        }

        // 简单环：中间 Token 不得重复（回到起点已在上方按闭环处理）。
        if used_tokens.iter().any(|t| t == next) {
            stats.pruned_token_reuse += 1;
            continue;
        }
        if depth + 1 >= limits.max_hops {
            continue;
        }

        path.push(edge.to_hop());
        used_pools.push(edge.pool_id.clone());
        used_tokens.push(next.to_string());
        dfs(
            graph,
            start,
            next,
            depth + 1,
            limits,
            state,
            path,
            used_pools,
            used_tokens,
            routes,
            seen,
            stats,
        );
        used_tokens.pop();
        used_pools.pop();
        path.pop();
        if *state.truncated {
            return;
        }
    }
}

/// 单条路线的静态描述（供日志/调试）。
pub fn describe(edges: &[GraphEdge]) -> String {
    edges
        .iter()
        .map(|e| format!("{}:{}/{}", e.pool_id, e.token_in, e.token_out))
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::{PoolKind, PoolRef, PoolState, Token, TokenStatus};
    use crate::domain::money::dec;
    use crate::dex::math;
    use crate::dex::pool::PoolSet;
    use crate::dex::registry::{PoolLimits, TokenRegistry};
    use std::collections::HashMap;

    const TOKENS: [&str; 6] = ["A", "B", "C", "D", "E", "F"];

    fn registry() -> TokenRegistry {
        let mut tokens = HashMap::new();
        for s in TOKENS {
            tokens.insert(
                s.to_string(),
                Token {
                    symbol: s.into(),
                    chain_id: 1,
                    address: format!("0x{s}"),
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
                },
            );
        }
        TokenRegistry::build(&tokens)
    }

    fn add_pool(set: &mut PoolSet, id: &str, t0: &str, t1: &str) {
        let now = chrono::Utc::now();
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
                tvl_usd: dec("1000000"),
                verified: true,
                execution_allowed: true,
                added_at: now,
            },
            PoolState {
                pool_id: id.into(),
                block_number: 1,
                block_hash: "0xb".into(),
                updated_at: now,
                reserve0: dec("1000000"),
                reserve1: dec("1000000"),
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

    fn graph() -> TokenGraph {
        let mut set = PoolSet::new(1);
        add_pool(&mut set, "p_ab", "A", "B");
        add_pool(&mut set, "p_bc", "B", "C");
        add_pool(&mut set, "p_ca", "C", "A");
        add_pool(&mut set, "p_cd", "C", "D");
        add_pool(&mut set, "p_da", "D", "A");
        TokenGraph::build(&set, &registry(), &PoolLimits::default(), chrono::Utc::now())
    }

    #[test]
    fn finds_triangle_and_cross_pool_cycle() {
        let mut set = PoolSet::new(1);
        add_pool(&mut set, "p_ab", "A", "B");
        add_pool(&mut set, "p_ab2", "A", "B");
        add_pool(&mut set, "p_bc", "B", "C");
        add_pool(&mut set, "p_ca", "C", "A");
        let g = TokenGraph::build(&set, &registry(), &PoolLimits::default(), chrono::Utc::now());

        let out = search(&g, &["A".to_string()], &SearchLimits::default());
        // 2 跳跨池 2 条（p_ab→p_ab2 / p_ab2→p_ab），
        // 三角 4 条（A→B→C→A 的两种 A→B 边 + 反向 A→C→B→A）。
        assert_eq!(out.routes.len(), 6, "{:?}", out.routes);
        assert!(out.routes.iter().any(|r| r.route_type == RouteType::CrossPool));
        assert!(out.routes.iter().any(|r| r.route_type == RouteType::Triangle));
        for r in &out.routes {
            assert_eq!(r.edges.first().unwrap().token_in, "A");
            assert_eq!(r.edges.last().unwrap().token_out, "A");
            assert_eq!(r.hops(), r.edges.len());
            // 无重复池
            let mut ids: Vec<&str> = r.edges.iter().map(|e| e.pool_id.as_str()).collect();
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n);
        }
        assert!(!out.stats.truncated);
        assert!(out.stats.visits > 0);
        assert_eq!(out.stats.candidates, out.routes.len());
    }

    #[test]
    fn hop_limits_are_respected() {
        let g = graph();
        let two = search(
            &g,
            &["A".to_string()],
            &SearchLimits {
                min_hops: 2,
                max_hops: 2,
                ..SearchLimits::default()
            },
        );
        assert!(two.routes.is_empty(), "该图没有 2 跳闭环");

        let three = search(
            &g,
            &["A".to_string()],
            &SearchLimits {
                min_hops: 3,
                max_hops: 3,
                ..SearchLimits::default()
            },
        );
        assert!(three.routes.iter().all(|r| r.hops() == 3));
        assert!(!three.routes.is_empty());

        let four = search(
            &g,
            &["A".to_string()],
            &SearchLimits {
                min_hops: 3,
                max_hops: 4,
                ..SearchLimits::default()
            },
        );
        assert!(four.routes.iter().any(|r| r.hops() == 4));
    }

    #[test]
    fn candidate_budget_truncates() {
        let mut set = PoolSet::new(1);
        for i in 0..4 {
            add_pool(&mut set, &format!("ab{i}"), "A", "B");
            add_pool(&mut set, &format!("bc{i}"), "B", "C");
            add_pool(&mut set, &format!("ca{i}"), "C", "A");
        }
        let g = TokenGraph::build(&set, &registry(), &PoolLimits::default(), chrono::Utc::now());
        let out = search(
            &g,
            &["A".to_string()],
            &SearchLimits {
                max_candidates: 5,
                ..SearchLimits::default()
            },
        );
        assert_eq!(out.routes.len(), 5);
        assert!(out.stats.truncated);
    }

    #[test]
    fn visit_budget_truncates() {
        let g = graph();
        let out = search(
            &g,
            &["A".to_string()],
            &SearchLimits {
                max_visits: 1,
                ..SearchLimits::default()
            },
        );
        assert!(out.stats.truncated);
        assert!(out.stats.visits >= 1);
    }

    #[test]
    fn search_is_deterministic() {
        let g = graph();
        let limits = SearchLimits::default();
        let a = search(&g, &["A".to_string()], &limits);
        let b = search(&g, &["A".to_string()], &limits);
        let ids = |o: &SearchOutcome| {
            o.routes
                .iter()
                .map(|r| r.fingerprints.directed.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn unknown_start_token_yields_nothing() {
        let g = graph();
        let out = search(&g, &["ZZZ".to_string()], &SearchLimits::default());
        assert!(out.routes.is_empty());
        assert_eq!(out.stats.visits, 0);
    }

    #[test]
    fn description_is_stable() {
        let mut set = PoolSet::new(1);
        add_pool(&mut set, "p_ab", "A", "B");
        let g = TokenGraph::build(&set, &registry(), &PoolLimits::default(), chrono::Utc::now());
        assert_eq!(describe(g.edges_from("A")), "p_ab:A/B");
    }
}