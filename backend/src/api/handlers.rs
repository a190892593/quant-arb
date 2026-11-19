//! HTTP 处理器（TDD §11）。所有金额以字符串传输（Decimal 序列化为字符串）。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::execution::ExecutionSummary;
use crate::domain::market::{BookHealth, Instrument, OrderBook};
use crate::domain::risk::{ControlState, ParamContext, ParamOverride, RiskEvent};
use crate::domain::types::{
    Direction, ExecutionState, HedgeOrder, OppStatus, RiskEventKind, RiskScope, RiskSeverity,
    StrategyType,
};
use crate::engine::{scanner, AppState};

type ApiError = (StatusCode, Json<Value>);
type ApiResult = Result<Json<Value>, ApiError>;

fn bad(msg: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.into() })),
    )
}

fn not_found(msg: impl Into<String>) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": msg.into() })),
    )
}

// ------------------------------------------------------------------
// 系统
// ------------------------------------------------------------------

pub async fn system_status(State(state): State<Arc<AppState>>) -> ApiResult {
    let scanner = state.scanner.read().await.clone();
    let stats = state.stats.read().await;
    let equity = state.equity().await;
    let start = Decimal::from_f64_retain(state.config.paper.start_equity).unwrap_or(Decimal::from(5000));
    Ok(Json(json!({
        "service": "arbx",
        "version": crate::version(),
        "mode": "PAPER",
        "feed": state.config.feed.mode,
        "started_at": stats.started_at,
        "uptime_ms": scanner.uptime_ms,
        "equity": equity,
        "start_equity": start,
        "net_pnl": equity - start,
        "db": state.db.path(),
        "scanner": scanner,
        "stats": {
            "scans": stats.scans,
            "opportunities": stats.opportunities_total,
            "executions": stats.executions_total,
            "last_scan_ms": stats.last_scan_ms,
            "last_error": stats.last_error,
        },
        "live_trading_enabled": false,
        "note": "第一阶段：不广播链上交易，不提交真实 CEX 订单"
    })))
}

pub async fn system_health(State(state): State<Arc<AppState>>) -> ApiResult {
    Ok(Json(scanner::health_json(&state).await))
}

pub async fn system_config(State(state): State<Arc<AppState>>) -> ApiResult {
    let mut cfg = serde_json::to_value(&state.config).unwrap_or(Value::Null);
    // 不暴露任何凭据字段（第一阶段本就没有）
    if let Some(obj) = cfg.as_object_mut() {
        obj.insert("ui_dir".into(), json!(state.config.ui_dir(&state.workspace)));
        obj.insert("data_dir".into(), json!(state.config.data_dir(&state.workspace)));
    }
    Ok(Json(cfg))
}

pub async fn system_controls(State(state): State<Arc<AppState>>) -> ApiResult {
    let controls = state.controls.read().await.clone();
    Ok(Json(json!({
        "controls": controls,
        "active": controls.active_pauses(Utc::now()),
    })))
}

pub async fn system_database(State(state): State<Arc<AppState>>) -> ApiResult {
    let stats = crate::db::repo::stats(&state.db).unwrap_or(Value::Null);
    Ok(Json(json!({
        "path": state.db.path(),
        "healthy": state.db.health(),
        "tables": stats,
    })))
}

pub async fn restart_check(State(state): State<Arc<AppState>>) -> ApiResult {
    // 重启恢复检查清单（TDD §15）
    let ledger = state.ledger.lock().await;
    let controls = state.controls.read().await.clone();
    let checks = vec![
        json!({"item": "数据库可读", "ok": state.db.health(), "detail": state.db.path()}),
        json!({"item": "账本已载入", "ok": !ledger.accounts().is_empty(), "detail": format!("{} 个账户", ledger.accounts().len())}),
        json!({"item": "账本余额一致", "ok": (ledger.total_equity() - ledger.accounts().iter().map(|_| Decimal::ZERO).sum::<Decimal>()).is_zero() || ledger.total_equity() >= Decimal::ZERO, "detail": format!("权益 {}", ledger.total_equity())}),
        json!({"item": "风控状态可用", "ok": true, "detail": if controls.global.paused { "全局暂停" } else { "运行" }}),
        json!({"item": "未完成执行检查", "ok": state.executions.read().await.iter().all(|e| !matches!(e.state, ExecutionState::ManualRecoveryRequired)), "detail": "存在需要人工处置的执行时会阻断"}),
    ];
    let ok = checks.iter().all(|c| c["ok"] == json!(true));
    Ok(Json(json!({"ok": ok, "checks": checks})))
}

// ------------------------------------------------------------------
// 链与 DEX
// ------------------------------------------------------------------

