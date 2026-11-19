//! CEX 订单簿引擎（TDD §5.2）。
//!
//! 维护多交易所 / 多交易对的盘口：安装完整快照、按序号应用增量，并在发现序列号缺口、
//! 乱序、断线或主动失效时立即把盘口标记为 `INVALID`，要求调用方重新拉取 Snapshot。
//!
//! 盘口不变式：`bids` 价格严格降序、`asks` 价格严格升序，同一侧价格唯一。
//! 只有 `valid = true` 且未超过最大年龄的盘口才能用于报价（风控 §6：默认 250 ms）。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::market::{BookHealth, BookLevel, OrderBook, Side};

/// 增量应用结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookUpdateResult {
    /// 增量已按序应用。
    Applied,
    /// 发现缺口 / 乱序 / 盘口已失效：盘口已标记 INVALID，调用方必须重新拉取 Snapshot。
    SnapshotRequired(String),
    /// 未知交易对或增量被拒绝。
    Rejected(String),
}

/// 订单簿维护器：以 `(exchange, symbol)` 为键保存最新盘口。
#[derive(Debug, Default)]
pub struct BookEngine {
    books: HashMap<(String, String), OrderBook>,
}

impl BookEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 安装 / 替换完整快照。
    ///
    /// 新快照序号必须 `>=` 现有序号，否则返回 `false` 且不改动现有盘口。
    /// 安装成功后盘口视为完整有效（`valid = true`、清空失效原因），
    /// `recv_ts` / `last_sync_ts` 取本地当前时间，`exchange_ts` 保留快照自带值。
    pub fn upsert_snapshot(&mut self, mut book: OrderBook) -> bool {
        let key = (book.exchange.clone(), book.symbol.clone());
        if let Some(existing) = self.books.get(&key)
            && book.seq < existing.seq
        {
            return false;
        }
        let now = Utc::now();
        book.bids = normalize_levels(book.bids, true);
        book.asks = normalize_levels(book.asks, false);
        book.valid = true;
        book.invalid_reason = None;
        book.recv_ts = now;
        book.last_sync_ts = now;
        self.books.insert(key, book);
        true
    }

    /// 应用增量。`bids` / `asks` 中 `qty = 0` 表示删除该档位。
    ///
    /// 序号规则：新 `seq` 必须恰好等于当前 `seq + 1`。
    /// - `seq <= 当前 seq`（乱序）或 `seq > 当前 seq + 1`（缺口）都会立即把盘口标记为
    ///   `INVALID` 并返回 [`BookUpdateResult::SnapshotRequired`]；
    /// - 盘口已失效时不再消费增量，同样要求重新拉取快照；
    /// - 未知交易对返回 [`BookUpdateResult::Rejected`]。
    pub fn apply_delta(
        &mut self,
        exchange: &str,
        symbol: &str,
        seq: u64,
        bids: &[BookLevel],
        asks: &[BookLevel],
        exchange_ts: DateTime<Utc>,
    ) -> BookUpdateResult {
        let key = (exchange.to_string(), symbol.to_string());
        let Some(book) = self.books.get_mut(&key) else {
            return BookUpdateResult::Rejected(format!("未知盘口 {exchange}/{symbol}"));
        };

        if !book.valid {
            let reason = book
                .invalid_reason
                .clone()
                .unwrap_or_else(|| "未提供原因".to_string());
            return BookUpdateResult::SnapshotRequired(format!(
                "{exchange}/{symbol} 盘口已失效，必须重新拉取 Snapshot：{reason}"
            ));
        }

        if seq <= book.seq {
            let msg = format!(
                "{exchange}/{symbol} 增量乱序：seq {seq} <= 当前 seq {}",
                book.seq
            );
            mark_invalid(book, &msg);
            return BookUpdateResult::SnapshotRequired(msg);
        }

        if seq != book.seq + 1 {
            let msg = format!(
                "{exchange}/{symbol} 序列号缺口：期望 seq {}，实际 {seq}",
                book.seq + 1
            );
            mark_invalid(book, &msg);
            return BookUpdateResult::SnapshotRequired(msg);
        }

        merge_levels(&mut book.bids, bids, true);
        merge_levels(&mut book.asks, asks, false);
        book.seq = seq;
        book.exchange_ts = exchange_ts;
        book.recv_ts = Utc::now();
        BookUpdateResult::Applied
    }

    pub fn get(&self, exchange: &str, symbol: &str) -> Option<&OrderBook> {
        self.books.get(&(exchange.to_string(), symbol.to_string()))
    }

    /// 主动标记盘口失效（断线、维护、规则变更等）。未知盘口为空操作。
    pub fn invalidate(&mut self, exchange: &str, symbol: &str, reason: &str) {
        if let Some(book) = self
            .books
            .get_mut(&(exchange.to_string(), symbol.to_string()))
        {
            mark_invalid(book, reason);
        }
    }

    /// 单个盘口健康度。`valid` 同时要求结构有效且年龄不超过 `max_age_ms`。
    pub fn health(
        &self,
        exchange: &str,
        symbol: &str,
        max_age_ms: i64,
        now: DateTime<Utc>,
    ) -> Option<BookHealth> {
        let book = self.get(exchange, symbol)?;
        Some(health_of(book, max_age_ms, now))
    }

    /// 全部盘口健康度，按 `(exchange, symbol)` 排序以保证输出稳定。
    pub fn all_health(&self, max_age_ms: i64, now: DateTime<Utc>) -> Vec<BookHealth> {
        let mut out: Vec<BookHealth> = self
            .books
            .values()
            .map(|b| health_of(b, max_age_ms, now))
            .collect();
        out.sort_by(|a, b| {
            a.exchange
                .cmp(&b.exchange)
                .then_with(|| a.symbol.cmp(&b.symbol))
        });
        out
    }

    /// 清理超过 `max_age_ms` 未更新或已失效的盘口，返回清理数量。
    pub fn evict_stale(&mut self, max_age_ms: i64, now: DateTime<Utc>) -> usize {
        let before = self.books.len();
        self.books
            .retain(|_, book| book.valid && book.age_ms(now) <= max_age_ms);
        before - self.books.len()
    }

    pub fn len(&self) -> usize {
        self.books.len()
    }

    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }
}

