//! 合成市场：确定性随机游走 + 定期价格错位（dislocation）。
//!
//! 目的不是「造出漂亮收益」，而是让系统在受控条件下经历：
//! 三角机会、跨池机会、CEX–DEX 价差、深度不足、部分成交、回滚与补偿。

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::collections::HashMap;

use crate::cex::BookEngine;
use crate::config::Config;
use crate::domain::market::{
    BookLevel, Chain, DexProtocol, Instrument, OrderBook as DomainBook, PoolRef, PoolState, Token,
};
use crate::domain::money::{dec, q8};
use crate::domain::types::{ChainStage, DataQuality, PoolKind, TokenStatus};
use crate::dex::PoolSet;
use crate::paper::scenario::Rng;

/// 一个池的运行期模板：基础储备 + 当前价格因子。
struct PoolSeed {
    pool: PoolRef,
    base0: Decimal,
    base1: Decimal,
    /// 价格相对基准的倍数（1.0 表示与基准价一致）。
    factor: f64,
    amplification: Decimal,
    weight0: Decimal,
    weight1: Decimal,
}

/// 合成市场。
pub struct DemoMarket {
    pub chain: Chain,
    pub head_block: u64,
    pub tokens: HashMap<String, Token>,
    pub pools: PoolSet,
    pub instruments: HashMap<String, Instrument>,
    pub books: BookEngine,
    pub prices: HashMap<String, Decimal>,
    seeds: Vec<PoolSeed>,
    /// CEX 价格相对基准的倍数。
    cex_factors: HashMap<String, f64>,
    base_prices: HashMap<String, Decimal>,
    rng: Rng,
    tick_count: u64,
}