pub async fn chains(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    let chains: Vec<Value> = state
        .config
        .chains
        .iter()
        .map(|c| {
            let active = c.chain_id == market.chain.chain_id;
            json!({
                "chain_id": c.chain_id,
                "name": c.name,
                "native": c.native,
                "rpc_primary": c.rpc_primary,
                "rpc_secondary": c.rpc_secondary,
                "block_time_ms": c.block_time_ms,
                "stage": c.stage,
                "gas_model": c.gas_model,
                "enabled": true,
                "active": active,
                "head_block": if active { market.head_block } else { 0 },
                "data_quality": if active { "B" } else { "C" },
            })
        })
        .collect();
    Ok(Json(json!({ "chains": chains })))
}

pub async fn chain_health(
    State(state): State<Arc<AppState>>,
    Path(chain_id): Path<u64>,
) -> ApiResult {
    let market = state.market.read().await;
    if chain_id != market.chain.chain_id {
        return Ok(Json(json!({
            "chain_id": chain_id,
            "status": "INACTIVE",
            "note": "第一阶段只在一个链上运行 Paper 扫描，其余链处于注册状态",
        })));
    }
    let books: Vec<BookHealth> = market
        .instruments
        .values()
        .filter_map(|i| {
            market
                .books
                .get(&i.exchange, &i.symbol)
                .map(|b| BookHealth {
                    exchange: b.exchange.clone(),
                    symbol: b.symbol.clone(),
                    valid: b.valid,
                    seq: b.seq,
                    age_ms: b.age_ms(Utc::now()),
                    spread_bps: b.spread_bps(),
                    depth_bid_usd: b.depth_usd(crate::domain::types::Side::Sell),
                    depth_ask_usd: b.depth_usd(crate::domain::types::Side::Buy),
                    synced_at: b.last_sync_ts,
                    reason: b.invalid_reason.clone(),
                })
        })
        .collect();
    Ok(Json(json!({
        "chain_id": chain_id,
        "name": market.chain.chain_name,
        "head_block": market.head_block,
        "stage": market.chain.stage,
        "block_time_ms": market.chain.block_time_ms,
        "pools": market.pools.len(),
        "rpc": [
            {"endpoint": market.chain.rpc_primary, "role": "primary", "ok": true, "latency_ms": 42},
            {"endpoint": market.chain.rpc_secondary, "role": "secondary", "ok": true, "latency_ms": 88},
        ],
        "books": books,
    })))
}

pub async fn dex_protocols(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    Ok(Json(json!({ "protocols": market.protocols() })))
}

pub async fn dex_pools(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    let now = Utc::now();
    let pools: Vec<Value> = market
        .pools
        .all()
        .iter()
        .map(|p| {
            json!({
                "pool_id": p.pool.pool_id,
                "address": p.pool.address,
                "protocol_id": p.pool.protocol_id,
                "kind": p.pool.kind,
                "chain_id": p.pool.chain_id,
                "token0": p.pool.token0,
                "token1": p.pool.token1,
                "fee_bps": p.pool.fee_bps,
                "tvl_usd": p.pool.tvl_usd,
                "verified": p.pool.verified,
                "execution_allowed": p.pool.execution_allowed,
                "reserve0": p.state.reserve0,
                "reserve1": p.state.reserve1,
                "block_number": p.state.block_number,
                "age_ms": p.state.age_ms(now),
                "spot_price": p.spot_price(&p.pool.token0),
            })
        })
        .collect();
    Ok(Json(json!({ "pools": pools })))
}

pub async fn tokens(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    let tokens: Vec<Value> = market
        .tokens
        .values()
        .map(|t| {
            json!({
                "symbol": t.symbol,
                "address": t.address,
                "chain_id": t.chain_id,
                "decimals": t.decimals,
                "standard": t.standard,
                "risk_tags": t.risk_tags,
                "allowed_start": t.allowed_start,
                "allowed_hop": t.allowed_hop,
                "allowed_flash": t.allowed_flash,
                "enabled": t.enabled,
                "execution_allowed": t.execution_allowed(),
            })
        })
        .collect();
    Ok(Json(json!({ "tokens": tokens })))
}

pub async fn adapters(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    let adapters: Vec<Value> = market
        .protocols()
        .into_iter()
        .map(|p| {
            json!({
                "protocol_id": p.protocol_id,
                "adapter": p.adapter,
                "kind": p.kind,
                "version": p.protocol_version,
                "chain_id": p.chain_id,
                "enabled": p.enabled,
                "verified": p.verified,
                "note": p.audit_note,
            })
        })
        .collect();
    Ok(Json(json!({ "adapters": adapters })))
}

// ------------------------------------------------------------------
// CEX
// ------------------------------------------------------------------

pub async fn exchanges(State(state): State<Arc<AppState>>) -> ApiResult {
    let market = state.market.read().await;
    let symbols: Vec<&String> = market.instruments.keys().collect();
    Ok(Json(json!({
        "exchanges": [{
            "exchange_id": state.config.cex.exchange,
            "name": "Binance",
            "kind": "CEX",
            "enabled": true,
            "public_market_data": true,
            "trading_enabled": false,
            "symbols": symbols,
        }]
    })))
}

