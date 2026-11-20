//! 数据库 schema。对应 TDD §10 的实体集合（按第一阶段实际使用裁剪）。
//!
//! 说明：
//! - `*_json` 列保存完整领域对象（机会、执行、事件），关键字段单独建列用于查询。
//! - 金额/数量一律 TEXT，避免浮点误差。
//! - 迁移使用「幂等 CREATE TABLE IF NOT EXISTS」，新增列通过 ALTER 迁移块处理。

pub const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  INTEGER NOT NULL
);

-- ---------- 链与 DEX ----------
CREATE TABLE IF NOT EXISTS chains (
    chain_id    INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    stage       TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    json        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS dex_protocols (
    protocol_id TEXT PRIMARY KEY,
    chain_id    INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tokens (
    chain_id    INTEGER NOT NULL,
    symbol      TEXT NOT NULL,
    address     TEXT NOT NULL,
    decimals    INTEGER NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    json        TEXT NOT NULL,
    PRIMARY KEY (chain_id, symbol)
);

CREATE TABLE IF NOT EXISTS pools (
    pool_id     TEXT PRIMARY KEY,
    chain_id    INTEGER NOT NULL,
    protocol_id TEXT NOT NULL,
    token0      TEXT NOT NULL,
    token1      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    verified    INTEGER NOT NULL DEFAULT 0,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pool_states (
    pool_id       TEXT PRIMARY KEY,
    block_number  INTEGER NOT NULL,
    block_hash    TEXT NOT NULL,
    updated_at    INTEGER NOT NULL,
    json          TEXT NOT NULL
);

-- ---------- CEX ----------
CREATE TABLE IF NOT EXISTS cex_exchanges (
    exchange_id TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cex_instruments (
    exchange    TEXT NOT NULL,
    symbol      TEXT NOT NULL,
    base        TEXT NOT NULL,
    quote       TEXT NOT NULL,
    status      TEXT NOT NULL,
    fee_known   INTEGER NOT NULL DEFAULT 0,
    json        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (exchange, symbol)
);

CREATE TABLE IF NOT EXISTS cex_data_health (
    exchange   TEXT NOT NULL,
    symbol     TEXT NOT NULL,
    valid      INTEGER NOT NULL,
    age_ms     INTEGER NOT NULL,
    seq        INTEGER NOT NULL,
    json       TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (exchange, symbol)
);

-- ---------- 策略与机会 ----------
CREATE TABLE IF NOT EXISTS strategies (
    strategy_id TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    mode        TEXT NOT NULL,
    json        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS opportunities (
    opportunity_id TEXT PRIMARY KEY,
    detected_at    INTEGER NOT NULL,
    strategy_id    TEXT NOT NULL,
    chain_id       INTEGER NOT NULL,
    status         TEXT NOT NULL,
    net_profit     TEXT NOT NULL,
    notional       TEXT NOT NULL,
    json           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_opportunities_detected ON opportunities(detected_at DESC);
CREATE INDEX IF NOT EXISTS idx_opportunities_strategy ON opportunities(strategy_id, detected_at DESC);

CREATE TABLE IF NOT EXISTS trade_plans (
    plan_id        TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    state          TEXT NOT NULL,
    plan_hash      TEXT NOT NULL,
    json           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trade_plans_opp ON trade_plans(opportunity_id);

CREATE TABLE IF NOT EXISTS executions (
    execution_id   TEXT PRIMARY KEY,
    plan_id        TEXT NOT NULL,
    opportunity_id TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    strategy_id    TEXT NOT NULL,
    state          TEXT NOT NULL,
    net_pnl        TEXT NOT NULL,
    json           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_executions_created ON executions(created_at DESC);

CREATE TABLE IF NOT EXISTS cex_fills (
    fill_id      TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL,
    ts           INTEGER NOT NULL,
    exchange     TEXT NOT NULL,
    symbol       TEXT NOT NULL,
    side         TEXT NOT NULL,
    price        TEXT NOT NULL,
    qty          TEXT NOT NULL,
    fee          TEXT NOT NULL,
    json         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fills_execution ON cex_fills(execution_id);

CREATE TABLE IF NOT EXISTS execution_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    execution_id TEXT NOT NULL,
    ts           INTEGER NOT NULL,
    from_state   TEXT NOT NULL,
    to_state     TEXT NOT NULL,
    level        TEXT NOT NULL,
    message      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_execution ON execution_events(execution_id);

CREATE TABLE IF NOT EXISTS unwind_events (
    unwind_id    TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL,
    ts           INTEGER NOT NULL,
    json         TEXT NOT NULL
);

-- ---------- 资金与风险 ----------
CREATE TABLE IF NOT EXISTS accounts (
    account_id  TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    venue       TEXT NOT NULL,
    asset       TEXT NOT NULL,
    protected   INTEGER NOT NULL DEFAULT 0,
    json        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ledger_entries (
    entry_id       TEXT PRIMARY KEY,
    ts             INTEGER NOT NULL,
    account_id     TEXT NOT NULL,
    kind           TEXT NOT NULL,
    asset          TEXT NOT NULL,
    amount         TEXT NOT NULL,
    balance_after  TEXT NOT NULL,
    usd_value      TEXT NOT NULL,
    price_usd      TEXT NOT NULL,
    strategy_id    TEXT,
    opportunity_id TEXT,
    plan_id        TEXT,
    execution_id   TEXT,
    ref_id         TEXT,
    memo           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ledger_ts ON ledger_entries(ts DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_account ON ledger_entries(account_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_execution ON ledger_entries(execution_id);

CREATE TABLE IF NOT EXISTS inventory_snapshots (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    ts       INTEGER NOT NULL,
    venue    TEXT NOT NULL,
    asset    TEXT NOT NULL,
    json     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS risk_events (
    event_id    TEXT PRIMARY KEY,
    triggered_at INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    severity    TEXT NOT NULL,
    scope       TEXT NOT NULL,
    scope_ref   TEXT NOT NULL,
    active      INTEGER NOT NULL,
    json        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_risk_events_ts ON risk_events(triggered_at DESC);

CREATE TABLE IF NOT EXISTS risk_params (
    key         TEXT PRIMARY KEY,
    scope       TEXT NOT NULL,
    severity    TEXT NOT NULL,
    value       TEXT NOT NULL,
    version     TEXT NOT NULL,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS risk_overrides (
    key        TEXT NOT NULL,
    scope      TEXT NOT NULL,
    scope_ref  TEXT NOT NULL,
    value      TEXT NOT NULL,
    version    INTEGER NOT NULL,
    json       TEXT NOT NULL,
    PRIMARY KEY (key, scope, scope_ref)
);

CREATE TABLE IF NOT EXISTS system_controls (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    json        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS system_configs (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- ---------- 审计 ----------
CREATE TABLE IF NOT EXISTS audit_jobs (
    job_id      TEXT PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    state       TEXT NOT NULL,
    dataset     TEXT NOT NULL,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_reports (
    report_id   TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    verdict     TEXT NOT NULL,
    json        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS scanner_state (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    json        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- ---------- DEX 多池多跳引擎（PRD §13.3）----------
CREATE TABLE IF NOT EXISTS dex_routes (
    route_id     TEXT PRIMARY KEY,
    fingerprint  TEXT NOT NULL,
    chain_id     INTEGER NOT NULL,
    start_token  TEXT NOT NULL,
    hops         INTEGER NOT NULL,
    route_type   TEXT NOT NULL,
    lifecycle    TEXT NOT NULL,
    amount_in    TEXT NOT NULL,
    amount_out   TEXT NOT NULL,
    notional     TEXT NOT NULL,
    net_usd      TEXT NOT NULL,
    block_number INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    json         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_dex_routes_updated ON dex_routes(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_dex_routes_net ON dex_routes(notional, net_usd);

CREATE TABLE IF NOT EXISTS route_fingerprints (
    fingerprint    TEXT PRIMARY KEY,
    undirected     TEXT NOT NULL,
    pool_sequence  TEXT NOT NULL,
    token_sequence TEXT NOT NULL,
    start_asset    TEXT NOT NULL,
    route_id       TEXT NOT NULL,
    json           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_route_fp_route ON route_fingerprints(route_id);

CREATE TABLE IF NOT EXISTS simulations (
    simulation_id TEXT PRIMARY KEY,
    route_id      TEXT NOT NULL,
    level         TEXT NOT NULL,
    status        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    json          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_simulations_route ON simulations(route_id, created_at DESC);

CREATE TABLE IF NOT EXISTS scan_runs (
    run_id       TEXT PRIMARY KEY,
    started_at   INTEGER NOT NULL,
    elapsed_ms   INTEGER NOT NULL,
    full_rescan  INTEGER NOT NULL,
    candidates   INTEGER NOT NULL,
    requoted     INTEGER NOT NULL,
    reused       INTEGER NOT NULL,
    visits       INTEGER NOT NULL,
    truncated    INTEGER NOT NULL,
    json         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scan_runs_started ON scan_runs(started_at DESC);

-- ---------- 指标事件（V22-101）----------
CREATE TABLE IF NOT EXISTS metric_events (
    event_id    TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    chain_id    INTEGER NOT NULL DEFAULT 0,
    metric      TEXT NOT NULL,
    value       TEXT NOT NULL,
    unit        TEXT NOT NULL,
    ts_ms       INTEGER NOT NULL,
    labels_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_metric_events_kind_ts ON metric_events(kind, ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_metric_events_metric_ts ON metric_events(metric, ts_ms DESC);

-- ---------- 机会生命周期事件（V22-103）----------
CREATE TABLE IF NOT EXISTS opportunity_lifecycle_events (
    event_id       TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL,
    chain_id       INTEGER NOT NULL DEFAULT 0,
    stage          TEXT NOT NULL,
    status         TEXT NOT NULL,
    ts_ms          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_opp_lifecycle_opp ON opportunity_lifecycle_events(opportunity_id, ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_opp_lifecycle_ts ON opportunity_lifecycle_events(ts_ms DESC);

-- ---------- 竞争决策（V23-603）----------
CREATE TABLE IF NOT EXISTS competition_decisions (
    decision_id  TEXT PRIMARY KEY,
    route_id     TEXT NOT NULL DEFAULT '',
    chain_id     INTEGER NOT NULL DEFAULT 0,
    tier         TEXT NOT NULL,
    decision     TEXT NOT NULL,
    expected_net TEXT NOT NULL,
    ts_ms        INTEGER NOT NULL,
    json         TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_comp_decisions_ts ON competition_decisions(ts_ms DESC);

-- ---------- 竞争者交易（V23-401/402，Swap 日志重建）----------
CREATE TABLE IF NOT EXISTS competitor_transactions (
    tx_hash     TEXT PRIMARY KEY,
    chain_id    INTEGER NOT NULL DEFAULT 0,
    block       INTEGER NOT NULL,
    tx_index    INTEGER NOT NULL,
    log_count   INTEGER NOT NULL,
    confidence  TEXT NOT NULL,
    json        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_comp_tx_block ON competitor_transactions(block DESC);

CREATE TABLE IF NOT EXISTS competitor_addresses (
    address    TEXT PRIMARY KEY,
    chain_id   INTEGER NOT NULL DEFAULT 0,
    tx_count   INTEGER NOT NULL,
    last_block INTEGER NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS competitor_clusters (
    cluster_id TEXT PRIMARY KEY,
    chain_id   INTEGER NOT NULL DEFAULT 0,
    addresses  INTEGER NOT NULL,
    tx_count   INTEGER NOT NULL,
    confidence TEXT NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS route_competition_snapshots (
    route_key  TEXT PRIMARY KEY,
    chain_id   INTEGER NOT NULL DEFAULT 0,
    level      TEXT NOT NULL,
    tx_count   INTEGER NOT NULL,
    ts_ms      INTEGER NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS paper_competition_results (
    result_id      TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL,
    state          TEXT NOT NULL,
    p_capture      TEXT NOT NULL,
    net_usd        TEXT NOT NULL,
    ts_ms          INTEGER NOT NULL,
    json           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS historical_replay_jobs (
    job_id     TEXT PRIMARY KEY,
    route_id   TEXT NOT NULL,
    status     TEXT NOT NULL,
    block      INTEGER NOT NULL,
    window     INTEGER NOT NULL,
    ts_ms      INTEGER NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS historical_replay_results (
    result_id  TEXT PRIMARY KEY,
    job_id     TEXT NOT NULL,
    basis      TEXT NOT NULL,
    legs       INTEGER NOT NULL,
    chained    INTEGER NOT NULL,
    ts_ms      INTEGER NOT NULL,
    json       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS opportunity_lifetime_stats (
    stat_id    TEXT PRIMARY KEY,
    samples    INTEGER NOT NULL,
    p50_ms     TEXT,
    p95_ms     TEXT,
    p99_ms     TEXT,
    ts_ms      INTEGER NOT NULL,
    json       TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, 0);
INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, 0);
INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (3, 0);
"#;
