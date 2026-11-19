//! 扫描循环：行情 tick → 策略扫描 → 风控判定 → Paper 执行 → 持久化与推送。
//!
//! 扫描停止不影响已持久化的机会、执行与账本；重启后从数据库恢复。

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::opportunity::Opportunity;
use crate::domain::types::{ExecutionState, OppStatus, StrategyType};
use crate::engine::{AppState, WsEvent};
use crate::paper::ExecMarket;
use crate::strategy::{MarketView, ScanConfig, ScanContext};

/// 启动扫描任务。
pub fn spawn(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = Duration::from_millis(state.config.paper.scan_interval_ms.max(200));
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = state.scan_now.notified() => {}
            }
            if let Err(e) = scan_once(&state, true).await {
                tracing::error!("扫描失败: {e}");
                state.stats.write().await.last_error = Some(e.to_string());
            }
        }
    })
}

/// 执行一次扫描（`execute=true` 时按配置概率执行机会）。
pub async fn scan_once(state: &Arc<AppState>, execute: bool) -> anyhow::Result<usize> {
    let running = {
        let s = state.scanner.read().await;
        s.state == "RUNNING"
    };
    if !running {
        // 未启动时仍然 tick 行情，保持盘口与价格新鲜，但不产出机会
        state.market.write().await.tick();
        return Ok(0);
    }

    let started = Instant::now();

    // ---- 行情推进 ----
    {
        let mut m = state.market.write().await;
        m.tick();
        // 价格同步到账本：权益按最新价格计算，未实现盈亏才可解释
        let prices = m.prices.clone();
        drop(m);
        let mut ledger = state.ledger.lock().await;
        for (asset, price) in prices {
            ledger.set_price(&asset, price);
        }
    }

    // ---- 构建扫描视图 ----
    let equity = state.equity().await;
    let peak = equity.max(Decimal::from_f64(state.config.paper.start_equity).unwrap_or(Decimal::from(5000)));
    let account_loss = state.account_daily_loss().await;
    let mut strategy_loss = HashMap::new();
    for s in StrategyType::ALL {
        strategy_loss.insert(*s, state.strategy_daily_loss(*s).await);
    }
    let dex_available = state.available_inventory("dex-").await;
    let cex_available = state.available_inventory("cex-").await;
    let controls = state.controls.read().await.clone();

    let mut fill_rate = HashMap::new();
    {
        let execs = state.executions.read().await;
        for s in StrategyType::ALL {
            let related: Vec<_> = execs.iter().filter(|e| e.strategy_type == *s).collect();
            let requested: Decimal = related.iter().map(|e| e.qty_requested).sum();
            let filled: Decimal = related.iter().map(|e| e.qty_filled).sum();
            let rate = if requested.is_zero() {
                Decimal::ONE
            } else {
                (filled / requested).min(Decimal::ONE).max(Decimal::ZERO)
            };
            fill_rate.insert(*s, rate);
        }
    }

    let (opps, dex_routes, dex_stats, dex_graph) = {
        let market = state.market.read().await;
        let prices = market.prices.clone();
        let head_block = market.head_block;
        let chain_id = market.chain.chain_id;
        let chain_name = market.chain.chain_name.clone();
        let instruments = market.instruments.clone();

        let view = MarketView {
            now: Utc::now(),
            chain_id,
            chain_name,
            head_block,
            pools: &market.pools,
            tokens: &market.tokens,
            prices: &prices,
            books: &market.books,
            instruments: &instruments,
        };

        let ctx = ScanContext {
            equity_usd: equity,
            peak_equity_usd: peak,
            account_daily_loss_usd: account_loss,
            strategy_daily_loss: strategy_loss,
            unrealized_pnl_usd: Decimal::ZERO,
            controls,
            head_block,
            dex_available,
            cex_available,
            fill_rate,
        };

        let cfg = ScanConfig {
            notional_ladder: state
                .config
                .paper
                .notional_ladder
                .iter()
                .filter_map(|v| Decimal::from_f64(*v))
                .collect(),
            base_symbols: vec!["USDC".into(), "USDT".into()],
            cex_first: state
                .stats
                .read()
                .await
                .scans
                .is_multiple_of(2),
        };
        // ---- DEX 多池多跳引擎（新）：图 → 搜索 → 优化 → 模拟 → 门禁 ----
        let risk = state.risk.read().await;
        let (mut all, dex_routes, dex_stats, dex_graph) = if state.dex_params.enabled {
            let mut cache = state.dex_cache.lock().await;
            let out = crate::strategy::dex_scan::scan(&view, &state.dex_params, &ctx, &mut cache);
            let mut all: Vec<Opportunity> = out
                .opportunities
                .into_iter()
                .map(|o| state.strategy.apply_risk_and_score(&view, &ctx, &risk, o))
                .collect();
            // ---- CEX–DEX 双腿对冲（非原子，保持原有引擎）----
            let mut cex_opps = state.strategy.scan_cex_dex(&view, &cfg, &ctx, &risk);
            all.append(&mut cex_opps);
            (all, out.routes, out.stats, out.graph)
        } else {
            // 回退路径：dex_engine.enabled = false 时使用旧版三角/跨池扫描。
            let all = state.strategy.scan(&view, &cfg, &ctx, &risk);
            let stats = crate::domain::dex_scan::ScanRunStats {
                run_id: crate::domain::new_id("scan"),
                reason: "旧引擎（dex_engine.enabled=false）".into(),
                model_level: crate::domain::types::ModelLevel::M1,
                ..Default::default()
            };
            // scan() 已包含 DEX 与 CEX–DEX，无需再调用 scan_cex_dex 以免重复。
            (
                all,
                Vec::new(),
                stats,
                crate::domain::dex_scan::GraphStats::default(),
            )
        };
        drop(risk);

        // 去重：同策略 + 同规模 + 同路线只保留净利最高者
        all.sort_by(|a, b| b.conservative_net_profit.cmp(&a.conservative_net_profit));
        let mut seen = std::collections::HashSet::new();
        all.retain(|o| {
            let key = format!(
                "{}|{}|{}|{}",
                o.strategy_type.as_str(),
                o.target_notional,
                o.dex_route.as_ref().map(|r| r.route_id.clone()).unwrap_or_default(),
                o.cex_symbol.clone().unwrap_or_default()
            );
            seen.insert(key)
        });

        // 每轮只保留有信息量的候选：按策略分别配额，保证三类策略在界面上都能看到
        // 「可执行 / 观察 / 最接近门槛的拒绝样本」，同时避免淹没前端与数据库。
        let mut kept: Vec<Opportunity> = Vec::new();
        for strategy in StrategyType::ALL {
            let mut actionable = Vec::new();
            let mut watch = Vec::new();
            let mut rejected = Vec::new();
            for o in all.iter().filter(|o| o.strategy_type == *strategy) {
                match o.status {
                    OppStatus::Actionable => actionable.push(o.clone()),
                    OppStatus::Watch => watch.push(o.clone()),
                    _ => rejected.push(o.clone()),
                }
            }
            actionable.truncate(12);
            watch.truncate(10);
            rejected.truncate(4);
            kept.extend(actionable);
            kept.extend(watch);
            kept.extend(rejected);
        }
        kept.sort_by(|a, b| b.conservative_net_profit.cmp(&a.conservative_net_profit));

        (kept, dex_routes, dex_stats, dex_graph)
    };

    let actionable = opps.iter().filter(|o| o.status == OppStatus::Actionable).count();
    let rejected = opps.iter().filter(|o| o.status == OppStatus::Rejected).count();
    let watch = opps.iter().filter(|o| o.status == OppStatus::Watch).count();

    // ---- 推送与持久化 ----
    for o in &opps {
        state.push_opportunity(o.clone()).await;
        if state.config.feed.mode != "silent" {
            state.broadcast(WsEvent::Opportunity {
                opportunity: Box::new(o.clone()),
            });
        }
        if let Err(e) = crate::db::repo::save_opportunity(&state.db, o) {
            tracing::warn!("机会落库失败: {e}");
        }
    }
    let _ = crate::db::repo::prune_opportunities(&state.db, state.config.data.keep_opportunities);

    // ---- DEX 路线记录：内存 + 落库 + 推送 ----
    {
        // 只保留本轮快照：同一 route_id 的历史版本由数据库保留。
        let mut store = state.dex_routes.write().await;
        *store = dex_routes.iter().take(500).cloned().collect();
    }
    *state.dex_graph.write().await = dex_graph.clone();
    *state.dex_scan_stats.write().await = dex_stats.clone();
    {
        let mut sims = state.simulations.write().await;
        for r in dex_routes.iter().take(100) {
            if let Some(s) = &r.simulation {
                sims.insert(0, s.clone());
            }
            if let Some(s) = &r.stress {
                sims.insert(0, s.clone());
            }
        }
        if sims.len() > 1_000 {
            sims.truncate(1_000);
        }
    }
    if state.config.feed.mode != "silent" {
        state.broadcast(WsEvent::Graph {
            stats: Box::new(dex_graph.clone()),
        });
        for r in dex_routes.iter().take(20) {
            state.broadcast(WsEvent::Route {
                route: Box::new(r.clone()),
            });
        }
        for r in dex_routes.iter().take(20) {
            if let Some(s) = &r.simulation {
                state.broadcast(WsEvent::Simulation {
                    simulation: Box::new(s.clone()),
                });
            }
        }
    }
    if let Err(e) = crate::db::repo::save_route_records(&state.db, &dex_routes) {
        tracing::warn!("路线落库失败: {e}");
    }
    if let Err(e) = crate::db::repo::save_scan_run(&state.db, &dex_stats) {
        tracing::warn!("扫描统计落库失败: {e}");
    }

    // ---- 更新扫描状态 ----
    {
        let mut s = state.scanner.write().await;
        let now = Utc::now();
        if s.started_at.is_none() {
            s.started_at = Some(now);
        }
        s.state = "RUNNING".into();
        s.detected += opps.len() as u64;
        s.actionable += actionable as u64;
        s.rejected += rejected as u64;
        s.watch += watch as u64;
        s.quotes = s.quotes.saturating_add(dex_stats.visits);
        s.engine_routes = dex_stats.candidates as u64;
        s.engine_candidates = dex_stats.candidates as u64;
        s.engine_requoted = dex_stats.requoted as u64;
        s.engine_reused = dex_stats.reused as u64;
        s.engine_edges = dex_stats.edges as u64;
        s.engine_tokens = dex_stats.tokens as u64;
        s.engine_visits = dex_stats.visits;
        s.engine_truncated = dex_stats.truncated;
        s.engine_full_rescan = dex_stats.full_rescan;
        s.uptime_ms = s
            .started_at
            .map(|t| (now - t).num_milliseconds().max(0) as u64)
            .unwrap_or(0);
        s.last_scan_ms = started.elapsed().as_millis() as u64;
        s.scan_interval_ms = state.config.paper.scan_interval_ms;
        s.chains = 1;
        s.protocols = 6;
        s.pools = {
            let m = state.market.read().await;
            m.pools.len() as u64
        };
    }
    {
        let mut st = state.stats.write().await;
        st.scans += 1;
        st.opportunities_total += opps.len() as u64;
        st.started_at.get_or_insert_with(Utc::now);
        st.last_scan_ms = started.elapsed().as_millis() as u64;
    }

    // ---- 执行（Paper）----
    if execute && state.config.paper.auto_execute {
        // 交替选择策略：Paper 阶段需要同时覆盖「原子闭环」与「双腿对冲」两条链路，
        // 否则高频的原子机会会长期挤占 CEX–DEX 的执行样本。
        let executions_total = state.stats.read().await.executions_total;
        let prefer_cex = executions_total % 2 == 1;
        let actionable: Vec<Opportunity> = opps
            .iter()
            .filter(|o| o.status == OppStatus::Actionable && o.conservative_net_profit > Decimal::ZERO)
            .cloned()
            .collect();
        let candidate = actionable
            .iter()
            .find(|o| (o.strategy_type == StrategyType::CexDex) == prefer_cex)
            .or_else(|| actionable.first())
            .cloned();

        if let Some(opp) = candidate {
            let p = state.config.paper.execute_probability;
            let roll = {
                let m = state.market.read().await;
                let r = m.tick_count();
                let h = crate::domain::plan::fnv1a_64(&format!("{}:{r}", opp.opportunity_id));
                (h % 1000) as f64 / 1000.0
            };
            if roll < p {
                execute_opportunity(state, opp).await?;
            }
        }
    }

    // ---- 广播扫描状态 ----
    let status = state.scanner.read().await.clone();
    state.broadcast(WsEvent::Scanner {
        status: Box::new(status.clone()),
    });
    let _ = crate::db::repo::save_scanner_state(&state.db, &status);
    Ok(opps.len())
}

