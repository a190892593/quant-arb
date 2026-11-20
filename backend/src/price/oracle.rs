//! 标价（mark price）：链上稳定币池为主源，Chainlink 与 CEX 交叉校验。
//!
//! 设计原则：
//! - 计价货币是稳定币（USDC/USDT/DAI），按 1 美元处理，不需要外部行情；
//! - 其他 Token 的美元价优先用**链上最深计价池的 spot 价**，同链、原子一致；
//! - Chainlink 聚合价用于交叉校验（抗单池瞬时操纵）；
//! - CEX（Binance）只作为第三票与缺失兜底，不再作为主源；
//! - 三源分歧超过阈值时，该 Token 标记为分歧，相关机会降级为观察。

use std::collections::HashMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::dex::PoolSet;

/// 计价货币：按 1 美元处理。
pub const NUMERAIRES: [&str; 3] = ["USDC", "USDT", "DAI"];

/// 稳定币固定价。
pub fn stable_marks() -> HashMap<String, Decimal> {
    NUMERAIRES
        .iter()
        .map(|s| (s.to_string(), Decimal::ONE))
        .collect()
}

/// Chainlink 聚合器（Arbitrum One，8 位小数）。
pub fn chainlink_feeds() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ETH", "0x639Fe6ab55C921f74e7fac1ee960C0B6293ba612"),
        ("BTC", "0x6ce185860a4963106506C203335A2910413708e9"),
        ("LINK", "0x86E53CF1B870786351Da77A57575e79CB55812CB"),
        ("ARB", "0xb2A824043730FE05F3DA2efaFa1CBbe83fa548D6"),
        ("USDC", "0x50834F3163758fcC1Df9973b6e91f0F0F0434aD3"),
    ]
}

/// 解析 `latestRoundData()` 返回：answer + updatedAt（秒）。
pub fn parse_chainlink(data: &[u8]) -> Option<(Decimal, i64)> {
    if data.len() < 64 * 4 {
        return None;
    }
    let word = |i: usize| -> [u8; 32] {
        let mut w = [0u8; 32];
        w.copy_from_slice(&data[i * 32..i * 32 + 32]);
        w
    };
    let ans = u128::from_be_bytes(word(1)[16..32].try_into().ok()?);
    let updated = u128::from_be_bytes(word(3)[16..32].try_into().ok()?);
    let usd = Decimal::from(ans) / Decimal::from(100_000_000u64);
    Some((usd, updated as i64))
}

/// 链上推导：对每个 Token 找「与计价货币配对、TVL 最深」的池，取其 spot 价。
/// 不经过成交，因此没有滑点污染；同时要求池子状态新鲜（由调用方在刷新阶段保证）。
pub fn derive_pool_marks(pools: &PoolSet) -> HashMap<String, Decimal> {
    let mut best: HashMap<String, (Decimal, Decimal)> = HashMap::new(); // token -> (price, tvl)
    for rt in pools.all() {
        let (t0, t1) = (rt.pool.token0.as_str(), rt.pool.token1.as_str());
        let tvl = rt.pool.tvl_usd;
        for (base, quote) in [(t0, t1), (t1, t0)] {
            if !NUMERAIRES.contains(&quote) || NUMERAIRES.contains(&base) {
                continue;
            }
            let Some(px) = rt.spot_price(base) else {
                continue;
            };
            if px <= Decimal::ZERO {
                continue;
            }
            let entry = best.entry(base.to_string()).or_insert((px, tvl));
            if tvl > entry.1 {
                *entry = (px, tvl);
            }
        }
    }
    best.into_iter().map(|(k, (p, _))| (k, p)).collect()
}

/// 三源合并结果。
#[derive(Clone, Debug, Default)]
pub struct Marks {
    pub prices: HashMap<String, Decimal>,
    /// 每个 Token 采用的来源：onchain / chainlink / cex / stable
    pub source: HashMap<String, String>,
    /// 与中位数的最大偏差（bps）
    pub divergence_bps: HashMap<String, Decimal>,
    pub max_divergence_bps: Decimal,
    /// 可选摘要，写入 health 便于前端展示
    pub summary: String,
}

fn median3(a: Option<Decimal>, b: Option<Decimal>, c: Option<Decimal>) -> Option<Decimal> {
    let mut v: Vec<Decimal> = [a, b, c].into_iter().flatten().filter(|x| *x > Decimal::ZERO).collect();
    if v.is_empty() {
        return None;
    }
    v.sort();
    Some(v[v.len() / 2])
}

/// 合并链上 / Chainlink / CEX 三个来源。
pub fn merge(
    onchain: &HashMap<String, Decimal>,
    chainlink: &HashMap<String, Decimal>,
    cex: &HashMap<String, Decimal>,
    diverge_threshold_bps: Decimal,
) -> Marks {
    let mut out = Marks::default();
    let stables = stable_marks();
    out.prices.extend(stables.clone());
    for s in stables.keys() {
        out.source.insert(s.clone(), "stable".into());
    }

    let mut symbols: Vec<String> = onchain
        .keys()
        .chain(chainlink.keys())
        .chain(cex.keys())
        .cloned()
        .collect();
    symbols.sort();
    symbols.dedup();

    for sym in symbols {
        if stables.contains_key(&sym) {
            continue;
        }
        let a = onchain.get(&sym).copied();
        let b = chainlink.get(&sym).copied();
        let c = cex.get(&sym).copied();
        let Some(med) = median3(a, b, c) else {
            continue;
        };

        // 选择最近似中位数且优先级最高（链上 > chainlink > cex）的源
        let mut chosen = ("onchain", a);
        if b.map(|v| (v - med).abs() < (a.unwrap_or(med) - med).abs()).unwrap_or(false) && a.is_none() {
            chosen = ("chainlink", b);
        }
        if a.is_none() && b.is_none() {
            chosen = ("cex", c);
        }

        let mut worst = Decimal::ZERO;
        for v in [a, b, c].into_iter().flatten() {
            let d = ((v - med).abs() / med) * Decimal::from(10_000);
            if d > worst {
                worst = d;
            }
        }
        out.prices.insert(sym.clone(), med);
        out.source.insert(sym.clone(), chosen.0.to_string());
        out.divergence_bps.insert(sym.clone(), worst);
        if worst > out.max_divergence_bps {
            out.max_divergence_bps = worst;
        }
        if worst > diverge_threshold_bps {
            out.summary.push_str(&format!("{sym} 分歧 {:.0}bps · ", worst.to_f64().unwrap_or(0.0)));
        }
    }
    out.summary = out.summary.trim_end_matches(" · ").to_string();
    out
}