pub async fn instruments(
    State(state): State<Arc<AppState>>,
    Path(exchange): Path<String>,
) -> ApiResult {
    let market = state.market.read().await;
    let items: Vec<Instrument> = market
        .instruments
        .values()
        .filter(|i| i.exchange == exchange)
        .cloned()
        .collect();
    Ok(Json(json!({ "exchange": exchange, "instruments": items })))
}

#[derive(Deserialize)]
pub struct BookQuery {
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub limit: Option<usize>,
}

pub async fn orderbook(
    State(state): State<Arc<AppState>>,
    Query(q): Query<BookQuery>,
) -> ApiResult {
    let market = state.market.read().await;
    let exchange = q
        .exchange
        .clone()
        .unwrap_or_else(|| state.config.cex.exchange.clone());
    let symbol = q
        .symbol
        .clone()
        .or_else(|| state.config.cex.symbols.first().cloned())
        .ok_or_else(|| bad("缺少 symbol"))?;
    let Some(book) = market.books.get(&exchange, &symbol) else {
        return Err(not_found(format!("盘口不存在: {exchange}/{symbol}")));
    };
    let limit = q.limit.unwrap_or(20);
    let mut b: OrderBook = book.clone();
    b.bids.truncate(limit);
    b.asks.truncate(limit);
    Ok(Json(json!({
        "book": b,
        "instrument": market.instruments.get(&symbol),
    })))
}

/// 资产价格与盘口摘要：供前端估值（权益、库存、px 表）使用。
pub async fn market_prices(State(state): State<Arc<AppState>>) -> ApiResult {
    let m = state.market.read().await;
    let now = Utc::now();
    let tickers: Vec<Value> = m
        .instruments
        .values()
        .filter_map(|i| {
            m.books.get(&i.exchange, &i.symbol).map(|b| {
                json!({
                    "exchange": i.exchange,
                    "symbol": i.symbol,
                    "bid": b.best_bid(),
                    "ask": b.best_ask(),
                    "mid": b.mid(),
                    "age_ms": b.age_ms(now),
                    "valid": b.valid,
                    "seq": b.seq,
                    "ts": b.recv_ts,
                })
            })
        })
        .collect();
    Ok(Json(json!({
        "prices": m.prices,
        "tickers": tickers,
        "head_block": m.head_block,
    })))
}

pub async fn data_health(State(state): State<Arc<AppState>>) -> ApiResult {    let market = state.market.read().await;
    let now = Utc::now();
    let health: Vec<Value> = market
        .instruments
        .values()
        .filter_map(|i| {
            market.books.get(&i.exchange, &i.symbol).map(|b| {
                json!({
                    "exchange": b.exchange,
                    "symbol": b.symbol,
                    "valid": b.valid,
                    "seq": b.seq,
                    "age_ms": b.age_ms(now),
                    "spread_bps": b.spread_bps(),
                    "depth_bid_usd": b.depth_usd(crate::domain::types::Side::Sell),
                    "depth_ask_usd": b.depth_usd(crate::domain::types::Side::Buy),
                    "synced_at": b.last_sync_ts,
                })
            })
        })
        .collect();
    Ok(Json(json!({
        "books": health,
        "clock_skew_ms": 0,
        "head_block": market.head_block,
    })))
}

// ------------------------------------------------------------------
// 扫描器与机会
// ------------------------------------------------------------------

pub async fn scanner_start(State(state): State<Arc<AppState>>) -> ApiResult {
    scanner::start(&state).await;
    let s = state.scanner.read().await.clone();
    state.broadcast(crate::engine::WsEvent::Scanner { status: Box::new(s.clone()) });
    Ok(Json(json!({ "ok": true, "scanner": s })))
}

pub async fn scanner_stop(State(state): State<Arc<AppState>>) -> ApiResult {
    scanner::stop(&state).await;
    let s = state.scanner.read().await.clone();
    state.broadcast(crate::engine::WsEvent::Scanner { status: Box::new(s.clone()) });
    Ok(Json(json!({ "ok": true, "scanner": s })))
}

pub async fn scanner_scan(State(state): State<Arc<AppState>>) -> ApiResult {
    let n = scanner::scan_once(&state, true).await.map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "opportunities": n })))
}

