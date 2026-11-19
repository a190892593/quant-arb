//! 池运行时：把 [`PoolRef`]（注册信息）与 [`PoolState`]（链上快照）绑定，
//! 按池类型分派到 [`crate::dex::math`] 的精确报价，并提供池集合与路径报价。
//!
//! 关键语义：
//! - **方向**：`token_in == pool.token0` 视为 `zero_for_one`，否则视为 `one_for_zero`；
//!   两种方向的报价与即期价必须不同（测试覆盖）。
//! - **单位**：储备量与 V3 的 sqrtPrice 均按池内**原始单位（raw units）**计算；
//!   需要人类可读价格时调用方需结合 token decimals（V3 见
//!   [`crate::dex::math::sqrt_price_x96_to_price`]）。
//! - **失败即错误**：任一跳报价失败返回 [`QuoteError`]，不允许悄悄跳过；
//!   但“金额过小导致输出为 0”属于可成交性判定，返回 `Ok` 且 `amount_out = 0`，
//!   由上层策略引擎决定是否拒绝。
//! - **费率来源**：优先 `state.fee_bps`（快照即事实），为 0 时回退到
//!   `pool.fee_bps`（注册表费率档位）。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::market::{PoolKind, PoolRef, PoolState};
use crate::domain::money::bps_of;

use super::graph;
use super::math;

/// 单池 exact-in 报价结果。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwapQuote {
    pub pool_id: String,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub fee_amount: Decimal,
    /// 即期价（token_out per token_in），不含手续费与冲击。
    pub spot_price: Decimal,
    /// 实际成交均价（token_out per token_in）。
    pub exec_price: Decimal,
    pub price_impact_bps: Decimal,
    pub ticks_crossed: usize,
}

/// 报价错误。任何一项都必须让上层拒绝该路线，而不是按 0 成交。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuoteError {
    /// 池 id 不在 [`PoolSet`] 中。
    UnknownPool(String),
    /// 池不支持该 token，或路径边与池方向不一致。
    UnknownToken(String),
    /// 池（或对应方向）流动性为零。
    ZeroLiquidity,
    /// 输入金额 <= 0。
    AmountTooSmall,
    /// 中间计算溢出，或池参数（如权重）非法。
    ArithmeticOverflow,
}

impl std::fmt::Display for QuoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuoteError::UnknownPool(id) => write!(f, "未知流动性池：{id}"),
            QuoteError::UnknownToken(token) => {
                write!(f, "该池不支持此代币方向：{token}")
            }
            QuoteError::ZeroLiquidity => write!(f, "池流动性为零，无法报价"),
            QuoteError::AmountTooSmall => write!(f, "报价金额无效（必须大于 0）"),
            QuoteError::ArithmeticOverflow => write!(f, "报价计算溢出或池参数非法"),
        }
    }
}

impl std::error::Error for QuoteError {}

/// 池运行时：注册信息 + 最新状态快照。
#[derive(Clone, Debug)]
pub struct PoolRuntime {
    pub pool: PoolRef,
    pub state: PoolState,
}

impl PoolRuntime {
    /// 该池的另一侧 token；不支持 `token_in` 时返回 `None`。
    pub fn token_out(&self, token_in: &str) -> Option<&str> {
        if token_in == self.pool.token0 {
            Some(&self.pool.token1)
        } else if token_in == self.pool.token1 {
            Some(&self.pool.token0)
        } else {
            None
        }
    }

    /// 实际生效的费率（bps）：状态快照优先，缺失时回退到注册表费率档位。
    pub fn fee_bps(&self) -> i64 {
        if self.state.fee_bps > 0 {
            self.state.fee_bps
        } else {
            self.pool.fee_bps
        }
    }

