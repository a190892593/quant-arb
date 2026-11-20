//! Binance 公共行情适配器（TDD §5.1）。
//!
//! 只调用 Binance 现货 **公共只读** 接口：
//! `/api/v3/time`、`/api/v3/exchangeInfo`、`/api/v3/depth`、
//! `/api/v3/ticker/bookTicker`、`/api/v3/ticker/price`。
//! 不涉及 API Key、签名、账户或任何下单接口。
//!
//! 费率说明：账户费率无法通过公共接口获得，因此 [`Instrument`] 的 `maker_bps` /
//! `taker_bps` 一律填 0 且 `fee_known = false`。**禁止假装知道费率**：调用方必须
//! 显式同步费率后才能生成可执行机会（风控 §6「CEX 费率缺失」）。
//!
//! 所有 HTTP 请求超时 10s，错误信息包含 URL 与状态码。

use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::Utc;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::domain::market::{BookLevel, Instrument, OrderBook};

/// 默认 Binance 现货公共接口。
const DEFAULT_BASE: &str = "https://api.binance.com";
/// 交易所标识（与 `OrderBook.exchange` / `Instrument.exchange` 一致）。
const EXCHANGE: &str = "binance";
/// 所有 HTTP 请求超时。
const HTTP_TIMEOUT_SECS: u64 = 5;

/// Binance 公共行情客户端。
#[derive(Clone)]
pub struct BinanceClient {
    client: Client,
    base: String,
}

impl BinanceClient {
    /// 创建客户端。`base` 为空或 `None` 时使用 `https://api.binance.com`。
    pub fn new(base: Option<String>) -> Self {
        let base = base
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        let base = base.trim_end_matches('/').to_string();
        let client = Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .user_agent(concat!("arb-backend/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("构建 Binance HTTP 客户端失败");
        Self { client, base }
    }

    /// 交易所服务器时间（毫秒）。用于本机与交易所时差监控（风控 §6）。
    pub async fn server_time(&self) -> anyhow::Result<i64> {
        let url = self.url("/api/v3/time", None);
        let body = self.get_text(&url).await?;
        let resp: ServerTimeResp = serde_json::from_str(&body)
            .with_context(|| format!("解析 serverTime 响应失败: GET {url}"))?;
        Ok(resp.server_time)
    }

    /// 交易规则：解析 `symbols[].filters` 中的 `PRICE_FILTER.tickSize`、
    /// `LOT_SIZE.stepSize/minQty`、`NOTIONAL|MIN_NOTIONAL.minNotional`。
    pub async fn exchange_info(&self, symbols: &[&str]) -> anyhow::Result<Vec<Instrument>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.url("/api/v3/exchangeInfo", Some(symbols_query(symbols)));
        let body = self.get_text(&url).await?;
        parse_exchange_info(&body).with_context(|| format!("解析 exchangeInfo 响应失败: GET {url}"))
    }

    /// 盘口快照。`seq` 使用 Binance 的 `lastUpdateId`。
    pub async fn depth(&self, symbol: &str, limit: u16) -> anyhow::Result<OrderBook> {
        let query = format!("symbol={}&limit={limit}", percent_encode(symbol));
        let url = self.url("/api/v3/depth", Some(query));
        let body = self.get_text(&url).await?;
        parse_depth(&body, symbol).with_context(|| format!("解析 depth 响应失败: GET {url}"))
    }

    /// 最优买卖价：`(symbol, bid, ask)`。
    pub async fn book_ticker(
        &self,
        symbols: &[&str],
    ) -> anyhow::Result<Vec<(String, Decimal, Decimal)>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.url("/api/v3/ticker/bookTicker", Some(symbols_query(symbols)));
        let body = self.get_text(&url).await?;
        parse_book_tickers(&body).with_context(|| format!("解析 bookTicker 响应失败: GET {url}"))
    }

    /// 最新成交价：`(symbol, price)`。
    pub async fn ticker_prices(&self, symbols: &[&str]) -> anyhow::Result<Vec<(String, Decimal)>> {
        if symbols.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.url("/api/v3/ticker/price", Some(symbols_query(symbols)));
        let body = self.get_text(&url).await?;
        parse_prices(&body).with_context(|| format!("解析 ticker/price 响应失败: GET {url}"))
    }

    fn url(&self, path: &str, query: Option<String>) -> String {
        match query {
            Some(q) if !q.is_empty() => format!("{}{path}?{q}", self.base),
            _ => format!("{}{path}", self.base),
        }
    }

