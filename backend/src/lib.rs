//! ARBX 原子套利与 CEX–DEX 对冲交易系统 —— 核心后端库。
//!
//! 第一阶段：本地运行、Paper Trading、无真实下单/广播。
//!
//! 模块分层（对应 TDD §1.2）：
//! - [`domain`] 领域模型：金额、市场、机会、计划、执行、风控、账本、报表。
//! - [`dex`] 链上池状态、报价数学与路由搜索。
//! - [`cex`] 订单簿引擎、深度成交模拟与交易所适配。
//! - [`strategy`] 统一策略引擎：成本模型、评分、三类机会搜索。
//! - [`risk`] 风控参数基线、评估引擎与熔断器。
//! - [`ledger`] 资金账本、库存与对账。
//! - [`paper`] Paper Trading（DEX 模拟、CEX 深度成交、对冲状态机）。
//! - [`feed`] 行情来源（本地合成市场）。
//! - [`api`] Axum HTTP + WebSocket 服务。
//! - [`db`] SQLite 持久化（本地单机，替代独立 PostgreSQL 服务）。
//! - [`engine`] 应用状态、扫描编排与启动恢复。

pub mod api;
pub mod attribution;
pub mod bid;
pub mod cex;
pub mod competition;
pub mod config;
pub mod db;
pub mod dex;
pub mod domain;
pub mod engine;
pub mod evm;
pub mod feed;
pub mod ledger;
pub mod metrics;
pub mod paper;
pub mod price;
pub mod risk;
pub mod search_config;
pub mod source_config;
pub mod strategy;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod smoke {
    use rust_decimal::Decimal;

    #[test]
    fn deps_available() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t(x INTEGER);").unwrap();
        let v: i64 = c.query_row("SELECT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        let d: Decimal = "1.5".parse().unwrap();
        assert_eq!(d + d, "3.0".parse::<Decimal>().unwrap());
        let _ = toml::from_str::<toml::Value>("a = 1").unwrap();
        let _ = serde_json::json!({"ok": true});
    }
}
