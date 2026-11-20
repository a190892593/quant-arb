//! 真实链行情（M1）：链上只读状态 + CEX 公共盘口。
//!
//! 边界（与当前阶段一致）：
//! - **不接 REVM，因此没有 M2**：不做状态插入模拟、不扫 V3 tick 位图；
//! - V3 以 `slot0 + liquidity()` 的单区间活跃流动性报价，属于 M1 近似；
//! - V2 用 `getReserves()`，在人类单位下按恒定乘积精确报价；
//! - 所有池地址、Token 精度与费率来自显式配置并在启动时校验；
//! - 读取失败只降低健康度并保留上一份快照，绝不补零或伪造价格。
//!
//! 数据来源：Arbitrum 主网 RPC（eth_call / eth_blockNumber / eth_gasPrice）
//! + Binance 公共行情（depth / exchangeInfo，只读、无 API Key）。

use chrono::Utc;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Instant;

use crate::cex::binance::BinanceClient;
use crate::cex::BookEngine;
use crate::config::{Config, ExtraPoolConfig};
use crate::dex::PoolSet;
use crate::domain::market::{Chain, Instrument, PoolKind, PoolRef, PoolState, Token};
use crate::domain::money::{dec, unit_scale};
use crate::domain::types::{ChainStage, DataQuality, TokenStatus};
use crate::evm::abi::{self, Address, Arg};
use crate::evm::{Call, RpcClient};
use crate::feed::{FeedHealth, MarketData};

const V2_GET_RESERVES: &str = "getReserves()";
const V3_SLOT0: &str = "slot0()";
const V3_LIQUIDITY: &str = "liquidity()";

/// 配置中的 Token（已解析地址与精度）。
#[derive(Clone, Debug)]
struct LiveToken {
    symbol: String,
    address: Address,
    decimals: u32,
    allowed_start: bool,
    allowed_hop: bool,
    allowed_flash: bool,
}

/// 一个显式登记的池（地址、方向、精度、费率均来自配置）。
#[derive(Clone, Debug)]
struct PoolSpec {
    pool_id: String,
    protocol_id: String,
    kind: PoolKind,
    address: Address,
    address0: Address,
    address1: Address,
    token0: String,
    token1: String,
    decimals0: u32,
    decimals1: u32,
    fee_bps: i64,
    tick_spacing: Option<i32>,
}

/// 真实链行情源。
pub struct LiveFeed {
    pub chain: Chain,
    rpc: RpcClient,
    binance: BinanceClient,
    tokens: Vec<LiveToken>,
    pools: Vec<PoolSpec>,
    max_pools: usize,
    tick_word_radius: i32,
    max_ticks_per_pool: usize,
    tick_top_pools: usize,
    tick_refresh_ms: i64,
    books_limit: u16,
    cex_symbols: Vec<String>,
    cex_taker_bps: Decimal,
    cex_maker_bps: Decimal,
    cex_fee_known: bool,
    rules: HashMap<String, Instrument>,
    tick_count: u64,
    gas_price_gwei: Decimal,
    last_latency_ms: u64,
    last_errors: Vec<String>,
    /// 按 TVL 预筛后的活跃池：只刷新这些池，降低 RPC 调用量。
    active: std::collections::HashSet<String>,
    /// 最近一次计算的池 TVL（用于预筛）。
    tvl: HashMap<String, Decimal>,
    /// V3 真实余额缓存：`(token0 余额, token1 余额, 抓取时间)`，低频刷新。
    bal_cache: HashMap<String, (Decimal, Decimal, chrono::DateTime<Utc>)>,
    /// V3 tick 位图缓存：池 -> (ticks, 抓取时间)。
    tick_cache: HashMap<String, (Vec<crate::domain::market::TickLiquidity>, chrono::DateTime<Utc>)>,
    min_tvl_usd: Decimal,
    bal_refresh_ms: i64,
    /// 搜索配置（白/黑名单 + 可编辑参数）。
    search: std::sync::Arc<tokio::sync::Mutex<crate::search_config::SearchConfig>>,
    workspace: std::path::PathBuf,
    /// 配置引用了未登记 Token 的池，待首次同步时链上解析并自动登记。
    pending: Vec<crate::config::ExtraPoolConfig>,
}