    async fn get_text(&self, url: &str) -> anyhow::Result<String> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("请求 Binance 失败: GET {url}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .with_context(|| format!("读取 Binance 响应失败: GET {url} status={status}"))?;
        if !status.is_success() {
            return Err(anyhow!(
                "Binance 返回非 2xx: GET {url} status={status} body={}",
                truncate(&body, 300)
            ));
        }
        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// 纯解析函数（便于用内联 JSON 做单元测试）
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ServerTimeResp {
    #[serde(rename = "serverTime")]
    server_time: i64,
}

#[derive(Debug, Deserialize)]
struct ExchangeInfoResp {
    #[serde(default)]
    symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Deserialize)]
struct SymbolInfo {
    symbol: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(rename = "baseAsset")]
    base_asset: String,
    #[serde(rename = "quoteAsset")]
    quote_asset: String,
    #[serde(default)]
    filters: Vec<SymbolFilter>,
}

#[derive(Debug, Deserialize)]
struct SymbolFilter {
    #[serde(rename = "filterType")]
    filter_type: String,
    #[serde(rename = "tickSize", default)]
    tick_size: Option<String>,
    #[serde(rename = "stepSize", default)]
    step_size: Option<String>,
    #[serde(rename = "minQty", default)]
    min_qty: Option<String>,
    #[serde(rename = "minNotional", default)]
    min_notional: Option<String>,
}