impl DemoMarket {
    /// 构建初始市场。
    pub fn new(cfg: &Config, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let chain_seed = cfg
            .chains
            .iter()
            .find(|c| c.stage == "PAPER_READY")
            .or_else(|| cfg.chains.first());
        let chain = Chain {
            chain_id: chain_seed.map(|c| c.chain_id).unwrap_or(42161),
            chain_name: chain_seed.map(|c| c.name.clone()).unwrap_or_else(|| "Arbitrum".into()),
            native_token: chain_seed.map(|c| c.native.clone()).unwrap_or_else(|| "ETH".into()),
            rpc_primary: chain_seed.map(|c| c.rpc_primary.clone()).unwrap_or_default(),
            rpc_secondary: chain_seed.map(|c| c.rpc_secondary.clone()).unwrap_or_default(),
            websocket_endpoint: None,
            block_time_ms: chain_seed.map(|c| c.block_time_ms).unwrap_or(260),
            finality_model: "L2 soft finality + L1 确认".into(),
            gas_model: chain_seed.map(|c| c.gas_model.clone()).unwrap_or_default(),
            stage: ChainStage::PaperReady,
            data_quality: DataQuality::High,
            adapter_version: "demo-mkt-v1".into(),
            enabled: true,
        };

        // ---- Token ----
        let mut tokens = HashMap::new();
        for (sym, decimals, addr) in [
            ("USDC", 6u32, "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
            ("USDT", 6, "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9"),
            ("WETH", 18, "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1"),
            ("WBTC", 8, "0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f"),
            ("ARB", 18, "0x912CE59144191C1204E64559FE8253a0e49E6548"),
        ] {
            tokens.insert(
                sym.to_string(),
                Token {
                    symbol: sym.into(),
                    chain_id: chain.chain_id,
                    address: addr.into(),
                    decimals,
                    standard: "ERC20".into(),
                    code_hash: format!("0x{:064x}", sym.len() * 977),
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

        // ---- 基准价格 ----
        let mut base_prices = HashMap::new();
        base_prices.insert("USDC".to_string(), dec("1.0"));
        // 稳定币按 1.0 估值：偏离作为独立风险项监控，不混入盈亏核算
        base_prices.insert("USDT".to_string(), dec("1.0"));
        base_prices.insert("WETH".to_string(), dec("2793.10"));
        base_prices.insert("ETH".to_string(), dec("2793.10"));
        base_prices.insert("WBTC".to_string(), dec("68640.00"));
        base_prices.insert("BTC".to_string(), dec("68640.00"));
        base_prices.insert("ARB".to_string(), dec("0.6120"));

        // ---- 池模板 ----
        // 解析度：每跳手续费与 TVL 覆盖不同规模，制造真实的跨池差异。
        struct SeedDef {
            pool_id: &'static str,
            protocol: &'static str,
            kind: PoolKind,
            address: &'static str,
            t0: &'static str,
            t1: &'static str,
            fee_bps: i64,
            tvl_usd: &'static str,
        }
        let defs = [
            SeedDef {
                pool_id: "uni-v2-weth-usdc",
                protocol: "uniswap-v2",
                kind: PoolKind::UniswapV2,
                address: "0x8c0e6f0b8d1b4a6a6b0f2a1e6c5b3a9d2f4e7c11",
                t0: "WETH",
                t1: "USDC",
                fee_bps: 30,
                tvl_usd: "4200000",
            },
            SeedDef {
                pool_id: "uni-v3-weth-usdc-5",
                protocol: "uniswap-v3",
                kind: PoolKind::UniswapV3,
                address: "0xC6962004f452bE9203591991D15f6b388e09E8D0",
                t0: "WETH",
                t1: "USDC",
                fee_bps: 5,
                tvl_usd: "9800000",
            },
            SeedDef {
                pool_id: "uni-v3-weth-usdt-5",
                protocol: "uniswap-v3",
                kind: PoolKind::UniswapV3,
                address: "0x641C00A822e8b671738d32a431a4Fb6074E5c79d",
                t0: "WETH",
                t1: "USDT",
                fee_bps: 5,
                tvl_usd: "6100000",
            },
            SeedDef {
                pool_id: "uni-v2-weth-usdt",
                protocol: "uniswap-v2",
                kind: PoolKind::UniswapV2,
                address: "0x6Ec731d5b0d1aC0e2fB4bC6B7dA2e9f3a1b8c7d6",
                t0: "WETH",
                t1: "USDT",
                fee_bps: 30,
                tvl_usd: "2400000",
            },
            SeedDef {
                pool_id: "curve-usdc-usdt",
                protocol: "curve-stableswap",
                kind: PoolKind::StableSwap,
                address: "0x7f90122BF0700F9E7e1F688fe926940E8839F353",
                t0: "USDC",
                t1: "USDT",
                fee_bps: 1,
                tvl_usd: "14000000",
            },
            SeedDef {
                pool_id: "sushi-weth-usdc",
                protocol: "sushi-v2",
                kind: PoolKind::UniswapV2,
                address: "0x905dfCD5649217c42684f23958568e533C711Aa3",
                t0: "WETH",
                t1: "USDC",
                fee_bps: 30,
                tvl_usd: "1350000",
            },
            SeedDef {
                pool_id: "bal-weth-usdc",
                protocol: "balancer-weighted",
                kind: PoolKind::WeightedPool,
                address: "0x8a8f6f0a2f9d4b0c8e5a1d7b3c6f9e2a4b8d1c5e",
                t0: "WETH",
                t1: "USDC",
                fee_bps: 5,
                tvl_usd: "3100000",
            },
            SeedDef {
                pool_id: "uni-v3-wbtc-weth-30",
                protocol: "uniswap-v3",
                kind: PoolKind::UniswapV3,
                address: "0x2f5e87C9312fa29aed5c179E456625D79015299c",
                t0: "WBTC",
                t1: "WETH",
                fee_bps: 30,
                tvl_usd: "7600000",
            },
        ];

        let mut seeds = Vec::new();
        for d in defs {
            let px0 = base_prices.get(d.t0).copied().unwrap_or(Decimal::ONE);
            let px1 = base_prices.get(d.t1).copied().unwrap_or(Decimal::ONE);
            let tvl = d.tvl_usd.parse::<Decimal>().unwrap_or(dec("1000000"));
            // 储备：使两侧价值各占一半
            let base0 = tvl / Decimal::from(2) / px0;
            let base1 = tvl / Decimal::from(2) / px1;
            let (amp, w0, w1) = match d.kind {
                PoolKind::StableSwap => (dec("120"), dec("0.5"), dec("0.5")),
                PoolKind::WeightedPool => (Decimal::ZERO, dec("0.5"), dec("0.5")),
                _ => (Decimal::ZERO, dec("0.5"), dec("0.5")),
            };
            seeds.push(PoolSeed {
                pool: PoolRef {
                    pool_id: d.pool_id.into(),
                    chain_id: chain.chain_id,
                    protocol_id: d.protocol.into(),
                    kind: d.kind,
                    address: d.address.into(),
                    token0: d.t0.into(),
                    token1: d.t1.into(),
                    fee_bps: d.fee_bps,
                    tick_spacing: None,
                    tvl_usd: tvl,
                    verified: true,
                    execution_allowed: true,
                    added_at: Utc::now() - chrono::Duration::days(30),
                },
                base0,
                base1,
                factor: 1.0,
                amplification: amp,
                weight0: w0,
                weight1: w1,
            });
        }

        // ---- CEX 合约与盘口 ----
        let mut instruments = HashMap::new();
        let mut books = BookEngine::new();
        let mut cex_factors = HashMap::new();
        for sym in &cfg.cex.symbols {
            let (base, quote) = split_symbol(sym);
            let inst = Instrument {
                exchange: cfg.cex.exchange.clone(),
                symbol: sym.clone(),
                base: base.clone(),
                quote: quote.clone(),
                status: "TRADING".into(),
                min_qty: dec("0.0001"),
                qty_step: dec("0.0001"),
                min_notional: dec("5"),
                price_tick: dec("0.01"),
                maker_bps: Decimal::from_f64(cfg.cex.maker_bps).unwrap_or(dec("8")),
                taker_bps: Decimal::from_f64(cfg.cex.taker_bps).unwrap_or(dec("10")),
                fee_known: cfg.cex.fee_confirmed,
                updated_at: Utc::now(),
            };
            let mid = base_prices.get(&base).copied().unwrap_or(Decimal::ONE)
                / base_prices.get(&quote).copied().unwrap_or(Decimal::ONE);
            cex_factors.insert(sym.clone(), factor_noise(&mut rng, 6.0));
            books.upsert_snapshot(make_book(
                &cfg.cex.exchange,
                sym,
                mid,
                cfg.cex.book_levels,
                &mut rng,
            ));
            instruments.insert(sym.clone(), inst);
        }

        let mut market = Self {
            chain,
            head_block: 24_310_500,
            tokens,
            pools: PoolSet::new(chain_id_of(cfg)),
            instruments,
            books,
            prices: base_prices.clone(),
            seeds,
            cex_factors,
            base_prices,
            rng,
            tick_count: 0,
        };
        market.rebuild();
        market
    }

    /// 每次 tick：随机游走 + 均值回归 + 偶发错位。
    pub fn tick(&mut self) {
        self.tick_count += 1;
        self.head_block += 1;

        // DEX 池价格因子
        for s in self.seeds.iter_mut() {
            // 均值回归 15%
            s.factor = 1.0 + (s.factor - 1.0) * 0.85;
            // 常规噪声 ~5bps
            s.factor *= 1.0 + self.rng.range(-0.0005, 0.0005);
            // 偶发错位：约每 20 个 tick 每个池一次，40–150bps
            if self.rng.chance(0.05) {
                let dir = if self.rng.chance(0.5) { 1.0 } else { -1.0 };
                s.factor *= 1.0 + dir * self.rng.range(0.0040, 0.0150);
            }
            // 限制极端偏离，保持市场可信
            s.factor = s.factor.clamp(0.93, 1.07);
        }

        // CEX 价格因子（噪声更小）
        for f in self.cex_factors.values_mut() {
            *f = 1.0 + (*f - 1.0) * 0.8;
            *f *= 1.0 + self.rng.range(-0.0002, 0.0002);
            if self.rng.chance(0.06) {
                let dir = if self.rng.chance(0.5) { 1.0 } else { -1.0 };
                *f *= 1.0 + dir * self.rng.range(0.0030, 0.0120);
            }
            *f = f.clamp(0.93, 1.07);
        }

        self.rebuild();
    }

    /// 用当前因子重建池状态、盘口与价格表。
    fn rebuild(&mut self) {
        let now = Utc::now();

        // ---- 池状态 ----
        let mut pools = Vec::with_capacity(self.seeds.len());
        let mut states = Vec::with_capacity(self.seeds.len());
        for s in &self.seeds {
            let f = Decimal::from_f64(s.factor).unwrap_or(Decimal::ONE);
            let sqrt_f = decimal_sqrt(f);
            let reserve0 = q8(s.base0 / sqrt_f);
            let reserve1 = q8(s.base1 * sqrt_f);
            let px0 = self.base_prices.get(&s.pool.token0).copied().unwrap_or(Decimal::ONE);
            let px1 = self.base_prices.get(&s.pool.token1).copied().unwrap_or(Decimal::ONE);
            let human_price = (reserve1 * px1) / (reserve0 * px0) * px0 / px1;
            // 池状态统一以人类可读单位归一化存储：
            // V3 的 sqrtPriceX96 按 decimals=0 归一化，使 raw 与人类单位一致，
            // 上层策略无需在每一跳做 decimals 换算。真实 RPC 适配器在边界处转换。
            let (sqrt_price_x96, liquidity) = sqrt_price_and_liquidity(
                human_price,
                0,
                0,
                reserve0,
                reserve1,
                s.pool.kind,
            );
            states.push(PoolState {
                pool_id: s.pool.pool_id.clone(),
                block_number: self.head_block,
                block_hash: format!("0x{:064x}", self.head_block),
                updated_at: now,
                reserve0,
                reserve1,
                sqrt_price_x96,
                liquidity,
                tick: price_to_tick(human_price),
                ticks: vec![],
                amplification: s.amplification,
                weight0: s.weight0,
                weight1: s.weight1,
                fee_bps: s.pool.fee_bps,
            });
            pools.push(s.pool.clone());
        }
        for (p, s) in pools.into_iter().zip(states.into_iter()) {
            self.pools.upsert(p, s);
        }

        // ---- CEX 盘口 ----
        let insts: Vec<Instrument> = self.instruments.values().cloned().collect();
        let base_prices = self.base_prices.clone();
        for inst in insts {
            let f = self.cex_factors.get(&inst.symbol).copied().unwrap_or(1.0);
            let base_px = base_prices.get(&inst.base).copied().unwrap_or(Decimal::ONE);
            let quote_px = base_prices.get(&inst.quote).copied().unwrap_or(Decimal::ONE);
            let mid = base_px / quote_px * Decimal::from_f64(f).unwrap_or(Decimal::ONE);
            let levels = 12;
            self.books.upsert_snapshot(make_book(
                &inst.exchange,
                &inst.symbol,
                mid,
                levels,
                &mut self.rng,
            ));
        }

        // ---- 价格表（用于 USD 估值）----
        let weth = self
            .cex_factors
            .get("ETHUSDT")
            .copied()
            .map(|f| {
                let base = self.base_prices.get("WETH").copied().unwrap_or(dec("2793.10"));
                base * Decimal::from_f64(f).unwrap_or(Decimal::ONE)
            })
            .unwrap_or_else(|| self.base_prices.get("WETH").copied().unwrap_or(dec("2793.10")));
        self.prices.insert("WETH".into(), weth);
        self.prices.insert("ETH".into(), weth);
        self.prices.insert(
            "USDC".into(),
            self.base_prices.get("USDC").copied().unwrap_or(Decimal::ONE),
        );
        self.prices.insert(
            "USDT".into(),
            self.base_prices.get("USDT").copied().unwrap_or(Decimal::ONE),
        );
        self.prices.insert(
            "WBTC".into(),
            self.base_prices.get("WBTC").copied().unwrap_or(dec("68640")),
        );
        self.prices.insert(
            "ARB".into(),
            self.base_prices.get("ARB").copied().unwrap_or(dec("0.612")),
        );
    }

    /// 当前可用资产数量（用于库存与风控输入）。
    pub fn price(&self, asset: &str) -> Decimal {
        self.prices.get(asset).copied().unwrap_or(Decimal::ZERO)
    }

    /// 基准价查询（ETH 与 WETH、BTC 与 WBTC 视为同一资产）。
    pub fn base_price_of(&self, asset: &str) -> Decimal {
        self.base_prices
            .get(asset)
            .copied()
            .or_else(|| self.base_prices.get(&format!("W{asset}")).copied())
            .or_else(|| {
                self.base_prices
                    .iter()
                    .find(|(k, _)| k.ends_with(asset) || asset.ends_with(k.as_str()))
                    .map(|(_, v)| *v)
            })
            .unwrap_or(Decimal::ZERO)
    }

    /// 供 UI 展示的协议注册表。
    pub fn protocols(&self) -> Vec<DexProtocol> {
        let mut seen = HashMap::new();
        for s in &self.seeds {
            seen.entry(s.pool.protocol_id.clone())
                .or_insert_with(|| DexProtocol {
                    protocol_id: s.pool.protocol_id.clone(),
                    protocol_version: "v1".into(),
                    kind: s.pool.kind,
                    chain_id: s.pool.chain_id,
                    factory: format!("0x{:040x}", 0xfa_u64),
                    router: format!("0x{:040x}", 0x7a25_0000_u64),
                    adapter: format!("adapter-{}", s.pool.protocol_id),
                    enabled: true,
                    verified: true,
                    audit_note: "本地适配验证通过（Paper）".into(),
                });
        }
        seen.into_values().collect()
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }
}

fn chain_id_of(cfg: &Config) -> u64 {
    cfg.chains
        .iter()
        .find(|c| c.stage == "PAPER_READY")
        .map(|c| c.chain_id)
        .unwrap_or(42161)
}

fn factor_noise(rng: &mut Rng, bps: f64) -> f64 {
    1.0 + rng.range(-bps / 10_000.0, bps / 10_000.0)
}

/// 拆分 `ETHUSDT` → (`ETH`, `USDT`)。
pub fn split_symbol(symbol: &str) -> (String, String) {
    for q in ["USDT", "USDC", "BUSD", "FDUSD", "USD"] {
        if let Some(base) = symbol.strip_suffix(q) {
            if !base.is_empty() {
                return (base.to_string(), q.to_string());
            }
        }
    }
    (symbol.to_string(), "USDT".to_string())
}

/// 构造盘口：围绕中间价按 0.5bps 递进，档位数量保证 ~5 万美元可见深度。
fn make_book(
    exchange: &str,
    symbol: &str,
    mid: Decimal,
    levels: usize,
    rng: &mut Rng,
) -> DomainBook {
    let now = Utc::now();
    let half_spread = mid * Decimal::new(3, 4); // 3bps 半价差
    let step = mid * Decimal::new(5, 5); // 每档 0.5bps
    let mut bids = Vec::with_capacity(levels);
    let mut asks = Vec::with_capacity(levels);
    // 每档名义约 4000–9000 USDT
    for i in 0..levels {
        let offset = step * Decimal::from(i as u64);
        let noise = Decimal::from_f64(rng.range(0.8, 1.2)).unwrap_or(Decimal::ONE);
        let qty = mid * Decimal::from(4_000 + (i as u64) * 900) * noise / mid / mid;
        // qty(base) = notional / price
        let qty_base = Decimal::from(4_000 + (i as u64) * 900) * noise / mid;
        bids.push(BookLevel {
            price: mid - half_spread - offset,
            qty: q8(qty_base.max(Decimal::new(1, 4))),
        });
        asks.push(BookLevel {
            price: mid + half_spread + offset,
            qty: q8(qty_base.max(Decimal::new(1, 4))),
        });
        let _ = qty;
    }
    DomainBook {
        exchange: exchange.to_string(),
        symbol: symbol.to_string(),
        seq: rng.next_u64() % 1_000_000,
        bids,
        asks,
        exchange_ts: now,
        recv_ts: now,
        valid: true,
        invalid_reason: None,
        last_sync_ts: now,
    }
}

/// 由人类可读价格（token1 per token0）计算 V3 sqrtPriceX96 与流动性。
fn sqrt_price_and_liquidity(
    human_price: Decimal,
    dec0: u32,
    dec1: u32,
    reserve0: Decimal,
    reserve1: Decimal,
    kind: PoolKind,
) -> (u128, u128) {
    if !matches!(kind, PoolKind::UniswapV3) {
        return (0, 0);
    }
    let price_f = human_price.to_f64_lossy();
    if !price_f.is_finite() || price_f <= 0.0 {
        return (0, 0);
    }
    let raw = price_f * 10f64.powi(dec1 as i32 - dec0 as i32);
    let sqrt_raw = raw.sqrt();
    let sqrt_x96 = sqrt_raw * 2f64.powi(96);
    if !sqrt_x96.is_finite() || sqrt_x96 <= 0.0 || sqrt_x96 > (u128::MAX as f64) {
        return (0, 0);
    }
    // L = x * sqrtP（x 为 token0 原始单位）
    let x_raw = reserve0.to_f64_lossy() * 10f64.powi(dec0 as i32);
    let l = x_raw * sqrt_raw;
    let _ = reserve1;
    (
        sqrt_x96 as u128,
        l.max(0.0).min(u128::MAX as f64) as u128,
    )
}

/// 价格 → 近似 tick（仅用于展示与日志）。
fn price_to_tick(price: Decimal) -> i32 {
    let p = price.to_f64_lossy();
    if p <= 0.0 {
        return 0;
    }
    (p.ln() / 1.0001f64.ln()).round() as i32
}

fn decimal_sqrt(v: Decimal) -> Decimal {
    let f = v.to_f64_lossy();
    if !f.is_finite() || f <= 0.0 {
        return Decimal::ONE;
    }
    Decimal::from_f64(f.sqrt()).unwrap_or(Decimal::ONE)
}

trait ToF64Lossy {
    fn to_f64_lossy(&self) -> f64;
}

impl ToF64Lossy for Decimal {
    fn to_f64_lossy(&self) -> f64 {
        use rust_decimal::prelude::ToPrimitive;
        self.to_f64().unwrap_or(0.0)
    }
}

/// BookEngine 需要能被插入快照：直接使用其固有方法。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_symbol_handles_quotes() {
        assert_eq!(split_symbol("ETHUSDT"), ("ETH".into(), "USDT".into()));
        assert_eq!(split_symbol("BTCUSDC"), ("BTC".into(), "USDC".into()));
        assert_eq!(split_symbol("WEIRD"), ("WEIRD".into(), "USDT".into()));
    }

    #[test]
    fn demo_market_builds_pools_and_books() {
        let cfg = Config::default();
        let m = DemoMarket::new(&cfg, 7);
        assert!(!m.pools.is_empty());
        assert_eq!(m.instruments.len(), cfg.cex.symbols.len());
        assert!(m.prices.get("WETH").copied().unwrap_or_default() > Decimal::ZERO);
        let book = m
            .books
            .get(&cfg.cex.exchange, &cfg.cex.symbols[0])
            .expect("盘口应存在");
        assert!(book.valid);
        assert!(book.best_bid().is_some() && book.best_ask().is_some());
        assert!(book.best_ask().unwrap() > book.best_bid().unwrap());
    }

    #[test]
    fn ticks_advance_block_and_move_prices() {
        let cfg = Config::default();
        let mut m = DemoMarket::new(&cfg, 11);
        let block0 = m.head_block;
        let px0 = m.prices.get("WETH").copied().unwrap();
        for _ in 0..50 {
            m.tick();
        }
        assert_eq!(m.head_block, block0 + 50);
        assert_ne!(m.prices.get("WETH").copied().unwrap(), px0);
    }
}
