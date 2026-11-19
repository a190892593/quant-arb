//! CEX 市场数据层（TDD §5）。
//!
//! - [`book`]：订单簿引擎，负责 Snapshot 安装、增量按序应用、序列号缺口/乱序检测与健康度。
//! - [`depth`]：基于盘口真实档位的成交模拟、价格保护与深度占用计算。
//! - [`binance`]：Binance 公共只读行情适配器（无 API Key、无下单）。
//!
//! 语义基线：成交估算必须逐档消耗盘口深度，禁止使用中间价或 K 线价假设整笔成交
//! （TDD §5.3）；只有完整且未超时的盘口才能生成可执行机会（风控 §6）。

pub mod binance;
pub mod book;
pub mod depth;

pub use book::{BookEngine, BookUpdateResult};
pub use depth::{DepthQuote, depth_within_bps, max_qty_within_depth_pct, simulate_order};