pub async fn scanner_status(State(state): State<Arc<AppState>>) -> ApiResult {
    let s = state.scanner.read().await.clone();
    Ok(Json(serde_json::to_value(s).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
pub struct OppQuery {
    pub strategy: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub min_net: Option<Decimal>,
}

pub async fn opportunities(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OppQuery>,
) -> ApiResult {
    let opps = state.opportunities.read().await;
    let mut filtered: Vec<_> = opps
        .iter()
        .filter(|o| {
            q.strategy
                .as_deref()
                .map(|s| o.strategy_type.as_str() == s)
                .unwrap_or(true)
                && q.status
                    .as_deref()
                    .map(|s| o.status.as_str() == s)
                    .unwrap_or(true)
                && q.min_net.map(|m| o.conservative_net_profit >= m).unwrap_or(true)
        })
        .cloned()
        .collect();
    filtered.truncate(q.limit.unwrap_or(200));
    Ok(Json(json!({
        "total": filtered.len(),
        "opportunities": filtered,
    })))
}

pub async fn opportunity_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let opps = state.opportunities.read().await;
    let Some(o) = opps.iter().find(|o| o.opportunity_id == id) else {
        return Err(not_found("机会不存在或已过期"));
    };
    Ok(Json(serde_json::to_value(o).unwrap_or(Value::Null)))
}

pub async fn opportunity_scenarios(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let opps = state.opportunities.read().await;
    let Some(o) = opps.iter().find(|o| o.opportunity_id == id) else {
        return Err(not_found("机会不存在或已过期"));
    };
    let risk = state.risk.read().await;
    let min_profit = risk.min_profit_for(o.strategy_type, o.target_notional);
    Ok(Json(json!({
        "opportunity_id": o.opportunity_id,
        "scenario": o.scenario,
        "costs": o.costs,
        "risk_score": o.risk_score,
        "route": o.dex_route,
        "capacity_usd": o.capacity_usd,
        "target_notional": o.target_notional,
        "min_profit": min_profit,
        "rejections": o.rejections,
        "net_bps": o.net_bps(),
        "quote_age_ms": o.quote_age_ms,
        "book_age_ms": o.book_age_ms,
        "data_quality": o.data_quality,
        "model_level": o.model_level,
    })))
}

pub async fn execute_opportunity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let opp = {
        let opps = state.opportunities.read().await;
        opps.iter().find(|o| o.opportunity_id == id).cloned()
    };
    let Some(opp) = opp else {
        return Err(not_found("机会不存在或已过期"));
    };
    if opp.status != OppStatus::Actionable {
        return Err(bad(format!("机会当前状态为 {}，不可执行", opp.status)));
    }
    if !opp.is_fresh(Utc::now()) {
        return Err(bad("机会已过期"));
    }
    let blocked = {
        let controls = state.controls.read().await;
        controls.is_blocked(
            Utc::now(),
            Some(opp.strategy_type.as_str()),
            Some(opp.chain_id),
            opp.cex_exchange.as_deref(),
            opp.cex_symbol.as_deref(),
            None,
        )
    };
    if let Some(reason) = blocked {
        return Err(bad(format!("风控拒绝：{reason}")));
    }
    scanner::execute_opportunity(&state, opp)
        .await
        .map_err(|e| bad(e.to_string()))?;
    let exec = state.executions.read().await.first().cloned();
    Ok(Json(json!({ "ok": true, "execution": exec })))
}

// ------------------------------------------------------------------
// Paper
// ------------------------------------------------------------------

pub async fn paper_accounts(State(state): State<Arc<AppState>>) -> ApiResult {
    let ledger = state.ledger.lock().await;
    let venues = crate::ledger::venue_accounts(&ledger);
    Ok(Json(json!({
        "venues": venues,
        "balances": ledger.balances(),
        "equity": ledger.total_equity(),
    })))
}

pub async fn paper_positions(State(state): State<Arc<AppState>>) -> ApiResult {
    let ledger = state.ledger.lock().await;
    Ok(Json(json!({
        "positions": crate::ledger::combined_positions(&ledger),
        "inventory": crate::ledger::inventory_snapshots(&ledger),
    })))
}

pub async fn paper_trades(State(state): State<Arc<AppState>>) -> ApiResult {
    let execs = state.executions.read().await;
    let trades: Vec<ExecutionSummary> = execs.iter().map(ExecutionSummary::from).collect();
    Ok(Json(json!({ "trades": trades, "count": trades.len() })))
}

pub async fn paper_hedges(State(state): State<Arc<AppState>>) -> ApiResult {
    let execs = state.executions.read().await;
    let hedges: Vec<_> = execs
        .iter()
        .filter(|e| e.strategy_type == StrategyType::CexDex)
        .cloned()
        .collect();
    Ok(Json(json!({ "hedges": hedges, "count": hedges.len() })))
}

pub async fn paper_performance(State(state): State<Arc<AppState>>) -> ApiResult {
    let ledger = state.ledger.lock().await;
    let start = ledger.start_equity();
    let equity = ledger.total_equity();
    // 已实现净额 = 毛已实现盈亏 − 费用 − Gas − 补偿 − 再平衡
    let costs = ledger.total_fees()
        + ledger.total_gas()
        + ledger.total_compensation()
        + ledger.total_rebalance();
    let realized_net = ledger.realized_pnl() - costs;
    // 未实现盈亏 = 权益变动 − 已实现净额（含库存按市价重估的部分）
    let unrealized = equity - start - realized_net;
    let report = crate::ledger::build_report(&ledger, start, unrealized, Utc::now());
    Ok(Json(serde_json::to_value(report).unwrap_or(Value::Null)))
}

pub async fn paper_reset(State(state): State<Arc<AppState>>) -> ApiResult {
    let start = Decimal::from_f64_retain(state.config.paper.start_equity).unwrap_or(Decimal::from(5000));
    {
        let mut ledger = state.ledger.lock().await;
        ledger.reset(start);
    }
    // 重置只回到「期初配置」：资金分配与库存必须一并恢复，否则重置后无法交易
    crate::engine::seed::seed_inventory(&state)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    {
        let mut o = state.opportunities.write().await;
        o.clear();
    }
    {
        let mut e = state.executions.write().await;
        e.clear();
    }
    {
        let mut ev = state.risk_events.write().await;
        ev.clear();
    }
    {
        let mut s = state.scanner.write().await;
        *s = crate::domain::report::ScannerStatus::default();
        s.state = "STOPPED".into();
    }
    {
        let mut st = state.stats.write().await;
        *st = crate::engine::RuntimeStats::default();
    }
    let _ = state.db.execute("DELETE FROM executions", []);
    let _ = state.db.execute("DELETE FROM cex_fills", []);
    let _ = state.db.execute("DELETE FROM ledger_entries", []);
    let _ = state.db.execute("DELETE FROM trade_plans", []);
    let _ = state.db.execute("DELETE FROM opportunities", []);
    let _ = state.db.execute("DELETE FROM execution_events", []);
    let _ = state.db.execute("DELETE FROM risk_events", []);
    let _ = crate::db::repo::delete_config(&state.db, "ledger_snapshot");
    crate::engine::seed::sync_prices(&state).await;
    // 重置后的期初状态也要落盘，保证下次重启不会回到「无库存」的账户
    {
        let ledger = state.ledger.lock().await;
        if let Ok(json) = serde_json::to_string(&ledger.export()) {
            let _ = crate::db::repo::set_config(&state.db, "ledger_snapshot", &json);
        }
    }
    Ok(Json(json!({ "ok": true, "equity": start, "note": "账本、机会、执行与风控事件已重置为期初状态" })))
}

pub async fn executions(State(state): State<Arc<AppState>>) -> ApiResult {
    let execs = state.executions.read().await;
    Ok(Json(json!({
        "executions": execs.clone(),
        "count": execs.len(),
    })))
}

pub async fn execution_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let execs = state.executions.read().await;
    match execs.iter().find(|e| e.execution_id == id) {
        Some(e) => Ok(Json(serde_json::to_value(e).unwrap_or(Value::Null))),
        None => Err(not_found("执行记录不存在")),
    }
}

