//! 路由图搜索：从起始 token 出发枚举简单环（三角 / 四跳）与跨池闭环。
//!
//! 图的定义：每个 [`PoolRuntime`] 贡献两条**有向边**（`token0 -> token1` 与
//! `token1 -> token0`），边权由 [`crate::dex::math`] 的报价函数决定。枚举结果只保证
//! “结构上是一个简单环”，是否盈利由上层策略引擎用 [`crate::dex::pool::quote_path`]
//! 逐腿报价后判定。
//!
//! 约束：
//! - 同一环内不得重复使用同一个 `pool_id`（也即不允许自环 / 踩回同池）。
//! - 环内回到起始 token 即结束，不允许把起始 token 当作中间跳。
//! - 结果按“池序列 + 方向”去重，并按 `(跳数, 池序列)` 排序以保证确定性。
//! - 反向环（同一组池、相反方向）是**不同**的套利方向，分别保留。

use std::collections::HashSet;

use crate::domain::market::PoolKind;

use super::pool::PoolSet;

/// 单条有向跳：在 `pool_id` 上把 `token_in` 换成 `token_out`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HopEdge {
    pub pool_id: String,
    pub token_in: String,
    pub token_out: String,
    pub kind: PoolKind,
}

/// 防御性上限：异常状态下避免指数爆炸（正常路由搜索远达不到）。
const MAX_CYCLES: usize = 4096;
/// 跳数上限（风控 §4.1：单路线最大交易跳数 4 跳，超过需额外验证）。
const MAX_HOPS_CAP: usize = 8;