/// 解析 `exchangeInfo` 响应。费率未知（`fee_known = false`，bps 填 0）。
pub(crate) fn parse_exchange_info(body: &str) -> anyhow::Result<Vec<Instrument>> {
    let resp: ExchangeInfoResp =
        serde_json::from_str(body).context("解析 Binance exchangeInfo JSON 失败")?;
    let now = Utc::now();
    Ok(resp
        .symbols
        .into_iter()
        .map(|s| {
            let mut price_tick = Decimal::ZERO;
            let mut qty_step = Decimal::ZERO;
            let mut min_qty = Decimal::ZERO;
            let mut min_notional = Decimal::ZERO;
            for filter in &s.filters {
                match filter.filter_type.as_str() {
                    "PRICE_FILTER" => {
                        if let Some(v) = filter.tick_size.as_deref().and_then(parse_dec) {
                            price_tick = v;
                        }
                    }
                    "LOT_SIZE" => {
                        if let Some(v) = filter.step_size.as_deref().and_then(parse_dec) {
                            qty_step = v;
                        }
                        if let Some(v) = filter.min_qty.as_deref().and_then(parse_dec) {
                            min_qty = v;
                        }
                    }
                    "NOTIONAL" | "MIN_NOTIONAL" => {
                        if let Some(v) = filter.min_notional.as_deref().and_then(parse_dec) {
                            min_notional = v;
                        }
                    }
                    _ => {}
                }
            }
            Instrument {
                exchange: EXCHANGE.to_string(),
                symbol: s.symbol,
                base: s.base_asset,
                quote: s.quote_asset,
                status: s.status.unwrap_or_else(|| "UNKNOWN".to_string()),
                min_qty,
                qty_step,
                min_notional,
                price_tick,
                // 公共接口无法获得账户费率：明确标记未知，禁止假装知道。
                maker_bps: Decimal::ZERO,
                taker_bps: Decimal::ZERO,
                fee_known: false,
                updated_at: now,
            }
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct DepthResp {
    #[serde(rename = "lastUpdateId")]
    last_update_id: u64,
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
}

/// 解析 `depth` 响应为完整盘口（`valid = true`，`seq = lastUpdateId`）。
///
/// Binance 快照不带交易所时间戳，`exchange_ts` 使用本地接收时间近似；
/// 调用方必须用 `recv_ts` 判定盘口年龄（风控 §6：250 ms）。
pub(crate) fn parse_depth(body: &str, symbol: &str) -> anyhow::Result<OrderBook> {
    let resp: DepthResp = serde_json::from_str(body).context("解析 Binance depth JSON 失败")?;
    let bids = parse_levels(&resp.bids, "bids")?;
    let asks = parse_levels(&resp.asks, "asks")?;
    let now = Utc::now();
    Ok(OrderBook {
        exchange: EXCHANGE.to_string(),
        symbol: symbol.to_string(),
        seq: resp.last_update_id,
        bids,
        asks,
        exchange_ts: now,
        recv_ts: now,
        valid: true,
        invalid_reason: None,
        last_sync_ts: now,
    })
}

#[derive(Debug, Deserialize)]
struct BookTickerRow {
    symbol: String,
    #[serde(rename = "bidPrice")]
    bid_price: String,
    #[serde(rename = "askPrice")]
    ask_price: String,
}

fn parse_book_tickers(body: &str) -> anyhow::Result<Vec<(String, Decimal, Decimal)>> {
    let rows: Vec<BookTickerRow> =
        serde_json::from_str(body).context("解析 Binance bookTicker JSON 失败")?;
    rows.into_iter()
        .map(|row| {
            let bid = parse_dec(&row.bid_price)
                .ok_or_else(|| anyhow!("{} 的 bidPrice 非法: {}", row.symbol, row.bid_price))?;
            let ask = parse_dec(&row.ask_price)
                .ok_or_else(|| anyhow!("{} 的 askPrice 非法: {}", row.symbol, row.ask_price))?;
            Ok((row.symbol, bid, ask))
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct PriceRow {
    symbol: String,
    price: String,
}

fn parse_prices(body: &str) -> anyhow::Result<Vec<(String, Decimal)>> {
    let rows: Vec<PriceRow> =
        serde_json::from_str(body).context("解析 Binance ticker/price JSON 失败")?;
    rows.into_iter()
        .map(|row| {
            let price = parse_dec(&row.price)
                .ok_or_else(|| anyhow!("{} 的 price 非法: {}", row.symbol, row.price))?;
            Ok((row.symbol, price))
        })
        .collect()
}

fn parse_levels(rows: &[Vec<String>], field: &str) -> anyhow::Result<Vec<BookLevel>> {
    let mut levels = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        let raw_price = row
            .first()
            .ok_or_else(|| anyhow!("{field}[{idx}] 缺少 price"))?;
        let raw_qty = row
            .get(1)
            .ok_or_else(|| anyhow!("{field}[{idx}] 缺少 qty"))?;
        let price = parse_dec(raw_price)
            .ok_or_else(|| anyhow!("{field}[{idx}] price 非法: {raw_price}"))?;
        let qty =
            parse_dec(raw_qty).ok_or_else(|| anyhow!("{field}[{idx}] qty 非法: {raw_qty}"))?;
        // 防御性过滤：快照不应包含非正价格或 0 数量档位。
        if price > Decimal::ZERO && qty > Decimal::ZERO {
            levels.push(BookLevel { price, qty });
        }
    }
    Ok(levels)
}

fn parse_dec(s: &str) -> Option<Decimal> {
    s.trim().parse::<Decimal>().ok()
}

// ---------------------------------------------------------------------------
// 查询字符串拼接（不依赖 reqwest 的可选 `query` feature）
// ---------------------------------------------------------------------------

fn symbols_query(symbols: &[&str]) -> String {
    let json = serde_json::to_string(symbols).unwrap_or_else(|_| "[]".to_string());
    format!("symbols={}", percent_encode(&json))
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;

    const EXCHANGE_INFO_JSON: &str = r#"{
        "timezone": "UTC",
        "serverTime": 1700000000000,
        "rateLimits": [],
        "exchangeFilters": [],
        "symbols": [
            {
                "symbol": "BTCUSDT",
                "status": "TRADING",
                "baseAsset": "BTC",
                "baseAssetPrecision": 8,
                "quoteAsset": "USDT",
                "quotePrecision": 8,
                "orderTypes": ["LIMIT", "MARKET"],
                "icebergAllowed": true,
                "filters": [
                    {"filterType": "PRICE_FILTER", "minPrice": "0.01000000",
                     "maxPrice": "1000000.00000000", "tickSize": "0.01000000"},
                    {"filterType": "LOT_SIZE", "minQty": "0.00001000",
                     "maxQty": "9000.00000000", "stepSize": "0.00001000"},
                    {"filterType": "NOTIONAL", "minNotional": "5.00000000",
                     "applyToMarket": true, "avgPriceMins": 5},
                    {"filterType": "MAX_NUM_ORDERS", "maxNumOrders": 200}
                ]
            },
            {
                "symbol": "ETHUSDT",
                "status": "BREAK",
                "baseAsset": "ETH",
                "quoteAsset": "USDT",
                "filters": [
                    {"filterType": "PRICE_FILTER", "tickSize": "0.01"},
                    {"filterType": "LOT_SIZE", "minQty": "0.0010", "stepSize": "0.0010"},
                    {"filterType": "MIN_NOTIONAL", "minNotional": "10.0"}
                ]
            }
        ]
    }"#;

    const DEPTH_JSON: &str = r#"{
        "lastUpdateId": 1027024,
        "bids": [
            ["4.00000000", "431.00000000"],
            ["3.90000000", "10.00000000"]
        ],
        "asks": [
            ["4.00000200", "12.00000000"],
            ["4.00000400", "50.00000000"]
        ]
    }"#;

    #[test]
    fn parse_exchange_info_extracts_trading_rules() {
        let instruments = parse_exchange_info(EXCHANGE_INFO_JSON).unwrap();
        assert_eq!(instruments.len(), 2);

        let btc = &instruments[0];
        assert_eq!(btc.exchange, "binance");
        assert_eq!(btc.symbol, "BTCUSDT");
        assert_eq!(btc.base, "BTC");
        assert_eq!(btc.quote, "USDT");
        assert_eq!(btc.status, "TRADING");
        assert_eq!(btc.price_tick, dec("0.01000000"));
        assert_eq!(btc.qty_step, dec("0.00001000"));
        assert_eq!(btc.min_qty, dec("0.00001000"));
        assert_eq!(btc.min_notional, dec("5.00000000"));
        // 公共接口拿不到费率：必须标记未知
        assert_eq!(btc.maker_bps, Decimal::ZERO);
        assert_eq!(btc.taker_bps, Decimal::ZERO);
        assert!(!btc.fee_known);

        let eth = &instruments[1];
        assert_eq!(eth.status, "BREAK");
        assert_eq!(eth.price_tick, dec("0.01"));
        assert_eq!(eth.qty_step, dec("0.0010"));
        assert_eq!(eth.min_qty, dec("0.0010"));
        assert_eq!(eth.min_notional, dec("10.0"));
        assert!(!eth.fee_known);
    }

    #[test]
    fn parse_exchange_info_rejects_malformed_json() {
        assert!(parse_exchange_info("{not json").is_err());
        // 缺少 symbols 字段时返回空列表而不是 panic
        let empty = parse_exchange_info("{}").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn parse_depth_builds_valid_snapshot() {
        let book = parse_depth(DEPTH_JSON, "BTCUSDT").unwrap();
        assert_eq!(book.exchange, "binance");
        assert_eq!(book.symbol, "BTCUSDT");
        assert_eq!(book.seq, 1027024);
        assert!(book.valid);
        assert!(book.invalid_reason.is_none());
        assert_eq!(book.bids.len(), 2);
        assert_eq!(book.asks.len(), 2);
        assert_eq!(book.best_bid(), Some(dec("4.00000000")));
        assert_eq!(book.best_ask(), Some(dec("4.00000200")));
        assert_eq!(book.bids[0].qty, dec("431.00000000"));
        assert_eq!(book.asks[1].qty, dec("50.00000000"));
        assert_eq!(book.recv_ts, book.last_sync_ts);
    }

    #[test]
    fn parse_depth_rejects_malformed_payload() {
        assert!(parse_depth("{not json", "BTCUSDT").is_err());
        let bad_level = r#"{"lastUpdateId": 1, "bids": [["abc", "1"]], "asks": []}"#;
        assert!(parse_depth(bad_level, "BTCUSDT").is_err());
        let missing_field = r#"{"lastUpdateId": 1, "bids": [["1.0"]], "asks": []}"#;
        assert!(parse_depth(missing_field, "BTCUSDT").is_err());
        // 0 数量档位被过滤
        let zero_qty = r#"{"lastUpdateId": 2, "bids": [["1.0", "0"]], "asks": [["2.0", "1"]]}"#;
        let book = parse_depth(zero_qty, "X").unwrap();
        assert!(book.bids.is_empty());
        assert_eq!(book.asks.len(), 1);
    }

    #[test]
    fn parse_book_ticker_and_prices() {
        let tickers = parse_book_tickers(
            r#"[{"symbol":"BTCUSDT","bidPrice":"100.1","bidQty":"1",
                 "askPrice":"100.2","askQty":"2"}]"#,
        )
        .unwrap();
        assert_eq!(tickers.len(), 1);
        assert_eq!(tickers[0].0, "BTCUSDT");
        assert_eq!(tickers[0].1, dec("100.1"));
        assert_eq!(tickers[0].2, dec("100.2"));

        let prices = parse_prices(
            r#"[{"symbol":"BTCUSDT","price":"101.5"},
                {"symbol":"ETHUSDT","price":"0.00000001"}]"#,
        )
        .unwrap();
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].1, dec("101.5"));
        assert_eq!(prices[1].1, dec("0.00000001"));

        assert!(parse_book_tickers("[]").unwrap().is_empty());
    }

    #[test]
    fn query_encoding_matches_binance_expectations() {
        assert_eq!(
            symbols_query(&["BTCUSDT", "ETHUSDT"]),
            "symbols=%5B%22BTCUSDT%22%2C%22ETHUSDT%22%5D"
        );
        assert_eq!(percent_encode("BTCUSDT"), "BTCUSDT");
        assert_eq!(percent_encode("a b"), "a%20b");
    }

    #[test]
    fn client_base_url_normalization() {
        let default = BinanceClient::new(None);
        assert_eq!(default.base, "https://api.binance.com");

        let trimmed = BinanceClient::new(Some("https://testnet.binance.vision/".to_string()));
        assert_eq!(trimmed.base, "https://testnet.binance.vision");
        assert_eq!(
            trimmed.url("/api/v3/depth", Some("symbol=BTCUSDT&limit=5".to_string())),
            "https://testnet.binance.vision/api/v3/depth?symbol=BTCUSDT&limit=5"
        );

        let blank = BinanceClient::new(Some("   ".to_string()));
        assert_eq!(blank.base, "https://api.binance.com");
    }
}