/// 执行单个机会并落账。
pub async fn execute_opportunity(
    state: &Arc<AppState>,
    opp: Opportunity,
) -> anyhow::Result<()> {
    // 1) 计划
    let (plan, exec_market_data) = {
        let market = state.market.read().await;
        let prices = market.prices.clone();
        let head_block = market.head_block;
        let chain_name = market.chain.chain_name.clone();
        let book = opp
            .cex_symbol
            .as_ref()
            .and_then(|s| market.books.get(&market.chain.chain_name, s).or_else(|| {
                market.instruments.get(s).and_then(|i| market.books.get(&i.exchange, s))
            }))
            .cloned();
        let instrument = opp
            .cex_symbol
            .as_ref()
            .and_then(|s| market.instruments.get(s).cloned());
        (
            (prices, head_block, chain_name, book, instrument),
            (),
        )
    };
    let (prices, head_block, chain_name, book, instrument) = plan;
    let _ = exec_market_data;

    let exec_venue_dex = crate::engine::DEX_VENUE.to_string();
    let exec_venue_cex = crate::engine::CEX_VENUE.to_string();

    let mut paper = state.paper.lock().await;
    let mkt = ExecMarket {
        now: Utc::now(),
        head_block,
        book: book.as_ref(),
        instrument: instrument.as_ref(),
        prices: &prices,
        chain_name: chain_name.clone(),
        cex_venue: exec_venue_cex,
        dex_venue: exec_venue_dex,
    };
    let plan = paper.build_plan(&opp, &mkt);

    // 2) 账本执行前快照（用于落库增量流水）
    let mut ledger = state.ledger.lock().await;
    let before_len = ledger.entries().len();
    let rec = paper.execute(&opp, &plan, &mut ledger, &mkt);
    let new_entries: Vec<_> = ledger.entries()[before_len..].to_vec();

    // 3) 统计
    ledger.record_trade(crate::ledger::engine::TradeStat {
        execution_id: rec.execution_id.clone(),
        strategy_type: rec.strategy_type,
        net_pnl: rec.net_pnl,
        gross_pnl: rec.gross_pnl,
        cost: rec.costs.total(),
        filled_ratio: rec.fill_ratio(),
        reverted: matches!(
            rec.state,
            ExecutionState::DexReverted | ExecutionState::HedgeFailed
        ),
        exposure_ms: rec.exposure_ms,
        notional: rec.target_notional,
        closed_at: Utc::now(),
    });
    // 账本快照：进程重启后能恢复账户、余额与成交统计
    let snapshot = ledger.export();
    drop(ledger);
    if let Ok(json) = serde_json::to_string(&snapshot) {
        if let Err(e) = crate::db::repo::set_config(&state.db, "ledger_snapshot", &json) {
            tracing::warn!("账本快照保存失败: {e}");
        }
    }

    // 4) 持久化
    if let Err(e) = crate::db::repo::save_plan(&state.db, &plan) {
        tracing::warn!("计划落库失败: {e}");
    }
    if let Err(e) = crate::db::repo::save_execution(&state.db, &rec) {
        tracing::warn!("执行落库失败: {e}");
    }
    if let Err(e) = crate::db::repo::append_ledger_entries(&state.db, &new_entries) {
        tracing::warn!("账本落库失败: {e}");
    }

    // 5) 推送
    state.push_execution(rec.clone()).await;
    state.broadcast(WsEvent::Execution {
        execution: Box::new(rec.clone()),
    });
    for e in &new_entries {
        state.broadcast(WsEvent::Ledger {
            entry_id: e.entry_id.clone(),
            account_id: e.account_id.clone(),
            asset: e.asset.clone(),
            amount: e.amount,
        });
    }
    {
        let mut st = state.stats.write().await;
        st.executions_total += 1;
    }
    Ok(())
}

