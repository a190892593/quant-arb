//! DEX 池状态与报价数学。
//!
//! 模块分层（TDD §4）：
//! - [`math`]：纯报价数学（V2 恒定乘积、V3 集中流动性、StableSwap、Weighted），
//!   不 panic、除零返回 0，统一 `round_dp(18)`。
//! - [`pool`]：池运行时（`PoolRef` + `PoolState`）、池集合索引、单池与多跳路径报价。
//! - [`graph`]：路由图搜索（三角 / 四跳简单环、跨池闭环）。
//!
//! 使用约束：
//! - 只有 [`PoolSet::execution_pools`] 返回的池（已验证、允许执行、状态未过期且协议
//!   类型可执行）才可以进入真实执行计划；报价阶段可以更宽松，但必须携带
//!   `block_number` 与状态年龄供上层校验。
//! - 报价单位是池内**原始单位（raw units）**，V3 的 sqrtPrice 亦按原始单位换算。

pub mod graph;
pub mod math;
pub mod pool;

pub use graph::{HopEdge, enumerate_cross_pool_cycles, enumerate_triangles};
pub use pool::{PathLegQuote, PathQuote, PoolRuntime, PoolSet, QuoteError, SwapQuote};