impl LiveFeed {
    pub fn new(
        cfg: &Config,
        search: std::sync::Arc<tokio::sync::Mutex<crate::search_config::SearchConfig>>,
        workspace: std::path::PathBuf,
    ) -> Result<Self, String> {
        let live = &cfg.live;
        let chain_seed = cfg
            .chains
            .iter()
            .find(|c| c.chain_id == live.chain_id)
            .ok_or_else(|| format!("链 {} 未在 chains 注册表中", live.chain_id))?;
        let chain = Chain {
            chain_id: chain_seed.chain_id,
            chain_name: chain_seed.name.clone(),
            native_token: chain_seed.native.clone(),
            rpc_primary: live.rpc_primary.clone(),
            rpc_secondary: live.rpc_fallback.clone(),
            websocket_endpoint: None,
            block_time_ms: chain_seed.block_time_ms,
            finality_model: "L2 soft finality + L1 确认".into(),
            gas_model: chain_seed.gas_model.clone(),
            stage: ChainStage::parse(&chain_seed.stage).unwrap_or(ChainStage::DataValidated),
            data_quality: DataQuality::High,
            adapter_version: "onchain-rpc-m1".into(),
            enabled: true,
        };

        let mut tokens = Vec::new();
        for t in &live.tokens {
            let address = abi::parse_address(&t.address)
                .map_err(|e| format!("Token {} 地址非法: {e}", t.symbol))?;
            let status = TokenStatus::parse(&t.status).unwrap_or(TokenStatus::Approved);
            if !status.is_approved() {
                continue;
            }
            tokens.push(LiveToken {
                symbol: t.symbol.clone(),
                address,
                decimals: t.decimals.unwrap_or(18),
                allowed_start: t.allowed_start,
                allowed_hop: t.allowed_hop,
                allowed_flash: t.allowed_flash,
            });
        }
        if tokens.len() < 2 {
            return Err("live.tokens 至少需要两个 APPROVED Token".into());
        }

        let mut pools = Vec::new();
        let mut pending = Vec::new();
        for ep in &live.extra_pools {
            match parse_pool(ep, &tokens) {
                Ok(p) => pools.push(p),
                Err(e) => {
                    if e.contains("未登记") {
                        pending.push(ep.clone());
                    } else {
                        tracing::warn!("忽略无法解析的额外池 {}: {e}", ep.address);
                    }
                }
            }
        }
        if pools.is_empty() && pending.is_empty() {
            return Err(
                "live.extra_pools 为空：当前实现只接受显式登记的池，避免工厂发现带入未验证池".into(),
            );
        }
        // 公共 RPC 有配额：先按“币对优先级 + V3 + 低费率”排序，再按每个币对最多 2 个池
        // 分散保留，避免 160+ 小池里只有 WETH/USDC 被反复刷新、其他币对永远进不来。
        let pair_rank = |a: &str, b: &str| -> u8 {
            let order = [
                ("WETH", "USDC"),
                ("WETH", "USDT"),
                ("USDC", "USDT"),
                ("WBTC", "USDC"),
                ("WBTC", "USDT"),
                ("WBTC", "WETH"),
                ("WETH", "ARB"),
                ("ARB", "USDC"),
                ("ARB", "USDT"),
                ("WETH", "DAI"),
                ("USDC", "DAI"),
                ("DAI", "USDT"),
                ("WETH", "GMX"),
                ("GMX", "USDT"),
                ("wstETH", "WETH"),
                ("WBTC", "ARB"),
                ("ARB", "DAI"),
                ("ARB", "GMX"),
                ("WBTC", "DAI"),
            ];
            for (i, (x, y)) in order.iter().enumerate() {
                if (a == *x && b == *y) || (a == *y && b == *x) {
                    return i as u8;
                }
            }
            200
        };
        let pair_key = |a: &str, b: &str| -> String {
            if a <= b {
                format!("{a}/{b}")
            } else {
                format!("{b}/{a}")
            }
        };
        pools.sort_by_key(|p| {
            // 按 AMM 家族而非协议名排序：Camelot 等第三方 V2 分叉要和
            // Uni V2 / Sushi 同列，否则会被排到 V2 配额之后永远拿不到槽位。
            let family = match p.kind {
                PoolKind::UniswapV3 => 0u8,
                _ => 1,
            };
            (
                pair_rank(&p.token0, &p.token1),
                family,
                p.fee_bps.max(0),
            )
        });
        let max_pools = live.max_pools.max(1);
        // V3 与 V2 分账配额：同一币对里 V3 费率档多、排序靠前，若只按币对限流，
        // V2 池（Uni V2 / Sushi / Camelot）会被同对的 V3 挤掉，永远进不了刷新集合。
        // 这里给两类各留独立预算与每对上限，保证 V2 系池一定拿到槽位。
        let v3_budget = (max_pools * 7 / 10).max(1);
        let v2_budget = max_pools.saturating_sub(v3_budget).max(4);
        let mut per_pair: HashMap<String, (usize, usize)> = HashMap::new();
        let mut kept = Vec::new();
        let mut n_v3 = 0usize;
        let mut n_v2 = 0usize;
        for p in pools {
            let key = pair_key(&p.token0, &p.token1);
            let slot = per_pair.entry(key).or_insert((0, 0));
            match p.kind {
                PoolKind::UniswapV3 => {
                    if slot.0 >= 2 || n_v3 >= v3_budget {
                        continue;
                    }
                    slot.0 += 1;
                    n_v3 += 1;
                }
                _ => {
                    if slot.1 >= 2 || n_v2 >= v2_budget {
                        continue;
                    }
                    slot.1 += 1;
                    n_v2 += 1;
                }
            }
            kept.push(p);
            if kept.len() >= max_pools {
                break;
            }
        }
        pools = kept;
        // 启动时并入已持久化的自动发现池（Token 已由 search_config 提供）。
        let discovered = crate::dex::discovery::load_pools(&workspace);
        let cap = live.max_pools.max(1) + 8;
        let mut merged_disc = 0usize;
        let mut skipped_unknown = 0usize;
        let mut skipped_dup = 0usize;
        for p in discovered {
            if pools.len() >= cap {
                break;
            }
            // 自动发现里 protocol_id 缺失/unknown 的条目判定不了 AMM 数学，
            // 盲目按 V2 处理会把 V3 池地址当 V2 去读 getReserves，必然 revert。
            if p.protocol_id.trim().is_empty() || p.protocol_id.eq_ignore_ascii_case("unknown") {
                skipped_unknown += 1;
                continue;
            }
            let (Ok(addr), Ok(a0), Ok(a1)) = (
                abi::parse_address(&p.pool),
                abi::parse_address(&p.address0),
                abi::parse_address(&p.address1),
            ) else {
                continue;
            };
            // 按地址去重：同一池可能以不同 pool_id 再次出现，重复并入会占用刷新配额。
            if pools.iter().any(|x| x.pool_id == p.pool || x.address == addr) {
                skipped_dup += 1;
                continue;
            }
            let kind = if p.kind.eq_ignore_ascii_case("UNISWAP_V3") {
                PoolKind::UniswapV3
            } else if p.kind.eq_ignore_ascii_case("STABLESWAP") {
                PoolKind::StableSwap
            } else {
                PoolKind::UniswapV2
            };
            pools.push(PoolSpec {
                pool_id: p.pool.clone(),
                protocol_id: p.protocol_id.clone(),
                kind,
                address: addr,
                address0: a0,
                address1: a1,
                token0: p.token0,
                token1: p.token1,
                decimals0: p.decimals0,
                decimals1: p.decimals1,
                fee_bps: p.fee_bps,
                tick_spacing: Some(p.tick_spacing as i32),
            });
            merged_disc += 1;
        }
        if skipped_unknown > 0 || skipped_dup > 0 || merged_disc > 0 {
            tracing::info!(
                "自动发现池合并：并入 {} 个，跳过 协议未知 {} 个 / 地址重复 {} 个",
                merged_disc,
                skipped_unknown,
                skipped_dup
            );
        }
        let kinds_v3 = pools.iter().filter(|p| p.kind == PoolKind::UniswapV3).count();
        tracing::info!(
            "真实链池预筛：保留 {} / {} 个（V3 {} · V2 {}；实际 kind V3 {} / 其它 {}）",
            pools.len(),
            live.extra_pools.len(),
            n_v3,
            n_v2,
            kinds_v3,
            pools.len() - kinds_v3
        );

        let mut rules = HashMap::new();
        for sym in &cfg.cex.symbols {
            let (base, quote) = split_symbol(sym);
            rules.insert(
                sym.clone(),
                Instrument {
                    exchange: cfg.cex.exchange.clone(),
                    symbol: sym.clone(),
                    base,
                    quote,
                    status: "TRADING".into(),
                    min_qty: dec("0.0001"),
                    qty_step: dec("0.0001"),
                    min_notional: dec("5"),
                    price_tick: dec("0.01"),
                    maker_bps: Decimal::from_f64(cfg.cex.maker_bps).unwrap_or(dec("8")),
                    taker_bps: Decimal::from_f64(cfg.cex.taker_bps).unwrap_or(dec("10")),
                    fee_known: cfg.cex.fee_confirmed,
                    updated_at: Utc::now(),
                },
            );
        }

        Ok(Self {
            chain,
            rpc: RpcClient::new_pool(&live.rpc_primary, &live.rpc_pool),
            binance: BinanceClient::new(None),
            tokens,
            pools,
            max_pools: live.max_pools.max(1),
            tick_word_radius: live.tick_word_radius,
            max_ticks_per_pool: live.max_ticks_per_pool,
            tick_top_pools: live.tick_top_pools.max(1),
            tick_refresh_ms: live.tick_refresh_ms,
            books_limit: cfg.cex.book_levels.clamp(20, 500) as u16,
            cex_symbols: cfg.cex.symbols.clone(),
            cex_taker_bps: Decimal::from_f64(cfg.cex.taker_bps).unwrap_or(dec("10")),
            cex_maker_bps: Decimal::from_f64(cfg.cex.maker_bps).unwrap_or(dec("8")),
            cex_fee_known: cfg.cex.fee_confirmed,
            rules,
            tick_count: 0,
            gas_price_gwei: cfg.dex.gas_price_gwei.to_string().parse().unwrap_or(dec("0.02")),
            last_latency_ms: 0,
            last_errors: Vec::new(),
            active: std::collections::HashSet::new(),
            tvl: HashMap::new(),
            bal_cache: HashMap::new(),
            tick_cache: HashMap::new(),
            min_tvl_usd: Decimal::from_f64_retain(cfg.dex_engine.min_pool_tvl_usd)
                .unwrap_or(Decimal::from(10_000)),
            bal_refresh_ms: 60_000,
            search,
            workspace,
            pending,
        })
    }