/// 启动/停止扫描。
pub async fn start(state: &Arc<AppState>) {
    let mut s = state.scanner.write().await;
    if s.state != "RUNNING" {
        s.state = "RUNNING".into();
        s.started_at = Some(Utc::now());
    }
}

pub async fn stop(state: &Arc<AppState>) {
    let mut s = state.scanner.write().await;
    s.state = "STOPPED".into();
    s.started_at = None;
    s.uptime_ms = 0;
}

/// 健康快照（供 `/system/health`）。
pub async fn health_json(state: &Arc<AppState>) -> serde_json::Value {
    let market = state.market.read().await;
    let books: Vec<_> = market
        .instruments
        .keys()
        .filter_map(|s| {
            market
                .instruments
                .get(s)
                .and_then(|i| market.books.get(&i.exchange, s))
                .map(|b| {
                    serde_json::json!({
                        "exchange": b.exchange,
                        "symbol": b.symbol,
                        "valid": b.valid,
                        "age_ms": b.age_ms(Utc::now()),
                        "spread_bps": b.spread_bps(),
                        "seq": b.seq,
                    })
                })
        })
        .collect();
    serde_json::json!({
        "db": state.db.health(),
        "db_path": state.db.path(),
        "feed": state.config.feed.mode,
        "head_block": market.head_block,
        "pools": market.pools.len(),
        "books": books,
        "scanner": state.scanner.read().await.clone(),
        "stats": {
            "scans": state.stats.read().await.scans,
            "opportunities": state.stats.read().await.opportunities_total,
            "executions": state.stats.read().await.executions_total,
            "last_scan_ms": state.stats.read().await.last_scan_ms,
            "last_error": state.stats.read().await.last_error,
        },
        "equity": state.equity().await.to_string(),
    })
}