    /// 按池类型分派的 exact-in 报价。
    ///
    /// - `amount_in <= 0` → [`QuoteError::AmountTooSmall`]
    /// - `token_in` 不属于该池 → [`QuoteError::UnknownToken`]
    /// - 流动性为零 → [`QuoteError::ZeroLiquidity`]
    /// - `PoolKind::Unknown` → [`QuoteError::UnknownPool`]（未验证协议不得报价）
    pub fn quote_exact_in(
        &self,
        token_in: &str,
        amount_in: Decimal,
    ) -> Result<SwapQuote, QuoteError> {
        if amount_in <= Decimal::ZERO {
            return Err(QuoteError::AmountTooSmall);
        }
        let token_out = self
            .token_out(token_in)
            .ok_or_else(|| QuoteError::UnknownToken(token_in.to_string()))?
            .to_string();
        let zero_for_one = token_in == self.pool.token0;
        let fee_bps = self.fee_bps();

        let (reserve_in, reserve_out) = if zero_for_one {
            (self.state.reserve0, self.state.reserve1)
        } else {
            (self.state.reserve1, self.state.reserve0)
        };

        let (amount_out, spot, ticks_crossed) = match self.pool.kind {
            PoolKind::UniswapV2 => {
                if reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
                    return Err(QuoteError::ZeroLiquidity);
                }
                let out = math::v2_amount_out(amount_in, reserve_in, reserve_out, fee_bps);
                let spot = math::v2_spot_price(reserve_in, reserve_out);
                (out, spot, 0usize)
            }
            PoolKind::UniswapV3 => {
                if self.state.liquidity == 0 || self.state.sqrt_price_x96 == 0 {
                    return Err(QuoteError::ZeroLiquidity);
                }
                // math::v3_quote_with_ticks 从 state.fee_bps 读取费率；快照缺失时
                // 用注册表费率补齐，避免把 0 费率带入报价。
                let patched;
                let v3_state: &PoolState = if self.state.fee_bps <= 0 && self.pool.fee_bps > 0 {
                    patched = {
                        let mut s = self.state.clone();
                        s.fee_bps = self.pool.fee_bps;
                        s
                    };
                    &patched
                } else {
                    &self.state
                };
                let (out, crossed) = math::v3_quote_with_ticks(v3_state, zero_for_one, amount_in);
                (out, self.v3_spot_price(zero_for_one), crossed)
            }
            PoolKind::StableSwap => {
                if reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
                    return Err(QuoteError::ZeroLiquidity);
                }
                let out = math::stableswap_amount_out(
                    amount_in,
                    reserve_in,
                    reserve_out,
                    self.state.amplification,
                    fee_bps,
                );
                let spot = math::v2_spot_price(reserve_in, reserve_out);
                (out, spot, 0usize)
            }
            PoolKind::WeightedPool => {
                if reserve_in <= Decimal::ZERO || reserve_out <= Decimal::ZERO {
                    return Err(QuoteError::ZeroLiquidity);
                }
                let (w_in, w_out) = if zero_for_one {
                    (self.state.weight0, self.state.weight1)
                } else {
                    (self.state.weight1, self.state.weight0)
                };
                if w_in <= Decimal::ZERO || w_out <= Decimal::ZERO {
                    return Err(QuoteError::ArithmeticOverflow);
                }
                let out = math::weighted_amount_out(
                    amount_in,
                    reserve_in,
                    reserve_out,
                    w_in,
                    w_out,
                    fee_bps,
                );
                let spot = self.weighted_spot_price(reserve_in, reserve_out, w_in, w_out);
                (out, spot, 0usize)
            }
            PoolKind::Unknown => {
                return Err(QuoteError::UnknownPool(self.pool.pool_id.clone()));
            }
        };

        let amount_out = if amount_out <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            amount_out.round_dp(18)
        };
        let fee_amount = bps_of(amount_in, fee_bps).round_dp(18);
        let exec_price = if amount_in <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            (amount_out / amount_in).round_dp(18)
        };
        let spot = if spot <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            spot.round_dp(18)
        };
        let price_impact = math::price_impact_bps(spot, exec_price);

        Ok(SwapQuote {
            pool_id: self.pool.pool_id.clone(),
            token_in: token_in.to_string(),
            token_out,
            amount_in,
            amount_out,
            fee_amount,
            spot_price: spot,
            exec_price,
            price_impact_bps: price_impact,
            ticks_crossed,
        })
    }

    /// 即期价（token_out per token_in，raw units），不含手续费与冲击。
    ///
    /// `PoolKind::Unknown` 或未知 token 返回 `None`；储备为零时返回 `Some(0)`。
    pub fn spot_price(&self, token_in: &str) -> Option<Decimal> {
        let _ = self.token_out(token_in)?;
        let zero_for_one = token_in == self.pool.token0;
        let (reserve_in, reserve_out) = if zero_for_one {
            (self.state.reserve0, self.state.reserve1)
        } else {
            (self.state.reserve1, self.state.reserve0)
        };
        let price = match self.pool.kind {
            PoolKind::UniswapV2 | PoolKind::StableSwap => {
                math::v2_spot_price(reserve_in, reserve_out)
            }
            PoolKind::UniswapV3 => self.v3_spot_price(zero_for_one),
            PoolKind::WeightedPool => {
                let (w_in, w_out) = if zero_for_one {
                    (self.state.weight0, self.state.weight1)
                } else {
                    (self.state.weight1, self.state.weight0)
                };
                if w_in <= Decimal::ZERO || w_out <= Decimal::ZERO {
                    Decimal::ZERO
                } else {
                    self.weighted_spot_price(reserve_in, reserve_out, w_in, w_out)
                }
            }
            PoolKind::Unknown => return None,
        };
        Some(price.round_dp(18))
    }

    /// 预估 Gas（units）：V2 约 90k/跳，V3 约 130k/跳，StableSwap 约 150k，
    /// Weighted 约 180k。未验证池返回 0（不应进入执行）。
    pub fn gas_units(&self) -> u64 {
        match self.pool.kind {
            PoolKind::UniswapV2 => 90_000,
            PoolKind::UniswapV3 => 130_000,
            PoolKind::StableSwap => 150_000,
            PoolKind::WeightedPool => 180_000,
            PoolKind::Unknown => 0,
        }
    }

    /// V3 原始单位即期价（token_out per token_in）。
    fn v3_spot_price(&self, zero_for_one: bool) -> Decimal {
        // dec0 = dec1 = 0：保持池内原始单位，与储备量口径一致。
        let price = math::sqrt_price_x96_to_price(self.state.sqrt_price_x96, 0, 0);
        if zero_for_one {
            price
        } else if price <= Decimal::ZERO {
            Decimal::ZERO
        } else {
            (Decimal::ONE / price).round_dp(18)
        }
    }

    fn weighted_spot_price(
        &self,
        reserve_in: Decimal,
        reserve_out: Decimal,
        w_in: Decimal,
        w_out: Decimal,
    ) -> Decimal {
        if reserve_in <= Decimal::ZERO || w_in <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        (reserve_out / reserve_in * (w_in / w_out)).round_dp(18)
    }
}

