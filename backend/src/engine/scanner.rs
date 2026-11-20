//! 扫描循环：行情 tick → 策略扫描 → 风控判定 → Paper 执行 → 持久化与推送。
//!
//! 扫描停止不影响已持久化的机会、执行与账本；重启后从数据库恢复。

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::market::{DataQuality, ModelLevel, RiskViolation};
use crate::domain::opportunity::Opportunity;
use crate::domain::types::{ExecutionState, OppStatus, RiskSeverity, StrategyType};
use crate::engine::{AppState, WsEvent};
use crate::evm::RpcClient;
use crate::paper::ExecMarket;
use crate::strategy::{MarketView, ScanConfig, ScanContext, m2};

/// 启动扫描任务。
pub fn spawn(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = Duration::from_millis(state.config.paper.scan_interval_ms.max(200));
        loop {
            let cycle_started = Instant::now();
            if let Err(e) = scan_once(&state, true).await {
                tracing::error!("扫描失败: {e}");
                state.stats.write().await.last_error = Some(e.to_string());
            }
            // 保持固定节拍：扫描耗时从间隔中扣除，避免固定 sleep 造成周期翻倍。
            let wait = interval.saturating_sub(cycle_started.elapsed());
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = state.scan_now.notified() => {}
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
        // 行情由 feed 任务独立刷新；停止扫描不会停止真实链同步。
        return Ok(0);
    }

    let started = Instant::now();

    // ---- 价格同步到账本 ----
    {
        // 权益按最新真实价格计算，未实现盈亏才可解释。
        let prices = state.market.read().await.prices.clone();
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

    let (mut opps, dex_routes, dex_stats, dex_graph) = {
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

        let mut dex_params = state.dex_params.read().await.clone();
        let cfg = ScanConfig {
            notional_ladder: dex_params.notional_ladder.clone(),
            base_symbols: dex_params.base_symbols.clone(),
            cex_first: state
                .stats
                .read()
                .await
                .scans
                .is_multiple_of(2),
        };
        // ---- DEX 多池多跳引擎（新）：图 → 搜索 → 优化 → 模拟 → 门禁 ----
        let risk = state.risk.read().await;
        // 真实链下 Gas 价格与原生资产价格来自链上快照，覆盖配置占位值。
        {
            let m = state.market.read().await;
            if m.gas_price_gwei > Decimal::ZERO {
                dex_params.gas_price_gwei = m.gas_price_gwei;
            }
            if let Some(p) = m.prices.get("WETH") {
                if *p > Decimal::ZERO {
                    dex_params.native_price_usd = *p;
                }
            }
        }
        let (mut all, dex_routes, dex_stats, dex_graph) = if dex_params.enabled {
            let mut cache = state.dex_cache.lock().await;
            let out = crate::strategy::dex_scan::scan(&view, &dex_params, &ctx, &mut cache);
            let mut all: Vec<Opportunity> = out
                .opportunities
                .into_iter()
                .map(|o| state.strategy.apply_risk_and_score(&view, &ctx, &risk, o))
                .collect();
            // 已移除 CEX–DEX 扫描：本系统只做 DEX 原子套利搜索。
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
            // 旧引擎只产出 DEX 原子机会。
            (
                all,
                Vec::new(),
                stats,
                crate::domain::dex_scan::GraphStats::default(),
            )
        };
        // ---- 标价分歧门禁：三源价格分歧过大的 Token，相关机会降级为观察 ----
        {
            let div = market.price_divergence.clone();
            let threshold = Decimal::from(50);
            for o in all.iter_mut() {
                let Some(route) = o.dex_route.as_ref() else {
                    continue;
                };
                let mut worst = Decimal::ZERO;
                let mut worst_token = String::new();
                for t in route
                    .legs
                    .iter()
                    .map(|l| &l.token_in)
                    .chain(route.legs.iter().map(|l| &l.token_out))
                    .chain(std::iter::once(&route.start_token))
                {
                    if let Some(d) = div.get(t) {
                        if *d > worst {
                            worst = *d;
                            worst_token = t.clone();
                        }
                    }
                }
                if worst > threshold {
                    o.rejections.push(RiskViolation {
                        key: "price_divergence".into(),
                        label: "标价分歧".into(),
                        severity: RiskSeverity::SoftBlock,
                        limit: threshold,
                        actual: worst,
                        unit: "bps".into(),
                        message: format!(
                            "{} 的三源标价偏差 {:.0} bps，超过 {:.0} bps 阈值，机会降级为观察",
                            worst_token, worst, threshold
                        ),
                    });
                    o.status = OppStatus::Watch;
                }
            }
        }

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
            // 配额需覆盖路线表展示量（最多 100 条路线），否则合并表里大量行
            // 找不到对应机会，风险/质量列会显示为「—」。
            let (a_q, w_q, r_q) = match strategy {
                StrategyType::DexTriple | StrategyType::DexCross => (20, 20, 140),
                _ => (12, 10, 4),
            };
            actionable.truncate(a_q);
            watch.truncate(w_q);
            rejected.truncate(r_q);
            kept.extend(actionable);
            kept.extend(watch);
            kept.extend(rejected);
        }
        kept.sort_by(|a, b| b.conservative_net_profit.cmp(&a.conservative_net_profit));

        (kept, dex_routes, dex_stats, dex_graph)
    };

    // ---- 继承 M2 证据（按 route_id 保存，金额 5% 容差、区块 150 内有效）----
    {
        let evidence = state.m2_evidence.read().await;
        for o in opps.iter_mut() {
            let Some(route) = o.dex_route.as_ref() else {
                continue;
            };
            let Some(ev) = evidence.get(&route.route_id).cloned() else {
                continue;
            };
            let matches_amount = if ev.amount_in.is_zero() || route.amount_in.is_zero() {
                route.amount_in == ev.amount_in
            } else {
                ((route.amount_in - ev.amount_in).abs() / route.amount_in) <= Decimal::new(5, 2)
            };
            if !matches_amount {
                continue;
            }
            // 证据始终展示；只有足够新（≈78s / 300 区块）才据此升级 M2 + VERIFIED。
            let fresh = o.quote_block_number.saturating_sub(ev.block) <= 300;
            o.m2 = Some(ev.clone());
            if ev.passed && fresh {
                o.model_level = ModelLevel::M2;
                o.data_quality = DataQuality::Verified;
                o.confidence = format!(
                    "M2 REVM 分叉复核通过（偏差 {:.2} bps）",
                    ev.diff_bps.to_f64().unwrap_or(0.0)
                );
                o.rejections
                    .retain(|r| r.key != "data_quality" && r.key != "state_snapshot");
                o.status = if o
                    .rejections
                    .iter()
                    .any(|r| matches!(r.severity, RiskSeverity::HardStop | RiskSeverity::SoftBlock))
                {
                    OppStatus::Rejected
                } else if o.rejections.is_empty() {
                    OppStatus::Actionable
                } else {
                    OppStatus::Watch
                };
            }
        }
    }

    let actionable = opps.iter().filter(|o| o.status == OppStatus::Actionable).count();
    let rejected = opps.iter().filter(|o| o.status == OppStatus::Rejected).count();
    let watch = opps.iter().filter(|o| o.status == OppStatus::Watch).count();

    // ---- 推送与持久化 ----
    // 本轮快照整体替换内存列表（只保留 kept），避免跨轮累积同一路线的重复条目；
    // M2 证据保存在 state.m2_evidence，按 route_id 在下一轮重新附加。
    {
        // 空快照（本轮全是复用路线且未生成机会）不清空已有列表，
        // 否则前端机会流与路线表的风险/质量列会周期性闪空。
        let mut list = state.opportunities.write().await;
        if !opps.is_empty() {
            *list = opps.iter().take(500).cloned().collect();
        }
    }
    for o in &opps {
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
        // 空快照（引擎刚初始化 / 本轮全量重建尚未产出）不清空已有路线，
        // 否则前端会看到路线表在每个重建周期闪成空表。
        let mut store = state.dex_routes.write().await;
        if !dex_routes.is_empty() {
            *store = dex_routes.iter().take(500).cloned().collect();
        }
    }
    *state.dex_graph.write().await = dex_graph.clone();
    // 引擎刚初始化时统计全为 0，若直接覆盖会让界面上的「候选/访问量」闪回 0。
    if dex_stats.candidates > 0 || dex_stats.reason != "init" {
        *state.dex_scan_stats.write().await = dex_stats.clone();
    }
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
    // 首轮扫描时先做一次即时回收（升级前的历史数据可能已积累到 GB 级）。
    if state.stats.read().await.scans == 1 {
        let _ = crate::db::repo::prune_simulations(&state.db, state.config.data.keep_simulations);
    }
    // 每 60 轮清理一次扫描记录、路线快照与模拟结果，避免 DB 无限增长。
    if state.stats.read().await.scans % 60 == 0 {
        let _ = crate::db::repo::prune_routes(&state.db, 2_000);
        let _ = crate::db::repo::prune_scan_runs(&state.db, 2_000);
        let _ = crate::db::repo::prune_simulations(&state.db, state.config.data.keep_simulations);
    }

    // ---- 执行（Paper）----
    if execute && state.config.paper.auto_execute {
        // 只做 DEX 原子策略：取净利最高的可执行机会。
        let actionable: Vec<Opportunity> = opps
            .iter()
            .filter(|o| o.status == OppStatus::Actionable && o.conservative_net_profit > Decimal::ZERO)
            .cloned()
            .collect();
        let candidate = actionable.first().cloned();

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
        // 计数按「本次运行」统计，避免把演示/上一轮的数字带进实时扫描。
        s.detected = 0;
        s.actionable = 0;
        s.rejected = 0;
        s.watch = 0;
        s.expired = 0;
        s.quotes = 0;
        s.engine_routes = 0;
        s.engine_visits = 0;
    }
}

