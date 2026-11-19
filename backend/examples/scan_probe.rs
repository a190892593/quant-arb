//! 扫描诊断工具：离线构造合成市场，打印各策略的候选与拒绝原因。
//!
//! 用法：
//! ```text
//! cargo run --example scan_probe
//! ```
//! 用于验证「为什么没有机会」这类问题，不参与生产运行。

use std::collections::HashMap;

use arb_backend::config::Config;
use arb_backend::feed::demo::DemoMarket;
use arb_backend::strategy::{cex_dex, dex_arb, CexDexParams, DexCostParams, MarketView};
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

    println!("\n== CEX–DEX（1000 USDC）==");
    let cd = CexDexParams::default();
    let cex_opps = cex_dex::scan(&view, &cd, Decimal::from(1000), &available, true);
    println!("候选 {} 条", cex_opps.len());
    for o in cex_opps.iter().take(6) {
        println!(
            "{:?} dir={:?} qty={} gross={} cost={} net={} theo_bps={} quality={}",
            o.cex_symbol,
            o.direction,
            o.target_qty.round_dp(6),
            o.executable_gross_profit.round_dp(4),
            o.total_cost.round_dp(4),
            o.conservative_net_profit.round_dp(4),
            o.theoretical_edge_bps.round_dp(2),
            o.data_quality
        );
    }

    // 手动检查一条 DEX 路径与 CEX 成本，帮助定位「为什么没有机会」
    println!("\n== 路径与成本分解 ==");
    if let Some((path, edges)) = cex_dex::find_best_path(
        &view,
        "USDC",
        "WETH",
        Decimal::from(1000),
        2,
    ) {
        println!(
            "best USDC->WETH: out={} impact={}bps hops={} gas={}",
            path.amount_out.round_dp(8),
            path.price_impact_bps.round_dp(2),
            edges.len(),
            path.gas_units
        );
        for l in &path.legs {
            println!(
                "   {} {}->{} in={} out={} fee={}bps",
                l.pool_id,
                l.token_in,
                l.token_out,
                l.amount_in.round_dp(6),
                l.amount_out.round_dp(6),
                l.fee_bps
            );
        }
    } else {
        println!("未找到 USDC->WETH 路径！");
    }
}