/// 同链池集合：按 `pool_id` 与 token 双向索引，供路由搜索使用。
#[derive(Clone, Debug)]
pub struct PoolSet {
    chain_id: u64,
    pools: Vec<PoolRuntime>,
    index: HashMap<String, usize>,
    token_index: HashMap<String, Vec<usize>>,
}

impl PoolSet {
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            pools: Vec::new(),
            index: HashMap::new(),
            token_index: HashMap::new(),
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// 插入或替换池状态。已存在的 `pool_id` 保持原索引与插入顺序。
    pub fn upsert(&mut self, pool: PoolRef, state: PoolState) {
        let id = pool.pool_id.clone();
        let tokens = (pool.token0.clone(), pool.token1.clone());
        let runtime = PoolRuntime { pool, state };

        if let Some(&idx) = self.index.get(&id) {
            let prev_tokens = (
                self.pools[idx].pool.token0.clone(),
                self.pools[idx].pool.token1.clone(),
            );
            if prev_tokens != tokens {
                for t in [&prev_tokens.0, &prev_tokens.1] {
                    if let Some(list) = self.token_index.get_mut(t) {
                        list.retain(|x| *x != idx);
                    }
                }
                self.add_token_index(&tokens.0, idx);
                self.add_token_index(&tokens.1, idx);
            }
            self.pools[idx] = runtime;
        } else {
            let idx = self.pools.len();
            self.add_token_index(&tokens.0, idx);
            self.add_token_index(&tokens.1, idx);
            self.index.insert(id, idx);
            self.pools.push(runtime);
        }
    }

    fn add_token_index(&mut self, token: &str, idx: usize) {
        let list = self.token_index.entry(token.to_string()).or_default();
        if !list.contains(&idx) {
            list.push(idx);
        }
    }

    pub fn get(&self, pool_id: &str) -> Option<&PoolRuntime> {
        self.index.get(pool_id).and_then(|i| self.pools.get(*i))
    }

