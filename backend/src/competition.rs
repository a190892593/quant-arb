//! 竞争强度分类（PRD v2.2 §10.5，任务 V22-501）。
//!
//! 只按**已有数据**分类，不按地址数量直接推断机器人数量（PRD §10.3）。
//! 数据不足时给 `INSUFFICIENT_DATA`，不估算。

use serde::Serialize;

pub const LOW: &str = "LOW_COMPETITION";
pub const MEDIUM: &str = "MEDIUM_COMPETITION";
pub const HIGH: &str = "HIGH_COMPETITION";
pub const INSUFFICIENT_DATA: &str = "INSUFFICIENT_DATA";
pub const RANK_VOLATILE: &str = "RANK_VOLATILE";

/// 分类输入（来自竞争者采集，PH4）。
#[derive(Clone, Debug, Default)]
pub struct CompetitionInput {
    /// 近 30 天该路线竞争交易数。
    pub tx_count_30d: u64,
    /// 近 30 天活跃竞争地址数。
    pub active_addresses_30d: u64,
    /// 该路线的样本量（用于判断数据是否足够）。
    pub route_samples: u64,
    /// 排名变化幅度（%），用于识别排名剧烈波动。
    pub rank_changed_pct: Option<f64>,
}

/// 分类结果。
#[derive(Clone, Debug, Serialize)]
pub struct CompetitionLevel {
    pub level: String,
    pub reason: String,
    pub tx_count_30d: u64,
    pub active_addresses_30d: u64,
}

/// 被抢概率估计（V22-801）：泊松到达近似 P(至少一个竞争者) = 1 − e^(−λ·t)。
///
/// λ = 竞争者交易数 / 观察窗口区块数，t = 机会暴露区块数。
/// 数据不足（窗口或暴露为 0）返回 0，不臆造。
pub fn capture_probability(competitor_txs: u64, window_blocks: u64, exposure_blocks: u64) -> f64 {
    if window_blocks == 0 || exposure_blocks == 0 || competitor_txs == 0 {
        return 0.0;
    }
    let lambda = competitor_txs as f64 / window_blocks as f64;
    let p = 1.0 - (-(lambda * exposure_blocks as f64)).exp();
    p.clamp(0.0, 1.0)
}

/// 一条竞争者套利候选（V23-401/402）。
#[derive(Clone, Debug, Serialize)]
pub struct DetectedArb {
    pub tx_hash: String,
    pub chain_id: u64,
    pub block: u64,
    pub tx_index: u64,
    pub log_count: usize,
    pub pools: Vec<String>,
    pub net_token0: String,
    pub net_token1: String,
    /// `HIGH` = 全 tx 净额归零（闭环）；`LOW` = 仅多池多跳候选。
    pub confidence: String,
    /// 发起方（Swap 事件 topics[1]）；空表示该协议未提供。
    pub sender: String,
}

const V2_SWAP: &str = "0xd78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822";
const V3_SWAP: &str = "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67";

/// 取 data 的第 `idx` 个 32 字节字（十六进制）。
fn word(data: &str, idx: usize) -> Option<u128> {
    let h = data.trim_start_matches("0x");
    let start = idx * 64;
    if h.len() < start + 64 {
        return None;
    }
    u128::from_str_radix(&h[start..start + 64], 16).ok()
}

