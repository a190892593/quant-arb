//! SQLite 持久化层。
//!
//! 第一阶段为 Local First（TDD §2.1），避免额外安装 PostgreSQL 服务：
//! 使用嵌入式 SQLite，单文件数据库即可完成备份/恢复，表结构仍对应 TDD §10 的实体。
//! 迁移到 PostgreSQL 时只需替换本层，领域模型与策略引擎不变。
//!
//! 约定：
//! - 结构化对象（机会、执行、风险事件等）以 JSON 列保存，关键筛选字段单独建列。
//! - 账本流水与账户按列保存，保证可审计查询。
//! - 所有金额以 TEXT 保存 Decimal 原文，禁止用浮点。

pub mod repo;
pub mod schema;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 数据库句柄。内部单连接 + 互斥，适配本地单用户场景。
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    path: String,
}

impl Db {
    /// 打开（或创建）数据库并执行迁移。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建数据目录失败: {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("打开数据库失败: {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )
        .context("设置 SQLite PRAGMA 失败")?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: path.display().to_string(),
        };
        db.migrate()?;
        Ok(db)
    }

    /// 内存数据库（测试用）。
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: ":memory:".into(),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(schema::MIGRATIONS)
            .context("执行数据库迁移失败")?;
        Ok(())
    }

    /// 在连接锁内执行操作。
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }

    /// 在事务中执行操作。
    pub fn with_tx<T>(&self, f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().context("开启事务失败")?;
        let out = f(&tx)?;
        tx.commit().context("提交事务失败")?;
        Ok(out)
    }

    /// 执行一条写入语句。
    pub fn execute(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(sql, params).with_context(|| format!("执行 SQL 失败: {sql}"))?;
        Ok(n)
    }

    /// 简单计数。
    pub fn count(&self, table: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let n: i64 = conn.query_row(&sql, [], |r| r.get(0))?;
        Ok(n)
    }

    /// 健康检查。
    pub fn health(&self) -> bool {
        self.with(|c| {
            let v: i64 = c.query_row("SELECT 1", [], |r| r.get(0))?;
            Ok(v)
        })
        .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_create_expected_tables() {
        let db = Db::open_memory().unwrap();
        for table in [
            "chains",
            "tokens",
            "pools",
            "pool_states",
            "cex_instruments",
            "opportunities",
            "trade_plans",
            "executions",
            "cex_fills",
            "accounts",
            "ledger_entries",
            "risk_events",
            "risk_params",
            "risk_overrides",
            "system_controls",
            "system_configs",
            "audit_jobs",
            "audit_reports",
            "inventory_snapshots",
            "scanner_state",
            "dex_routes",
            "route_fingerprints",
            "simulations",
            "scan_runs",
        ] {
            let n = db.count(table).unwrap_or(-1);
            assert!(n >= 0, "表 {table} 不存在");
        }
    }

    #[test]
    fn transaction_rolls_back_on_error() {
        let db = Db::open_memory().unwrap();
        let r: Result<()> = db.with_tx(|tx| {
            tx.execute("INSERT INTO system_configs(key, value, updated_at) VALUES('x','1',0)", [])?;
            anyhow::bail!("boom");
        });
        assert!(r.is_err());
        let n: i64 = db
            .with(|c| Ok(c.query_row("SELECT COUNT(*) FROM system_configs WHERE key='x'", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("arbx-db-test-{}", crate::domain::new_id("t")));
        let path = dir.join("t.db");
        {
            let db = Db::open(&path).unwrap();
            db.execute(
                "INSERT INTO system_configs(key, value, updated_at) VALUES(?1,?2,?3)",
                rusqlite::params!["k", "v", 1i64],
            )
            .unwrap();
        }
        {
            let db = Db::open(&path).unwrap();
            let v: String = db
                .with(|c| Ok(c.query_row("SELECT value FROM system_configs WHERE key='k'", [], |r| r.get(0))?))
                .unwrap();
            assert_eq!(v, "v");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
