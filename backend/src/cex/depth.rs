//! CEX 盘口深度成交模拟（TDD §5.3）。
//!
//! 所有成交估算必须逐档消耗真实盘口深度，禁止用中间价或 K 线价假设整笔成交。
//! 本模块只负责「给定盘口与交易规则，一笔订单会怎样成交」；盘口是否过期由调用方
//! 用 [`crate::domain::market::OrderBook::age_ms`] 或
//! [`crate::cex::book::BookEngine::health`] 判定（风控 §6：默认 250 ms）。
//!
//! 语义约定：
//! - 买单从最低 ask 向上吃，卖单从最高 bid 向下吃；
//! - `limit` 是价格保护：买单只吃 `price <= limit`，卖单只吃 `price >= limit`；
//! - IOC / LIMIT / MARKET 允许部分成交，`remaining_qty > 0` 表示 IOC 剩余被撤销；
//! - FOK 无法全部成交时 `filled_qty = 0`（视为未成交，不产生任何成交与手续费）；
//! - 手续费按 taker 费率以 quote 币计价，保留 8 位小数。

use rust_decimal::Decimal;

use crate::domain::market::{Instrument, OrderBook};
use crate::domain::money::q8;
use crate::domain::types::{OrderType, Side};

/// bps 分母：10_000 = 100%。
const BPS_DENOM: i64 = 10_000;

/// 一次盘口深度成交模拟的结果。
#[derive(Clone, Debug)]
pub struct DepthQuote {
    pub filled_qty: Decimal,
    pub remaining_qty: Decimal,
    /// 成交金额（quote 计价，不含手续费）。
    pub notional: Decimal,
    pub avg_price: Decimal,
    pub worst_price: Decimal,
    /// 以 quote 计价的 taker 手续费。
    pub fee: Decimal,
    pub fee_asset: String,
    pub levels: usize,
    pub filled: bool,
    /// 相对 best price 的冲击（bps，非负）。
    pub impact_bps: Decimal,
    pub valid: bool,
    pub invalid_reason: Option<String>,
}

