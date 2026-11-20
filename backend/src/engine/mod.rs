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
use crate::domain::types::OppStatus;
use crate::domain::report::ScannerStatus;
use crate::domain::risk::{ControlState, RiskEvent};
use crate::domain::types::StrategyType;
use crate::feed::live::LiveFeed;
use crate::feed::MarketData;
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
    /// 真实链行情快照（由 feed 任务持续刷新）。
    pub market: Arc<RwLock<MarketData>>,
    /// 真实链同步器（RPC 发现/状态/价格）。
    pub live: Arc<Mutex<LiveFeed>>,
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
    /// 多跳 DEX 扫描参数（可在线修改，持久化在 search_config.json）。
    pub dex_params: Arc<RwLock<DexScanParams>>,
    /// 搜索配置：Token 白/黑名单 + 可编辑参数。
    pub search: Arc<tokio::sync::Mutex<crate::search_config::SearchConfig>>,
    /// 全链自动发现状态。
    pub discovery: Arc<RwLock<crate::dex::discovery::DiscoveryStatus>>,
    /// 扫描数据源注册表（Key/启用/优先级/策略）。
    pub sources: Arc<tokio::sync::Mutex<crate::source_config::SourceConfig>>,
    /// 路线集合 + 池 → 路线反向索引（增量扫描）。
    pub dex_cache: Arc<Mutex<MultiHopCache>>,
    /// 最近一轮的路线记录（API / 前端）。
    pub dex_routes: Arc<RwLock<Vec<RouteScanRecord>>>,
    pub dex_graph: Arc<RwLock<GraphStats>>,
    pub dex_scan_stats: Arc<RwLock<ScanRunStats>>,
    /// 最近模拟结果（M1/M3，API 查询与审计）。
    pub simulations: Arc<RwLock<Vec<SimulationOutcome>>>,
    /// M2（REVM 分叉）复核证据，按 route_id 保存，跨扫描周期复用。
    pub m2_evidence: Arc<RwLock<HashMap<String, crate::domain::market::M2Evidence>>>,
    /// 手工触发一次扫描的开关（API `/scanner/scan`）。
    pub scan_now: Arc<tokio::sync::Notify>,
    /// 归一化指标事件（V22-101）：内存环形窗口 + 落库。
    pub metrics: Arc<tokio::sync::Mutex<crate::metrics::MetricStore>>,
}