fn mark_invalid(book: &mut OrderBook, reason: &str) {
    book.valid = false;
    book.invalid_reason = Some(reason.to_string());
}

/// 计算健康度：结构有效但超龄同样视为不健康（风控 §6：超时拒绝新机会）。
fn health_of(book: &OrderBook, max_age_ms: i64, now: DateTime<Utc>) -> BookHealth {
    let age_ms = book.age_ms(now);
    let fresh = age_ms <= max_age_ms;
    let mut reason = book.invalid_reason.clone();
    if book.valid && !fresh {
        reason = Some(format!("盘口年龄 {age_ms}ms 超过上限 {max_age_ms}ms"));
    }
    BookHealth {
        exchange: book.exchange.clone(),
        symbol: book.symbol.clone(),
        valid: book.valid && fresh,
        seq: book.seq,
        age_ms,
        spread_bps: if book.valid { book.spread_bps() } else { None },
        depth_bid_usd: if book.valid {
            book.depth_usd(Side::Sell)
        } else {
            Decimal::ZERO
        },
        depth_ask_usd: if book.valid {
            book.depth_usd(Side::Buy)
        } else {
            Decimal::ZERO
        },
        synced_at: book.last_sync_ts,
        reason,
    }
}

/// 快照规整：丢弃非正价格/数量、按买卖方向排序、同一价格只保留一档。
fn normalize_levels(mut levels: Vec<BookLevel>, is_bid: bool) -> Vec<BookLevel> {
    levels.retain(|l| l.price > Decimal::ZERO && l.qty > Decimal::ZERO);
    sort_levels(levels.as_mut_slice(), is_bid);
    let mut out: Vec<BookLevel> = Vec::with_capacity(levels.len());
    for level in levels {
        if out.last().is_some_and(|prev| prev.price == level.price) {
            continue;
        }
        out.push(level);
    }
    out
}

/// 应用增量：`qty <= 0` 删除档位，否则覆盖/插入，最后重新排序。
fn merge_levels(levels: &mut Vec<BookLevel>, updates: &[BookLevel], is_bid: bool) {
    for update in updates {
        if update.price <= Decimal::ZERO {
            continue;
        }
        match levels.iter().position(|l| l.price == update.price) {
            Some(idx) if update.qty <= Decimal::ZERO => {
                levels.remove(idx);
            }
            Some(idx) => {
                levels[idx].qty = update.qty;
            }
            None if update.qty > Decimal::ZERO => levels.push(*update),
            None => {}
        }
    }
    sort_levels(levels.as_mut_slice(), is_bid);
}