/// 按盘口档位逐级消耗模拟成交。
///
/// 前置条件：`book` 的档位已按 [`OrderBook`] 不变式排序（由
/// [`crate::cex::book::BookEngine`] 或交易所快照解析保证）。
/// 盘口为空、`valid = false`、数量不满足交易规则时返回 `filled = false` 的零成交结果，
/// 绝不 panic、绝不假设整笔成交。
pub fn simulate_order(
    book: &OrderBook,
    inst: &Instrument,
    side: Side,
    qty: Decimal,
    limit: Option<Decimal>,
    order_type: OrderType,
) -> DepthQuote {
    let fee_asset = inst.quote.clone();

    if !book.valid {
        let reason = book
            .invalid_reason
            .clone()
            .unwrap_or_else(|| "未提供原因".to_string());
        return zero_quote(&fee_asset, false, format!("盘口无效：{reason}"));
    }

    let levels = match side {
        Side::Buy => &book.asks,
        Side::Sell => &book.bids,
    };
    if levels.is_empty() {
        return zero_quote(
            &fee_asset,
            false,
            format!("{} 侧盘口为空", book_side_name(side)),
        );
    }

    let best_price = levels[0].price;
    if best_price <= Decimal::ZERO {
        return zero_quote(&fee_asset, false, "盘口最优价非正数");
    }

    // 交易规则量化：数量按 qty_step 向下取整，不足最小步长时不可成交。
    let requested = inst.quantize_qty(qty);
    if requested <= Decimal::ZERO {
        return zero_quote(
            &fee_asset,
            true,
            format!(
                "数量 {qty} 按 qty_step {} 向下取整后为 0，不可下单",
                inst.qty_step
            ),
        );
    }
    if !inst.min_qty.is_zero() && requested < inst.min_qty {
        return zero_quote(
            &fee_asset,
            true,
            format!("数量 {requested} 小于 min_qty {}", inst.min_qty),
        );
    }

    // 价格保护价按 tick 取整：买价向下、卖价向上，保证不越过保护价。
    let limit = limit.map(|l| quantize_limit(inst, side, l));
    let reference_price = limit.unwrap_or(best_price);
    if !inst.min_notional.is_zero() && requested * reference_price < inst.min_notional {
        return zero_quote(
            &fee_asset,
            true,
            format!(
                "名义价值 {} 低于 min_notional {}",
                q8(requested * reference_price),
                inst.min_notional
            ),
        );
    }

    let mut remaining = requested;
    let mut filled_qty = Decimal::ZERO;
    let mut notional = Decimal::ZERO;
    let mut worst_price = Decimal::ZERO;
    let mut consumed = 0usize;

    for level in levels.iter() {
        if remaining.is_zero() {
            break;
        }
        if level.qty <= Decimal::ZERO || level.price <= Decimal::ZERO {
            continue;
        }
        if let Some(protected) = limit {
            let breached = match side {
                Side::Buy => level.price > protected,
                Side::Sell => level.price < protected,
            };
            if breached {
                break;
            }
        }
        let take = remaining.min(level.qty);
        filled_qty += take;
        notional += take * level.price;
        worst_price = level.price;
        remaining -= take;
        consumed += 1;
    }

    if filled_qty.is_zero() {
        return zero_quote(&fee_asset, true, "价格保护范围内没有可成交深度");
    }

    // FOK：不能全部成交则整笔未成交，不产生成交、手续费与已消耗档位。
    if order_type == OrderType::Fok && !remaining.is_zero() {
        return zero_quote(
            &fee_asset,
            true,
            format!("FOK 无法全部成交：可成交 {filled_qty}，缺口 {remaining}"),
        );
    }

    let notional = q8(notional);
    let avg_price = q8(notional / filled_qty);
    let impact_bps = match side {
        Side::Buy => (avg_price - best_price) / best_price * Decimal::from(BPS_DENOM),
        Side::Sell => (best_price - avg_price) / best_price * Decimal::from(BPS_DENOM),
    }
    .round_dp(4)
    .max(Decimal::ZERO);
    // 手续费 = notional × taker_bps / 10000，以 quote 计价，保留 8 位。
    let fee = q8(notional * inst.taker_bps / Decimal::from(BPS_DENOM));
    let remaining_qty = if order_type == OrderType::Fok {
        Decimal::ZERO
    } else {
        remaining
    };

    DepthQuote {
        filled_qty,
        remaining_qty,
        notional,
        avg_price,
        worst_price,
        fee,
        fee_asset,
        levels: consumed,
        filled: remaining.is_zero(),
        impact_bps,
        valid: true,
        invalid_reason: None,
    }
}

/// 允许价格区间内（`ref_price ± bps`）的可见深度，按 quote 币计价。
///
/// 买单统计 asks、卖单统计 bids；价格区间为闭区间。盘口无效时返回 0。
pub fn depth_within_bps(book: &OrderBook, side: Side, ref_price: Decimal, bps: Decimal) -> Decimal {
    if !book.valid || ref_price <= Decimal::ZERO || bps < Decimal::ZERO {
        return Decimal::ZERO;
    }
    let ratio = bps / Decimal::from(BPS_DENOM);
    let lower = ref_price * (Decimal::ONE - ratio);
    let upper = ref_price * (Decimal::ONE + ratio);
    let levels = match side {
        Side::Buy => &book.asks,
        Side::Sell => &book.bids,
    };
    let mut total = Decimal::ZERO;
    for level in levels.iter() {
        if level.price <= Decimal::ZERO || level.qty <= Decimal::ZERO {
            continue;
        }
        if level.price >= lower && level.price <= upper {
            total += level.price * level.qty;
        }
    }
    q8(total)
}

