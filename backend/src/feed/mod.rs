//! 行情来源。
//!
//! 运行时只接受真实链（[live]）；[demo] 仅用于单元测试与离线回放，不会出现在
//! 默认配置里，也不会被服务自动启用。
//!
//! [`MarketData`] 是扫描器与 API 共同读取的不可变快照：一次同步生成一份快照，
//! 扫描器只读快照，不存在“扫描过程中池状态被改写”的问题。

pub mod demo;
pub mod live;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::cex::BookEngine;
use crate::domain::market::{Chain, DexProtocol, Instrument, PoolRef, Token};
use crate::domain::types::PoolKind;
use crate::dex::PoolSet;

/// 数据源健康状态。前端必须展示它，不能在后端失联时继续展示旧机会。
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct FeedHealth {
    pub ok: bool,
    pub mode: String,
    pub source: String,
    pub rpc_primary: String,
    pub rpc_fallback: Option<String>,
    pub last_error: Option<String>,
    pub last_sync: Option<DateTime<Utc>>,
    pub latency_ms: u64,
    pub block_age_ms: i64,
    pub pools_total: usize,
    pub pools_refreshed: usize,
    pub ticks_pools: usize,
    /// 标价主源：onchain / chainlink / cex 的组合说明。
    pub price_source: String,
    /// 各 Token 标价与三源中位数的最大偏差（bps）。
    pub price_divergence_bps: f64,
    /// 分歧超阈值的 Token 摘要（为空表示一致）。
    pub price_notes: Option<String>,
}

/// 一次同步得到的市场快照。
#[derive(Debug)]
pub struct MarketData {
    pub chain: Chain,
    pub head_block: u64,
    pub block_timestamp: u64,
    pub tokens: HashMap<String, Token>,
    pub pools: PoolSet,
    pub instruments: HashMap<String, Instrument>,
    pub books: BookEngine,
    pub prices: HashMap<String, Decimal>,
    /// 每个 Token 的三源标价分歧（bps），用于机会降级。
    pub price_divergence: HashMap<String, Decimal>,
    pub mode: String,
    pub source: String,
    pub updated_at: DateTime<Utc>,
    pub tick_count: u64,
    pub gas_price_gwei: Decimal,
    pub health: FeedHealth,
}

impl MarketData {
    pub fn empty(chain: Chain) -> Self {
        Self {
            chain,
            head_block: 0,
            block_timestamp: 0,
            tokens: HashMap::new(),
            pools: PoolSet::new(0),
            instruments: HashMap::new(),
            books: BookEngine::new(),
            prices: HashMap::new(),
            price_divergence: HashMap::new(),
            mode: "live".into(),
            source: String::new(),
            updated_at: Utc::now(),
            tick_count: 0,
            gas_price_gwei: Decimal::ZERO,
            health: FeedHealth {
                ok: false,
                mode: "live".into(),
                source: String::new(),
                rpc_primary: String::new(),
                rpc_fallback: None,
                last_error: Some("尚未完成首次链上同步".into()),
                last_sync: None,
                latency_ms: 0,
                block_age_ms: i64::MAX,
                pools_total: 0,
                pools_refreshed: 0,
                ticks_pools: 0,
                price_source: "none".into(),
                price_divergence_bps: 0.0,
                price_notes: None,
            },
        }
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }

    pub fn price(&self, asset: &str) -> Decimal {
        self.prices.get(asset).copied().unwrap_or(Decimal::ZERO)
    }

    /// 协议注册表（由已发现的池反推，避免展示未接入的协议）。
    pub fn protocols(&self) -> Vec<DexProtocol> {
        let mut seen: HashMap<String, DexProtocol> = HashMap::new();
        for rt in self.pools.all() {
            let p = &rt.pool;
            seen.entry(p.protocol_id.clone()).or_insert_with(|| DexProtocol {
                protocol_id: p.protocol_id.clone(),
                protocol_version: "onchain".into(),
                kind: p.kind,
                chain_id: p.chain_id,
                factory: String::new(),
                router: String::new(),
                adapter: format!("adapter-{}", p.protocol_id),
                enabled: true,
                verified: p.verified,
                audit_note: "真实链只读适配（RPC eth_call）".into(),
            });
        }
        let mut out: Vec<_> = seen.into_values().collect();
        out.sort_by(|a, b| a.protocol_id.cmp(&b.protocol_id));
        out
    }

    /// 池按协议分组统计（API 展示）。
    pub fn protocol_kinds(&self) -> HashMap<String, PoolKind> {
        let mut m = HashMap::new();
        for rt in self.pools.all() {
            m.insert(rt.pool.protocol_id.clone(), rt.pool.kind);
        }
        m
    }

    pub fn pool_refs(&self) -> Vec<&PoolRef> {
        self.pools.all().iter().map(|p| &p.pool).collect()
    }
}
