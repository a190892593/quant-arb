//! DEX 多池多跳引擎端到端测试（PRD §4–§7、§9）。
//!
//! 覆盖：
//! - 合成市场的准入 → 图 → 搜索 → 优化 → M1/M3 模拟 → 记录链路；
//! - 路线有向指纹唯一、跳数/类型/生命周期与优化结果自洽；
//! - 增量扫描在池状态与价格未变时复用报价；
//! - 路线记录可落库并被重新读取（可复现性）。

use std::collections::HashMap;

use arb_backend::db::{repo, Db};
use arb_backend::domain::dex_scan::RouteScanRecord;
use arb_backend::domain::market::{Instrument, Token};
use arb_backend::domain::money::dec;
use arb_backend::domain::risk::ControlState;
use arb_backend::domain::types::{
    ModelLevel, OpportunityLifecycle, RouteType, SimulationStatus,
};
use arb_backend::feed::demo::DemoMarket;
use arb_backend::strategy::dex_scan::{self, DexScanParams, MultiHopCache};
use arb_backend::strategy::{MarketView, ScanContext};
use rust_decimal::Decimal;

fn context(available_usdc: Decimal) -> ScanContext {
    let mut dex_available = HashMap::new();
    dex_available.insert("USDC".to_string(), available_usdc);
    ScanContext {
        equity_usd: dec("5000"),
        peak_equity_usd: dec("5000"),
        account_daily_loss_usd: Decimal::ZERO,
        strategy_daily_loss: HashMap::new(),
        unrealized_pnl_usd: Decimal::ZERO,
        controls: ControlState::new(),
        head_block: 0,
        dex_available,
        cex_available: HashMap::new(),
        fill_rate: HashMap::new(),
    }
}

fn scan_market<'a>(
    market: &'a DemoMarket,
    params: &DexScanParams,
    ctx: &mut ScanContext,
    cache: &mut MultiHopCache,
) -> dex_scan::DexScanOutput {
    let tokens: &HashMap<String, Token> = &market.tokens;
    let instruments: &HashMap<String, Instrument> = &market.instruments;
    let view = MarketView {
        now: chrono::Utc::now(),
        chain_id: market.chain.chain_id,
        chain_name: market.chain.chain_name.clone(),
        head_block: market.head_block,
        pools: &market.pools,
        tokens,
        prices: &market.prices,
        books: &market.books,
        instruments,
    };
    ctx.head_block = market.head_block;
    dex_scan::scan(&view, params, ctx, cache)
}