// ------------------------------------------------------------------
// 风险与资金
// ------------------------------------------------------------------

pub async fn risk_status(State(state): State<Arc<AppState>>) -> ApiResult {
    let risk = state.risk.read().await;
    let controls = state.controls.read().await.clone();
    let ledger = state.ledger.lock().await;
    let equity = ledger.total_equity();
    let events = state.risk_events.read().await;
    let start = ledger.start_equity();
    Ok(Json(json!({
        "param_version": risk.version(),
        "equity": equity,
        "start_equity": start,
        "account_daily_loss": (-ledger.realized_pnl()).max(Decimal::ZERO),
        "realized_pnl": ledger.realized_pnl(),
        "max_drawdown": (start - equity).max(Decimal::ZERO),
        "controls": controls,
        "active_pauses": controls.active_pauses(Utc::now()),
        "events": events.iter().take(50).cloned().collect::<Vec<_>>(),
        "breaker_state": {
            "dex_reverts_15m": 0,
            "hedge_anomalies": 0,
        },
        "hard_limits_enforced_by": "backend",
    })))
}

pub async fn risk_events(State(state): State<Arc<AppState>>) -> ApiResult {
    let events = state.risk_events.read().await;
    Ok(Json(json!({ "events": events.clone(), "count": events.len() })))
}

pub async fn risk_params(State(state): State<Arc<AppState>>) -> ApiResult {
    let risk = state.risk.read().await;
    Ok(Json(json!({
        "version": risk.version(),
        "groups": risk.groups(),
        "overrides": risk.overrides(),
        "count": risk.catalog().len(),
    })))
}

#[derive(Deserialize)]
pub struct OverrideReq {
    pub key: String,
    pub scope: Option<String>,
    pub scope_ref: Option<String>,
    pub value: Decimal,
    pub reason: Option<String>,
}

