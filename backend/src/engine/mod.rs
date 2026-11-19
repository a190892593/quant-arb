//! 应用状态与后台任务编排。
//!
//! 第一阶段把 API、扫描器、Paper 执行与账本放在同一个进程里（TDD §2.1 允许），
//! 通过 Tokio 任务与内存队列传递事件；历史审计保留独立 CLI 入口。

pub mod scanner;
pub mod seed;

/// 账本中的场所标识（与 LedgerEngine 的默认账户命名保持一致）。
pub const DEX_VENUE: &str = "dex-arb";
pub const CEX_VENUE: &str = "cex-binance";

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::config::Config;
use crate::db::Db;
use crate::domain::dex_scan::{
    GraphStats, RouteScanRecord, ScanRunStats, SimulationOutcome,
};
use crate::domain::execution::ExecutionRecord;
use crate::domain::market::{Instrument, Token};
use crate::domain::opportunity::Opportunity;
use crate::domain::report::ScannerStatus;
use crate::domain::risk::{ControlState, RiskEvent};
use crate::domain::types::StrategyType;
use crate::feed::demo::DemoMarket;
use crate::ledger::LedgerEngine;
use crate::paper::{PaperEngine, PaperParams};
use crate::risk::RiskEngine;
use crate::strategy::dex_scan::{DexScanParams, MultiHopCache};
use crate::strategy::StrategyEngine;

/// 实时事件（WebSocket 推送）。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    Opportunity { opportunity: Box<Opportunity> },
    Execution { execution: Box<ExecutionRecord> },
    RiskEvent { event: Box<RiskEvent> },
    Scanner { status: Box<ScannerStatus> },
    Route { route: Box<RouteScanRecord> },
    Graph { stats: Box<GraphStats> },
    Simulation { simulation: Box<SimulationOutcome> },
    Book { exchange: String, symbol: String, bid: Decimal, ask: Decimal, age_ms: i64 },
    Ledger { entry_id: String, account_id: String, asset: String, amount: Decimal },
    System { message: String },
}

/// 运行时统计。
#[derive(Clone, Debug, Default)]
pub struct RuntimeStats {
    pub scans: u64,
    pub opportunities_total: u64,
    pub executions_total: u64,
    pub started_at: Option<DateTime<Utc>>,
    pub last_scan_ms: u64,
    pub last_error: Option<String>,
}

/// 全局应用状态。
pub struct AppState {
    pub config: Config,
    pub workspace: std::path::PathBuf,
    pub db: Db,
    pub risk: Arc<RwLock<RiskEngine>>,
    pub ledger: Arc<Mutex<LedgerEngine>>,
    pub market: Arc<RwLock<DemoMarket>>,
    pub scanner: Arc<RwLock<ScannerStatus>>,
    pub opportunities: Arc<RwLock<Vec<Opportunity>>>,
    pub executions: Arc<RwLock<Vec<ExecutionRecord>>>,
    pub controls: Arc<RwLock<ControlState>>,
    pub risk_events: Arc<RwLock<Vec<RiskEvent>>>,
    pub instruments: Arc<RwLock<HashMap<String, Instrument>>>,
    pub tokens: Arc<RwLock<HashMap<String, Token>>>,
    pub events: broadcast::Sender<WsEvent>,
    pub stats: Arc<RwLock<RuntimeStats>>,
    pub paper: Arc<Mutex<PaperEngine>>,
    pub strategy: Arc<StrategyEngine>,
    /// 多跳 DEX 扫描参数（由配置派生）。
    pub dex_params: Arc<DexScanParams>,
    /// 路线集合 + 池 → 路线反向索引（增量扫描）。
    pub dex_cache: Arc<Mutex<MultiHopCache>>,
    /// 最近一轮的路线记录（API / 前端）。
    pub dex_routes: Arc<RwLock<Vec<RouteScanRecord>>>,
    pub dex_graph: Arc<RwLock<GraphStats>>,
    pub dex_scan_stats: Arc<RwLock<ScanRunStats>>,
    /// 最近模拟结果（M1/M3，API 查询与审计）。
    pub simulations: Arc<RwLock<Vec<SimulationOutcome>>>,
    /// 手工触发一次扫描的开关（API `/scanner/scan`）。
    pub scan_now: Arc<tokio::sync::Notify>,
}

