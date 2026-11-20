//! Token–Pool 有向多重图（PRD §4.1、§4.2）。
//!
//! 节点是 Token，边是「某个具体 Pool 提供的单方向兑换能力」。一个交易对有几个池，
//! 两个 Token 之间就有几条边——禁止把不同池合并成一条无来源的抽象报价边。
//!
//! 建图是扫描的第一层过滤：只有通过 Token/Pool 准入、状态新鲜且流动性可用的池
//! 才会贡献边；被排除的池不会进入搜索空间。

use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};

use crate::domain::dex_scan::GraphStats;
use crate::domain::market::PoolKind;
use crate::domain::plan::fnv1a_64;
use crate::dex::graph::HopEdge;

use super::pool::PoolSet;
use super::registry::{assess_pool, PoolLimits, TokenRegistry};

/// 图中的一条有向边，绑定唯一 Pool。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub pool_id: String,
    pub protocol_id: String,
    pub token_in: String,
    pub token_out: String,
    pub kind: PoolKind,
    pub fee_bps: i64,
    pub tvl_usd: rust_decimal::Decimal,
}

impl GraphEdge {
    /// 转为报价使用的跳（保持数据结构向后兼容）。
    pub fn to_hop(&self) -> HopEdge {
        HopEdge {
            pool_id: self.pool_id.clone(),
            token_in: self.token_in.clone(),
            token_out: self.token_out.clone(),
            kind: self.kind,
        }
    }

    fn key(&self) -> String {
        format!(
            "{}:{}>{}:{}:{}",
            self.pool_id, self.token_in, self.token_out, self.kind, self.fee_bps
        )
    }
}

/// 同链交易图。构建后不可变，可安全共享给搜索与增量缓存。
#[derive(Clone, Debug)]
pub struct TokenGraph {
    chain_id: u64,
    edges: Vec<GraphEdge>,
    by_token: HashMap<String, Vec<GraphEdge>>,
    stats: GraphStats,
}