fn sort_levels(levels: &mut [BookLevel], is_bid: bool) {
    if is_bid {
        // bids 降序
        levels.sort_by_key(|level| std::cmp::Reverse(level.price));
    } else {
        // asks 升序
        levels.sort_by_key(|level| level.price);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;
    use chrono::Duration;

    fn level(price: &str, qty: &str) -> BookLevel {
        BookLevel {
            price: dec(price),
            qty: dec(qty),
        }
    }

    fn snapshot(seq: u64, bids: Vec<BookLevel>, asks: Vec<BookLevel>) -> OrderBook {
        let now = Utc::now();
        OrderBook {
            exchange: "binance".to_string(),
            symbol: "TESTUSDT".to_string(),
            seq,
            bids,
            asks,
            exchange_ts: now,
            recv_ts: now,
            valid: true,
            invalid_reason: None,
            last_sync_ts: now,
        }
    }

    fn base_snapshot() -> OrderBook {
        snapshot(
            10,
            vec![level("100", "1"), level("99", "2")],
            vec![level("101", "1"), level("102", "1")],
        )
    }

    #[test]
    fn snapshot_install_replace_and_seq_guard() {
        let mut engine = BookEngine::new();
        assert!(engine.upsert_snapshot(base_snapshot()));
        assert_eq!(engine.len(), 1);
        let book = engine.get("binance", "TESTUSDT").unwrap();
        assert!(book.valid);
        assert_eq!(book.seq, 10);

        // 更小的序号不允许覆盖
        assert!(!engine.upsert_snapshot(snapshot(9, vec![], vec![])));
        assert_eq!(engine.get("binance", "TESTUSDT").unwrap().seq, 10);

        // 相同序号允许覆盖（重放同一快照）
        assert!(engine.upsert_snapshot(snapshot(10, vec![level("100", "5")], vec![])));
        let book = engine.get("binance", "TESTUSDT").unwrap();
        assert_eq!(book.seq, 10);
        assert_eq!(book.bids[0].qty, dec("5"));
    }

    #[test]
    fn snapshot_normalizes_sorting_and_dedupes() {
        let mut engine = BookEngine::new();
        let mut book = base_snapshot();
        // 乱序 + 重复价格 + 0 数量
        book.bids = vec![
            level("99", "2"),
            level("100", "1"),
            level("101", "3"),
            level("100", "7"),
            level("98", "0"),
        ];
        book.asks = vec![level("103", "1"), level("101", "2"), level("102", "3")];
        assert!(engine.upsert_snapshot(book));

        let book = engine.get("binance", "TESTUSDT").unwrap();
        let bid_prices: Vec<Decimal> = book.bids.iter().map(|l| l.price).collect();
        assert_eq!(
            bid_prices,
            vec![dec("101"), dec("100"), dec("99")],
            "bids 必须降序且价格唯一"
        );
        let ask_prices: Vec<Decimal> = book.asks.iter().map(|l| l.price).collect();
        assert_eq!(
            ask_prices,
            vec![dec("101"), dec("102"), dec("103")],
            "asks 必须升序且价格唯一"
        );
    }

    #[test]
    fn delta_applies_in_order_and_keeps_sorting() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());

        let exchange_ts = Utc::now();
        let result = engine.apply_delta(
            "binance",
            "TESTUSDT",
            11,
            // 删除 99，新增 98
            &[level("99", "0"), level("98", "5")],
            // 覆盖 101 数量，新增 100.5
            &[level("101", "3"), level("100.5", "1")],
            exchange_ts,
        );
        assert_eq!(result, BookUpdateResult::Applied);

        let book = engine.get("binance", "TESTUSDT").unwrap();
        assert_eq!(book.seq, 11);
        assert_eq!(book.exchange_ts, exchange_ts);
        assert!(book.valid);

        let bids: Vec<(Decimal, Decimal)> = book.bids.iter().map(|l| (l.price, l.qty)).collect();
        assert_eq!(bids, vec![(dec("100"), dec("1")), (dec("98"), dec("5"))]);
        let asks: Vec<(Decimal, Decimal)> = book.asks.iter().map(|l| (l.price, l.qty)).collect();
        assert_eq!(
            asks,
            vec![
                (dec("100.5"), dec("1")),
                (dec("101"), dec("3")),
                (dec("102"), dec("1"))
            ]
        );
    }

    #[test]
    fn sequence_gap_invalidates_book() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());

        let result = engine.apply_delta(
            "binance",
            "TESTUSDT",
            12,
            &[level("100", "2")],
            &[],
            Utc::now(),
        );
        match result {
            BookUpdateResult::SnapshotRequired(reason) => {
                assert!(reason.contains("缺口"), "reason = {reason}");
            }
            other => panic!("期望 SnapshotRequired，实际 {other:?}"),
        }
        let book = engine.get("binance", "TESTUSDT").unwrap();
        assert!(!book.valid, "缺口后盘口必须标记为 invalid");
        assert!(book.invalid_reason.is_some());

        // 之后仍未重新同步，继续送增量只能继续要求快照
        let result = engine.apply_delta(
            "binance",
            "TESTUSDT",
            13,
            &[level("100", "2")],
            &[],
            Utc::now(),
        );
        assert!(matches!(result, BookUpdateResult::SnapshotRequired(_)));
    }

    #[test]
    fn out_of_order_invalidates_book() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());

        let result = engine.apply_delta(
            "binance",
            "TESTUSDT",
            10,
            &[level("100", "2")],
            &[],
            Utc::now(),
        );
        match result {
            BookUpdateResult::SnapshotRequired(reason) => {
                assert!(reason.contains("乱序"), "reason = {reason}");
            }
            other => panic!("期望 SnapshotRequired，实际 {other:?}"),
        }
        assert!(!engine.get("binance", "TESTUSDT").unwrap().valid);
    }

    #[test]
    fn unknown_book_delta_is_rejected() {
        let mut engine = BookEngine::new();
        let result = engine.apply_delta("binance", "NOPEUSDT", 1, &[], &[], Utc::now());
        assert!(matches!(result, BookUpdateResult::Rejected(_)));
    }

    #[test]
    fn health_reports_age_depth_and_spread() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());

        let now = Utc::now();
        let health = engine.health("binance", "TESTUSDT", 250, now).unwrap();
        assert!(health.valid);
        assert_eq!(health.seq, 10);
        assert_eq!(health.depth_bid_usd, dec("298")); // 100*1 + 99*2
        assert_eq!(health.depth_ask_usd, dec("203")); // 101*1 + 102*1
        assert!(health.spread_bps.is_some());
        assert!(health.reason.is_none());

        // 时间推进 1s：超龄后健康度必须为 false
        let later = now + Duration::milliseconds(1_000);
        let stale = engine.health("binance", "TESTUSDT", 250, later).unwrap();
        assert!(!stale.valid);
        assert!(stale.age_ms >= 1_000);
        assert!(stale.reason.unwrap().contains("超过上限"));

        assert!(engine.health("binance", "NOPEUSDT", 250, now).is_none());
    }

    #[test]
    fn invalidate_and_evict() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());
        let mut other = base_snapshot();
        other.symbol = "OTHERUSDT".to_string();
        other.seq = 1;
        engine.upsert_snapshot(other);
        assert_eq!(engine.len(), 2);

        engine.invalidate("binance", "OTHERUSDT", "websocket 断线");
        let health = engine
            .health("binance", "OTHERUSDT", 250, Utc::now())
            .unwrap();
        assert!(!health.valid);
        assert!(health.reason.unwrap().contains("断线"));

        // 失效盘口立即被清理
        assert_eq!(engine.evict_stale(250, Utc::now()), 1);
        assert_eq!(engine.len(), 1);
        assert!(engine.get("binance", "OTHERUSDT").is_none());

        // 超龄盘口被清理
        let later = Utc::now() + Duration::milliseconds(5_000);
        assert_eq!(engine.evict_stale(250, later), 1);
        assert!(engine.is_empty());
    }

    #[test]
    fn all_health_is_sorted() {
        let mut engine = BookEngine::new();
        engine.upsert_snapshot(base_snapshot());
        let mut other = base_snapshot();
        other.exchange = "okx".to_string();
        other.symbol = "AAAUSDT".to_string();
        other.seq = 1;
        engine.upsert_snapshot(other);

        let healths = engine.all_health(250, Utc::now());
        assert_eq!(healths.len(), 2);
        assert_eq!(healths[0].exchange, "binance");
        assert_eq!(healths[1].exchange, "okx");
    }
}
