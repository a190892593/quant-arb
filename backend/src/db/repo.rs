//! 领域对象 ↔ 数据库表的读写。
//!
//! 每个 `save_*` 都以幂等 upsert 实现，可安全重复调用；
//! 每个 `load_*` 在解析失败时跳过坏行并记录告警，避免单条脏数据阻塞启动。

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::Db;
use crate::domain::dt_to_ms;
use crate::domain::execution::ExecutionRecord;
use crate::domain::ledger::{Account, LedgerEntry};
use crate::domain::market::{DataQuality, ModelLevel};
use crate::domain::market::{Chain, Instrument, PoolRef, PoolState, Token};
use crate::domain::opportunity::Opportunity;
use crate::domain::plan::TradePlan;
use crate::domain::risk::{ControlState, ParamOverride, RiskEvent, RiskParam};
use crate::domain::report::ScannerStatus;

fn to_json<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).context("序列化失败")
}

fn from_json<T: DeserializeOwned>(raw: &str) -> Option<T> {
    match serde_json::from_str::<T>(raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("跳过无法解析的记录: {e}");
            None
        }
    }
}

// ---------------- 链 / Token / 池 ----------------

pub fn save_chains(db: &Db, chains: &[Chain]) -> Result<()> {
    db.with_tx(|tx| {
        for c in chains {
            tx.execute(
                "INSERT INTO chains(chain_id, name, stage, enabled, json, updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(chain_id) DO UPDATE SET name=excluded.name, stage=excluded.stage,
                   enabled=excluded.enabled, json=excluded.json, updated_at=excluded.updated_at",
                params![
                    c.chain_id as i64,
                    c.chain_name,
                    c.stage.as_str(),
                    c.enabled as i64,
                    to_json(c)?,
                    crate::domain::now_ms() as i64
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_chains(db: &Db) -> Result<Vec<Chain>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM chains ORDER BY chain_id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<Chain>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_tokens(db: &Db, tokens: &[Token]) -> Result<()> {
    db.with_tx(|tx| {
        for t in tokens {
            tx.execute(
                "INSERT INTO tokens(chain_id, symbol, address, decimals, enabled, json)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(chain_id, symbol) DO UPDATE SET address=excluded.address,
                   decimals=excluded.decimals, enabled=excluded.enabled, json=excluded.json",
                params![
                    t.chain_id as i64,
                    t.symbol,
                    t.address,
                    t.decimals as i64,
                    t.enabled as i64,
                    to_json(t)?
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_tokens(db: &Db) -> Result<Vec<Token>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM tokens ORDER BY chain_id, symbol")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<Token>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_pools(db: &Db, pools: &[PoolRef], states: &[PoolState]) -> Result<()> {
    db.with_tx(|tx| {
        for p in pools {
            tx.execute(
                "INSERT INTO pools(pool_id, chain_id, protocol_id, token0, token1, kind, verified, json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(pool_id) DO UPDATE SET token0=excluded.token0, token1=excluded.token1,
                   kind=excluded.kind, verified=excluded.verified, json=excluded.json",
                params![
                    p.pool_id,
                    p.chain_id as i64,
                    p.protocol_id,
                    p.token0,
                    p.token1,
                    p.kind.as_str(),
                    p.verified as i64,
                    to_json(p)?
                ],
            )?;
        }
        for s in states {
            tx.execute(
                "INSERT INTO pool_states(pool_id, block_number, block_hash, updated_at, json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(pool_id) DO UPDATE SET block_number=excluded.block_number,
                   block_hash=excluded.block_hash, updated_at=excluded.updated_at, json=excluded.json",
                params![
                    s.pool_id,
                    s.block_number as i64,
                    s.block_hash,
                    dt_to_ms(s.updated_at),
                    to_json(s)?
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_pools(db: &Db) -> Result<(Vec<PoolRef>, Vec<PoolState>)> {
    db.with(|c| {
        let mut pools = Vec::new();
        let mut stmt = c.prepare("SELECT json FROM pools ORDER BY pool_id")?;
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<PoolRef>(&r?) {
                pools.push(v);
            }
        }
        let mut states = Vec::new();
        let mut stmt = c.prepare("SELECT json FROM pool_states ORDER BY pool_id")?;
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<PoolState>(&r?) {
                states.push(v);
            }
        }
        Ok((pools, states))
    })
}

// ---------------- CEX ----------------

pub fn save_instruments(db: &Db, instruments: &[Instrument]) -> Result<()> {
    db.with_tx(|tx| {
        for i in instruments {
            tx.execute(
                "INSERT INTO cex_instruments(exchange, symbol, base, quote, status, fee_known, json, updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(exchange, symbol) DO UPDATE SET base=excluded.base, quote=excluded.quote,
                   status=excluded.status, fee_known=excluded.fee_known, json=excluded.json,
                   updated_at=excluded.updated_at",
                params![
                    i.exchange,
                    i.symbol,
                    i.base,
                    i.quote,
                    i.status,
                    i.fee_known as i64,
                    to_json(i)?,
                    crate::domain::now_ms() as i64
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_instruments(db: &Db) -> Result<Vec<Instrument>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM cex_instruments ORDER BY exchange, symbol")?;
        let mut out = Vec::new();
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<Instrument>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

// ---------------- 机会 / 计划 / 执行 ----------------

pub fn save_opportunity(db: &Db, o: &Opportunity) -> Result<()> {
    db.execute(
        "INSERT INTO opportunities(opportunity_id, detected_at, strategy_id, chain_id, status, net_profit, notional, json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(opportunity_id) DO UPDATE SET status=excluded.status,
           net_profit=excluded.net_profit, json=excluded.json",
        params![
            o.opportunity_id,
            dt_to_ms(o.detected_at),
            o.strategy_type.as_str(),
            o.chain_id as i64,
            o.status.as_str(),
            o.conservative_net_profit.to_string(),
            o.target_notional.to_string(),
            to_json(o)?
        ],
    )?;
    Ok(())
}

pub fn load_opportunities(db: &Db, limit: usize) -> Result<Vec<Opportunity>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT json FROM opportunities ORDER BY detected_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(mut v) = from_json::<Opportunity>(&r?) {
                // 本机未接 REVM：历史/外部记录中的 M2 与 VERIFIED 不再成立。
                if !crate::domain::types::M2_AVAILABLE {
                    if v.model_level == ModelLevel::M2 {
                        v.model_level = ModelLevel::M1;
                    }
                    if v.data_quality == DataQuality::Verified {
                        v.data_quality = DataQuality::High;
                    }
                }
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn prune_opportunities(db: &Db, keep: usize) -> Result<usize> {
    db.execute(
        "DELETE FROM opportunities WHERE opportunity_id NOT IN (
            SELECT opportunity_id FROM opportunities ORDER BY detected_at DESC LIMIT ?1
        )",
        params![keep as i64],
    )
}

pub fn save_plan(db: &Db, p: &TradePlan) -> Result<()> {
    db.execute(
        "INSERT INTO trade_plans(plan_id, opportunity_id, created_at, state, plan_hash, json)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(plan_id) DO UPDATE SET state=excluded.state, json=excluded.json",
        params![
            p.plan_id,
            p.opportunity_id,
            dt_to_ms(p.created_at),
            p.state.as_str(),
            p.plan_hash,
            to_json(p)?
        ],
    )?;
    Ok(())
}

pub fn load_plan(db: &Db, plan_id: &str) -> Result<Option<TradePlan>> {
    db.with(|c| {
        let raw: Option<String> = c
            .query_row("SELECT json FROM trade_plans WHERE plan_id=?1", params![plan_id], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| from_json::<TradePlan>(&s)))
    })
}

pub fn save_execution(db: &Db, e: &ExecutionRecord) -> Result<()> {
    db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO executions(execution_id, plan_id, opportunity_id, created_at, updated_at, strategy_id, state, net_pnl, json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(execution_id) DO UPDATE SET updated_at=excluded.updated_at,
               state=excluded.state, net_pnl=excluded.net_pnl, json=excluded.json",
            params![
                e.execution_id,
                e.plan_id,
                e.opportunity_id,
                dt_to_ms(e.created_at),
                dt_to_ms(e.updated_at),
                e.strategy_type.as_str(),
                e.state.as_str(),
                e.net_pnl.to_string(),
                to_json(e)?
            ],
        )?;
        for f in &e.fills {
            tx.execute(
                "INSERT INTO cex_fills(fill_id, execution_id, ts, exchange, symbol, side, price, qty, fee, json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(fill_id) DO NOTHING",
                params![
                    f.fill_id,
                    e.execution_id,
                    dt_to_ms(f.ts),
                    f.exchange,
                    f.symbol,
                    f.side.as_str(),
                    f.price.to_string(),
                    f.qty.to_string(),
                    f.fee.to_string(),
                    to_json(f)?
                ],
            )?;
        }
        for ev in &e.events {
            tx.execute(
                "INSERT INTO execution_events(execution_id, ts, from_state, to_state, level, message)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    e.execution_id,
                    dt_to_ms(ev.ts),
                    ev.from.as_str(),
                    ev.to.as_str(),
                    ev.level,
                    ev.message
                ],
            )?;
        }
        if let Some(u) = &e.unwind {
            tx.execute(
                "INSERT INTO unwind_events(unwind_id, execution_id, ts, json)
                 VALUES(?1,?2,?3,?4)
                 ON CONFLICT(unwind_id) DO UPDATE SET json=excluded.json",
                params![u.unwind_id, e.execution_id, dt_to_ms(u.started_at), to_json(u)?],
            )?;
        }
        Ok(())
    })
}

pub fn load_executions(db: &Db, limit: usize) -> Result<Vec<ExecutionRecord>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM executions ORDER BY created_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<ExecutionRecord>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

// ---------------- 账本 ----------------

pub fn save_accounts(db: &Db, accounts: &[Account]) -> Result<()> {
    db.with_tx(|tx| {
        for a in accounts {
            tx.execute(
                "INSERT INTO accounts(account_id, kind, venue, asset, protected, json, created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(account_id) DO UPDATE SET kind=excluded.kind, json=excluded.json",
                params![
                    a.account_id,
                    a.kind.as_str(),
                    a.venue,
                    a.asset,
                    a.protected as i64,
                    to_json(a)?,
                    dt_to_ms(a.created_at)
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_accounts(db: &Db) -> Result<Vec<Account>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM accounts ORDER BY account_id")?;
        let mut out = Vec::new();
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<Account>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn append_ledger_entries(db: &Db, entries: &[LedgerEntry]) -> Result<()> {
    db.with_tx(|tx| {
        for e in entries {
            tx.execute(
                "INSERT INTO ledger_entries(entry_id, ts, account_id, kind, asset, amount, balance_after,
                    usd_value, price_usd, strategy_id, opportunity_id, plan_id, execution_id, ref_id, memo)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(entry_id) DO NOTHING",
                params![
                    e.entry_id,
                    dt_to_ms(e.ts),
                    e.account_id,
                    e.kind.as_str(),
                    e.asset,
                    e.amount.to_string(),
                    e.balance_after.to_string(),
                    e.usd_value.to_string(),
                    e.price_usd.to_string(),
                    e.strategy_type.map(|s| s.as_str()),
                    e.opportunity_id,
                    e.plan_id,
                    e.execution_id,
                    e.ref_id,
                    e.memo
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_ledger_entries(db: &Db, limit: usize) -> Result<Vec<LedgerEntry>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT entry_id, ts, account_id, kind, asset, amount, balance_after, usd_value, price_usd,
                    strategy_id, opportunity_id, plan_id, execution_id, ref_id, memo
             FROM ledger_entries ORDER BY ts ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, Option<String>>(12)?,
                r.get::<_, Option<String>>(13)?,
                r.get::<_, String>(14)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                entry_id,
                ts,
                account_id,
                kind,
                asset,
                amount,
                balance_after,
                usd_value,
                price_usd,
                strategy_id,
                opportunity_id,
                plan_id,
                execution_id,
                ref_id,
                memo,
            ) = row?;
            out.push(LedgerEntry {
                entry_id,
                ts: crate::domain::ms_to_dt(ts),
                account_id,
                kind: crate::domain::types::EntryKind::parse(&kind)
                    .unwrap_or(crate::domain::types::EntryKind::Adjustment),
                asset,
                amount: amount.parse().unwrap_or_default(),
                balance_after: balance_after.parse().unwrap_or_default(),
                usd_value: usd_value.parse().unwrap_or_default(),
                price_usd: price_usd.parse().unwrap_or_default(),
                strategy_type: strategy_id
                    .as_deref()
                    .and_then(crate::domain::types::StrategyType::parse),
                opportunity_id,
                plan_id,
                execution_id,
                ref_id,
                memo,
            });
        }
        Ok(out)
    })
}

// ---------------- 风控 ----------------

pub fn save_risk_event(db: &Db, e: &RiskEvent) -> Result<()> {
    db.execute(
        "INSERT INTO risk_events(event_id, triggered_at, kind, severity, scope, scope_ref, active, json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(event_id) DO UPDATE SET active=excluded.active, json=excluded.json",
        params![
            e.event_id,
            dt_to_ms(e.triggered_at),
            e.kind.as_str(),
            e.severity.as_str(),
            e.scope.as_str(),
            e.scope_ref,
            e.is_active() as i64,
            to_json(e)?
        ],
    )?;
    Ok(())
}

pub fn load_risk_events(db: &Db, limit: usize) -> Result<Vec<RiskEvent>> {
    db.with(|c| {
        let mut stmt =
            c.prepare("SELECT json FROM risk_events ORDER BY triggered_at DESC LIMIT ?1")?;
        let mut out = Vec::new();
        for r in stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<RiskEvent>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_risk_params(db: &Db, params_list: &[RiskParam]) -> Result<()> {
    db.with_tx(|tx| {
        for p in params_list {
            tx.execute(
                "INSERT INTO risk_params(key, scope, severity, value, version, json)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(key) DO UPDATE SET scope=excluded.scope, severity=excluded.severity,
                   value=excluded.value, version=excluded.version, json=excluded.json",
                params![
                    p.key,
                    p.scope.as_str(),
                    p.severity.as_str(),
                    p.value.to_string(),
                    p.version,
                    to_json(p)?
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_risk_params(db: &Db) -> Result<Vec<RiskParam>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM risk_params ORDER BY key")?;
        let mut out = Vec::new();
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<RiskParam>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_overrides(db: &Db, overrides: &[ParamOverride]) -> Result<()> {
    db.with_tx(|tx| {
        for o in overrides {
            tx.execute(
                "INSERT INTO risk_overrides(key, scope, scope_ref, value, version, json)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(key, scope, scope_ref) DO UPDATE SET value=excluded.value,
                   version=excluded.version, json=excluded.json",
                params![
                    o.key,
                    o.scope.as_str(),
                    o.scope_ref,
                    o.value.to_string(),
                    o.version as i64,
                    to_json(o)?
                ],
            )?;
        }
        Ok(())
    })
}

pub fn load_overrides(db: &Db) -> Result<Vec<ParamOverride>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM risk_overrides")?;
        let mut out = Vec::new();
        for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(v) = from_json::<ParamOverride>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_controls(db: &Db, controls: &ControlState) -> Result<()> {
    db.execute(
        "INSERT INTO system_controls(id, json, updated_at) VALUES(1,?1,?2)
         ON CONFLICT(id) DO UPDATE SET json=excluded.json, updated_at=excluded.updated_at",
        params![to_json(controls)?, crate::domain::now_ms() as i64],
    )?;
    Ok(())
}

pub fn load_controls(db: &Db) -> Result<Option<ControlState>> {
    db.with(|c| {
        let raw: Option<String> = c
            .query_row("SELECT json FROM system_controls WHERE id=1", [], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| from_json::<ControlState>(&s)))
    })
}

// ---------------- 扫描器状态 / 审计 ----------------

pub fn save_scanner_state(db: &Db, s: &ScannerStatus) -> Result<()> {
    db.execute(
        "INSERT INTO scanner_state(id, json, updated_at) VALUES(1,?1,?2)
         ON CONFLICT(id) DO UPDATE SET json=excluded.json, updated_at=excluded.updated_at",
        params![to_json(s)?, crate::domain::now_ms() as i64],
    )?;
    Ok(())
}

pub fn load_scanner_state(db: &Db) -> Result<Option<ScannerStatus>> {
    db.with(|c| {
        let raw: Option<String> = c
            .query_row("SELECT json FROM scanner_state WHERE id=1", [], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| from_json::<ScannerStatus>(&s)))
    })
}

pub fn save_audit_job(db: &Db, job_id: &str, created_at: i64, state: &str, dataset: &str, json: &str) -> Result<()> {
    db.execute(
        "INSERT INTO audit_jobs(job_id, created_at, state, dataset, json) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(job_id) DO UPDATE SET state=excluded.state, json=excluded.json",
        params![job_id, created_at, state, dataset, json],
    )?;
    Ok(())
}

// ---------------- 键值配置（启动恢复用） ----------------

pub fn set_config(db: &Db, key: &str, value: &str) -> Result<()> {
    db.execute(
        "INSERT INTO system_configs(key, value, updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        params![key, value, crate::domain::now_ms() as i64],
    )?;
    Ok(())
}

pub fn get_config(db: &Db, key: &str) -> Result<Option<String>> {
    db.with(|c| {
        Ok(c.query_row(
            "SELECT value FROM system_configs WHERE key=?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
    })
}

pub fn delete_config(db: &Db, key: &str) -> Result<()> {
    db.execute("DELETE FROM system_configs WHERE key=?1", params![key])?;
    Ok(())
}

pub fn load_audit_jobs(db: &Db, limit: usize) -> Result<Vec<String>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM audit_jobs ORDER BY created_at DESC LIMIT ?1")?;
        let mut out = Vec::new();
        for r in stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))? {
            out.push(r?);
        }
        Ok(out)
    })
}

// ---------------- 运维 ----------------

/// 数据库统计（系统页面展示）。
pub fn stats(db: &Db) -> Result<serde_json::Value> {
    let tables = [
        "opportunities",
        "trade_plans",
        "executions",
        "cex_fills",
        "ledger_entries",
        "risk_events",
        "pools",
        "tokens",
    ];
    let mut map = serde_json::Map::new();
    for t in tables {
        map.insert(
            t.to_string(),
            serde_json::Value::from(db.count(t).unwrap_or(0)),
        );
    }
    Ok(serde_json::Value::Object(map))
}

// ---------------- DEX 多跳路线 / 模拟 / 扫描运行 ----------------

use crate::domain::dex_scan::{RouteScanRecord, ScanRunStats, SimulationOutcome};

/// 保存路线记录（upsert）与关联指纹、模拟结果。按扫描轮次重复调用是幂等的。
pub fn save_route_records(db: &Db, routes: &[RouteScanRecord]) -> Result<()> {
    if routes.is_empty() {
        return Ok(());
    }
    db.with_tx(|tx| {
        let now = crate::domain::now_ms();
        for r in routes {
            tx.execute(
                "INSERT INTO dex_routes(route_id, fingerprint, chain_id, start_token, hops, route_type,
                     lifecycle, amount_in, amount_out, notional, net_usd, block_number, updated_at, json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                 ON CONFLICT(route_id) DO UPDATE SET lifecycle=excluded.lifecycle,
                   amount_in=excluded.amount_in, amount_out=excluded.amount_out,
                   notional=excluded.notional, net_usd=excluded.net_usd,
                   block_number=excluded.block_number, updated_at=excluded.updated_at,
                   json=excluded.json",
                params![
                    r.route_id,
                    r.fingerprints.directed,
                    r.chain_id as i64,
                    r.start_token,
                    r.hops as i64,
                    r.route_type.as_str(),
                    r.lifecycle.as_str(),
                    r.amount_in.to_string(),
                    r.amount_out.to_string(),
                    r.notional_usd.to_string(),
                    r.net_usd.to_string(),
                    r.block_number as i64,
                    now,
                    to_json(r)?,
                ],
            )?;
            tx.execute(
                "INSERT INTO route_fingerprints(fingerprint, undirected, pool_sequence, token_sequence, start_asset, route_id, json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(fingerprint) DO UPDATE SET route_id=excluded.route_id, json=excluded.json",
                params![
                    r.fingerprints.directed,
                    r.fingerprints.undirected,
                    r.fingerprints.pool_sequence,
                    r.fingerprints.token_sequence,
                    r.fingerprints.start_asset,
                    r.route_id,
                    to_json(&r.fingerprints)?,
                ],
            )?;
            // 注意：M1/M3 结果已随 r 一起写入 dex_routes.json，无需再逐条复制到
            // simulations 表。此前每条路线每轮扫描都会插入 2 行（190 路线 × 1.5s
            // ≈ 250 行/秒，一天 500 万行 / 5.5 GB）。simulations 表只保存
            // 用户通过 POST /api/simulations 主动触发的运行记录。
        }
        Ok(())
    })
}

/// 保存单次模拟结果（API 主动触发时使用）。
pub fn save_simulation(db: &Db, route_id: &str, sim: &SimulationOutcome) -> Result<()> {
    db.execute(
        "INSERT INTO simulations(simulation_id, route_id, level, status, created_at, json)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(simulation_id) DO UPDATE SET status=excluded.status, json=excluded.json",
        params![
            sim.simulation_id,
            route_id,
            sim.level.as_str(),
            sim.status.as_str(),
            dt_to_ms(sim.created_at),
            to_json(sim)?,
        ],
    )?;
    Ok(())
}

pub fn load_simulations(db: &Db, limit: usize) -> Result<Vec<SimulationOutcome>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT json FROM simulations ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<SimulationOutcome>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn save_scan_run(db: &Db, s: &ScanRunStats) -> Result<()> {
    db.execute(
        "INSERT INTO scan_runs(run_id, started_at, elapsed_ms, full_rescan, candidates, requoted,
             reused, visits, truncated, json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(run_id) DO UPDATE SET json=excluded.json",
        params![
            s.run_id,
            s.started_at.map(dt_to_ms).unwrap_or(0),
            s.elapsed_ms as i64,
            s.full_rescan as i64,
            s.candidates as i64,
            s.requoted as i64,
            s.reused as i64,
            s.visits as i64,
            s.truncated as i64,
            to_json(s)?,
        ],
    )?;
    Ok(())
}

pub fn load_scan_runs(db: &Db, limit: usize) -> Result<Vec<ScanRunStats>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM scan_runs ORDER BY started_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<ScanRunStats>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn load_routes(db: &Db, limit: usize) -> Result<Vec<RouteScanRecord>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM dex_routes ORDER BY updated_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(v) = from_json::<RouteScanRecord>(&r?) {
                out.push(v);
            }
        }
        Ok(out)
    })
}

pub fn prune_routes(db: &Db, keep: usize) -> Result<usize> {
    let n = db.execute(
        "DELETE FROM dex_routes WHERE route_id NOT IN (
            SELECT route_id FROM dex_routes ORDER BY updated_at DESC LIMIT ?1
        )",
        params![keep as i64],
    )?;
    Ok(n)
}

pub fn prune_scan_runs(db: &Db, keep: usize) -> Result<usize> {
    let n = db.execute(
        "DELETE FROM scan_runs WHERE run_id NOT IN (
            SELECT run_id FROM scan_runs ORDER BY started_at DESC LIMIT ?1
        )",
        params![keep as i64],
    )?;
    Ok(n)
}

/// 模拟结果按时间保留最近 `keep` 条。
///
/// 历史上 `simulations` 没有清理入口：18 小时就能积累 500 万行 / 5.5 GB。
/// 现在由扫描循环定期裁剪，并在启动时执行一次。
pub fn prune_simulations(db: &Db, keep: usize) -> Result<usize> {
    let n = db.execute(
        "DELETE FROM simulations WHERE simulation_id NOT IN (
            SELECT simulation_id FROM simulations ORDER BY created_at DESC LIMIT ?1
        )",
        params![keep as i64],
    )?;
    Ok(n)
}

// ---------------- 指标事件（V22-101）----------------

/// 批量写入指标事件（幂等：event_id 冲突忽略）。
pub fn save_metric_events(db: &Db, events: &[crate::metrics::MetricEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    db.with_tx(|tx| {
        for e in events {
            tx.execute(
                "INSERT INTO metric_events(event_id, kind, chain_id, metric, value, unit, ts_ms, labels_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(event_id) DO NOTHING",
                params![
                    e.event_id,
                    e.kind,
                    e.chain_id as i64,
                    e.metric,
                    e.value.to_string(),
                    e.unit,
                    e.ts_ms,
                    to_json(&e.labels)?,
                ],
            )?;
        }
        Ok(())
    })
}

/// 指标事件按时间保留最近 `keep` 条（防止无限增长）。
pub fn prune_metric_events(db: &Db, keep: usize) -> Result<usize> {
    let n = db.execute(
        "DELETE FROM metric_events WHERE event_id NOT IN (
            SELECT event_id FROM metric_events ORDER BY ts_ms DESC LIMIT ?1
        )",
        params![keep as i64],
    )?;
    Ok(n)
}

/// 批量写入机会生命周期事件（幂等：event_id 冲突忽略）。
pub fn save_lifecycle_events(
    db: &Db,
    events: &[crate::domain::opportunity::LifecycleEvent],
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    db.with_tx(|tx| {
        for e in events {
            tx.execute(
                "INSERT INTO opportunity_lifecycle_events(event_id, opportunity_id, chain_id, stage, status, ts_ms)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(event_id) DO NOTHING",
                params![e.event_id, e.opportunity_id, e.chain_id as i64, e.stage, e.status, e.ts_ms],
            )?;
        }
        Ok(())
    })
}

/// 机会存活时长（毫秒）：DISCOVERED → 首次 EXPIRED 的样本（V23-501）。
pub fn opportunity_lifetimes(db: &Db, limit: usize) -> Result<Vec<i64>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT MIN(CASE WHEN stage='DISCOVERED' THEN ts_ms END) AS first_seen,
                    MAX(CASE WHEN stage='EXPIRED' THEN ts_ms END) AS expired_at
             FROM opportunity_lifecycle_events
             GROUP BY opportunity_id
             HAVING first_seen IS NOT NULL AND expired_at IS NOT NULL
             ORDER BY expired_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let a: i64 = r.get(0)?;
            let b: i64 = r.get(1)?;
            Ok(b - a)
        })?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    })
}

/// 读取竞争者交易（json 原文，按区块倒序）。
pub fn competitor_transactions(db: &Db, limit: usize) -> Result<Vec<String>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT json FROM competitor_transactions ORDER BY block DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    })
}

/// 竞争者交易汇总：(总数, HIGH 置信数, 涉及区块数)。
pub fn competitor_counts(db: &Db) -> Result<(usize, usize, usize)> {
    db.with(|c| {
        let total: i64 =
            c.query_row("SELECT COUNT(*) FROM competitor_transactions", [], |r| r.get(0))?;
        let high: i64 = c.query_row(
            "SELECT COUNT(*) FROM competitor_transactions WHERE confidence='HIGH'",
            [],
            |r| r.get(0),
        )?;
        let blocks: i64 = c.query_row(
            "SELECT COUNT(DISTINCT block) FROM competitor_transactions",
            [],
            |r| r.get(0),
        )?;
        Ok((total as usize, high as usize, blocks as usize))
    })
}

/// 竞争者交易落库（V23-402）。
pub fn save_competitor_transactions(
    db: &Db,
    items: &[crate::competition::DetectedArb],
) -> Result<usize> {
    if items.is_empty() {
        return Ok(0);
    }
    let mut n = 0usize;
    db.with_tx(|tx| {
        for it in items {
            let changed = tx.execute(
                "INSERT INTO competitor_transactions(tx_hash, chain_id, block, tx_index, log_count, confidence, json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(tx_hash) DO NOTHING",
                params![
                    it.tx_hash,
                    it.chain_id as i64,
                    it.block as i64,
                    it.tx_index as i64,
                    it.log_count as i64,
                    it.confidence,
                    to_json(it)?,
                ],
            )?;
            n += changed;
            if !it.sender.is_empty() {
                tx.execute(
                    "INSERT INTO competitor_addresses(address, chain_id, tx_count, last_block, json)
                     VALUES(?1,?2,1,?3,'{}')
                     ON CONFLICT(address) DO UPDATE SET
                       tx_count = competitor_addresses.tx_count + 1,
                       last_block = excluded.last_block",
                    params![it.sender, it.chain_id as i64, it.block as i64],
                )?;
            }
        }
        Ok(())
    })?;
    Ok(n)
}

/// 读取竞争模拟结果 (state, net_usd)（V22-805/806）。
pub fn paper_competition_results(db: &Db, limit: usize) -> Result<Vec<(String, String)>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT state, net_usd FROM paper_competition_results ORDER BY ts_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    })
}

/// 竞争模拟结果落库（V22-803）。
pub fn save_paper_competition_result(
    db: &Db,
    result_id: &str,
    opportunity_id: &str,
    state: &str,
    p_capture: &str,
    net_usd: &str,
    json: &str,
) -> Result<()> {
    db.execute(
        "INSERT INTO paper_competition_results(result_id, opportunity_id, state, p_capture, net_usd, ts_ms, json)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(result_id) DO NOTHING",
        params![
            result_id,
            opportunity_id,
            state,
            p_capture,
            net_usd,
            crate::domain::now_ms(),
            json
        ],
    )?;
    Ok(())
}

/// 读取某条路线的记录 JSON（V22-309 回放用）。
pub fn route_record_json(db: &Db, route_id: &str) -> Result<Option<String>> {
    db.with(|c| {
        let mut stmt = c.prepare("SELECT json FROM dex_routes WHERE route_id = ?1")?;
        let mut rows = stmt.query(params![route_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(row.get::<_, String>(0)?));
        }
        Ok(None)
    })
}

/// 失败归因分布（V22-504）：按 12 类统计 failure_cost 事件。
pub fn failure_attribution_counts(db: &Db) -> Result<Vec<(String, usize)>> {
    db.with(|c| {
        let mut stmt = c.prepare(
            "SELECT COALESCE(json_extract(labels_json, '$.class'), 'UNKNOWN') AS cls, COUNT(*) AS n
             FROM metric_events WHERE metric = 'failure_cost'
             GROUP BY cls ORDER BY n DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let cls: String = r.get(0)?;
            let n: i64 = r.get(1)?;
            Ok((cls, n as usize))
        })?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    })
}

/// 竞争决策落库（V23-603）。
pub fn save_competition_decision(
    db: &Db,
    decision_id: &str,
    route_id: &str,
    chain_id: u64,
    tier: &str,
    decision: &str,
    expected_net: &str,
    json: &str,
) -> Result<()> {
    db.execute(
        "INSERT INTO competition_decisions(decision_id, route_id, chain_id, tier, decision, expected_net, ts_ms, json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(decision_id) DO NOTHING",
        params![
            decision_id,
            route_id,
            chain_id as i64,
            tier,
            decision,
            expected_net,
            crate::domain::now_ms(),
            json
        ],
    )?;
    Ok(())
}

/// 生命周期事件按时间保留最近 `keep` 条。
pub fn prune_lifecycle_events(db: &Db, keep: usize) -> Result<usize> {
    let n = db.execute(
        "DELETE FROM opportunity_lifecycle_events WHERE event_id NOT IN (
            SELECT event_id FROM opportunity_lifecycle_events ORDER BY ts_ms DESC LIMIT ?1
        )",
        params![keep as i64],
    )?;
    Ok(n)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::dec;
    use crate::domain::types::*;

    fn sample_opportunity() -> Opportunity {
        use crate::domain::market::{CostBreakdown, DataQuality, ModelLevel, ProfitScenario, RiskScore};
        use crate::domain::opportunity::FundingSource;
        Opportunity {
            opportunity_id: "opp_test".into(),
            strategy_type: StrategyType::DexTriple,
            chain_id: 42161,
            chain_name: "Arbitrum".into(),
            dex_route: None,
            cex_exchange: None,
            cex_symbol: None,
            direction: None,
            target_notional: dec("1000"),
            target_qty: dec("0.3579"),
            required_dex_inventory: vec![],
            required_cex_inventory: vec![],
            theoretical_edge_bps: dec("12"),
            executable_gross_profit: dec("9.4"),
            total_cost: dec("6.1"),
            conservative_net_profit: dec("3.3"),
            stress_net_profit: dec("1.2"),
            worst_case_loss: dec("4.8"),
            costs: CostBreakdown::new(),
            scenario: ProfitScenario::zero(),
            risk_score: RiskScore::from_components(vec![]),
            data_quality: DataQuality::High,
            model_level: ModelLevel::M2,
            confidence: "M2".into(),
            m2: None,
            quote_timestamp: crate::domain::now(),
            quote_block_number: 1,
            book_age_ms: 0,
            quote_age_ms: 0,
            status: OppStatus::Actionable,
            lifecycle: OpportunityLifecycle::Simulated,
            route_fingerprint: None,
            rejections: vec![],
            capacity_usd: dec("5000"),
            funding: FundingSource::OwnCapital,
            hedge_order: None,
            detected_at: crate::domain::now(),
            expires_at: crate::domain::now(),
        }
    }

    #[test]
    fn opportunity_roundtrip() {
        let db = Db::open_memory().unwrap();
        let o = sample_opportunity();
        save_opportunity(&db, &o).unwrap();
        let loaded = load_opportunities(&db, 10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].opportunity_id, o.opportunity_id);
        assert_eq!(loaded[0].conservative_net_profit, dec("3.3"));
        assert_eq!(loaded[0].status, OppStatus::Actionable);
    }

    #[test]
    fn scanner_state_roundtrip() {
        let db = Db::open_memory().unwrap();
        let mut s = ScannerStatus::default();
        s.state = "RUNNING".into();
        s.detected = 42;
        save_scanner_state(&db, &s).unwrap();
        let loaded = load_scanner_state(&db).unwrap().unwrap();
        assert_eq!(loaded.state, "RUNNING");
        assert_eq!(loaded.detected, 42);
    }

    #[test]
    fn prune_keeps_latest() {
        let db = Db::open_memory().unwrap();
        for i in 0..10 {
            let mut o = sample_opportunity();
            o.opportunity_id = format!("opp_{i}");
            o.detected_at = crate::domain::now() + chrono::Duration::milliseconds(i);
            save_opportunity(&db, &o).unwrap();
        }
        prune_opportunities(&db, 3).unwrap();
        assert_eq!(db.count("opportunities").unwrap(), 3);
        let left = load_opportunities(&db, 10).unwrap();
        assert_eq!(left[0].opportunity_id, "opp_9");
    }
}