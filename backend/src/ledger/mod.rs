//! 统一资金账本（TDD §9、PRD §5.7）。
//!
//! 子模块：
//! - [`engine`]：追加式流水、余额聚合、对账、交易统计（唯一资金变动入口）。
//! - [`inventory`]：场所账户、库存快照、合并净持仓与跨场所净敞口。
//! - [`performance`]：由账本 + 交易统计生成绩效报告与权益曲线。
//!
//! 原则：流水不可覆盖只能追加；余额是流水的累积结果；任何资金变动都必须走
//! [`engine::LedgerEngine::post`] / [`engine::LedgerEngine::post_batch`]，报表与前端不得自行
//! 推算余额。

pub mod engine;
pub mod inventory;
pub mod performance;

pub use engine::{LedgerEngine, LedgerError, PostRequest};
pub use inventory::{combined_positions, inventory_snapshots, venue_accounts};
pub use performance::{build_report, equity_series};
