//! 领域模型：金额、枚举、市场、机会、计划、执行、风控、账本、报表。
//!
//! 该层不依赖数据库、网络或 Axum，可被单元测试直接使用。
//! 所有金额与数量使用 [`rust_decimal::Decimal`] 保证账本可核对。

pub mod dex_scan;
pub mod execution;
pub mod ledger;
pub mod market;
pub mod money;
pub mod opportunity;
pub mod plan;
pub mod report;
pub mod risk;
pub mod types;

pub use money::{bps_of, dec, q2, q8, round_step_down, round_step_nearest};

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// 生成带类型前缀的唯一 ID，例如 `opp_7f3c...`。
pub fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4().simple())
}

/// 当前 UTC 时间。
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// 当前毫秒时间戳。
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// 毫秒时间戳转 UTC 时间。
pub fn ms_to_dt(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
}

/// 时间转毫秒时间戳。
pub fn dt_to_ms(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}