    pub fn get_mut(&mut self, pool_id: &str) -> Option<&mut PoolRuntime> {
        match self.index.get(pool_id) {
            Some(i) => self.pools.get_mut(*i),
            None => None,
        }
    }

    pub fn all(&self) -> &[PoolRuntime] {
        &self.pools
    }

    pub fn len(&self) -> usize {
        self.pools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }

    /// 包含该 token 的所有池（按插入顺序）。
    pub fn pools_with_token(&self, token: &str) -> Vec<&PoolRuntime> {
        match self.token_index.get(token) {
            Some(list) => list.iter().filter_map(|i| self.pools.get(*i)).collect(),
            None => Vec::new(),
        }
    }

    /// 所有池状态的最大区块号（无池时为 0）。
    pub fn block_number(&self) -> u64 {
        self.pools
            .iter()
            .map(|p| p.state.block_number)
            .max()
            .unwrap_or(0)
    }

    /// 可用于真实执行的池：
    /// `pool.verified && pool.execution_allowed && kind.execution_allowed()
    ///  && state.age_ms(now) <= max_age_ms`。
    pub fn execution_pools(&self, now: DateTime<Utc>, max_age_ms: i64) -> Vec<&PoolRuntime> {
        self.pools
            .iter()
            .filter(|p| {
                p.pool.verified
                    && p.pool.execution_allowed
                    && p.pool.kind.execution_allowed()
                    && p.state.age_ms(now) <= max_age_ms
            })
            .collect()
    }
}

/// 路径中单跳的成交结果。
#[derive(Clone, Debug, PartialEq)]
pub struct PathLegQuote {
    pub pool_id: String,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub fee_bps: i64,
    pub price_impact_bps: Decimal,
}

/// 多跳路径报价。
#[derive(Clone, Debug, PartialEq)]
pub struct PathQuote {
    pub amount_in: Decimal,
    pub amount_out: Decimal,
    pub legs: Vec<PathLegQuote>,
    /// 全路径冲击：用各跳即期价的乘积作为路径即期价，与实际成交均价比较
    /// （因此天然包含手续费拖累）。
    pub price_impact_bps: Decimal,
    pub gas_units: u64,
    /// 路径有效性受**最旧**区块约束（所有腿必须基于同一可复现状态）。
    pub block_number: u64,
}