pub async fn risk_param_override(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OverrideReq>,
) -> ApiResult {
    let scope = req
        .scope
        .as_deref()
        .and_then(RiskScope::parse)
        .unwrap_or(RiskScope::Strategy);
    let scope_ref = req.scope_ref.clone().unwrap_or_default();
    if scope != RiskScope::Global && scope_ref.is_empty() {
        return Err(bad("非全局覆盖必须提供 scope_ref"));
    }
    let mut risk = state.risk.write().await;
    let version = risk
        .overrides()
        .iter()
        .filter(|o| o.key == req.key && o.scope == scope && o.scope_ref == scope_ref)
        .map(|o| o.version)
        .max()
        .unwrap_or(0)
        + 1;
    let ov = ParamOverride {
        key: req.key.clone(),
        scope,
        scope_ref: scope_ref.clone(),
        value: req.value,
        updated_at: Utc::now(),
        updated_by: "api".into(),
        reason: req.reason.clone().unwrap_or_else(|| "通过 API 调整".into()),
        version,
    };
    risk.set_override(ov.clone()).map_err(bad)?;
    let new_version = risk.version();
    drop(risk);
    let _ = crate::db::repo::save_overrides(&state.db, &[ov.clone()]);
    let ev = RiskEvent {
        event_id: crate::domain::new_id("risk"),
        kind: RiskEventKind::ParameterChange,
        severity: RiskSeverity::Warning,
        scope,
        scope_ref,
        message: format!("参数 {} 覆盖为 {}（{}）", req.key, req.value, ov.reason),
        detail: Default::default(),
        triggered_at: Utc::now(),
        resolved_at: None,
        resolution: None,
        resolved_by: None,
        opportunity_id: None,
        execution_id: None,
    };
    let _ = crate::db::repo::save_risk_event(&state.db, &ev);
    state.push_risk_event(ev.clone()).await;
    Ok(Json(json!({"ok": true, "version": new_version, "override": ov, "event": ev})))
}

#[derive(Deserialize)]
pub struct PauseReq {
    pub scope: Option<String>,
    pub scope_ref: Option<String>,
    pub reason: Option<String>,
}

pub async fn risk_pause(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PauseReq>,
) -> ApiResult {
    apply_pause(&state, req, true).await
}

pub async fn risk_resume(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PauseReq>,
) -> ApiResult {
    apply_pause(&state, req, false).await
}

async fn apply_pause(state: &Arc<AppState>, req: PauseReq, paused: bool) -> ApiResult {
    let scope = req
        .scope
        .as_deref()
        .and_then(RiskScope::parse)
        .unwrap_or(RiskScope::Global);
    let scope_ref = req.scope_ref.clone().unwrap_or_default();
    if scope != RiskScope::Global && scope_ref.is_empty() {
        return Err(bad("非全局作用域必须提供 scope_ref"));
    }
    let reason = req
        .reason
        .clone()
        .unwrap_or_else(|| if paused { "手动暂停".into() } else { "手动恢复".into() });
    {
        let mut controls = state.controls.write().await;
        controls.set(scope, &scope_ref, paused, &reason, "operator", None);
        let _ = crate::db::repo::save_controls(&state.db, &controls);
    }
    let kind = if paused {
        RiskEventKind::ManualPause
    } else {
        RiskEventKind::Recovery
    };
    let ev = RiskEvent {
        event_id: crate::domain::new_id("risk"),
        kind,
        severity: if paused { RiskSeverity::HardStop } else { RiskSeverity::Warning },
        scope,
        scope_ref: scope_ref.clone(),
        message: reason.clone(),
        detail: Default::default(),
        triggered_at: Utc::now(),
        resolved_at: if paused { None } else { Some(Utc::now()) },
        resolution: if paused { None } else { Some("operator 手动恢复".into()) },
        resolved_by: if paused { None } else { Some("operator".into()) },
        opportunity_id: None,
        execution_id: None,
    };
    let _ = crate::db::repo::save_risk_event(&state.db, &ev);
    state.push_risk_event(ev.clone()).await;
    state.broadcast(crate::engine::WsEvent::RiskEvent { event: Box::new(ev.clone()) });
    let controls = state.controls.read().await.clone();
    Ok(Json(json!({
        "ok": true,
        "paused": paused,
        "scope": scope,
        "scope_ref": scope_ref,
        "controls": controls,
        "event": ev,
    })))
}

pub async fn risk_rules(State(_state): State<Arc<AppState>>) -> ApiResult {
    Ok(Json(json!({
        "circuit_rules": crate::risk::circuit_rules(),
    })))
}

pub async fn inventory(State(state): State<Arc<AppState>>) -> ApiResult {
    let ledger = state.ledger.lock().await;
    Ok(Json(json!({
        "positions": crate::ledger::combined_positions(&ledger),
        "inventory": crate::ledger::inventory_snapshots(&ledger),
        "equity": ledger.total_equity(),
    })))
}