#[test]
fn demo_market_multihop_scan_is_consistent_and_persistable() {
    let cfg = arb_backend::config::Config::default();
    let market = DemoMarket::new(&cfg, 20261119);
    let params = DexScanParams::from_config(&cfg);
    let mut ctx = context(dec("2500"));
    let mut cache = MultiHopCache::new();

    let out = scan_market(&market, &params, &mut ctx, &mut cache);

    // 图与搜索
    assert!(out.stats.edges > 0, "交易图应有边");
    assert!(out.stats.visits > 0, "搜索应至少访问一次");
    assert!(!out.routes.is_empty(), "合成市场应产生候选路线");

    // 指纹唯一 / 闭环 / 跳数配置
    let mut fps = std::collections::HashSet::new();
    for r in &out.routes {
        assert!(fps.insert(r.fingerprints.directed.clone()), "有向指纹必须唯一");
        assert_eq!(r.hops, r.legs.len());
        assert!((params.min_hops..=params.max_hops).contains(&r.hops), "{:?}", r.hops);
        assert_eq!(r.token_path.first().unwrap(), r.token_path.last().unwrap());
        assert_eq!(r.model_level, ModelLevel::M1);
        let sim = r.simulation.as_ref().expect("M1 模拟必须存在");
        assert_eq!(sim.level, ModelLevel::M1);
        assert_ne!(sim.status, SimulationStatus::Unavailable);
        assert!(r.stress.is_some(), "M3 压力模拟必须存在");
        assert_eq!(r.route_type, RouteType::of_hops(r.hops));
        if r.net_usd > Decimal::ZERO && sim.status == SimulationStatus::Passed {
            assert!(
                matches!(
                    r.lifecycle,
                    OpportunityLifecycle::Simulated | OpportunityLifecycle::Rejected
                ),
                "盈利且模拟通过的机会应推进到 SIMULATED 或 Rejected: {:?}",
                r.lifecycle
            );
        }
    }

    // 优化结果自洽
    for r in &out.routes {
        let opt = r.optimization.as_ref().expect("优化曲线");
        assert!(!opt.points.is_empty());
        if let Some(best) = &opt.best {
            assert!(best.amount_in > Decimal::ZERO);
            assert!(best.amount_in <= opt.max_executable_amount);
            assert!(opt.points.windows(2).all(|w| w[0].amount_in <= w[1].amount_in));
        }
    }

    // 机会与路线一一对应（每个机会都有指纹与 M1 模型）
    for o in &out.opportunities {
        assert!(o.route_fingerprint.is_some());
        assert_eq!(o.model_level, ModelLevel::M1);
        assert!(o.dex_route.is_some());
        assert!(o.dex_route.as_ref().unwrap().is_closed_loop());
    }

    // 增量扫描：没有新 tick、价格未变 → 不重新报价
    let second = scan_market(&market, &params, &mut ctx, &mut cache);
    assert_eq!(second.stats.requoted, 0, "状态未变时应复用缓存");
    assert_eq!(second.stats.reused, second.stats.candidates);
    assert!(second.routes.iter().all(|r| r.source == "incremental"));

    // 落库与重读
    let db = Db::open_memory().unwrap();
    repo::save_route_records(&db, &out.routes).unwrap();
    let loaded: Vec<RouteScanRecord> = repo::load_routes(&db, 100).unwrap();
    assert_eq!(loaded.len(), out.routes.len());
    for r in &out.routes {
        let l = loaded.iter().find(|x| x.route_id == r.route_id).expect("路线应可重读");
        assert_eq!(l.net_usd, r.net_usd);
        assert_eq!(l.fingerprints, r.fingerprints);
        assert_eq!(l.lifecycle, r.lifecycle);
    }
}

#[test]
fn demo_market_scan_respects_pool_state_change() {
    let cfg = arb_backend::config::Config::default();
    let mut market = DemoMarket::new(&cfg, 7);
    let params = DexScanParams::from_config(&cfg);
    let mut ctx = context(dec("2500"));
    let mut cache = MultiHopCache::new();

    let first = scan_market(&market, &params, &mut ctx, &mut cache);
    assert!(first.stats.requoted > 0);

    // 推进一个区块：所有池状态都变化 → 必须重新报价
    market.tick();
    let second = scan_market(&market, &params, &mut ctx, &mut cache);
    assert_eq!(second.stats.requoted, second.stats.candidates);
    assert_eq!(second.stats.reused, 0);
    assert!(second.stats.candidates >= 1);
}


#[test]
fn fixed_size_ladder_produces_per_size_quotes() {
    let cfg = arb_backend::config::Config::default();
    let market = DemoMarket::new(&cfg, 20261119);
    let params = DexScanParams::from_config(&cfg);
    let mut ctx = context(dec("2500"));
    let mut cache = MultiHopCache::new();
    let out = scan_market(&market, &params, &mut ctx, &mut cache);

    let expected: Vec<Decimal> = params.notional_ladder.clone();
    let mut with_quotes = 0usize;
    for r in &out.routes {
        if r.size_quotes.is_empty() {
            continue;
        }
        with_quotes += 1;
        assert!(
            r.size_quotes.windows(2).all(|w| w[0].notional_usd <= w[1].notional_usd),
            "档位报价必须按名义金额升序"
        );
        for q in &r.size_quotes {
            assert!(q.notional_usd > Decimal::ZERO);
            assert!(q.amount_in > Decimal::ZERO);
            assert!(q.amount_out > Decimal::ZERO);
            assert!(
                expected.iter().any(|v| (*v - q.notional_usd).abs() < Decimal::from(2)),
                "档位 {} 应来自固定 ladder {:?}",
                q.notional_usd,
                expected
            );
        }
    }
    assert!(with_quotes > 0, "至少一条路线应有固定档位报价");
}