impl AppState {
    pub fn new(config: Config, workspace: std::path::PathBuf, db: Db) -> anyhow::Result<Arc<Self>> {
        let (tx, _rx) = broadcast::channel(512);
        let risk = Arc::new(RwLock::new(RiskEngine::new()));
        let ledger = Arc::new(Mutex::new(LedgerEngine::new(
            Decimal::from_f64_retain(config.paper.start_equity).unwrap_or(Decimal::from(5000)),
        )));
        let market = Arc::new(RwLock::new(DemoMarket::new(&config, config.feed.seed)));
        let paper = Arc::new(Mutex::new(PaperEngine::new(
            config.feed.seed ^ 0xA5A5,
            crate::paper::scenario::Scenario::default(),
            PaperParams {
                ttl_sec: config.paper.opportunity_ttl_sec,
                ..PaperParams::default()
            },
        )));
        let strategy = Arc::new(StrategyEngine::new());
        let dex_params = Arc::new(DexScanParams::from_config(&config));

        let state = Arc::new(Self {
            config,
            workspace,
            db,
            risk,
            ledger,
            market,
            scanner: Arc::new(RwLock::new(ScannerStatus::default())),
            opportunities: Arc::new(RwLock::new(Vec::new())),
            executions: Arc::new(RwLock::new(Vec::new())),
            controls: Arc::new(RwLock::new(ControlState::new())),
            risk_events: Arc::new(RwLock::new(Vec::new())),
            instruments: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
            events: tx,
            stats: Arc::new(RwLock::new(RuntimeStats::default())),
            paper,
            strategy,
            dex_params,
            dex_cache: Arc::new(Mutex::new(MultiHopCache::new())),
            dex_routes: Arc::new(RwLock::new(Vec::new())),
            dex_graph: Arc::new(RwLock::new(GraphStats::default())),
            dex_scan_stats: Arc::new(RwLock::new(ScanRunStats::default())),
            simulations: Arc::new(RwLock::new(Vec::new())),
            scan_now: Arc::new(tokio::sync::Notify::new()),
        });
        Ok(state)
    }

    pub fn broadcast(&self, ev: WsEvent) {
        let _ = self.events.send(ev);
    }

    /// 当前权益（USDC）。
    pub async fn equity(&self) -> Decimal {
        self.ledger.lock().await.total_equity()
    }

    /// 某策略的日亏损（正数表示亏损）。
    pub async fn strategy_daily_loss(&self, s: StrategyType) -> Decimal {
        let l = self.ledger.lock().await;
        (-l.strategy_realized_pnl(s)).max(Decimal::ZERO)
    }

    /// 全账户日亏损。
    pub async fn account_daily_loss(&self) -> Decimal {
        let l = self.ledger.lock().await;
        (-l.realized_pnl()).max(Decimal::ZERO)
    }

    /// 可用库存（按资产），来自账本。
    pub async fn available_inventory(&self, venue_prefix: &str) -> HashMap<String, Decimal> {
        let l = self.ledger.lock().await;
        let mut out = HashMap::new();
        for b in l.balances() {
            if b.venue.starts_with(venue_prefix) {
                *out.entry(b.asset.clone()).or_insert(Decimal::ZERO) += b.balance;
            }
        }
        out
    }

    pub async fn push_opportunity(&self, opp: Opportunity) {
        let mut list = self.opportunities.write().await;
        list.insert(0, opp.clone());
        if list.len() > 500 {
            list.truncate(500);
        }
    }

    pub async fn push_execution(&self, rec: ExecutionRecord) {
        let mut list = self.executions.write().await;
        list.insert(0, rec);
        if list.len() > 500 {
            list.truncate(500);
        }
    }

    pub async fn push_risk_event(&self, ev: RiskEvent) {
        let mut list = self.risk_events.write().await;
        list.insert(0, ev);
        if list.len() > 1_000 {
            list.truncate(1_000);
        }
    }
}