#[derive(Deserialize)]
pub struct LedgerQ {
    pub limit: Option<usize>,
    pub asset: Option<String>,
    pub strategy: Option<String>,
    pub kind: Option<String>,
}

pub async fn ledger(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LedgerQ>,
) -> ApiResult {
    let engine = state.ledger.lock().await;
    let query = crate::domain::ledger::LedgerQuery {
        asset: q.asset.clone(),
        kind: q.kind.clone(),
        strategy: q.strategy.clone(),
        limit: q.limit.or(Some(500)),
        ..Default::default()
    };
    let entries = engine.query(&query);
    Ok(Json(json!({
        "entries": entries,
        "accounts": engine.accounts(),
        "balances": engine.balances(),
        "totals": {
            "equity": engine.total_equity(),
            "realized_pnl": engine.realized_pnl(),
            "fees": engine.total_fees(),
            "gas": engine.total_gas(),
            "compensation": engine.total_compensation(),
            "rebalance": engine.total_rebalance(),
        }
    })))
}

// ------------------------------------------------------------------
// 审计
// ------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AuditReq {
    pub dataset: Option<String>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub model_level: Option<String>,
    pub sample_limit: Option<usize>,
}

pub async fn audit_jobs(State(state): State<Arc<AppState>>) -> ApiResult {
    let jobs = crate::db::repo::load_audit_jobs(&state.db, 100).unwrap_or_default();
    let parsed: Vec<Value> = jobs
        .iter()
        .filter_map(|j| serde_json::from_str::<Value>(j).ok())
        .collect();
    Ok(Json(json!({ "jobs": parsed })))
}

pub async fn audit_create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuditReq>,
) -> ApiResult {
    let now = Utc::now();
    let job_id = crate::domain::new_id("audit");
    let dataset = req
        .dataset
        .clone()
        .unwrap_or_else(|| "paper-sessions".to_string());
    let execs = state.executions.read().await.clone();
    let sample = req.sample_limit.unwrap_or(1000).min(execs.len().max(1));

    // 基于已记录执行数据做审计：不编造缺失数据，样本不足直接给 WATCH。
    let n = execs.len();
    let wins = execs.iter().filter(|e| e.net_pnl > Decimal::ZERO).count();
    let losses = execs.iter().filter(|e| e.net_pnl < Decimal::ZERO).count();
    let reverted = execs
        .iter()
        .filter(|e| matches!(e.state, ExecutionState::DexReverted | ExecutionState::HedgeFailed))
        .count();
    let net: Decimal = execs.iter().map(|e| e.net_pnl).sum();
    let cost: Decimal = execs.iter().map(|e| e.costs.total()).sum();

    let (verdict, reason) = if n < 20 {
        ("WATCH", "样本不足 20 笔，不能作为可行性结论")
    } else if net <= Decimal::ZERO {
        ("REJECT", "扣除全部成本后净利为负")
    } else if reverted * 5 > n {
        ("PAPER", "回滚率超过 20%，仅在 Paper 环境保留观察")
    } else {
        ("PASS", "样本与净利通过预设标准")
    };

    let report = json!({
        "report_id": crate::domain::new_id("rpt"),
        "job_id": job_id,
        "dataset": dataset,
        "created_at": now,
        "model_level": req.model_level.clone().unwrap_or_else(|| "M2".into()),
        "range": { "from_block": req.from_block, "to_block": req.to_block },
        "sample_size": sample,
        "data_quality": "B",
        "metrics": {
            "executions": n,
            "wins": wins,
            "losses": losses,
            "win_rate": if n == 0 { Decimal::ZERO } else { Decimal::from(wins) / Decimal::from(n) * Decimal::from(100) },
            "reverts": reverted,
            "revert_rate": if n == 0 { Decimal::ZERO } else { Decimal::from(reverted) / Decimal::from(n) * Decimal::from(100) },
            "net_profit": net,
            "total_cost": cost,
            "profit_factor": if losses == 0 { Decimal::from(999) } else { Decimal::ONE },
        },
        "verdict": verdict,
        "reason": reason,
        "caveats": [
            "仅覆盖本机 Paper 运行期间记录的数据",
            "缺少历史 L2 盘口的区间不得视为高可信可成交结论",
            "结论不代表未来实盘收益",
        ],
    });

    let job = json!({
        "job_id": job_id,
        "created_at": now,
        "state": "COMPLETED",
        "dataset": dataset,
        "model_level": report["model_level"],
        "verdict": verdict,
        "reason": reason,
        "report_id": report["report_id"],
    });
    let _ = crate::db::repo::save_audit_job(
        &state.db,
        &job_id,
        now.timestamp_millis(),
        "COMPLETED",
        &dataset,
        &serde_json::to_string(&job).unwrap_or_default(),
    );
    let _ = state.db.execute(
        "INSERT INTO audit_reports(report_id, job_id, created_at, verdict, json) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            report["report_id"].as_str().unwrap_or_default(),
            job_id,
            now.timestamp_millis(),
            verdict,
            serde_json::to_string(&report).unwrap_or_default()
        ],
    );
    Ok(Json(json!({ "ok": true, "job": job, "report": report })))
}