/// 从 `start` 出发、长度在 `3..=hops` 之间、回到 `start` 的简单环。
///
/// `hops` 是**最大**跳数（典型取值 3 或 4）。`hops < 3` 时返回空集。
/// 同一环内池不重复；返回结果已去重（同一池序列只出现一次）并排序。
pub fn enumerate_triangles(set: &PoolSet, start_token: &str, max_hops: usize) -> Vec<Vec<HopEdge>> {
    if start_token.is_empty() || max_hops < 3 {
        return Vec::new();
    }
    let cap = max_hops.min(MAX_HOPS_CAP);
    let mut out: Vec<Vec<HopEdge>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut path: Vec<HopEdge> = Vec::new();
    let mut used_pools: Vec<String> = Vec::new();

    walk(
        set,
        start_token,
        start_token,
        0,
        cap,
        &mut used_pools,
        &mut path,
        &mut out,
        &mut seen,
    );

    sort_cycles(&mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn walk(
    set: &PoolSet,
    start: &str,
    current: &str,
    depth: usize,
    cap: usize,
    used_pools: &mut Vec<String>,
    path: &mut Vec<HopEdge>,
    out: &mut Vec<Vec<HopEdge>>,
    seen: &mut HashSet<String>,
) {
    if out.len() >= MAX_CYCLES || depth >= cap {
        return;
    }
    for p in set.pools_with_token(current) {
        // 跳过 token0 == token1 的退化池（自环）
        if p.pool.token0 == p.pool.token1 {
            continue;
        }
        let Some(next) = p.token_out(current) else {
            continue;
        };
        if next == current {
            continue;
        }
        let pid = &p.pool.pool_id;
        if used_pools.iter().any(|x| x == pid) {
            continue;
        }

        path.push(HopEdge {
            pool_id: pid.clone(),
            token_in: current.to_string(),
            token_out: next.to_string(),
            kind: p.pool.kind,
        });
        used_pools.push(pid.clone());

        if next == start {
            // 只有 >= 3 跳的环才是候选套利路线（2 跳只是同一对 token 的往返）
            if depth + 1 >= 3 {
                let key = cycle_key(path);
                if seen.insert(key) {
                    out.push(path.clone());
                }
            }
        } else if depth + 1 < cap {
            walk(
                set,
                start,
                next,
                depth + 1,
                cap,
                used_pools,
                path,
                out,
                seen,
            );
        }

        used_pools.pop();
        path.pop();
    }
}

/// 两个**不同**池构成 `start -> X -> start` 的闭环（跨池套利）。
///
/// 只返回恰好 2 跳、且两个池 id 不同的结果；同一对池的正反两个方向分别保留。
pub fn enumerate_cross_pool_cycles(set: &PoolSet, start_token: &str) -> Vec<Vec<HopEdge>> {
    let mut out: Vec<Vec<HopEdge>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if start_token.is_empty() {
        return out;
    }

    for p1 in set.pools_with_token(start_token) {
        if p1.pool.token0 == p1.pool.token1 {
            continue;
        }
        let Some(mid) = p1.token_out(start_token) else {
            continue;
        };
        if mid == start_token {
            continue;
        }
        for p2 in set.pools_with_token(mid) {
            if p2.pool.pool_id == p1.pool.pool_id {
                continue; // 必须跨池
            }
            if p2.pool.token0 == p2.pool.token1 {
                continue;
            }
            if p2.token_out(mid) != Some(start_token) {
                continue; // p2 必须能把 mid 换回 start
            }
            let cycle = vec![
                HopEdge {
                    pool_id: p1.pool.pool_id.clone(),
                    token_in: start_token.to_string(),
                    token_out: mid.to_string(),
                    kind: p1.pool.kind,
                },
                HopEdge {
                    pool_id: p2.pool.pool_id.clone(),
                    token_in: mid.to_string(),
                    token_out: start_token.to_string(),
                    kind: p2.pool.kind,
                },
            ];
            let key = cycle_key(&cycle);
            if seen.insert(key) {
                out.push(cycle);
                if out.len() >= MAX_CYCLES {
                    sort_cycles(&mut out);
                    return out;
                }
            }
        }
    }

    sort_cycles(&mut out);
    out
}

/// 环的去重键：池 id + 方向。同一池序列且方向相同即视为同一环。
fn cycle_key(cycle: &[HopEdge]) -> String {
    let mut s = String::new();
    for e in cycle {
        s.push_str(&e.pool_id);
        s.push('|');
        s.push_str(&e.token_in);
        s.push('>');
        s.push_str(&e.token_out);
        s.push(';');
    }
    s
}

/// 排序：先按跳数，再按池序列字典序，保证输出确定性。
fn sort_cycles(cycles: &mut [Vec<HopEdge>]) {
    cycles.sort_by(|a, b| {
        a.len().cmp(&b.len()).then_with(|| {
            let ka: Vec<&str> = a.iter().map(|e| e.pool_id.as_str()).collect();
            let kb: Vec<&str> = b.iter().map(|e| e.pool_id.as_str()).collect();
            ka.cmp(&kb)
        })
    });
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::{PoolRef, PoolState};
    use crate::domain::money::dec;
    use chrono::Utc;
    use rust_decimal::Decimal;

    fn insert(set: &mut PoolSet, id: &str, t0: &str, t1: &str) {
        let pool = PoolRef {
            pool_id: id.to_string(),
            chain_id: 1,
            protocol_id: "test".to_string(),
            kind: PoolKind::UniswapV2,
            address: format!("0x{id}"),
            token0: t0.to_string(),
            token1: t1.to_string(),
            decimals0: 0,
            decimals1: 0,
            fee_bps: 30,
            tick_spacing: None,
            tvl_usd: dec("1000000"),
            verified: true,
            execution_allowed: true,
            added_at: Utc::now(),
        };
        let state = PoolState {
            pool_id: id.to_string(),
            block_number: 1,
            block_hash: String::new(),
            updated_at: Utc::now(),
            reserve0: dec("1000000"),
            reserve1: dec("1000000"),
            sqrt_price_x96: 0,
            liquidity: 0,
            tick: 0,
            ticks: Vec::new(),
            amplification: dec("100"),
            weight0: Decimal::ZERO,
            weight1: Decimal::ZERO,
            fee_bps: 30,
        };
        set.upsert(pool, state);
    }

    /// 断言环结构合法：首尾相接、token 连续、池不重复、无自环。
    fn assert_valid_cycle(cycle: &[HopEdge], start: &str) {
        assert!(cycle.len() >= 2, "环至少 2 跳: {cycle:?}");
        assert_eq!(cycle[0].token_in, start, "环必须从起始 token 出发");
        assert_eq!(
            cycle.last().unwrap().token_out,
            start,
            "环必须回到起始 token"
        );
        for w in cycle.windows(2) {
            assert_eq!(
                w[0].token_out, w[1].token_in,
                "相邻跳 token 必须连续: {cycle:?}"
            );
            assert_ne!(
                w[0].pool_id, w[1].pool_id,
                "相邻跳不得复用同一个池: {cycle:?}"
            );
        }
        for e in cycle {
            assert_ne!(e.token_in, e.token_out, "不允许自环边: {cycle:?}");
        }
        let mut ids: Vec<&str> = cycle.iter().map(|e| e.pool_id.as_str()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "同一环内不得重复使用池: {cycle:?}");
    }

    #[test]
    fn triangle_enumeration_is_correct_and_deduped() {
        let mut set = PoolSet::new(1);
        insert(&mut set, "p_ab", "A", "B");
        insert(&mut set, "p_bc", "B", "C");
        insert(&mut set, "p_ca", "C", "A");
        // 与三角无关的池
        insert(&mut set, "p_de", "D", "E");

        let cycles = enumerate_triangles(&set, "A", 3);
        // 正反两个方向各是一个独立套利环（池序列不同），共 2 个
        assert_eq!(cycles.len(), 2, "三角形应返回正反两个方向: {cycles:?}");
        for c in &cycles {
            assert_eq!(c.len(), 3);
            assert_valid_cycle(c, "A");
        }
        // 去重：池序列不得重复
        let mut keys: Vec<Vec<&str>> = cycles
            .iter()
            .map(|c| c.iter().map(|e| e.pool_id.as_str()).collect())
            .collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "同一池序列只能出现一次");
        // 所有环的池集合相同（就是三个池）
        for c in &cycles {
            let mut ids: Vec<&str> = c.iter().map(|e| e.pool_id.as_str()).collect();
            ids.sort_unstable();
            assert_eq!(ids, vec!["p_ab", "p_bc", "p_ca"]);
        }
        // 无自环、无重复池已由 assert_valid_cycle 覆盖
        assert_eq!(
            enumerate_triangles(&set, "A", 2).len(),
            0,
            "hops<3 必须为空"
        );
        assert_eq!(
            enumerate_triangles(&set, "Z", 3).len(),
            0,
            "孤立 token 无环"
        );
    }

    #[test]
    fn four_hop_cycle_enumeration() {
        let mut set = PoolSet::new(1);
        insert(&mut set, "p_ab", "A", "B");
        insert(&mut set, "p_bc", "B", "C");
        insert(&mut set, "p_cd", "C", "D");
        insert(&mut set, "p_da", "D", "A");

        // 只有 4 跳环，max_hops=3 时不应返回
        assert_eq!(enumerate_triangles(&set, "A", 3).len(), 0);
        let cycles = enumerate_triangles(&set, "A", 4);
        assert_eq!(cycles.len(), 2, "四跳环同样有正反两个方向: {cycles:?}");
        for c in &cycles {
            assert_eq!(c.len(), 4);
            assert_valid_cycle(c, "A");
        }
    }

    #[test]
    fn triangle_with_parallel_pools_yields_distinct_cycles() {
        let mut set = PoolSet::new(1);
        insert(&mut set, "p1_ab", "A", "B");
        insert(&mut set, "p2_ab", "A", "B"); // 同一对 token 的第二个池
        insert(&mut set, "p_bc", "B", "C");
        insert(&mut set, "p_ca", "C", "A");

        let cycles = enumerate_triangles(&set, "A", 3);
        // A→B 有两条可选边，正反两方向 → 4 个不同池序列
        assert_eq!(cycles.len(), 4, "并行池应产生不同池序列: {cycles:?}");
        for c in &cycles {
            assert_eq!(c.len(), 3);
            assert_valid_cycle(c, "A");
        }
    }

    #[test]
    fn cross_pool_cycles_only_two_distinct_pools() {
        let mut set = PoolSet::new(1);
        insert(&mut set, "p1", "A", "B");
        insert(&mut set, "p2", "A", "B");
        // 无关池
        insert(&mut set, "p3", "C", "D");

        let cycles = enumerate_cross_pool_cycles(&set, "A");
        assert_eq!(cycles.len(), 2, "两个池 × 两个方向: {cycles:?}");
        for c in &cycles {
            assert_eq!(c.len(), 2);
            assert_valid_cycle(c, "A");
            assert_ne!(c[0].pool_id, c[1].pool_id, "必须使用两个不同的池");
            let mut ids = vec![c[0].pool_id.as_str(), c[1].pool_id.as_str()];
            ids.sort_unstable();
            assert_eq!(ids, vec!["p1", "p2"]);
        }
        // 池序列去重
        let mut keys: Vec<Vec<&str>> = cycles
            .iter()
            .map(|c| c.iter().map(|e| e.pool_id.as_str()).collect())
            .collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "同一池序列只能出现一次");

        // 孤立 token 无跨池闭环
        assert_eq!(enumerate_cross_pool_cycles(&set, "Z").len(), 0);
        // 只有单个池时无法跨池
        let mut single = PoolSet::new(1);
        insert(&mut single, "p1", "A", "B");
        assert_eq!(enumerate_cross_pool_cycles(&single, "A").len(), 0);
    }

    #[test]
    fn graph_handles_three_pools_same_pair() {
        let mut set = PoolSet::new(1);
        insert(&mut set, "p1", "A", "B");
        insert(&mut set, "p2", "A", "B");
        insert(&mut set, "p3", "A", "B");
        // 有序池对：3*2 = 6 个闭环
        let cycles = enumerate_cross_pool_cycles(&set, "A");
        assert_eq!(cycles.len(), 6, "3 个池的有序两两组合: {cycles:?}");
        for c in &cycles {
            assert_valid_cycle(c, "A");
        }
    }
}