/// 沿给定边序列逐跳报价；任一跳失败返回 `Err`（不允许跳过失败跳）。
///
/// 空边序列视为恒等路径（`amount_out == amount_in`，gas = 0）。
pub fn quote_path(
    set: &PoolSet,
    start_token: &str,
    amount_in: Decimal,
    edges: &[graph::HopEdge],
) -> Result<PathQuote, QuoteError> {
    if amount_in <= Decimal::ZERO {
        return Err(QuoteError::AmountTooSmall);
    }
    if edges.is_empty() {
        return Ok(PathQuote {
            amount_in,
            amount_out: amount_in,
            legs: Vec::new(),
            price_impact_bps: Decimal::ZERO,
            gas_units: 0,
            block_number: set.block_number(),
        });
    }

    let mut amount = amount_in;
    let mut token = start_token.to_string();
    let mut legs = Vec::with_capacity(edges.len());
    let mut gas_units: u64 = 0;
    let mut block_number = u64::MAX;
    let mut spot_product = Decimal::ONE;

    for edge in edges {
        if edge.token_in != token {
            // 路径边与当前持仓 token 不一致，说明路线本身构造错误。
            return Err(QuoteError::UnknownToken(edge.token_in.clone()));
        }
        let pool = set
            .get(&edge.pool_id)
            .ok_or_else(|| QuoteError::UnknownPool(edge.pool_id.clone()))?;
        let quote = pool.quote_exact_in(&token, amount)?;
        if quote.token_out != edge.token_out {
            // 池的实际方向与边不一致，同样视为无效路线。
            return Err(QuoteError::UnknownToken(edge.token_out.clone()));
        }
        if quote.spot_price > Decimal::ZERO {
            spot_product = spot_product * quote.spot_price;
        } else {
            spot_product = Decimal::ZERO;
        }
        gas_units = gas_units.saturating_add(pool.gas_units());
        block_number = block_number.min(pool.state.block_number);
        legs.push(PathLegQuote {
            pool_id: pool.pool.pool_id.clone(),
            token_in: token.clone(),
            token_out: quote.token_out.clone(),
            amount_in: amount,
            amount_out: quote.amount_out,
            fee_bps: pool.fee_bps(),
            price_impact_bps: quote.price_impact_bps,
        });
        amount = quote.amount_out;
        token = quote.token_out;
    }

    let exec_price = amount / amount_in;
    let price_impact = if spot_product > Decimal::ZERO {
        math::price_impact_bps(spot_product, exec_price)
    } else {
        Decimal::ZERO
    };

    Ok(PathQuote {
        amount_in,
        amount_out: amount,
        legs,
        price_impact_bps: price_impact,
        gas_units,
        block_number,
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;
    use chrono::Duration;

    fn pool_ref(id: &str, kind: PoolKind, t0: &str, t1: &str, fee_bps: i64) -> PoolRef {
        PoolRef {
            pool_id: id.to_string(),
            chain_id: 1,
            protocol_id: "test-proto".to_string(),
            kind,
            address: format!("0x{id}"),
            token0: t0.to_string(),
            token1: t1.to_string(),
            fee_bps,
            tick_spacing: None,
            tvl_usd: dec("1000000"),
            verified: true,
            execution_allowed: true,
            added_at: Utc::now(),
        }
    }

    fn base_state(id: &str, r0: Decimal, r1: Decimal, fee_bps: i64) -> PoolState {
        PoolState {
            pool_id: id.to_string(),
            block_number: 100,
            block_hash: "0xblock".to_string(),
            updated_at: Utc::now(),
            reserve0: r0,
            reserve1: r1,
            sqrt_price_x96: math::Q96,
            liquidity: 0,
            tick: 0,
            ticks: Vec::new(),
            amplification: dec("100"),
            weight0: dec("0.5"),
            weight1: dec("0.5"),
            fee_bps,
        }
    }

    fn v2_pool(id: &str, r0: &str, r1: &str, fee_bps: i64) -> PoolRuntime {
        PoolRuntime {
            pool: pool_ref(id, PoolKind::UniswapV2, "USDC", "WETH", fee_bps),
            state: base_state(id, dec(r0), dec(r1), fee_bps),
        }
    }

    #[test]
    fn token_out_and_direction() {
        let p = v2_pool("p1", "1000000", "2000000", 30);
        assert_eq!(p.token_out("USDC"), Some("WETH"));
        assert_eq!(p.token_out("WETH"), Some("USDC"));
        assert_eq!(p.token_out("DAI"), None);

        let q01 = p.quote_exact_in("USDC", dec("1000")).unwrap();
        let q10 = p.quote_exact_in("WETH", dec("1000")).unwrap();
        assert_eq!(q01.token_in, "USDC");
        assert_eq!(q01.token_out, "WETH");
        assert_eq!(q10.token_out, "USDC");
        // 两个方向价格不同（1e6/2e6 与 2e6/1e6）
        assert_ne!(q01.amount_out, q10.amount_out);
        assert!(q01.amount_out > dec("1990") && q01.amount_out < dec("1993"));
        assert!(q10.amount_out > dec("498") && q10.amount_out < dec("499"));
    }

    #[test]
    fn quote_exact_in_rejects_bad_inputs() {
        let p = v2_pool("p1", "1000000", "2000000", 30);
        assert_eq!(
            p.quote_exact_in("DAI", dec("1000")),
            Err(QuoteError::UnknownToken("DAI".to_string()))
        );
        assert_eq!(
            p.quote_exact_in("USDC", Decimal::ZERO),
            Err(QuoteError::AmountTooSmall)
        );
        assert_eq!(
            p.quote_exact_in("USDC", dec("-1")),
            Err(QuoteError::AmountTooSmall)
        );

        // 零储备 → ZeroLiquidity
        let mut zero = v2_pool("p2", "1000000", "2000000", 30);
        zero.state.reserve1 = Decimal::ZERO;
        assert_eq!(
            zero.quote_exact_in("USDC", dec("1000")),
            Err(QuoteError::ZeroLiquidity)
        );

        // Unknown kind → UnknownPool
        let mut unknown = v2_pool("p3", "1000000", "2000000", 30);
        unknown.pool.kind = PoolKind::Unknown;
        assert_eq!(
            unknown.quote_exact_in("USDC", dec("1000")),
            Err(QuoteError::UnknownPool("p3".to_string()))
        );
        assert_eq!(unknown.spot_price("USDC"), None);
        assert_eq!(unknown.gas_units(), 0);
    }

    #[test]
    fn quote_reports_fee_and_impact() {
        let p = v2_pool("p1", "1000000", "2000000", 30);
        let q = p.quote_exact_in("USDC", dec("1000")).unwrap();
        assert_eq!(q.fee_amount, dec("3.0"));
        // spot = 2.0，exec = out/1000 ≈ 1.992，冲击约 40bps（含 30bps 手续费）
        assert_eq!(q.spot_price, dec("2.0"));
        assert!(q.price_impact_bps > dec("38") && q.price_impact_bps < dec("42"));
        assert_eq!(q.ticks_crossed, 0);
        // 即期价 API 与报价内 spot 一致
        assert_eq!(p.spot_price("USDC"), Some(dec("2.0")));
        assert_eq!(p.spot_price("WETH"), Some(dec("0.5")));
    }

    #[test]
    fn v3_pool_quotes_with_ticks_and_spot() {
        let mut p = v2_pool("v3", "0", "0", 30);
        p.pool.kind = PoolKind::UniswapV3;
        p.pool.token0 = "WETH".to_string();
        p.pool.token1 = "USDC".to_string();
        p.state.amplification = Decimal::ZERO;
        p.state.sqrt_price_x96 = math::Q96;
        p.state.liquidity = 1_000_000_000_000_000_000; // 1e18
        p.state.fee_bps = 30;

        let q = p.quote_exact_in("WETH", dec("1000000000000000")).unwrap();
        assert!(q.amount_out > Decimal::ZERO);
        assert_eq!(q.token_out, "USDC");
        assert_eq!(q.ticks_crossed, 0);
        // 价格 = 1（原始单位），扣费后 exec 略低于 1
        assert_eq!(q.spot_price, Decimal::ONE);
        assert!(q.exec_price < Decimal::ONE && q.exec_price > dec("0.99"));
        assert_eq!(p.gas_units(), 130_000);

        // 快照费率缺失时，必须回退到注册表费率（否则会算成零费率）
        let mut no_fee_state = p.clone();
        no_fee_state.state.fee_bps = 0;
        let q_fallback = no_fee_state
            .quote_exact_in("WETH", dec("1000000000000000"))
            .unwrap();
        assert_eq!(
            q_fallback.amount_out, q.amount_out,
            "快照费率缺失时必须回退到注册表费率"
        );

        // 注册表费率也为 0 时才是真正零费率，输出必须更多
        let mut truly_free = p.clone();
        truly_free.state.fee_bps = 0;
        truly_free.pool.fee_bps = 0;
        let q_free = truly_free
            .quote_exact_in("WETH", dec("1000000000000000"))
            .unwrap();
        assert!(
            q_free.amount_out > q.amount_out,
            "零费率必须给出更多输出: {} vs {}",
            q_free.amount_out,
            q.amount_out
        );

        // 零流动性 → Err
        let mut empty = p.clone();
        empty.state.liquidity = 0;
        assert_eq!(
            empty.quote_exact_in("WETH", dec("1")),
            Err(QuoteError::ZeroLiquidity)
        );
    }

    #[test]
    fn stableswap_and_weighted_pool_quotes() {
        let mut ss = v2_pool("ss", "1000000", "1000000", 4);
        ss.pool.kind = PoolKind::StableSwap;
        let q = ss.quote_exact_in("USDC", dec("1000")).unwrap();
        assert!(q.amount_out > dec("999.4") && q.amount_out < dec("999.6"));
        assert_eq!(ss.gas_units(), 150_000);

        let mut wp = v2_pool("wp", "1000000", "1000000", 30);
        wp.pool.kind = PoolKind::WeightedPool;
        wp.state.weight0 = dec("0.8");
        wp.state.weight1 = dec("0.2");
        let q = wp.quote_exact_in("USDC", dec("1000")).unwrap();
        assert!(q.amount_out > Decimal::ZERO);
        assert_eq!(wp.gas_units(), 180_000);
        // 权重非法 → ArithmeticOverflow
        let mut bad = wp.clone();
        bad.state.weight1 = Decimal::ZERO;
        assert_eq!(
            bad.quote_exact_in("USDC", dec("1000")),
            Err(QuoteError::ArithmeticOverflow)
        );
    }

    #[test]
    fn pool_set_queries() {
        let mut set = PoolSet::new(1);
        assert!(set.is_empty());
        set.upsert(
            pool_ref("p1", PoolKind::UniswapV2, "USDC", "WETH", 30),
            base_state("p1", dec("1000000"), dec("500"), 30),
        );
        let mut s2 = base_state("p2", dec("1000000"), dec("1000000"), 4);
        s2.block_number = 250;
        set.upsert(pool_ref("p2", PoolKind::StableSwap, "USDC", "USDT", 4), s2);
        assert_eq!(set.chain_id(), 1);
        assert_eq!(set.len(), 2);
        assert!(!set.is_empty());
        assert_eq!(set.block_number(), 250);
        assert!(set.get("p1").is_some());
        assert!(set.get("nope").is_none());
        assert_eq!(set.pools_with_token("USDC").len(), 2);
        assert_eq!(set.pools_with_token("USDT").len(), 1);
        assert_eq!(set.pools_with_token("DAI").len(), 0);
        assert_eq!(set.all().len(), 2);

        // 同 id 覆盖：保持长度并更新储备
        set.upsert(
            pool_ref("p1", PoolKind::UniswapV2, "USDC", "WETH", 30),
            base_state("p1", dec("2000000"), dec("500"), 30),
        );
        assert_eq!(set.len(), 2);
        assert_eq!(set.get("p1").unwrap().state.reserve0, dec("2000000"));
        // token 集合未变 → 索引仍然正确
        assert_eq!(set.pools_with_token("USDC").len(), 2);

        // get_mut 生效
        set.get_mut("p1").unwrap().state.block_number = 999;
        assert_eq!(set.block_number(), 999);
        assert!(set.get_mut("nope").is_none());
    }

    #[test]
    fn execution_pools_require_all_conditions() {
        let now = Utc::now();
        let mut set = PoolSet::new(1);

        // ok：已验证 + 允许执行 + 状态新鲜
        let mut ok = base_state("ok", dec("1000"), dec("1000"), 30);
        ok.updated_at = now - Duration::milliseconds(500);
        set.upsert(pool_ref("ok", PoolKind::UniswapV2, "USDC", "WETH", 30), ok);

        // 过期
        let mut stale = base_state("stale", dec("1000"), dec("1000"), 30);
        stale.updated_at = now - Duration::milliseconds(10_000);
        set.upsert(
            pool_ref("stale", PoolKind::UniswapV2, "USDC", "DAI", 30),
            stale,
        );

        // 未验证
        let mut unverified_ref = pool_ref("unverified", PoolKind::UniswapV2, "USDC", "USDT", 30);
        unverified_ref.verified = false;
        set.upsert(
            unverified_ref,
            base_state("unverified", dec("1000"), dec("1000"), 30),
        );

        // 禁止执行
        let mut blocked_ref = pool_ref("blocked", PoolKind::UniswapV2, "USDC", "WBTC", 30);
        blocked_ref.execution_allowed = false;
        set.upsert(
            blocked_ref,
            base_state("blocked", dec("1000"), dec("1000"), 30),
        );

        // 未验证协议类型
        set.upsert(
            pool_ref("unknown_kind", PoolKind::Unknown, "USDC", "LINK", 30),
            base_state("unknown_kind", dec("1000"), dec("1000"), 30),
        );

        let allowed = set.execution_pools(now, 3_000);
        assert_eq!(allowed.len(), 1, "只应保留完全合规的池");
        assert_eq!(allowed[0].pool.pool_id, "ok");
        assert_eq!(set.len(), 5);
    }

    #[test]
    fn quote_path_two_hops_decreases_and_counts_fee() {
        let mut set = PoolSet::new(1);
        // USDC → WETH
        set.upsert(
            pool_ref("p1", PoolKind::UniswapV2, "USDC", "WETH", 30),
            base_state("p1", dec("1000000"), dec("1000"), 30),
        );
        // WETH → USDC（不同储备 → 价格不同）
        let mut s2 = base_state("p2", dec("1200000"), dec("1000"), 30);
        s2.block_number = 120;
        set.upsert(pool_ref("p2", PoolKind::UniswapV2, "WETH", "USDC", 30), s2);

        let edges = vec![
            graph::HopEdge {
                pool_id: "p1".to_string(),
                token_in: "USDC".to_string(),
                token_out: "WETH".to_string(),
                kind: PoolKind::UniswapV2,
            },
            graph::HopEdge {
                pool_id: "p2".to_string(),
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                kind: PoolKind::UniswapV2,
            },
        ];

        let pq = quote_path(&set, "USDC", dec("1000"), &edges).unwrap();
        assert_eq!(pq.legs.len(), 2);
        assert_eq!(pq.block_number, 100, "取最旧区块");
        assert_eq!(pq.gas_units, 180_000);
        // 逐跳金额递减（每跳都有手续费）
        assert!(pq.legs[0].amount_out < pq.legs[0].amount_in);
        assert!(pq.legs[1].amount_out < pq.legs[1].amount_in);
        assert_eq!(pq.legs[1].amount_in, pq.legs[0].amount_out);
        assert_eq!(pq.amount_out, pq.legs[1].amount_out);
        assert!(pq.price_impact_bps > Decimal::ZERO);

        // 与零费率池对比：手续费必须被计入
        let mut free = PoolSet::new(1);
        free.upsert(
            pool_ref("p1", PoolKind::UniswapV2, "USDC", "WETH", 0),
            base_state("p1", dec("1000000"), dec("1000"), 0),
        );
        let mut s2f = base_state("p2", dec("1200000"), dec("1000"), 0);
        s2f.block_number = 120;
        free.upsert(pool_ref("p2", PoolKind::UniswapV2, "WETH", "USDC", 0), s2f);
        let free_q = quote_path(&free, "USDC", dec("1000"), &edges).unwrap();
        assert!(
            pq.amount_out < free_q.amount_out,
            "含手续费路径输出必须更少: {} vs {}",
            pq.amount_out,
            free_q.amount_out
        );
    }

    #[test]
    fn quote_path_errors() {
        let mut set = PoolSet::new(1);
        set.upsert(
            pool_ref("p1", PoolKind::UniswapV2, "USDC", "WETH", 30),
            base_state("p1", dec("1000000"), dec("1000"), 30),
        );
        // 未知池
        let bad = vec![graph::HopEdge {
            pool_id: "nope".to_string(),
            token_in: "USDC".to_string(),
            token_out: "WETH".to_string(),
            kind: PoolKind::UniswapV2,
        }];
        assert_eq!(
            quote_path(&set, "USDC", dec("1000"), &bad),
            Err(QuoteError::UnknownPool("nope".to_string()))
        );

        // 路径 token 不连续
        let bad2 = vec![graph::HopEdge {
            pool_id: "p1".to_string(),
            token_in: "DAI".to_string(),
            token_out: "WETH".to_string(),
            kind: PoolKind::UniswapV2,
        }];
        assert_eq!(
            quote_path(&set, "USDC", dec("1000"), &bad2),
            Err(QuoteError::UnknownToken("DAI".to_string()))
        );

        // 方向与池不一致（边说 USDC→WETH，但池的 token0 是 USDC 时该方向其实成立；
        // 这里构造真正不一致的边：先走 p1 到 WETH，却声称输出 USDC）
        let bad3 = vec![graph::HopEdge {
            pool_id: "p1".to_string(),
            token_in: "USDC".to_string(),
            token_out: "USDC".to_string(),
            kind: PoolKind::UniswapV2,
        }];
        assert_eq!(
            quote_path(&set, "USDC", dec("1000"), &bad3),
            Err(QuoteError::UnknownToken("USDC".to_string()))
        );

        // 金额非法
        assert_eq!(
            quote_path(&set, "USDC", Decimal::ZERO, &[]),
            Err(QuoteError::AmountTooSmall)
        );

        // 空路径 → 恒等
        let id = quote_path(&set, "USDC", dec("1000"), &[]).unwrap();
        assert_eq!(id.amount_out, id.amount_in);
        assert_eq!(id.gas_units, 0);
        assert_eq!(id.block_number, 100);
    }
}