/// 在不超过盘口可见深度 `usage_pct`% 的前提下，可交易的最大 base 数量。
///
/// `usage_pct = 10` 表示「允许区间内可见深度的 10%」（风控 §6）。深度按档位逐级消耗，
/// 最后一档按剩余额度部分成交；结果按 `inst.qty_step` 向下取整，绝不超过可见深度。
/// 盘口无效、深度为空或 `usage_pct <= 0` 时返回 0。
pub fn max_qty_within_depth_pct(
    book: &OrderBook,
    inst: &Instrument,
    side: Side,
    usage_pct: Decimal,
) -> Decimal {
    if !book.valid || usage_pct <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let levels = match side {
        Side::Buy => &book.asks,
        Side::Sell => &book.bids,
    };
    let depth: Decimal = levels
        .iter()
        .filter(|l| l.price > Decimal::ZERO && l.qty > Decimal::ZERO)
        .map(|l| l.price * l.qty)
        .sum();
    if depth <= Decimal::ZERO {
        return Decimal::ZERO;
    }

    let usage = usage_pct.min(Decimal::from(100)) / Decimal::from(100);
    let allowance = depth * usage;

    let mut spent = Decimal::ZERO;
    let mut qty = Decimal::ZERO;
    for level in levels.iter() {
        if spent >= allowance {
            break;
        }
        if level.price <= Decimal::ZERO || level.qty <= Decimal::ZERO {
            continue;
        }
        let level_notional = level.price * level.qty;
        let available = allowance - spent;
        if available >= level_notional {
            qty += level.qty;
            spent += level_notional;
        } else {
            qty += available / level.price;
            spent = allowance;
        }
    }
    inst.quantize_qty(qty)
}

fn zero_quote(fee_asset: &str, valid: bool, reason: impl Into<String>) -> DepthQuote {
    DepthQuote {
        filled_qty: Decimal::ZERO,
        remaining_qty: Decimal::ZERO,
        notional: Decimal::ZERO,
        avg_price: Decimal::ZERO,
        worst_price: Decimal::ZERO,
        fee: Decimal::ZERO,
        fee_asset: fee_asset.to_string(),
        levels: 0,
        filled: false,
        impact_bps: Decimal::ZERO,
        valid,
        invalid_reason: Some(reason.into()),
    }
}

fn quantize_limit(inst: &Instrument, side: Side, limit: Decimal) -> Decimal {
    let tick = inst.price_tick;
    if tick <= Decimal::ZERO {
        return limit;
    }
    match side {
        // 买单保护价向下取整，不会买得比预期更贵。
        Side::Buy => (limit / tick).floor() * tick,
        // 卖单保护价向上取整，不会卖得比预期更便宜。
        Side::Sell => (limit / tick).ceil() * tick,
    }
}

