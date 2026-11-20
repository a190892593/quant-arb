//! Axum HTTP + WebSocket 服务（TDD §11）。
//!
//! 所有数据接口挂在 `/api` 前缀下，避免与前端静态文件冲突；
//! `/ws` 推送机会、执行、风控、盘口与账本事件。

pub mod handlers;
pub mod ws;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::engine::AppState;

/// 构建完整路由。
pub fn router(state: Arc<AppState>) -> Router {
    let ui_dir = state.config.ui_dir(&state.workspace);
    let index = ui_dir.join("index.html");

    let api: Router<Arc<AppState>> = Router::new()
        // ---- 系统 ----
        .route("/system/status", get(handlers::system_status))
        .route("/system/health", get(handlers::system_health))
        .route("/system/config", get(handlers::system_config))
.route("/system/prices", get(handlers::system_prices))
        .route("/system/controls", get(handlers::system_controls))
        .route("/system/database", get(handlers::system_database))
        .route("/system/restart-check", get(handlers::restart_check))
        .route(
            "/search/config",
            get(handlers::search_config_get).put(handlers::search_config_put),
        )
        .route("/search/tokens", post(handlers::search_tokens_post))
        .route(
            "/sources",
            get(handlers::sources_get).put(handlers::sources_put),
        )
        .route("/sources/preset", post(handlers::sources_preset))
        .route("/discovery/status", get(handlers::discovery_status))
        .route("/discovery/scan", post(handlers::discovery_scan))
        // ---- 链与 DEX ----
        .route("/chains", get(handlers::chains))
        .route("/chains/{chain_id}/health", get(handlers::chain_health))
        .route("/dex/protocols", get(handlers::dex_protocols))
        .route("/dex/pools", get(handlers::dex_pools))
        .route("/tokens", get(handlers::tokens))
        .route("/adapters", get(handlers::adapters))
        // ---- CEX ----
        .route("/exchanges", get(handlers::exchanges))
        .route("/exchanges/{exchange}/instruments", get(handlers::instruments))
        .route("/markets/orderbook", get(handlers::orderbook))
        .route("/markets/prices", get(handlers::market_prices))
        .route("/markets/data-health", get(handlers::data_health))
        // ---- 扫描与机会 ----
        .route("/scanner/start", post(handlers::scanner_start))
        .route("/scanner/stop", post(handlers::scanner_stop))
        .route("/scanner/scan", post(handlers::scanner_scan))
        .route("/scanner/status", get(handlers::scanner_status))
        .route("/scanner/config", get(handlers::scanner_config))
        .route("/scanner/engine", get(handlers::scanner_engine))
        .route("/scanner/restart", post(handlers::scanner_restart))
        .route("/scanner/runs", get(handlers::scanner_runs))
        // ---- DEX 多池多跳引擎（PRD §4–§7）----
        .route("/routes", get(handlers::dex_routes))
        .route("/routes/{id}", get(handlers::dex_route_detail))
        .route("/dex/graph", get(handlers::dex_graph))
        .route(
            "/simulations",
            get(handlers::simulations).post(handlers::create_simulation),
        )
        .route("/simulations/{id}", get(handlers::simulation_detail))
        .route("/opportunities", get(handlers::opportunities))
        .route("/opportunities/{id}", get(handlers::opportunity_detail))
        .route(
            "/opportunities/{id}/scenarios",
            get(handlers::opportunity_scenarios),
        )
        .route(
            "/opportunities/{id}/execute",
            post(handlers::execute_opportunity),
        )
        // ---- Paper ----
        .route("/paper/accounts", get(handlers::paper_accounts))
        .route("/paper/positions", get(handlers::paper_positions))
        .route("/paper/trades", get(handlers::paper_trades))
        .route("/paper/hedges", get(handlers::paper_hedges))
        .route("/paper/performance", get(handlers::paper_performance))
        .route("/paper/reset", post(handlers::paper_reset))
        .route("/executions", get(handlers::executions))
        .route("/executions/{id}", get(handlers::execution_detail))
        // ---- 风险与资金 ----
        .route("/risk/status", get(handlers::risk_status))
        .route("/risk/events", get(handlers::risk_events))
        .route("/risk/params", get(handlers::risk_params))
        .route("/risk/params/override", post(handlers::risk_param_override))
        .route("/risk/pause", post(handlers::risk_pause))
        .route("/risk/resume", post(handlers::risk_resume))
        .route("/risk/rules", get(handlers::risk_rules))
        .route("/inventory", get(handlers::inventory))
        .route("/ledger", get(handlers::ledger))
        // ---- 审计 ----
        .route(
            "/audit/jobs",
            get(handlers::audit_jobs).post(handlers::audit_create),
        )
        .route("/audit/jobs/{id}", get(handlers::audit_job))
        .route("/audit/reports/{id}", get(handlers::audit_report))
        // ---- 合约 ----
        .route("/contract/status", get(handlers::contract_status));

    // PRD §15 的 v1 契约：复用同一批处理器，避免两套逻辑漂移。
    let v1: Router<Arc<AppState>> = Router::new()
        .route("/system/status", get(handlers::system_status))
        .route("/chains", get(handlers::chains))
        .route("/dex/protocols", get(handlers::dex_protocols))
        .route("/tokens", get(handlers::tokens))
        .route("/pools", get(handlers::dex_pools))
        .route("/scanner/config", get(handlers::scanner_config))
        .route("/scanner/start", post(handlers::scanner_start))
        .route("/scanner/stop", post(handlers::scanner_stop))
        .route("/scanner/restart", post(handlers::scanner_restart))
        .route("/routes", get(handlers::dex_routes))
        .route("/routes/{id}", get(handlers::dex_route_detail))
        .route("/opportunities", get(handlers::opportunities))
        .route("/opportunities/{id}", get(handlers::opportunity_detail))
        .route(
            "/simulations",
            get(handlers::simulations).post(handlers::create_simulation),
        )
        .route("/simulations/{id}", get(handlers::simulation_detail))
        .route(
            "/audits",
            get(handlers::audit_jobs).post(handlers::audit_create),
        )
        .route("/audits/{id}", get(handlers::audit_job))
        .route("/paper/accounts", get(handlers::paper_accounts))
        .route("/paper/trades", get(handlers::paper_trades))
        .route(
            "/risk/rules",
            get(handlers::risk_rules).put(handlers::risk_param_override),
        );
    let api = api.nest("/v1", v1);

    let mut app: Router<Arc<AppState>> = Router::new()
        .nest("/api", api)
        .route("/ws", get(ws::ws_handler))
        .route("/healthz", get(|| async { "ok" }));

    if ui_dir.exists() {
        app = app.fallback_service(
            ServeDir::new(&ui_dir).not_found_service(ServeFile::new(index)),
        );
    } else {
        app = app.route(
            "/",
            get(|| async { "ARBX API 正在运行（未找到 web/ 静态目录）" }),
        );
    }

    if state.config.server.cors {
        app = app.layer(CorsLayer::permissive());
    }
    app.layer(TraceLayer::new_for_http()).with_state(state)
}