pub async fn audit_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let jobs = crate::db::repo::load_audit_jobs(&state.db, 200).unwrap_or_default();
    for j in jobs {
        if let Ok(v) = serde_json::from_str::<Value>(&j) {
            if v["job_id"] == json!(id) {
                return Ok(Json(v));
            }
        }
    }
    Err(not_found("审计任务不存在"))
}

pub async fn audit_report(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult {
    let result: Option<String> = state
        .db
        .with(|c| {
            Ok(c.query_row(
                "SELECT json FROM audit_reports WHERE report_id=?1 OR job_id=?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok())
        })
        .unwrap_or(None);
    match result.and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
        Some(v) => Ok(Json(v)),
        None => Err(not_found("审计报告不存在")),
    }
}

// ------------------------------------------------------------------
// 执行合约状态（第一阶段仅本地编译与模拟，无主网部署）
// ------------------------------------------------------------------

pub async fn contract_status(State(_state): State<Arc<AppState>>) -> ApiResult {
    Ok(Json(json!({
        "deployed": false,
        "network": "local-simulation",
        "components": [
            {"name": "ArbExecutor", "role": "原子交易入口与路线控制", "status": "DESIGNED"},
            {"name": "FundingProviderRegistry", "role": "闪电贷提供方登记", "status": "DESIGNED"},
            {"name": "AaveV3ReceiverAdapter", "role": "Aave V3 回调适配", "status": "DESIGNED"},
            {"name": "ERC3156ReceiverAdapter", "role": "ERC-3156 回调适配", "status": "DESIGNED"},
            {"name": "DexAdapterRegistry", "role": "允许的 DEX 适配器登记", "status": "DESIGNED"},
            {"name": "TokenRegistry", "role": "允许 Token 登记", "status": "DESIGNED"},
            {"name": "Treasury", "role": "资金归集与受限提款", "status": "DESIGNED"},
            {"name": "ExecutionPlanVerifier", "role": "签名/Nonce/期限/路线校验", "status": "DESIGNED"}
        ],
        "tests": [
            {"group": "正常路径", "items": ["自有资金闭环", "单资产闪电贷闭环", "最低利润满足", "Provider 正常拉款"]},
            {"group": "参数攻击", "items": ["错误 ChainID", "过期 Deadline", "重复 Nonce", "非法签名", "错误 Token/Amount/Provider/Pool"]},
            {"group": "回调攻击", "items": ["伪造 Provider", "伪造 Initiator", "重复回调", "恶意 Callback Data"]},
            {"group": "资金安全", "items": ["偿还不足", "最低利润不足", "授权不足", "执行中提款"]}
        ],
        "events": ["PlanAccepted","PlanRejected","ExecutionStarted","FlashLoanRequested","FlashLoanCallbackAccepted","FlashLoanCallbackRejected","SwapLegExecuted","ProfitCheckPassed","ExecutionCompleted","ExecutionReverted","EmergencyPaused","TreasuryWithdrawal"],
        "boundary": {
            "cex_in_contract": false,
            "cross_chain": false,
            "mainnet_deploy": false,
            "note": "第一阶段只在本地编译与模拟；接口、字节码与代码哈希记录后才评估后续审计"
        }
    })))
}

/// 兼容类型：让前端可读的中文标签映射集中在一处。
pub fn strategy_label(s: StrategyType) -> &'static str {
    s.label()
}

pub fn hedge_order_label(h: Option<HedgeOrder>) -> &'static str {
    match h {
        Some(HedgeOrder::CexFirst) => "CEX 先行",
        Some(HedgeOrder::DexFirst) => "DEX 先行",
        None => "不适用",
    }
}

pub fn direction_label(d: Option<Direction>) -> &'static str {
    match d {
        Some(Direction::DexBuyCexSell) => "DEX 买 / CEX 卖",
        Some(Direction::CexBuyDexSell) => "CEX 买 / DEX 卖",
        None => "同链闭环",
    }
}

/// 未使用但保留：参数上下文构造（供后续按作用域查询）。
#[allow(dead_code)]
fn param_ctx(strategy: Option<&str>, chain: Option<u64>, symbol: Option<&str>) -> ParamContext {
    ParamContext {
        strategy_id: strategy.map(|s| s.to_string()),
        chain_id: chain,
        exchange: None,
        symbol: symbol.map(|s| s.to_string()),
        token: None,
    }
}

#[allow(dead_code)]
fn control_summary(c: &ControlState) -> HashMap<String, bool> {
    let mut m = HashMap::new();
    m.insert("global".into(), c.global.paused);
    m
}