pub async fn stop(state: &Arc<AppState>) {
    let mut s = state.scanner.write().await;
    s.state = "STOPPED".into();
    s.started_at = None;
    s.uptime_ms = 0;
}

/// M2 后台复核任务：独立于扫描循环，避免 REVM 分叉读取拖慢行情扫描。
///
/// 每隔 `m2.every_ms` 取当前净利最高的一条全 V3 机会，用报价区块状态重放 QuoterV2：
/// - 通过：升级为 M2 + VERIFIED，移除纯行情陈旧导致的质量拒绝；
/// - 不通过：保留 M1，追加 `m2_mismatch` 硬拒绝与证据；
/// - 结果回写内存、数据库并广播。
pub fn spawn_m2(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let every = state.config.m2.every_ms.max(5_000) as u64;
        loop {
            tokio::time::sleep(Duration::from_millis(every)).await;
            if !state.config.m2.enabled {
                continue;
            }
            let candidate = {
                let opps = state.opportunities.read().await;
                opps.iter()
                    .filter(|o| o.dex_route.as_ref().map(m2::is_v3_route).unwrap_or(false))
                    .max_by(|a, b| a.conservative_net_profit.cmp(&b.conservative_net_profit))
                    .cloned()
            };
            let Some(mut opp) = candidate else { continue };
            let tokens = state.market.read().await.tokens.clone();
            if tokens.is_empty() {
                continue;
            }
            let Some(route) = opp.dex_route.clone() else {
                continue;
            };
            let rpc = RpcClient::new(&state.config.m2.rpc, None);
            match m2::verify_route(&rpc, &route, &tokens, &state.config.m2).await {
                Ok(ev) => {
                    let diff = ev.diff_bps;
                    let passed = ev.passed;
                    let detail = ev.detail.clone();
                    state
                        .m2_evidence
                        .write()
                        .await
                        .insert(route.route_id.clone(), ev.clone());
                    opp.m2 = Some(ev);
                    if passed {
                        opp.model_level = ModelLevel::M2;
                        opp.data_quality = DataQuality::Verified;
                        opp.confidence = format!(
                            "M2 REVM 分叉复核通过（偏差 {:.2} bps）",
                            diff.to_f64().unwrap_or(0.0)
                        );
                        opp.rejections
                            .retain(|r| r.key != "data_quality" && r.key != "state_snapshot");
                        opp.status = if opp.rejections.iter().any(|r| {
                            matches!(r.severity, RiskSeverity::HardStop | RiskSeverity::SoftBlock)
                        }) {
                            OppStatus::Rejected
                        } else if opp.rejections.is_empty() {
                            OppStatus::Actionable
                        } else {
                            OppStatus::Watch
                        };
                    } else {
                        opp.rejections.push(RiskViolation {
                            key: "m2_mismatch".into(),
                            label: "M2 分叉复核".into(),
                            severity: RiskSeverity::HardStop,
                            limit: Decimal::from_f64(state.config.m2.tolerance_bps)
                                .unwrap_or(Decimal::from(50)),
                            actual: diff,
                            unit: "bps".into(),
                            message: detail.clone(),
                        });
                    }
                    {
                        let mut opps = state.opportunities.write().await;
                        match opps
                            .iter_mut()
                            .find(|o| o.opportunity_id == opp.opportunity_id)
                        {
                            Some(slot) => *slot = opp.clone(),
                            None => {
                                // 复核耗时较长，目标可能已被高频扫描淘汰；插回列表供下一轮继承。
                                opps.insert(0, opp.clone());
                                if opps.len() > 500 {
                                    opps.pop();
                                }
                            }
                        }
                    }
                    if let Err(e) = crate::db::repo::save_opportunity(&state.db, &opp) {
                        tracing::warn!("M2 结果落库失败: {e}");
                    }
                    state.broadcast(WsEvent::Opportunity {
                        opportunity: Box::new(opp.clone()),
                    });
                    tracing::info!(
                        "M2 复核 {}：passed={} diff={:.2}bps {}",
                        opp.opportunity_id,
                        passed,
                        diff.to_f64().unwrap_or(0.0),
                        detail
                    );
                }
                Err(e) => tracing::warn!("M2 复核失败: {e}"),
            }
        }
    })
}

/// 健康快照（供 `/system/health`）。
pub async fn health_json(state: &Arc<AppState>) -> serde_json::Value {
    // 只在收集快照期间持锁：本函数后面还要拿 scanner/stats/ledger 的锁，
    // 持锁跨越 await 会和 feed 的写锁、扫描的读锁互相排队，导致接口长时间挂起。
    let (books, feed_health, head_block, block_age_ms, gas_price_gwei, pools_len) = {
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
        (
            books,
            market.health.clone(),
            market.head_block,
            market.health.block_age_ms,
            market.gas_price_gwei,
            market.pools.len(),
        )
    };
    serde_json::json!({
        "db": state.db.health_nonblocking(),
        "db_path": state.db.path(),
        "feed": state.config.feed.mode,
        "feed_health": feed_health,
        "head_block": head_block,
        "block_age_ms": block_age_ms,
        "gas_price_gwei": gas_price_gwei,
        "pools": pools_len,
        "books": books,
        "model_policy": {
            "revm": true,
            "m2_available": crate::domain::types::M2_AVAILABLE,
            "max_model": "M2",
            "stress_model": "M3",
            "verified_reserved_for": "M2",
        },
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
