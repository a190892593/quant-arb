//! 本地行情源。
//!
//! 第一阶段默认使用 [`demo`] 合成市场：可离线运行、可复现、能稳定产生
//! 三角/跨池/CEX–DEX 机会与各类失败场景，便于验收 Paper 全链路。
//! 实时公共行情适配器见 [`crate::cex::binance`]，由 `feed.mode = "live"` 切换。

pub mod demo;

use rust_decimal::Decimal;
use std::collections::HashMap;

/// 行情源统一输出的市场快照。
pub struct MarketSnapshot {
    pub prices: HashMap<String, Decimal>,
    pub head_block: u64,
}
