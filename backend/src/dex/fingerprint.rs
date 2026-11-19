//! 路线指纹（PRD §4.6）。
//!
//! 需要同时回答三个问题：
//! 1. 两条路线是不是同一个执行方向？（有向指纹）
//! 2. 它们是不是同一组池的镜像写法？（无方向指纹）
//! 3. 它们的池序列 / Token 序列 / 起始资产是否相同？
//!
//! 指纹只做规范化与哈希，不参与定价；哈希使用 FNV-1a 64 位，稳定且不依赖构建顺序。

use crate::dex::graph::HopEdge;
use crate::domain::dex_scan::RouteFingerprints;
use crate::domain::plan::fnv1a_64;

/// 计算一条闭环路线的全部指纹。
///
/// 要求 edges 首尾闭环（调用方保证）；本函数不校验，只做规范化。
pub fn fingerprints(start: &str, edges: &[HopEdge]) -> RouteFingerprints {
    RouteFingerprints {
        directed: hash(&directed_canonical(edges)),
        undirected: hash(&undirected_canonical(edges)),
        pool_sequence: hash(&pool_sequence(edges)),
        token_sequence: hash(&token_sequence(start, edges)),
        start_asset: hash(start),
    }
}

/// 有向指纹文本：池 id + 方向，顺序必须保留（从起点出发）。
pub fn directed_canonical(edges: &[HopEdge]) -> String {
    edges
        .iter()
        .map(|e| format!("{}:{}>{}", e.pool_id, e.token_in, e.token_out))
        .collect::<Vec<_>>()
        .join(";")
}

/// 无方向指纹文本：忽略旋转与整体方向，逐跳 Token 顺序也不参与。
fn undirected_canonical(edges: &[HopEdge]) -> String {
    if edges.is_empty() {
        return String::new();
    }
    let hops: Vec<String> = edges
        .iter()
        .map(|e| {
            let (a, b) = if e.token_in <= e.token_out {
                (e.token_in.as_str(), e.token_out.as_str())
            } else {
                (e.token_out.as_str(), e.token_in.as_str())
            };
            format!("{}:{a}={b}", e.pool_id)
        })
        .collect();

    let n = hops.len();
    let mut best: Option<String> = None;
    for rev in [false, true] {
        for rot in 0..n {
            let mut parts = Vec::with_capacity(n);
            for i in 0..n {
                let idx = if rev { (rot + n - i) % n } else { (rot + i) % n };
                parts.push(hops[idx].clone());
            }
            let s = parts.join("|");
            if best.as_ref().map(|b| s < *b).unwrap_or(true) {
                best = Some(s);
            }
        }
    }
    best.unwrap_or_default()
}

/// 池序列指纹文本：池 id 的排序集合（同池不同方向仍是同一组池）。
fn pool_sequence(edges: &[HopEdge]) -> String {
    let mut ids: Vec<&str> = edges.iter().map(|e| e.pool_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.join("|")
}

/// Token 序列指纹文本：忽略旋转与方向。
fn token_sequence(start: &str, edges: &[HopEdge]) -> String {
    if edges.is_empty() {
        return start.to_string();
    }
    let mut toks: Vec<String> = vec![start.to_string()];
    for e in edges {
        if e.token_out != start {
            toks.push(e.token_out.clone());
        }
    }
    let n = toks.len();
    let mut best: Option<String> = None;
    for rev in [false, true] {
        for rot in 0..n {
            let mut parts = Vec::with_capacity(n);
            for i in 0..n {
                let idx = if rev { (rot + n - i) % n } else { (rot + i) % n };
                parts.push(toks[idx].clone());
            }
            let s = parts.join("|");
            if best.as_ref().map(|b| s < *b).unwrap_or(true) {
                best = Some(s);
            }
        }
    }
    best.unwrap_or_default()
}

fn hash(s: &str) -> String {
    format!("{:016x}", fnv1a_64(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::PoolKind;

    fn e(pool: &str, tin: &str, tout: &str) -> HopEdge {
        HopEdge {
            pool_id: pool.into(),
            token_in: tin.into(),
            token_out: tout.into(),
            kind: PoolKind::UniswapV2,
        }
    }

    fn cycle() -> Vec<HopEdge> {
        vec![
            e("p1", "USDC", "WETH"),
            e("p2", "WETH", "WBTC"),
            e("p3", "WBTC", "USDC"),
        ]
    }

    #[test]
    fn direction_changes_directed_but_not_undirected() {
        let a = cycle();
        // 镜像方向：顺序反向、每跳 token 互换
        let b: Vec<HopEdge> = a
            .iter()
            .rev()
            .map(|x| e(&x.pool_id, &x.token_out, &x.token_in))
            .collect();
        let fa = fingerprints("USDC", &a);
        let fb = fingerprints("USDC", &b);
        assert_ne!(fa.directed, fb.directed, "两个方向必须可区分");
        assert_eq!(fa.undirected, fb.undirected, "镜像路线应共享无方向指纹");
        assert_eq!(fa.pool_sequence, fb.pool_sequence);
        assert_eq!(fa.token_sequence, fb.token_sequence);
        assert_eq!(fa.start_asset, fb.start_asset);
    }

    #[test]
    fn identical_routes_have_identical_fingerprints() {
        let f1 = fingerprints("USDC", &cycle());
        let f2 = fingerprints("USDC", &cycle());
        assert_eq!(f1, f2);
        assert_ne!(f1.directed, "".to_string());
        assert_eq!(f1.start_asset, hash("USDC"));
    }

    #[test]
    fn different_pool_sets_differ() {
        let mut other = cycle();
        other[1] = e("p9", "WETH", "WBTC");
        let fa = fingerprints("USDC", &cycle());
        let fb = fingerprints("USDC", &other);
        assert_ne!(fa.directed, fb.directed);
        assert_ne!(fa.undirected, fb.undirected);
        assert_ne!(fa.pool_sequence, fb.pool_sequence);
        assert_eq!(fa.token_sequence, fb.token_sequence);
    }

    #[test]
    fn empty_and_two_hop_routes_are_supported() {
        let f = fingerprints("USDC", &[]);
        assert_eq!(f.token_sequence, hash("USDC"));
        let two: Vec<HopEdge> = vec![
            e("p1", "USDC", "WETH"),
            e("p2", "WETH", "USDC"),
        ];
        let f2 = fingerprints("USDC", &two);
        assert_ne!(f2.directed, f2.undirected);
        assert_eq!(f2.pool_sequence, hash("p1|p2"));
    }
}
