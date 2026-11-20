//! 扫描诊断工具：离线构造合成市场，打印 DEX 候选与拒绝原因。
//!
//! 用法：
//! ```text
//! cargo run --example scan_probe
//! ```
//! 用于验证「为什么没有机会」这类问题，不参与生产运行。
//!
//! 说明：CEX–DEX 非原子策略已整体移除（PRD v2.2 §0.3），本探针只覆盖 DEX 侧。
//! 这里探测的是旧版三角/跨池引擎（`strategy::dex_arb`，v2.0 回退路径）；v2.2 主引擎为
//! `strategy::dex_scan`（多池多跳），其诊断通过 `/api/routes` 与 `#/scanner-dex` 页面完成。

use std::collections::HashMap;

use arb_backend::config::Config;
use arb_backend::feed::demo::DemoMarket;
use arb_backend::strategy::{dex_arb, DexCostParams, MarketView};
use rust_decimal::Decimal;

fn main() {
    let cfg = Config::default();
    let mut market = DemoMarket::new(&cfg, cfg.feed.seed);
    // 多推进几轮，制造价格错位
    for _ in 0..20 {
        market.tick();
    }

    let prices = market.prices.clone();
    let instruments = market.instruments.clone();
    let view = MarketView {
        now: chrono::Utc::now(),
        chain_id: market.chain.chain_id,
        chain_name: market.chain.chain_name.clone(),
        head_block: market.head_block,
        pools: &market.pools,
        tokens: &market.tokens,
        prices: &prices,
        books: &market.books,
        instruments: &instruments,
    };

    let mut available = HashMap::new();
    available.insert("USDC".to_string(), Decimal::from(1500));
    available.insert("USDT".to_string(), Decimal::from(500));
    available.insert("WETH".to_string(), Decimal::from(1));
    available.insert("ETH".to_string(), Decimal::from(1));

    println!("== 盘口 == ");
    for (sym, inst) in &instruments {
        if let Some(b) = market.books.get(&inst.exchange, sym) {
            println!(
                "{sym}: bid={:?} ask={:?} mid={:?} spread={:?} depth_bid={:?}",
                b.best_bid(),
                b.best_ask(),
                b.mid(),
                b.spread_bps(),
                b.depth_usd(arb_backend::domain::types::Side::Sell)
            );
        }
    }

    println!("\n== DEX 三角/跨池（1000 USDC）==");
    let dex_params = DexCostParams::default();
    let opps = dex_arb::scan(
        &view,
        &dex_params,
        &["USDC", "USDT"],
        Decimal::from(1000),
        &available,
    );
    println!("候选 {} 条", opps.len());
    for o in opps.iter().take(6) {
        println!(
            "{:?} gross={} cost={} net={} quality={} funding={:?}",
            o.strategy_type,
            o.executable_gross_profit.round_dp(4),
            o.total_cost.round_dp(4),
            o.conservative_net_profit.round_dp(4),
            o.data_quality,
            o.funding
        );
        if let Some(r) = &o.dex_route {
            for l in &r.legs {
                println!(
                    "    {} {}->{} in={} out={} fee={}bps impact={}bps",
                    l.pool_id,
                    l.token_in,
                    l.token_out,
                    l.amount_in.round_dp(6),
                    l.expected_out.round_dp(6),
                    l.fee_bps,
                    l.price_impact_bps.round_dp(2)
                );
            }
        }
    }
}