fn book_side_name(side: Side) -> &'static str {
    match side {
        Side::Buy => "asks",
        Side::Sell => "bids",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::market::BookLevel;
    use crate::domain::money::dec;
    use chrono::Utc;

    fn level(price: &str, qty: &str) -> BookLevel {
        BookLevel {
            price: dec(price),
            qty: dec(qty),
        }
    }

    fn book_with(asks: Vec<BookLevel>, bids: Vec<BookLevel>) -> OrderBook {
        let now = Utc::now();
        OrderBook {
            exchange: "binance".to_string(),
            symbol: "TESTUSDT".to_string(),
            seq: 1,
            bids,
            asks,
            exchange_ts: now,
            recv_ts: now,
            valid: true,
            invalid_reason: None,
            last_sync_ts: now,
        }
    }

    /// 五档盘口：asks 100/101/102/103/104，bids 99/98/97/96/95，数量 1..5。
    fn five_level_book() -> OrderBook {
        book_with(
            vec![
                level("100", "1"),
                level("101", "2"),
                level("102", "3"),
                level("103", "4"),
                level("104", "5"),
            ],
            vec![
                level("99", "1"),
                level("98", "2"),
                level("97", "3"),
                level("96", "4"),
                level("95", "5"),
            ],
        )
    }

    fn inst() -> Instrument {
        Instrument {
            exchange: "binance".to_string(),
            symbol: "TESTUSDT".to_string(),
            base: "TEST".to_string(),
            quote: "USDT".to_string(),
            status: "TRADING".to_string(),
            min_qty: dec("0.001"),
            qty_step: dec("0.001"),
            min_notional: dec("1"),
            price_tick: dec("0.01"),
            maker_bps: Decimal::ZERO,
            taker_bps: dec("10"),
            fee_known: true,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn buy_walks_asks_level_by_level() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("3"),
            None,
            OrderType::Ioc,
        );
        assert!(q.valid && q.filled);
        assert_eq!(q.filled_qty, dec("3"));
        assert_eq!(q.remaining_qty, Decimal::ZERO);
        // 1 * 100 + 2 * 101 = 302
        assert_eq!(q.notional, dec("302"));
        assert_eq!(q.avg_price, dec("100.66666667"));
        assert_eq!(q.worst_price, dec("101"));
        assert_eq!(q.levels, 2);
        assert_eq!(q.impact_bps, dec("66.6667"));
    }

    #[test]
    fn sell_walks_bids_level_by_level() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Sell,
            dec("3"),
            None,
            OrderType::Ioc,
        );
        assert!(q.valid && q.filled);
        assert_eq!(q.filled_qty, dec("3"));
        // 1 * 99 + 2 * 98 = 295
        assert_eq!(q.notional, dec("295"));
        assert_eq!(q.avg_price, dec("98.33333333"));
        assert_eq!(q.worst_price, dec("98"));
        assert_eq!(q.levels, 2);
        assert_eq!(q.impact_bps, dec("67.3401"));
    }

    #[test]
    fn market_order_never_uses_mid_price() {
        // 15 手必须吃穿五档：100+202+306+412+520 = 1540，而不是 15 * 中间价。
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("15"),
            None,
            OrderType::Market,
        );
        assert!(q.filled);
        assert_eq!(q.filled_qty, dec("15"));
        assert_eq!(q.notional, dec("1540"));
        assert_eq!(q.avg_price, dec("102.66666667"));
        assert_eq!(q.worst_price, dec("104"));
        assert_eq!(q.levels, 5);
        assert_eq!(q.impact_bps, dec("266.6667"));

        // 深度不足时 MARKET 也只能部分成交，并给出剩余量
        let partial = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("20"),
            None,
            OrderType::Market,
        );
        assert!(!partial.filled);
        assert_eq!(partial.filled_qty, dec("15"));
        assert_eq!(partial.remaining_qty, dec("5"));
    }

    #[test]
    fn ioc_partial_fill_reports_remaining() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("10"),
            Some(dec("102")),
            OrderType::Ioc,
        );
        // 只吃到 102 为止：1 + 2 + 3 = 6
        assert!(q.valid);
        assert!(!q.filled);
        assert_eq!(q.filled_qty, dec("6"));
        assert_eq!(q.remaining_qty, dec("4"));
        assert_eq!(q.notional, dec("608"));
        assert_eq!(q.avg_price, dec("101.33333333"));
        assert_eq!(q.worst_price, dec("102"));
        assert_eq!(q.levels, 3);
    }

    #[test]
    fn fok_rejects_whole_order_when_depth_insufficient() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("10"),
            Some(dec("102")),
            OrderType::Fok,
        );
        assert!(!q.filled);
        assert_eq!(q.filled_qty, Decimal::ZERO, "FOK 不能部分成交");
        assert_eq!(q.remaining_qty, Decimal::ZERO);
        assert_eq!(q.notional, Decimal::ZERO);
        assert_eq!(q.fee, Decimal::ZERO);
        assert_eq!(q.levels, 0);
        assert!(q.valid);
        assert!(q.invalid_reason.unwrap().contains("FOK"));
    }

    #[test]
    fn fok_fills_when_depth_is_sufficient() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("3"),
            Some(dec("102")),
            OrderType::Fok,
        );
        assert!(q.filled);
        assert_eq!(q.filled_qty, dec("3"));
        assert_eq!(q.remaining_qty, Decimal::ZERO);
        assert_eq!(q.notional, dec("302"));
    }

    #[test]
    fn limit_price_protection_blocks_worse_levels() {
        // 买：保护价 100 只能吃 1 手
        let buy = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("3"),
            Some(dec("100")),
            OrderType::Ioc,
        );
        assert_eq!(buy.filled_qty, dec("1"));
        assert_eq!(buy.remaining_qty, dec("2"));
        assert_eq!(buy.notional, dec("100"));
        assert_eq!(buy.worst_price, dec("100"));
        assert_eq!(buy.levels, 1);

        // 卖：保护价 99 只能吃 99 这一档，98 更差必须停止
        let sell = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Sell,
            dec("2"),
            Some(dec("99")),
            OrderType::Ioc,
        );
        assert_eq!(sell.filled_qty, dec("1"));
        assert_eq!(sell.remaining_qty, dec("1"));
        assert_eq!(sell.notional, dec("99"));
        assert_eq!(sell.worst_price, dec("99"));
        assert_eq!(sell.levels, 1);
    }

    #[test]
    fn limit_is_quantized_conservatively() {
        // 买保护价 100.004 -> 向下取整 100.00，不能吃 101
        let buy = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("3"),
            Some(dec("100.004")),
            OrderType::Ioc,
        );
        assert_eq!(buy.filled_qty, dec("1"));

        // 卖保护价 98.996 -> 向上取整 99.00，仍可成交 99 这一档
        let sell = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Sell,
            dec("2"),
            Some(dec("98.996")),
            OrderType::Ioc,
        );
        assert_eq!(sell.filled_qty, dec("1"));
        assert_eq!(sell.worst_price, dec("99"));
    }

    #[test]
    fn qty_is_rounded_down_to_step() {
        let mut instrument = inst();
        instrument.qty_step = dec("0.01");
        instrument.min_qty = dec("0.01");
        let book = book_with(vec![level("100", "10")], vec![level("99", "10")]);
        let q = simulate_order(
            &book,
            &instrument,
            Side::Buy,
            dec("1.2399"),
            None,
            OrderType::Ioc,
        );
        assert!(q.filled);
        assert_eq!(q.filled_qty, dec("1.23"), "数量必须按 qty_step 向下取整");
        assert_eq!(q.notional, dec("123"));
        assert_eq!(q.avg_price, dec("100"));

        // 小于一个步长的数量不可下单
        let tiny = simulate_order(
            &book,
            &instrument,
            Side::Buy,
            dec("0.004"),
            None,
            OrderType::Ioc,
        );
        assert!(!tiny.filled);
        assert_eq!(tiny.filled_qty, Decimal::ZERO);
        assert!(tiny.invalid_reason.unwrap().contains("qty_step"));
    }

    #[test]
    fn minimum_rules_reject_order() {
        let mut instrument = inst();
        instrument.min_qty = dec("0.5");
        let undq = simulate_order(
            &five_level_book(),
            &instrument,
            Side::Buy,
            dec("0.1"),
            None,
            OrderType::Ioc,
        );
        assert!(!undq.filled);
        assert_eq!(undq.filled_qty, Decimal::ZERO);
        assert!(undq.invalid_reason.unwrap().contains("min_qty"));

        let mut instrument = inst();
        instrument.qty_step = dec("0.001");
        instrument.min_notional = dec("500");
        let q = simulate_order(
            &five_level_book(),
            &instrument,
            Side::Buy,
            dec("1"),
            None,
            OrderType::Ioc,
        );
        assert!(!q.filled);
        assert_eq!(q.filled_qty, Decimal::ZERO);
        assert!(q.invalid_reason.unwrap().contains("min_notional"));
    }

    #[test]
    fn fee_uses_taker_bps_on_notional() {
        let q = simulate_order(
            &five_level_book(),
            &inst(),
            Side::Buy,
            dec("3"),
            None,
            OrderType::Ioc,
        );
        // 302 * 10 / 10000 = 0.302
        assert_eq!(q.fee, dec("0.302"));
        assert_eq!(q.fee_asset, "USDT");

        let mut instrument = inst();
        instrument.taker_bps = dec("2.5");
        let q = simulate_order(
            &five_level_book(),
            &instrument,
            Side::Buy,
            dec("3"),
            None,
            OrderType::Ioc,
        );
        assert_eq!(q.fee, dec("0.0755"));
    }

    #[test]
    fn invalid_book_returns_zero_quote_without_panic() {
        let mut book = five_level_book();
        book.valid = false;
        book.invalid_reason = Some("sequence gap".to_string());
        let q = simulate_order(&book, &inst(), Side::Buy, dec("1"), None, OrderType::Ioc);
        assert!(!q.valid);
        assert!(!q.filled);
        assert_eq!(q.filled_qty, Decimal::ZERO);
        assert_eq!(q.notional, Decimal::ZERO);
        assert_eq!(q.levels, 0);
        assert!(q.invalid_reason.unwrap().contains("sequence gap"));
    }

    #[test]
    fn empty_book_side_is_invalid() {
        let book = book_with(vec![], vec![level("99", "1")]);
        let buy = simulate_order(&book, &inst(), Side::Buy, dec("1"), None, OrderType::Ioc);
        assert!(!buy.valid);
        assert!(!buy.filled);
        assert!(buy.invalid_reason.unwrap().contains("asks"));

        let empty = book_with(vec![], vec![]);
        let sell = simulate_order(&empty, &inst(), Side::Sell, dec("1"), None, OrderType::Ioc);
        assert!(!sell.valid);
    }

    #[test]
    fn depth_within_bps_counts_only_visible_range() {
        let book = book_with(
            vec![level("100", "10"), level("200", "10"), level("300", "10")],
            vec![level("190", "5"), level("200", "5")],
        );

        // 买：ref 200 ±1% -> 只包含 200 档，10 * 200 = 2000
        assert_eq!(
            depth_within_bps(&book, Side::Buy, dec("200"), dec("100")),
            dec("2000")
        );
        // bps = 0 -> 只统计恰好等于 ref 的档位
        assert_eq!(
            depth_within_bps(&book, Side::Buy, dec("200"), Decimal::ZERO),
            dec("2000")
        );
        // ±50% -> 100/200/300 全部包含
        assert_eq!(
            depth_within_bps(&book, Side::Buy, dec("200"), dec("5000")),
            dec("6000")
        );
        // 卖：ref 200 ±1% -> 只包含 200 档，5 * 200 = 1000
        assert_eq!(
            depth_within_bps(&book, Side::Sell, dec("200"), dec("100")),
            dec("1000")
        );
        // 卖 ±50% -> 190 与 200 都包含：950 + 1000 = 1950
        assert_eq!(
            depth_within_bps(&book, Side::Sell, dec("200"), dec("5000")),
            dec("1950")
        );
        // 非法输入
        assert_eq!(
            depth_within_bps(&book, Side::Buy, Decimal::ZERO, dec("100")),
            Decimal::ZERO
        );
        let mut invalid = book.clone();
        invalid.valid = false;
        assert_eq!(
            depth_within_bps(&invalid, Side::Buy, dec("200"), dec("100")),
            Decimal::ZERO
        );
    }

    #[test]
    fn max_qty_within_depth_pct_uses_visible_share() {
        let book = book_with(
            vec![level("100", "10"), level("200", "10")],
            vec![level("100", "10"), level("200", "5")],
        );

        // 深度 1000 + 2000 = 3000；10% = 300 -> 3 手（100 价位）
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Buy, dec("10")),
            dec("3")
        );
        // 50% = 1500 -> 10 手 @100 + 500/200 = 2.5 手
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Buy, dec("50")),
            dec("12.5")
        );
        // 100% -> 全部 20 手
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Buy, dec("100")),
            dec("20")
        );
        // 超过 100% 按 100% 处理
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Buy, dec("250")),
            dec("20")
        );
        // 卖：bid 深度 1000 + 1000 = 2000；10% = 200 -> 2 手
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Sell, dec("10")),
            dec("2")
        );
        // 0% 与非法盘口 -> 0
        assert_eq!(
            max_qty_within_depth_pct(&book, &inst(), Side::Buy, Decimal::ZERO),
            Decimal::ZERO
        );
        let mut invalid = book.clone();
        invalid.valid = false;
        assert_eq!(
            max_qty_within_depth_pct(&invalid, &inst(), Side::Buy, dec("10")),
            Decimal::ZERO
        );
    }

    #[test]
    fn max_qty_is_step_aligned_and_capped() {
        let mut instrument = inst();
        instrument.qty_step = dec("0.1");
        let book = book_with(
            vec![level("100", "1"), level("200", "1")],
            vec![level("100", "1")],
        );
        // 深度 300；50% = 150 -> 1 手 @100 + 0.25 手 @200 = 1.25，按 0.1 向下取整为 1.2
        assert_eq!(
            max_qty_within_depth_pct(&book, &instrument, Side::Buy, dec("50")),
            dec("1.2")
        );
        // 100% 不能超过可见总量
        assert_eq!(
            max_qty_within_depth_pct(&book, &instrument, Side::Buy, dec("100")),
            dec("2")
        );
    }
}