/// 从 Swap 日志重建竞争者交易（V23-402）。
///
/// 判定：同一 tx 内 ≥2 条 Swap、涉及 ≥2 个池；全 tx 的 token0/token1 净额归零 → `HIGH`（闭环套利），
/// 否则为 `LOW` 候选。不做地址聚类，也不宣称地址=机器人。
pub fn detect_arbitrage(logs: &[serde_json::Value], chain_id: u64) -> Vec<DetectedArb> {
    use std::collections::{BTreeMap, BTreeSet};

    struct Acc {
        block: u64,
        tx_index: u64,
        log_count: usize,
        pools: BTreeSet<String>,
        net0: i128,
        net1: i128,
        sender: String,
    }

    let mut by_tx: BTreeMap<String, Acc> = BTreeMap::new();
    for l in logs {
        let Some(tx_hash) = l.get("transactionHash").and_then(|v| v.as_str()) else {
            continue;
        };
        let topic0 = l
            .get("topics")
            .and_then(|t| t.get(0))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let data = l.get("data").and_then(|v| v.as_str()).unwrap_or("0x");
        let (n0, n1) = match topic0 {
            V3_SWAP => (
                word(data, 0).map(|v| v as i128).unwrap_or(0),
                word(data, 1).map(|v| v as i128).unwrap_or(0),
            ),
            V2_SWAP => {
                let in0 = word(data, 0).unwrap_or(0) as i128;
                let in1 = word(data, 1).unwrap_or(0) as i128;
                let out0 = word(data, 2).unwrap_or(0) as i128;
                let out1 = word(data, 3).unwrap_or(0) as i128;
                (in0 - out0, in1 - out1)
            }
            _ => continue,
        };
        let block = l
            .get("blockNumber")
            .and_then(|v| v.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let tx_index = l
            .get("transactionIndex")
            .and_then(|v| v.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let pool = l
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let sender = l
            .get("topics")
            .and_then(|t| t.get(1))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let e = by_tx.entry(tx_hash.to_string()).or_insert(Acc {
            block,
            tx_index,
            log_count: 0,
            pools: BTreeSet::new(),
            net0: 0,
            net1: 0,
            sender,
        });
        e.log_count += 1;
        e.net0 += n0;
        e.net1 += n1;
        if !pool.is_empty() {
            e.pools.insert(pool);
        }
    }

    by_tx
        .into_iter()
        .filter_map(|(tx_hash, a)| {
            if a.log_count < 2 || a.pools.len() < 2 {
                return None;
            }
            let confidence = if a.net0 == 0 && a.net1 == 0 {
                "HIGH"
            } else {
                "LOW"
            };
            Some(DetectedArb {
                tx_hash,
                chain_id,
                block: a.block,
                tx_index: a.tx_index,
                log_count: a.log_count,
                pools: a.pools.into_iter().collect(),
                net_token0: a.net0.to_string(),
                net_token1: a.net1.to_string(),
                confidence: confidence.into(),
                sender: a.sender,
            })
        })
        .collect()
}

/// 按 PRD §10.5 分类。
pub fn classify(input: &CompetitionInput) -> CompetitionLevel {
    if input.route_samples < 20 {
        return CompetitionLevel {
            level: INSUFFICIENT_DATA.into(),
            reason: format!(
                "样本量 {} < 20：数据不足以判断竞争强度",
                input.route_samples
            ),
            tx_count_30d: input.tx_count_30d,
            active_addresses_30d: input.active_addresses_30d,
        };
    }
    if let Some(pct) = input.rank_changed_pct {
        if pct.abs() > 50.0 {
            return CompetitionLevel {
                level: RANK_VOLATILE.into(),
                reason: format!("排名变化 {pct:.1}% > 50%：排名不稳定，不能当稳定竞争等级"),
                tx_count_30d: input.tx_count_30d,
                active_addresses_30d: input.active_addresses_30d,
            };
        }
    }
    let (level, reason) = if input.tx_count_30d >= 500 || input.active_addresses_30d >= 50 {
        (
            HIGH,
            format!(
                "30 天 {} 笔 / {} 个活跃地址：竞争激烈",
                input.tx_count_30d, input.active_addresses_30d
            ),
        )
    } else if input.tx_count_30d <= 50 && input.active_addresses_30d <= 5 {
        (
            LOW,
            format!(
                "30 天 {} 笔 / {} 个活跃地址：竞争相对少",
                input.tx_count_30d, input.active_addresses_30d
            ),
        )
    } else {
        (
            MEDIUM,
            format!(
                "30 天 {} 笔 / {} 个活跃地址：中等竞争",
                input.tx_count_30d, input.active_addresses_30d
            ),
        )
    };
    CompetitionLevel {
        level: level.into(),
        reason,
        tx_count_30d: input.tx_count_30d,
        active_addresses_30d: input.active_addresses_30d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_samples_is_insufficient_data() {
        let c = classify(&CompetitionInput {
            route_samples: 5,
            ..Default::default()
        });
        assert_eq!(c.level, INSUFFICIENT_DATA);
    }

    #[test]
    fn low_competition_needs_few_txs_and_addresses() {
        let c = classify(&CompetitionInput {
            route_samples: 100,
            tx_count_30d: 10,
            active_addresses_30d: 2,
            ..Default::default()
        });
        assert_eq!(c.level, LOW);
    }

    #[test]
    fn high_competition_by_tx_count() {
        let c = classify(&CompetitionInput {
            route_samples: 100,
            tx_count_30d: 800,
            active_addresses_30d: 10,
            ..Default::default()
        });
        assert_eq!(c.level, HIGH);
    }

    #[test]
    fn volatile_rank_overrides_other_levels() {
        let c = classify(&CompetitionInput {
            route_samples: 100,
            tx_count_30d: 10,
            active_addresses_30d: 2,
            rank_changed_pct: Some(80.0),
        });
        assert_eq!(c.level, RANK_VOLATILE);
    }

    fn v3_log(tx: &str, pool: &str, a0: i128, a1: i128) -> serde_json::Value {
        let w0 = format!("{:064x}", a0 as u128);
        let w1 = format!("{:064x}", a1 as u128);
        serde_json::json!({
            "transactionHash": tx,
            "blockNumber": "0x10",
            "transactionIndex": "0x1",
            "address": pool,
            "topics": [V3_SWAP, "0x0", "0x0"],
            "data": format!("0x{w0}{w1}"),
        })
    }

    #[test]
    fn capture_probability_is_bounded_and_monotonic() {
        assert_eq!(capture_probability(0, 2000, 1), 0.0);
        assert_eq!(capture_probability(10, 0, 1), 0.0);
        let low = capture_probability(10, 2000, 1);
        let high = capture_probability(1000, 2000, 1);
        assert!(low > 0.0 && low < 0.1);
        assert!(high > low && high <= 1.0);
    }

    #[test]
    fn detects_closed_arbitrage_transaction() {
        let logs = vec![
            v3_log("0xaa", "0xpool1", -1000, 2000),
            v3_log("0xaa", "0xpool2", 1000, -2000),
        ];
        let d = detect_arbitrage(&logs, 42_161);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].confidence, "HIGH");
        assert_eq!(d[0].log_count, 2);
        assert_eq!(d[0].pools.len(), 2);
    }

    #[test]
    fn single_pool_swap_is_not_arbitrage() {
        let logs = vec![v3_log("0xbb", "0xpool1", -1000, 2000)];
        assert!(detect_arbitrage(&logs, 42_161).is_empty());
    }

    #[test]
    fn multi_pool_without_closure_is_low_confidence() {
        let logs = vec![
            v3_log("0xcc", "0xpool1", -1000, 2000),
            v3_log("0xcc", "0xpool2", 500, -900),
        ];
        let d = detect_arbitrage(&logs, 42_161);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].confidence, "LOW");
    }
}