impl AppState {
    pub fn new(config: Config, workspace: std::path::PathBuf, db: Db) -> anyhow::Result<Arc<Self>> {
        let (tx, _rx) = broadcast::channel(512);
        let risk = Arc::new(RwLock::new(RiskEngine::new()));
        let ledger = Arc::new(Mutex::new(LedgerEngine::new(
            Decimal::from_f64_retain(config.paper.start_equity).unwrap_or(Decimal::from(5000)),
        )));
        if config.feed.mode != "live" {
            anyhow::bail!(
                "运行时只支持真实链行情（feed.mode = \"live\"）；演示行情仅供单元测试构造，不允许用于扫描或展示"
            );
        }
        let search_cfg = crate::search_config::SearchConfig::load(&config, &workspace);
        let source_cfg = crate::source_config::SourceConfig::load(&workspace);
        let mut feed_cfg = config.clone();
        feed_cfg.live.tokens = search_cfg.token_configs();
        let search = Arc::new(tokio::sync::Mutex::new(search_cfg.clone()));
        let live_feed = LiveFeed::new(&feed_cfg, search.clone(), workspace.clone())
            .map_err(|e| anyhow::anyhow!("初始化真实链行情失败: {e}"))?;
        let chain = live_feed.chain.clone();
        let market = Arc::new(RwLock::new(MarketData::empty(chain)));
        let live = Arc::new(Mutex::new(live_feed));
        let paper = Arc::new(Mutex::new(PaperEngine::new(
            config.feed.seed ^ 0xA5A5,
            crate::paper::scenario::Scenario::default(),
            PaperParams {
                ttl_sec: config.paper.opportunity_ttl_sec,
                ..PaperParams::default()
            },
        )));
        let strategy = Arc::new(StrategyEngine::new());
        let mut dex_params = DexScanParams::from_config(&config);
        search_cfg.params.apply(&mut dex_params);
        let dex_params = Arc::new(RwLock::new(dex_params));

        let mut ds = crate::dex::discovery::DiscoveryStatus::default();
        ds.enabled = config.discovery.enabled;
        ds.total_pools = crate::dex::discovery::load_pools(&workspace).len();
        let state = Arc::new(Self {
            config,
            workspace,
            db,
            risk,
            ledger,
            market,
            live,
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
            search,
            discovery: Arc::new(RwLock::new(ds)),
            sources: Arc::new(tokio::sync::Mutex::new(source_cfg)),
            dex_cache: Arc::new(Mutex::new(MultiHopCache::new())),
            dex_routes: Arc::new(RwLock::new(Vec::new())),
            dex_graph: Arc::new(RwLock::new(GraphStats::default())),
            dex_scan_stats: Arc::new(RwLock::new(ScanRunStats::default())),
            simulations: Arc::new(RwLock::new(Vec::new())),
            m2_evidence: Arc::new(RwLock::new(HashMap::new())),
            scan_now: Arc::new(tokio::sync::Notify::new()),
            metrics: Arc::new(tokio::sync::Mutex::new(
                crate::metrics::MetricStore::default(),
            )),
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
        // 稳定 ID：同一路线/金额只保留一条，避免每轮扫描产生重复记录。
        if let Some(slot) = list
            .iter_mut()
            .find(|o| o.opportunity_id == opp.opportunity_id)
        {
            *slot = opp;
            return;
        }
        list.insert(0, opp);
        // 保留窗口按「状态优先」淘汰：先丢最旧的拒单样本，
        // 保证可执行 / 观察中的机会在 TTL 内始终能被前端看到。
        while list.len() > 500 {
            let victim = list
                .iter()
                .rposition(|o| !matches!(o.status, OppStatus::Actionable | OppStatus::Watch));
            match victim {
                Some(i) => {
                    list.remove(i);
                }
                None => {
                    list.pop();
                }
            }
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

/// 启动真实链同步任务：持续把最新池状态/价格写入 MarketData 快照。
///
/// 同步失败时只降低健康度并保留上一份真实快照，不生成替代数据；
/// 扫描器会依据报价年龄/区块差拒绝过期机会。
pub fn spawn_feed(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {    tokio::spawn(async move {
        let interval = std::time::Duration::from_millis(state.config.live.refresh_ms.max(500));
        // 单次同步的硬超时：公共 RPC 抖动或大批量调用不能无限期阻塞行情刷新。
        let budget = interval.saturating_mul(10).max(std::time::Duration::from_secs(30));
        // 可选 WS：有端点就尝试订阅 newHeads，失败自动降级为纯轮询（V22-107/108）。
        let (head_tx, mut head_rx) = tokio::sync::mpsc::channel::<u64>(8);
        let mut ws_alive = !state.config.live.ws_endpoints.is_empty();
        for url in state.config.live.ws_endpoints.clone() {
            let tx = head_tx.clone();
            tokio::spawn(async move {
                // 断连后 30s 重试；期间主循环按轮询工作，不受影响。
                loop {
                    match crate::evm::ws::watch_heads(&url, tx.clone()).await {
                        Ok(()) => {}
                        Err(e) => tracing::warn!("WS 订阅不可用，降级轮询: {url} — {e}"),
                    }
                    if tx.is_closed() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            });
        }

        loop {
            let started = std::time::Instant::now();
            let result = {
                let mut feed = state.live.lock().await;
                match tokio::time::timeout(budget, feed.sync()).await {
                    Ok(r) => r,
                    Err(_) => Err(format!("链上同步超时（>{budget:?}），已保留上一份快照")),
                }
            };
            let elapsed = started.elapsed();
            let ok = result.is_ok();
            match result {
                Ok(data) => {
                    let block = data.head_block;
                    let pools = data.pools.len();
                    let chain_id = data.chain.chain_id;
                    let errors = data.health.last_error.clone();
                    let prev_block = state.market.read().await.head_block;
                    let prev_hash = {
                        let m = state.market.read().await;
                        m.pools
                            .all()
                            .first()
                            .map(|p| p.state.block_hash.clone())
                            .unwrap_or_default()
                    };
                    let new_hash = data
                        .pools
                        .all()
                        .first()
                        .map(|p| p.state.block_hash.clone())
                        .unwrap_or_default();
                    *state.market.write().await = data;
                    if let Some(e) = errors {
                        tracing::warn!("链上同步部分失败: {e}");
                    }
                    tracing::debug!("链上同步完成 block={block} pools={pools}");
                    // ---- 指标（V22-102 / V22-110）----
                    let mut events = vec![
                        crate::metrics::MetricEvent::latency(
                            chain_id,
                            "feed_sync_ms",
                            elapsed.as_millis(),
                        ),
                        crate::metrics::MetricEvent::new(
                            crate::metrics::KIND_DATA_QUALITY,
                            chain_id,
                            "feed_pools",
                            Decimal::from(pools as u64),
                            "count",
                        ),
                        crate::metrics::MetricEvent::new(
                            crate::metrics::KIND_DATA_QUALITY,
                            chain_id,
                            "head_block",
                            Decimal::from(block),
                            "block",
                        ),
                    ];
                    if prev_block > 0 && block > prev_block + 1 {
                        events.push(crate::metrics::MetricEvent::new(
                            crate::metrics::KIND_DATA_QUALITY,
                            chain_id,
                            "block_gap",
                            Decimal::from(block - prev_block - 1),
                            "block",
                        ));
                    }
                    // V22-110：head 回退或同高不同哈希 → 视为重组。
                    let reorg = block < prev_block
                        || (block == prev_block && !prev_hash.is_empty() && new_hash != prev_hash);
                    if reorg {
                        events.push(crate::metrics::MetricEvent::new(
                            crate::metrics::KIND_DATA_QUALITY,
                            chain_id,
                            "reorg_detected",
                            Decimal::ONE,
                            "count",
                        ));
                        tracing::warn!("检测到链重组：head {} → {}", prev_block, block);
                    }
                    // V22-109：统计本轮新区间的 Swap 事件（只查受跟踪池，最多回看 3 个区块）。
                    let from_block = if prev_block == 0 {
                        block.saturating_sub(10)
                    } else {
                        prev_block.saturating_add(1).max(block.saturating_sub(10))
                    };
                    let swap_events = {
                        let feed = state.live.lock().await;
                        match feed.count_swap_logs(from_block, block).await {
                            Ok(n) => n,
                            Err(e) => {
                                tracing::warn!("Swap 事件查询失败: {e}");
                                0
                            }
                        }
                    };
                    if swap_events > 0 {
                        events.push(crate::metrics::MetricEvent::new(
                            crate::metrics::KIND_DATA_QUALITY,
                            chain_id,
                            "swap_events",
                            Decimal::from(swap_events as u64),
                            "count",
                        ));
                    }
                    let _ = crate::db::repo::save_metric_events(&state.db, &events);
                    {
                        let mut store = state.metrics.lock().await;
                        for e in events {
                            store.record(e);
                        }
                    }
                    if swap_events > 0 {
                        // V22-109：事件驱动——本轮检测到 Swap 就立即进入下一轮同步，不等轮询间隔。
                        continue;
                    }
                }
                Err(e) => {
                    tracing::error!("链上同步失败: {e}");
                    let mut m = state.market.write().await;
                    m.health.ok = false;
                    m.health.last_error = Some(e);
                    m.health.last_sync = Some(Utc::now());
                }
            }
            // 失败时退避到 4×间隔，避免对限流端点持续施压。
            let sleep_for = if ok { interval } else { interval.saturating_mul(4) };
            if ws_alive {
                // WS 有新块就提前同步；通道关闭（WS 不可用）后退回纯轮询。
                tokio::select! {
                    _ = tokio::time::sleep(sleep_for) => {}
                    head = head_rx.recv() => {
                        match head {
                            Some(block) => {
                                let chain_id = state.market.read().await.chain.chain_id;
                                let mut store = state.metrics.lock().await;
                                store.record(crate::metrics::MetricEvent::new(
                                    crate::metrics::KIND_DATA_QUALITY,
                                    chain_id,
                                    "ws_head",
                                    Decimal::from(block),
                                    "block",
                                ));
                            }
                            None => {
                                ws_alive = false;
                                tracing::warn!("WS 订阅通道关闭，后续按轮询同步");
                            }
                        }
                    }
                }
            } else {
                tokio::time::sleep(sleep_for).await;
            }
        }
    })
}

/// 全链池/Token 自动发现循环。
pub fn spawn_discovery(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let cfg = state.config.discovery.clone();
        if !cfg.enabled {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            {
                let mut s = state.discovery.write().await;
                s.running = true;
                s.enabled = true;
            }
            let result = crate::dex::discovery::scan_once(&state).await;
            {
                let mut s = state.discovery.write().await;
                s.running = false;
                match result {
                    Ok(st) => *s = st,
                    Err(e) => {
                        s.message = format!("discovery failed: {e}");
                        s.last_run_at = Utc::now().to_rfc3339();
                        s.total_pools = crate::dex::discovery::load_pools(&state.workspace).len();
                        s.enabled = true;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(cfg.interval_sec.max(120))).await;
        }
    })
}