impl TokenGraph {
    /// 由同链池集合 + Token 准入注册表构建。
    pub fn build(
        pools: &PoolSet,
        registry: &TokenRegistry,
        limits: &PoolLimits,
        now: DateTime<Utc>,
    ) -> Self {
        let mut runtimes: Vec<_> = pools.all().to_vec();
        runtimes.sort_by(|a, b| a.pool.pool_id.cmp(&b.pool.pool_id));

        let mut edges = Vec::new();
        let mut by_token: HashMap<String, Vec<GraphEdge>> = HashMap::new();
        let node_key = |token: &str| -> String { format!("{}:{}", pools.chain_id(), token) };
        let mut rejected_pools = 0usize;
        let mut eligible_pools = 0usize;
        let mut tokens: BTreeSet<String> = BTreeSet::new();

        for rt in &runtimes {
            let admission = assess_pool(rt, registry, limits, now);
            if !admission.eligible {
                rejected_pools += 1;
                continue;
            }
            eligible_pools += 1;
            for (token_in, token_out) in [
                (rt.pool.token0.clone(), rt.pool.token1.clone()),
                (rt.pool.token1.clone(), rt.pool.token0.clone()),
            ] {
                let edge = GraphEdge {
                    pool_id: rt.pool.pool_id.clone(),
                    protocol_id: rt.pool.protocol_id.clone(),
                    token_in: token_in.clone(),
                    token_out: token_out.clone(),
                    kind: rt.pool.kind,
                    fee_bps: rt.fee_bps(),
                    tvl_usd: rt.pool.tvl_usd,
                };
                tokens.insert(token_in.clone());
                tokens.insert(token_out.clone());
                by_token.entry(node_key(&token_in)).or_default().push(edge.clone());
                edges.push(edge);
            }
        }

        // 每条 token 的出边按 (pool_id, token_out) 排序，保证搜索确定性。
        for list in by_token.values_mut() {
            list.sort_by(|a, b| {
                a.pool_id
                    .cmp(&b.pool_id)
                    .then_with(|| a.token_out.cmp(&b.token_out))
            });
        }

        let approved = registry.counts().0;
        let mut signature_text = String::new();
        for e in &edges {
            signature_text.push_str(&e.key());
            signature_text.push(';');
        }
        let signature = format!("{:016x}", fnv1a_64(&signature_text));

        let stats = GraphStats {
            chain_id: pools.chain_id(),
            tokens: tokens.len(),
            approved_tokens: approved,
            pools: runtimes.len(),
            eligible_pools,
            edges: edges.len(),
            rejected_pools,
            built_at: Some(now),
            signature: signature.clone(),
        };

        Self {
            chain_id: pools.chain_id(),
            edges,
            by_token,
            stats,
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// 从某 Token 出发的所有边（不可变，按确定性顺序）。
    pub fn edges_from(&self, token: &str) -> &[GraphEdge] {
        let key = format!("{}:{}", self.chain_id, token);
        self.by_token
            .get(&key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn has_token(&self, token: &str) -> bool {
        self.by_token.contains_key(&format!("{}:{}", self.chain_id, token))
    }

    /// 某池贡献的全部边（用于反查与增量索引）。
    pub fn edges_of_pool<'a>(&'a self, pool_id: &'a str) -> impl Iterator<Item = &'a GraphEdge> {
        self.edges.iter().filter(move |e| e.pool_id == pool_id)
    }

    pub fn stats(&self) -> &GraphStats {
        &self.stats
    }

    /// 图的稳定签名：池集合、方向、费率或 AMM 类型变化都会改变它。
    pub fn signature(&self) -> &str {
        &self.stats.signature
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::{PoolRef, PoolState, Token, TokenStatus};
    use crate::domain::money::dec;
    use crate::dex::math;

    fn token(symbol: &str, status: TokenStatus) -> Token {
        Token {
            symbol: symbol.into(),
            chain_id: 1,
            address: format!("0x{symbol}"),
            decimals: 6,
            standard: "ERC20".into(),
            code_hash: "0x1".into(),
            status,
            risk_tags: vec![],
            allowed_start: true,
            allowed_hop: true,
            allowed_flash: true,
            special_transfer: false,
            enabled: true,
        }
    }

    fn registry() -> TokenRegistry {
        let mut tokens = HashMap::new();
        for s in ["USDC", "WETH", "WBTC"] {
            tokens.insert(s.to_string(), token(s, TokenStatus::Approved));
        }
        TokenRegistry::build(&tokens)
    }

    fn pool(set: &mut PoolSet, id: &str, t0: &str, t1: &str, fee: i64, tvl: &str) {
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
                fee_bps: fee,
                tick_spacing: None,
                tvl_usd: dec(tvl),
                verified: true,
                execution_allowed: true,
                added_at: now,
            },
            PoolState {
                pool_id: id.into(),
                block_number: 10,
                block_hash: "0xb".into(),
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
                fee_bps: fee,
            },
        );
    }

    #[test]
    fn graph_creates_two_directed_edges_per_pool() {
        let mut set = PoolSet::new(1);
        pool(&mut set, "p1", "USDC", "WETH", 30, "1000000");
        pool(&mut set, "p2", "WETH", "WBTC", 30, "1000000");
        let g = TokenGraph::build(&set, &registry(), &PoolLimits::default(), Utc::now());
        assert_eq!(g.stats().pools, 2);
        assert_eq!(g.stats().eligible_pools, 2);
        assert_eq!(g.edge_count(), 4);
        assert_eq!(g.edges_from("USDC").len(), 1);
        assert_eq!(g.edges_from("WETH").len(), 2);
        // 不可合并：两个不同池的 WETH 出边保持独立
        assert_eq!(g.edges_of_pool("p1").count(), 2);
        assert!(g.has_token("WBTC"));
        assert!(!g.has_token("DAI"));
        assert_eq!(g.edges().len(), 4);
    }

    #[test]
    fn ineligible_pools_do_not_contribute_edges() {
        let mut set = PoolSet::new(1);
        pool(&mut set, "ok", "USDC", "WETH", 30, "1000000");
        pool(&mut set, "poor", "USDC", "WBTC", 30, "1000");
        let g = TokenGraph::build(&set, &registry(), &PoolLimits::default(), Utc::now());
        assert_eq!(g.stats().eligible_pools, 1);
        assert_eq!(g.stats().rejected_pools, 1);
        assert_eq!(g.edge_count(), 2);
        assert!(!g.has_token("WBTC"));
    }

    #[test]
    fn blocked_token_removes_its_pools() {
        let mut tokens = HashMap::new();
        tokens.insert("USDC".into(), token("USDC", TokenStatus::Approved));
        tokens.insert("WETH".into(), token("WETH", TokenStatus::Approved));
        tokens.insert("SCAM".into(), token("SCAM", TokenStatus::Unknown));
        let reg = TokenRegistry::build(&tokens);
        let mut set = PoolSet::new(1);
        pool(&mut set, "p1", "USDC", "WETH", 30, "1000000");
        pool(&mut set, "p2", "USDC", "SCAM", 30, "1000000");
        let g = TokenGraph::build(&set, &reg, &PoolLimits::default(), Utc::now());
        assert_eq!(g.stats().eligible_pools, 1);
        assert!(!g.has_token("SCAM"));
    }

    #[test]
    fn signature_changes_with_fee() {
        let mut set = PoolSet::new(1);
        pool(&mut set, "p1", "USDC", "WETH", 30, "1000000");
        let a = TokenGraph::build(&set, &registry(), &PoolLimits::default(), Utc::now());
        pool(&mut set, "p1", "USDC", "WETH", 5, "1000000");
        let b = TokenGraph::build(&set, &registry(), &PoolLimits::default(), Utc::now());
        assert_ne!(a.signature(), b.signature());
        assert_eq!(a.signature().len(), 16);
    }
}