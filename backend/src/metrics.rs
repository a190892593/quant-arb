//! 归一化指标事件（PRD v2.2 §20、任务 V22-101）。
//!
//! 设计原则：
//! - **只记录可观测事实**：拿不到就不记录，不估算、不用 0 填充（PRD §8.3）；
//! - 数值一律 `Decimal` 原文，禁止浮点（与账本口径一致）；
//! - 内存环形窗口服务实时查询，落库服务滚动报告；统计一律 P50/P95/P99，不用平均值；
//! - 每条事件带 `chain_id`，双链可分离统计。

use std::collections::{BTreeMap, VecDeque};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// 类别：耗时。
pub const KIND_LATENCY: &str = "latency";
/// 类别：机会观测。
pub const KIND_OPPORTUNITY: &str = "opportunity";
/// 类别：提交（Paper）。
pub const KIND_SUBMIT: &str = "submit";
/// 类别：失败成本。
pub const KIND_FAILURE_COST: &str = "failure_cost";
/// 类别：数据质量。
pub const KIND_DATA_QUALITY: &str = "data_quality";

/// 一条指标事件。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricEvent {
    pub event_id: String,
    pub kind: String,
    pub chain_id: u64,
    /// 指标名，例如 `scan_ms` / `opportunities` / `pool_update_ms`。
    pub metric: String,
    /// 指标值。延迟用 ms、金额用 USDC、计数用整数，单位见 `unit`。
    pub value: Decimal,
    pub unit: String,
    pub ts_ms: i64,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl MetricEvent {
    pub fn new(kind: &str, chain_id: u64, metric: &str, value: Decimal, unit: &str) -> Self {
        Self {
            event_id: crate::domain::new_id("met"),
            kind: kind.to_string(),
            chain_id,
            metric: metric.to_string(),
            value,
            unit: unit.to_string(),
            ts_ms: crate::domain::now_ms(),
            labels: BTreeMap::new(),
        }
    }

    /// 附加标签（如 `pool_id` / `route_id` / `quality`）。
    pub fn with_label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }

    /// 耗时事件（毫秒）。
    pub fn latency(chain_id: u64, metric: &str, ms: u128) -> Self {
        let v = ms.min(u64::MAX as u128) as u64;
        Self::new(KIND_LATENCY, chain_id, metric, Decimal::from(v), "ms")
    }
}

/// 单指标窗口统计。
#[derive(Clone, Debug, Serialize)]
pub struct MetricSummary {
    pub metric: String,
    pub unit: String,
    pub count: usize,
    pub p50: Option<Decimal>,
    pub p95: Option<Decimal>,
    pub p99: Option<Decimal>,
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
}

/// 最近第 `p` 分位（nearest-rank，`p ∈ [0,1]`）；入参需已升序。空集返回 `None`。
pub fn percentile(sorted: &[Decimal], p: f64) -> Option<Decimal> {
    if sorted.is_empty() {
        return None;
    }
    let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
    let rank = (p * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[idx])
}

/// 内存环形窗口。
pub struct MetricStore {
    ring: VecDeque<MetricEvent>,
    cap: usize,
}

impl Default for MetricStore {
    fn default() -> Self {
        Self::new(20_000)
    }
}

impl MetricStore {
    pub fn new(cap: usize) -> Self {
        Self {
            ring: VecDeque::new(),
            cap: cap.max(64),
        }
    }

    pub fn record(&mut self, event: MetricEvent) {
        self.ring.push_back(event);
        while self.ring.len() > self.cap {
            self.ring.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// 最近 `window_ms` 内的事件（升序）。
    pub fn recent(&self, window_ms: i64, now_ms: i64) -> Vec<&MetricEvent> {
        let cutoff = now_ms.saturating_sub(window_ms.max(0));
        self.ring.iter().filter(|e| e.ts_ms >= cutoff).collect()
    }

    /// 最近 `limit` 条（新→旧）。
    pub fn latest(&self, limit: usize) -> Vec<MetricEvent> {
        self.ring.iter().rev().take(limit).cloned().collect()
    }

    /// 窗口内按 (metric, unit) 汇总。
    pub fn summary(&self, window_ms: i64, now_ms: i64) -> Vec<MetricSummary> {
        let mut by: BTreeMap<(&str, &str), Vec<Decimal>> = BTreeMap::new();
        for e in self.recent(window_ms, now_ms) {
            by.entry((e.metric.as_str(), e.unit.as_str()))
                .or_default()
                .push(e.value);
        }
        by.into_iter()
            .map(|((metric, unit), mut values)| {
                values.sort();
                MetricSummary {
                    metric: metric.to_string(),
                    unit: unit.to_string(),
                    count: values.len(),
                    p50: percentile(&values, 0.50),
                    p95: percentile(&values, 0.95),
                    p99: percentile(&values, 0.99),
                    min: values.first().copied(),
                    max: values.last().copied(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    fn ev(metric: &str, value: &str, ts_ms: i64) -> MetricEvent {
        let mut e = MetricEvent::new(KIND_LATENCY, 42161, metric, dec(value), "ms");
        e.ts_ms = ts_ms;
        e
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let v: Vec<Decimal> = (1..=10).map(Decimal::from).collect();
        assert_eq!(percentile(&v, 0.50), Some(Decimal::from(5)));
        assert_eq!(percentile(&v, 0.95), Some(Decimal::from(10)));
        assert_eq!(percentile(&v, 0.0), Some(Decimal::from(1)));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn summary_reports_distribution_not_average() {
        let mut s = MetricStore::new(64);
        for v in ["1", "2", "3", "4", "100"] {
            s.record(ev("scan_ms", v, 1_000));
        }
        let out = s.summary(10_000, 5_000);
        let row = out.iter().find(|r| r.metric == "scan_ms").expect("scan_ms");
        assert_eq!(row.count, 5);
        assert_eq!(row.p50, Some(dec("3")));
        assert_eq!(row.min, Some(dec("1")));
        assert_eq!(row.max, Some(dec("100")));
    }

    #[test]
    fn window_filters_old_events() {
        let mut s = MetricStore::new(64);
        s.record(ev("scan_ms", "1", 1_000));
        s.record(ev("scan_ms", "2", 9_000));
        let recent = s.recent(5_000, 10_000);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].value, dec("2"));
    }

    #[test]
    fn ring_capacity_is_bounded() {
        let mut s = MetricStore::new(64);
        for i in 0..100 {
            s.record(ev("scan_ms", &i.to_string(), i as i64));
        }
        assert_eq!(s.len(), 64);
        assert_eq!(s.latest(1)[0].value, dec("99"));
    }

    #[test]
    fn decimals_are_not_truncated() {
        let mut s = MetricStore::new(64);
        s.record(ev("net_usd", "0.00000001", 1_000));
        let out = s.summary(10_000, 5_000);
        let row = out.iter().find(|r| r.metric == "net_usd").unwrap();
        assert_eq!(row.p50, Some(dec("0.00000001")));
    }
}