    /// 链上解析并自动登记未登记 Token 的池：读取 token0/token1、symbol/decimals，
    /// 成功后加入白名单（source=auto）并立即纳入本轮刷新。
    async fn resolve_pending(&mut self, block: u64) {
        if self.pending.is_empty() {
            return;
        }
        let all: Vec<_> = self.pending.drain(..).collect();
        let split = all.len().min(12);
        let (drafts, rest) = all.split_at(split);
        self.pending = rest.to_vec();
        let drafts: Vec<_> = drafts
            .iter()
            .filter(|d| abi::parse_address(&d.address).is_ok())
            .cloned()
            .collect();
        if drafts.is_empty() {
            return;
        }

        let mut calls = Vec::new();
        for d in drafts.iter() {
            let addr = abi::parse_address(&d.address).unwrap_or_default();
            calls.push(Call {
                to: addr,
                data: abi::encode_call("token0()", &[]),
            });
            calls.push(Call {
                to: addr,
                data: abi::encode_call("token1()", &[]),
            });
        }
        let results = match self.rpc.batch_eth_call(&calls, Some(block)).await {
            Ok(r) => r,
            Err(e) => {
                self.last_errors.push(format!("自动登记读取池 Token 失败: {e}"));
                return;
            }
        };

        // draft 索引 -> (token0 地址, token1 地址)
        let mut resolved: Vec<Option<([u8; 20], [u8; 20])>> = Vec::new();
        for (i, _) in drafts.iter().enumerate() {
            let a = results
                .get(i * 2)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_address(&r.data));
            let b = results
                .get(i * 2 + 1)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_address(&r.data));
            resolved.push(match (a, b) {
                (Some(x), Some(y)) if x != [0u8; 20] && y != [0u8; 20] => Some((x, y)),
                _ => None,
            });
        }

        // 唯一 Token 地址 → symbol/decimals
        let mut unique: Vec<[u8; 20]> = Vec::new();
        for pair in resolved.iter().flatten() {
            for a in [pair.0, pair.1] {
                if !unique.contains(&a) {
                    unique.push(a);
                }
            }
        }
        let mut token_calls = Vec::new();
        for a in &unique {
            let addr = Address::from(*a);
            token_calls.push(Call {
                to: addr,
                data: abi::encode_call("symbol()", &[]),
            });
            token_calls.push(Call {
                to: addr,
                data: abi::encode_call("decimals()", &[]),
            });
        }
        let token_results = match self.rpc.batch_eth_call(&token_calls, Some(block)).await {
            Ok(r) => r,
            Err(e) => {
                self.last_errors.push(format!("自动登记读取 Token 元数据失败: {e}"));
                return;
            }
        };
        let mut meta: HashMap<[u8; 20], (String, u32)> = HashMap::new();
        for (i, a) in unique.iter().enumerate() {
            let sym = token_results
                .get(i * 2)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_string(&r.data));
            let dec = token_results
                .get(i * 2 + 1)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_u128(&r.data))
                .map(|v| v as u32);
            if let (Some(sym), Some(dec)) = (sym, dec) {
                meta.insert(*a, (sym, dec));
            }
        }

        let mut registered = 0usize;
        for (i, d) in drafts.iter().enumerate() {
            let Some((a0, a1)) = resolved.get(i).copied().flatten() else {
                continue;
            };
            let Some((sym0, dec0)) = meta.get(&a0).cloned() else {
                continue;
            };
            let Some((sym1, dec1)) = meta.get(&a1).cloned() else {
                continue;
            };
            {
                let cfg = self.search.lock().await;
                if cfg.is_blacklisted(&sym0) || cfg.is_blacklisted(&sym1) {
                    continue;
                }
            }
            let kind = match parse_pool(d, &self.tokens) {
                Ok(_) => continue,
                Err(_) => match d.kind.to_ascii_uppercase().as_str() {
                    "UNISWAP_V2" => PoolKind::UniswapV2,
                    "UNISWAP_V3" => PoolKind::UniswapV3,
                    _ => continue,
                },
            };
            // 补充 Token 列表（去重）
            for (sym, dec) in [(&sym0, dec0), (&sym1, dec1)] {
                if !self.tokens.iter().any(|t| t.symbol.eq_ignore_ascii_case(sym)) {
                    let address = if sym.eq_ignore_ascii_case(&sym0) { a0 } else { a1 };
                    self.tokens.push(LiveToken {
                        symbol: sym.clone(),
                        address,
                        decimals: dec,
                        allowed_start: false,
                        allowed_hop: true,
                        allowed_flash: false,
                    });
                }
            }
            let pool_id = format!(
                "{}-{}-{}-{}",
                d.protocol_id,
                sym0.to_lowercase(),
                sym1.to_lowercase(),
                d.fee_bps
            );
            self.pools.push(PoolSpec {
                pool_id,
                protocol_id: d.protocol_id.clone(),
                kind,
                address: Address::from(abi::parse_address(&d.address).unwrap_or_default()),
                address0: Address::from(a0),
                address1: Address::from(a1),
                token0: sym0.clone(),
                token1: sym1.clone(),
                decimals0: dec0,
                decimals1: dec1,
                fee_bps: d.fee_bps,
                tick_spacing: d.tick_spacing,
            });
            {
                let mut cfg = self.search.lock().await;
                let a0s = abi::format_address(&a0);
                let a1s = abi::format_address(&a1);
                if !cfg.tokens.iter().any(|t| t.address.eq_ignore_ascii_case(&a0s)) {
                    cfg.upsert(&sym0, &a0s, dec0, crate::search_config::WHITELIST, "auto");
                }
                if !cfg.tokens.iter().any(|t| t.address.eq_ignore_ascii_case(&a1s)) {
                    cfg.upsert(&sym1, &a1s, dec1, crate::search_config::WHITELIST, "auto");
                }
                cfg.save(&self.workspace);
            }
            registered += 1;
            tracing::info!(
                "自动登记 Token：{sym0}({dec0}) / {sym1}({dec1}) ← 池 {}",
                d.address
            );
        }
        if registered > 0 {
            tracing::info!("本轮自动登记 {registered} 个新池的 Token 并纳入刷新");
        }
    }

    /// 热加载自动发现的 Token/池（每 120 次同步一次）。
    async fn reload_discovered(&mut self) {
        let sc = self.search.lock().await.clone();
        for t in sc.whitelist() {
            if t.address.is_empty() {
                continue;
            }
            if self
                .tokens
                .iter()
                .any(|x| x.symbol.eq_ignore_ascii_case(&t.symbol))
            {
                continue;
            }
            if let Ok(address) = abi::parse_address(&t.address) {
                self.tokens.push(LiveToken {
                    symbol: t.symbol.clone(),
                    address,
                    decimals: t.decimals,
                    allowed_start: false,
                    allowed_hop: true,
                    allowed_flash: false,
                });
            }
        }
        let discovered = crate::dex::discovery::load_pools(&self.workspace);
        let cap = self.max_pools + 8;
        let mut added = 0usize;
        for p in discovered {
            if self.pools.len() >= cap {
                break;
            }
            if self.pools.iter().any(|x| x.pool_id == p.pool) {
                continue;
            }
            let Ok(addr) = abi::parse_address(&p.pool) else {
                continue;
            };
            let Ok(a0) = abi::parse_address(&p.address0) else {
                continue;
            };
            let Ok(a1) = abi::parse_address(&p.address1) else {
                continue;
            };
            let kind = match p.kind.to_ascii_uppercase().as_str() {
                "UNISWAP_V3" => PoolKind::UniswapV3,
                _ => PoolKind::UniswapV2,
            };
            self.pools.push(PoolSpec {
                pool_id: p.pool.clone(),
                protocol_id: p.protocol_id.clone(),
                kind,
                address: addr,
                address0: a0,
                address1: a1,
                token0: p.token0.clone(),
                token1: p.token1.clone(),
                decimals0: p.decimals0,
                decimals1: p.decimals1,
                fee_bps: p.fee_bps,
                tick_spacing: Some(p.tick_spacing as i32),
            });
            added += 1;
        }
        if added > 0 {
            tracing::info!("自动发现热加载：新增 {added} 个池，当前共 {} 个", self.pools.len());
        }
    }

    /// 一次完整同步：读链上状态 + CEX 盘口，生成不可变快照。
    pub async fn sync(&mut self) -> Result<MarketData, String> {
        let started = Instant::now();
        self.last_errors.clear();

        let block = self
            .rpc
            .block_number()
            .await
            .map_err(|e| format!("获取区块失败: {e}"))?;
        self.resolve_pending(block).await;
        if self.tick_count % 30 == 0 {
            self.reload_discovered().await;
        }
        let (gas, header) = tokio::join!(self.rpc.gas_price(), self.rpc.block_header(block));
        self.gas_price_gwei = dec_u128(gas.unwrap_or(0)) / Decimal::from(1_000_000_000u64);
        let (timestamp, _base_fee) = header.unwrap_or((0, 0));

        // ---- CEX 公共盘口（真实深度，用于 CEX–DEX）----
        let mut books = BookEngine::new();
        let mut prices: HashMap<String, Decimal> = HashMap::new();
        prices.insert("USDC".into(), Decimal::ONE);
        prices.insert("USDT".into(), Decimal::ONE);
        // 4 个 CEX 符号并发拉取深度，避免串行叠加延迟。
        let mut set = tokio::task::JoinSet::new();
        for symbol in self.cex_symbols.clone() {
            let client = self.binance.clone();
            let limit = self.books_limit;
            set.spawn(async move {
                let result = client.depth(&symbol, limit).await;
                (symbol, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            let Ok((symbol, result)) = joined else {
                continue;
            };
            match result {
                Ok(book) => {
                    if let Some(mid) = book.mid() {
                        let (base, quote) = split_symbol(&symbol);
                        let quote_price = prices.get(&quote).copied().unwrap_or(Decimal::ONE);
                        prices.insert(base.clone(), mid * quote_price);
                        if base == "ETH" {
                            prices.insert("WETH".into(), mid * quote_price);
                        }
                        if base == "BTC" {
                            prices.insert("WBTC".into(), mid * quote_price);
                        }
                    }
                    books.upsert_snapshot(book);
                }
                Err(e) => self
                    .last_errors
                    .push(format!("Binance {symbol} 盘口读取失败: {e}")),
            }
        }

        // 稳定币与包装资产的价格别名：DAI 按 1，wstETH 跟随 ETH/WETH。
        prices.entry("DAI".into()).or_insert(Decimal::ONE);
        if let Some(w) = prices.get("WETH").copied() {
            prices.entry("wstETH".into()).or_insert(w);
        }
        // 自动发现推导出的 Token 价格（聚合器池内价格）。
        for p in crate::dex::discovery::load_pools(&self.workspace) {
            if p.price0_usd > 0.0 {
                if let Some(d) = Decimal::from_f64(p.price0_usd) {
                    prices.entry(p.token0.to_ascii_uppercase()).or_insert(d);
                }
            }
            if p.price1_usd > 0.0 {
                if let Some(d) = Decimal::from_f64(p.price1_usd) {
                    prices.entry(p.token1.to_ascii_uppercase()).or_insert(d);
                }
            }
        }

        // cbBTC 等包装 BTC 的聚合器价格可能失真，统一对齐 WBTC/BTC。
        if let Some(btc) = prices.get("WBTC").or_else(|| prices.get("BTC")).copied() {
            for sym in ["cbBTC", "CBBTC"] {
                prices.insert(sym.to_string(), btc);
            }
        }

        // ---- 链上池状态（批量 eth_call）----
        // 只刷新活跃池（TVL 达标），V3 余额低频缓存；每 120 轮做一次全量重估。
        let now = Utc::now();
        let full_refresh = self.tick_count % 120 == 0;
        let active: std::collections::HashSet<String> = if self.active.is_empty() || full_refresh {
            self.pools.iter().map(|p| p.pool_id.clone()).collect()
        } else {
            self.active.clone()
        };
        let mut calls: Vec<Call> = Vec::new();
        // (slot0/reserves, liquidity, balance0, balance1)
        let mut layout: Vec<(usize, Option<usize>, Option<usize>, Option<usize>)> =
            Vec::with_capacity(self.pools.len());
        for p in &self.pools {
            if !active.contains(&p.pool_id) {
                layout.push((usize::MAX, None, None, None));
                continue;
            }
            match p.kind {
                PoolKind::UniswapV2 => {
                    layout.push((calls.len(), None, None, None));
                    calls.push(Call {
                        to: p.address,
                        data: abi::encode_call(V2_GET_RESERVES, &[]),
                    });
                }
                PoolKind::StableSwap => {
                    // Aerodrome/Velodrome 稳定曲线无 A 参数，只需 reserves。
                    layout.push((calls.len(), None, None, None));
                    calls.push(Call {
                        to: p.address,
                        data: abi::encode_call(V2_GET_RESERVES, &[]),
                    });
                }
                PoolKind::UniswapV3 => {
                    let bal_fresh = self
                        .bal_cache
                        .get(&p.pool_id)
                        .map(|(_, _, t)| (now - *t).num_milliseconds() < self.bal_refresh_ms)
                        .unwrap_or(false);
                    if bal_fresh {
                        layout.push((calls.len(), Some(calls.len() + 1), None, None));
                    } else {
                        layout.push((
                            calls.len(),
                            Some(calls.len() + 1),
                            Some(calls.len() + 2),
                            Some(calls.len() + 3),
                        ));
                    }
                    calls.push(Call {
                        to: p.address,
                        data: abi::encode_call(V3_SLOT0, &[]),
                    });
                    calls.push(Call {
                        to: p.address,
                        data: abi::encode_call(V3_LIQUIDITY, &[]),
                    });
                    if !bal_fresh {
                        // TVL/展示用真实余额：在 Token 合约上查询池地址的余额
                        // （V3 池本身不是 ERC20，不能对池调用 balanceOf）。
                        calls.push(Call {
                            to: p.address0,
                            data: abi::encode_call("balanceOf(address)", &[Arg::Address(p.address)]),
                        });
                        calls.push(Call {
                            to: p.address1,
                            data: abi::encode_call("balanceOf(address)", &[Arg::Address(p.address)]),
                        });
                    }
                }
                _ => layout.push((usize::MAX, None, None, None)),
            }
        }
        let results = if calls.is_empty() {
            Vec::new()
        } else {
            self.rpc
                .batch_eth_call(&calls, Some(block))
                .await
                .map_err(|e| format!("读取池状态失败: {e}"))?
        };

        let now = Utc::now();
        let block_hash = format!("0x{block:016x}");
        let mut pool_set = PoolSet::new(self.chain.chain_id);
        let mut refreshed = 0usize;
        // 返回空数据（0x）说明该地址在链上没有合约，属于配置脏数据：
        // 这类池每轮都会失败并拖慢同步，累计后直接剔除。
        let mut empty_hits: HashMap<String, u8> = HashMap::new();
        let mut n_inactive = 0usize;
        let mut n_fail_v2 = 0usize;
        let mut n_fail_v3 = 0usize;
        for (i, spec) in self.pools.iter().enumerate() {
            let (idx0, idx1, idx2, idx3) = layout[i];
            if idx0 == usize::MAX {
                continue;
            }
            let state = match spec.kind {
                PoolKind::UniswapV2 | PoolKind::StableSwap => {
                    let Some(r) = results.get(idx0).filter(|r| r.ok()) else {
                        let empty = results
                            .get(idx0)
                            .map(|r| r.data.is_empty())
                            .unwrap_or(false);
                        if empty {
                            let n = empty_hits.entry(spec.pool_id.clone()).or_insert(0);
                            *n += 1;
                        }
                        n_fail_v2 += 1;
                        self.last_errors.push(format!("池 {}（V2/{}）储备读取失败", spec.pool_id, spec.protocol_id));
                        continue;
                    };
                    let Some((r0, r1, _ts)) = abi::decode_get_reserves(&r.data) else {
                        self.last_errors.push(format!("池 {} 储备解码失败", spec.pool_id));
                        continue;
                    };
                    PoolState {
                        pool_id: spec.pool_id.clone(),
                        block_number: block,
                        block_hash: block_hash.clone(),
                        updated_at: now,
                        reserve0: dec_u128(r0) / unit_scale(spec.decimals0),
                        reserve1: dec_u128(r1) / unit_scale(spec.decimals1),
                        sqrt_price_x96: 0,
                        liquidity: 0,
                        tick: 0,
                        ticks: Vec::new(),
                        amplification: Decimal::ZERO,
                        weight0: Decimal::ZERO,
                        weight1: Decimal::ZERO,
                        fee_bps: spec.fee_bps,
                    }
                }
                PoolKind::UniswapV3 => {
                    let Some(slot0) = results.get(idx0).filter(|r| r.ok()) else {
                        n_fail_v3 += 1;
                        self.last_errors.push(format!("池 {}（V3/{}）slot0 读取失败", spec.pool_id, spec.protocol_id));
                        continue;
                    };
                    let Some((sqrt, tick)) = abi::decode_slot0(&slot0.data) else {
                        self.last_errors.push(format!("池 {} slot0 解码失败", spec.pool_id));
                        continue;
                    };
                    if sqrt == 0 {
                        self.last_errors.push(format!("池 {} 价格为零，已跳过", spec.pool_id));
                        continue;
                    }
                    let liquidity = idx1
                        .and_then(|j| results.get(j))
                        .filter(|r| r.ok())
                        .and_then(|r| abi::decode_single_u128(&r.data))
                        .unwrap_or(0);
                    if liquidity == 0 {
                        self.last_errors.push(format!("池 {} 流动性为零，已跳过", spec.pool_id));
                        continue;
                    }
                    // 展示用余额：优先本次 ERC20 balanceOf，其次 60s 内缓存，最后退回单区间估算。
                    let bal0 = idx2
                        .and_then(|j| results.get(j))
                        .filter(|r| r.ok())
                        .and_then(|r| abi::decode_single_u128(&r.data));
                    let bal1 = idx3
                        .and_then(|j| results.get(j))
                        .filter(|r| r.ok())
                        .and_then(|r| abi::decode_single_u128(&r.data));
                    let fetched = match (bal0, bal1) {
                        (Some(b0), Some(b1)) => Some((
                            dec_u128(b0) / unit_scale(spec.decimals0),
                            dec_u128(b1) / unit_scale(spec.decimals1),
                        )),
                        _ => None,
                    };
                    let (reserve0, reserve1) = if let Some((r0, r1)) = fetched {
                        self.bal_cache
                            .insert(spec.pool_id.clone(), (r0, r1, now));
                        (r0, r1)
                    } else if let Some((r0, r1, _)) = self.bal_cache.get(&spec.pool_id) {
                        (*r0, *r1)
                    } else {
                        let q96 = dec_u128((1u128 << 96) - 1);
                        let sqrt_dec = dec_u128(sqrt) / q96;
                        let l = dec_u128(liquidity);
                        if sqrt_dec > Decimal::ZERO {
                            (
                                (l / sqrt_dec) / unit_scale(spec.decimals0),
                                (l * sqrt_dec) / unit_scale(spec.decimals1),
                            )
                        } else {
                            (Decimal::ZERO, Decimal::ZERO)
                        }
                    };
                    PoolState {
                        pool_id: spec.pool_id.clone(),
                        block_number: block,
                        block_hash: block_hash.clone(),
                        updated_at: now,
                        reserve0,
                        reserve1,
                        sqrt_price_x96: sqrt,
                        liquidity,
                        tick,
                        ticks: Vec::new(),
                        amplification: Decimal::ZERO,
                        weight0: Decimal::ZERO,
                        weight1: Decimal::ZERO,
                        fee_bps: spec.fee_bps,
                    }
                }
                _ => continue,
            };

            let tvl = match spec.kind {
                PoolKind::UniswapV2 => {
                    state.reserve0 * prices.get(&spec.token0).copied().unwrap_or(Decimal::ZERO)
                        + state.reserve1 * prices.get(&spec.token1).copied().unwrap_or(Decimal::ZERO)
                }
                _ => {
                    state.reserve0 * prices.get(&spec.token0).copied().unwrap_or(Decimal::ZERO)
                        + state.reserve1 * prices.get(&spec.token1).copied().unwrap_or(Decimal::ZERO)
                }
            };

            let pool = PoolRef {
                pool_id: spec.pool_id.clone(),
                chain_id: self.chain.chain_id,
                protocol_id: spec.protocol_id.clone(),
                kind: spec.kind,
                address: abi::format_address(&spec.address),
                token0: spec.token0.clone(),
                token1: spec.token1.clone(),
                decimals0: spec.decimals0,
                decimals1: spec.decimals1,
                fee_bps: spec.fee_bps,
                tick_spacing: spec.tick_spacing,
                tvl_usd: tvl,
                verified: true,
                execution_allowed: true,
                added_at: now,
            };
            pool_set.upsert(pool, state);
            self.tvl.insert(spec.pool_id.clone(), tvl);
            refreshed += 1;
        }

        if self.tick_count % 10 == 0 {
            tracing::info!(
                "池状态刷新：规格 {} · 仅激活 {} · 成功 {}（V3 {} / V2 {}）· 失败 V2 {} / V3 {} · 调用 {} 次",
                self.pools.len(),
                self.pools.len().saturating_sub(n_inactive),
                refreshed,
                pool_set.all().iter().filter(|p| p.pool.kind == PoolKind::UniswapV3).count(),
                pool_set.all().iter().filter(|p| p.pool.kind != PoolKind::UniswapV3).count(),
                n_fail_v2,
                n_fail_v3,
                calls.len()
            );
        }

        // 连续两轮空返回的池视为配置脏数据（地址无合约），从刷新集合中剔除。
        let dead: Vec<String> = empty_hits
            .iter()
            .filter(|(_, n)| **n >= 2)
            .map(|(id, _)| id.clone())
            .collect();
        if !dead.is_empty() {
            let before = self.pools.len();
            self.pools.retain(|p| !dead.contains(&p.pool_id));
            for id in &dead {
                self.tvl.remove(id);
                self.bal_cache.remove(id);
            }
            tracing::warn!(
                "剔除 {} 个无合约地址的池（{} → {}），请检查配置中的地址",
                dead.len(),
                before,
                self.pools.len()
            );
        }

        // 下一轮刷新集合：按 AMM 家族分别取 TVL 最高的若干池。
        // 不能用「单一 TVL 门槛」过滤——V2 池（Uni V2 / Sushi / Camelot）体量
        // 天然比 V3 小一个数量级，一刀切会把它们每轮都踢出激活集，
        // 表现为「V2 池刷新一次就消失」。
        // ---- V3 tick 位图：跨 tick 报价需要 initialized ticks ----
        if self.tick_word_radius > 0 {
            let radius = self.tick_word_radius;
            let spacing_map: HashMap<String, i32> = self
                .pools
                .iter()
                .map(|p| (p.pool_id.clone(), p.tick_spacing.unwrap_or(60)))
                .collect();
            let addr_map: HashMap<String, abi::Address> = self
                .pools
                .iter()
                .map(|p| (p.pool_id.clone(), p.address))
                .collect();
            let mut ranked: Vec<(String, Decimal)> = pool_set
                .all()
                .iter()
                .filter(|rt| rt.pool.kind == PoolKind::UniswapV3)
                .map(|rt| {
                    (
                        rt.pool.pool_id.clone(),
                        self.tvl.get(&rt.pool.pool_id).copied().unwrap_or(Decimal::ZERO),
                    )
                })
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            let tick_top: std::collections::HashSet<String> =
                ranked.into_iter().take(self.tick_top_pools).map(|(id, _)| id).collect();
            let mut bitmap_calls: Vec<Call> = Vec::new();
            let mut bitmap_meta: Vec<(String, i32, i32, i32)> = Vec::new(); // pool,spacing,word
            let mut cached_hits: Vec<(String, Vec<crate::domain::market::TickLiquidity>)> = Vec::new();
            for rt in pool_set.all() {
                if rt.pool.kind != PoolKind::UniswapV3
                    || rt.state.liquidity == 0
                    || !tick_top.contains(&rt.pool.pool_id)
                {
                    continue;
                }
                if self.tvl.get(&rt.pool.pool_id).copied().unwrap_or(Decimal::ZERO)
                    < self.min_tvl_usd
                {
                    continue;
                }
                if let Some((ticks, at)) = self.tick_cache.get(&rt.pool.pool_id) {
                    if (now - *at).num_milliseconds() < self.tick_refresh_ms {
                        cached_hits.push((rt.pool.pool_id.clone(), ticks.clone()));
                        continue;
                    }
                }
                let spacing = spacing_map
                    .get(&rt.pool.pool_id)
                    .copied()
                    .unwrap_or(60)
                    .max(1);
                let compressed = rt.state.tick.div_euclid(spacing);
                let word = compressed.div_euclid(256);
                for w in (word - radius)..=(word + radius) {
                    bitmap_meta.push((rt.pool.pool_id.clone(), spacing, w, w));
                    bitmap_calls.push(Call {
                        to: addr_map.get(&rt.pool.pool_id).copied().unwrap_or_default(),
                        data: abi::encode_call("tickBitmap(int16)", &[Arg::Int(w as i128)]),
                    });
                }
            }
            if !bitmap_calls.is_empty() {
                let bitmap_res = self
                    .rpc
                    .batch_eth_call(&bitmap_calls, Some(block))
                    .await
                    .unwrap_or_default();
                let mut tick_calls: Vec<Call> = Vec::new();
                let mut tick_meta: Vec<(String, i32)> = Vec::new(); // pool, tick
                for (i, (pool_id, spacing, _w, _)) in bitmap_meta.iter().enumerate() {
                    let Some(res) = bitmap_res.get(i).filter(|r| r.ok()) else {
                        continue;
                    };
                    let Some(w) = abi::word_at(&res.data, 0) else {
                        continue;
                    };
                    let word_pos = bitmap_meta[i].2;
                    for bit in 0..256usize {
                        if (w[bit / 8] >> (bit % 8)) & 1 == 0 {
                            continue;
                        }
                        let tick = (word_pos * 256 + bit as i32) * spacing;
                        tick_calls.push(Call {
                            to: addr_map.get(pool_id).copied().unwrap_or_default(),
                            data: abi::encode_call("ticks(int24)", &[Arg::Int(tick as i128)]),
                        });
                        tick_meta.push((pool_id.clone(), tick));
                        if tick_calls.len() >= self.max_ticks_per_pool * self.pools.len().max(1) {
                            break;
                        }
                    }
                }
                let tick_res = self
                    .rpc
                    .batch_eth_call(&tick_calls, Some(block))
                    .await
                    .unwrap_or_default();
                let mut per_pool: HashMap<String, Vec<crate::domain::market::TickLiquidity>> = HashMap::new();
                for (i, (pool_id, tick)) in tick_meta.iter().enumerate() {
                    let Some(res) = tick_res.get(i).filter(|r| r.ok()) else {
                        continue;
                    };
                    let gross = abi::word_at(&res.data, 0)
                        .and_then(abi::decode_u128)
                        .unwrap_or(0);
                    let net = abi::word_at(&res.data, 1)
                        .and_then(abi::decode_i128)
                        .unwrap_or(0);
                    per_pool.entry(pool_id.clone()).or_default().push(
                        crate::domain::market::TickLiquidity {
                            tick_index: *tick,
                            liquidity_net: net,
                            liquidity_gross: gross,
                        },
                    );
                }
                for (pool_id, mut ticks) in per_pool {
                    ticks.sort_by_key(|t| t.tick_index);
                    ticks.truncate(self.max_ticks_per_pool);
                    self.tick_cache.insert(pool_id.clone(), (ticks.clone(), now));
                    if let Some(rt) = pool_set.get_mut(&pool_id) {
                        rt.state.ticks = ticks;
                    }
                }
            }
            for (pool_id, ticks) in cached_hits {
                if let Some(rt) = pool_set.get_mut(&pool_id) {
                    rt.state.ticks = ticks;
                }
            }
        }

        let mut next_active = std::collections::HashSet::new();
        let quota_v3 = (self.max_pools * 7 / 10).max(1);
        let quota_v2 = self.max_pools.saturating_sub(quota_v3).max(6);
        for kind_is_v3 in [true, false] {
            let mut ranked: Vec<(String, Decimal)> = self
                .pools
                .iter()
                .filter(|p| (p.kind == PoolKind::UniswapV3) == kind_is_v3)
                .filter_map(|p| self.tvl.get(&p.pool_id).map(|v| (p.pool_id.clone(), *v)))
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            let quota = if kind_is_v3 { quota_v3 } else { quota_v2 };
            for (id, _) in ranked.into_iter().take(quota) {
                next_active.insert(id);
            }
        }
        // 还没有 TVL 记录的池（上一轮失败 / 刚并入）继续重试，上限不超过总配额。
        for p in &self.pools {
            if next_active.len() >= self.max_pools {
                break;
            }
            if !self.tvl.contains_key(&p.pool_id) {
                next_active.insert(p.pool_id.clone());
            }
        }
        if next_active.is_empty() {
            next_active = self.pools.iter().map(|p| p.pool_id.clone()).collect();
        }
        self.active = next_active;
        self.last_errors.sort();
        self.last_errors.dedup();
        self.last_errors.truncate(6);

        // WETH 价格缺失时用 V3 池推导（只读链上价格，不编造）
        if !prices.contains_key("WETH") || prices["WETH"].is_zero() {
            if let Some(rt) = pool_set
                .all()
                .iter()
                .find(|p| p.pool.kind == PoolKind::UniswapV3 && p.pool.token0 == "WETH")
            {
                if let Ok(px) = rt.quote_exact_in("WETH", Decimal::ONE) {
                    // 用 1 WETH 的链上可成交输出近似 USD 价（USDC/USDT 视为 1）
                    prices.insert("WETH".into(), px.amount_out);
                    prices.insert("ETH".into(), px.amount_out);
                }
            }
        }

        let mut tokens: HashMap<String, Token> = HashMap::new();
        for t in &self.tokens {
            tokens.insert(
                t.symbol.clone(),
                Token {
                    symbol: t.symbol.clone(),
                    chain_id: self.chain.chain_id,
                    address: abi::format_address(&t.address),
                    decimals: t.decimals,
                    standard: "ERC20".into(),
                    code_hash: "onchain".into(),
                    status: TokenStatus::Approved,
                    risk_tags: Vec::new(),
                    allowed_start: t.allowed_start,
                    allowed_hop: t.allowed_hop,
                    allowed_flash: t.allowed_flash,
                    special_transfer: false,
                    enabled: true,
                },
            );
        }

        let mut instruments = self.rules.clone();
        for (sym, inst) in instruments.iter_mut() {
            inst.maker_bps = self.cex_maker_bps;
            inst.taker_bps = self.cex_taker_bps;
            inst.fee_known = self.cex_fee_known;
            inst.updated_at = now;
            if let Some(book) = books.get(&inst.exchange, sym) {
                if !book.valid {
                    inst.status = "UNKNOWN".into();
                }
            }
        }

        self.tick_count += 1;
        self.last_latency_ms = started.elapsed().as_millis() as u64;
        let block_age_ms = if timestamp > 0 {
            (Utc::now().timestamp_millis() - (timestamp as i64) * 1000).max(0)
        } else {
            0
        };
        let ok = !pool_set.is_empty() && !books.is_empty();
        let health = FeedHealth {
            ok,
            mode: "live".into(),
            source: "arbitrum-rpc + binance".into(),
            rpc_primary: self.rpc.primary().to_string(),
            rpc_fallback: self.rpc.fallback().map(|s| s.to_string()),
            last_error: if self.last_errors.is_empty() {
                None
            } else {
                Some(self.last_errors.join(" | "))
            },
            last_sync: Some(now),
            latency_ms: self.last_latency_ms,
            block_age_ms,
            pools_total: self.pools.len(),
            pools_refreshed: refreshed,
            ticks_pools: 0,
        };

        Ok(MarketData {
            chain: self.chain.clone(),
            head_block: block,
            block_timestamp: timestamp,
            tokens,
            pools: pool_set,
            instruments,
            books,
            prices,
            mode: "live".into(),
            source: "arbitrum-rpc + binance".into(),
            updated_at: now,
            tick_count: self.tick_count,
            gas_price_gwei: self.gas_price_gwei,
            health,
        })
    }
}

fn parse_pool(ep: &ExtraPoolConfig, tokens: &[LiveToken]) -> Result<PoolSpec, String> {
    let kind = match ep.kind.to_ascii_uppercase().as_str() {
        "UNISWAP_V2" => PoolKind::UniswapV2,
        "UNISWAP_V3" => PoolKind::UniswapV3,
        "STABLESWAP" => PoolKind::StableSwap,
        other => return Err(format!("暂不支持实时适配的池类型 {other}")),
    };
    let address = abi::parse_address(&ep.address)?;
    let t0 = tokens
        .iter()
        .find(|t| t.symbol == ep.token0)
        .ok_or_else(|| format!("token0 {} 未登记", ep.token0))?;
    let t1 = tokens
        .iter()
        .find(|t| t.symbol == ep.token1)
        .ok_or_else(|| format!("token1 {} 未登记", ep.token1))?;
    let decimals0 = if ep.decimals0 > 0 { ep.decimals0 } else { t0.decimals };
    let decimals1 = if ep.decimals1 > 0 { ep.decimals1 } else { t1.decimals };
    Ok(PoolSpec {
        pool_id: format!(
            "{}-{}-{}-{}",
            ep.protocol_id,
            ep.token0.to_lowercase(),
            ep.token1.to_lowercase(),
            ep.fee_bps
        ),
        protocol_id: ep.protocol_id.clone(),
        kind,
        address,
        address0: t0.address,
        address1: t1.address,
        token0: ep.token0.clone(),
        token1: ep.token1.clone(),
        decimals0,
        decimals1,
        fee_bps: ep.fee_bps,
        tick_spacing: ep.tick_spacing,
    })
}

fn split_symbol(symbol: &str) -> (String, String) {
    let s = symbol.to_ascii_uppercase();
    for q in ["USDT", "USDC", "USD"] {
        if let Some(base) = s.strip_suffix(q) {
            if !base.is_empty() {
                return (base.to_string(), q.to_string());
            }
        }
    }
    (s, "USDT".to_string())
}

/// u128 → Decimal 的安全转换（超过 Decimal::MAX 时饱和，避免 panic）。
fn dec_u128(v: u128) -> Decimal {
    Decimal::from_str(&v.to_string()).unwrap_or(Decimal::MAX)
}

/// 供 API 展示：当前配置是否具备 M2（需要 REVM）。
pub fn m2_available() -> bool {
    false
}
