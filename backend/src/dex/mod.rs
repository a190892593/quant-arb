//! DEX 池状态、准入、交易图、路线搜索、报价数学、Gas 与模拟。
//!
//! 模块分层（PRD §4–§6 / TDD §4）：
//! - [`math`]：纯报价数学（V2 恒定乘积、V3 集中流动性、StableSwap、Weighted）。
//! - [`pool`]：池运行时、池集合索引、单池与多跳路径报价。
//! - [`registry`]：Token / Pool 准入（PRD §3.3、§8）。
//! - [`multigraph`]：Token–Pool 有向多重图，一个池两条有向边，禁止合并报价边。
//! - [`fingerprint`]：有向 / 无方向 / 池序列 / Token 序列指纹（PRD §4.6）。
//! - [`route_search`]：预算约束下的简单环 DFS（跳数、池数、访问量、时间）。
//! - [`optimizer`]：投入金额优化（档位 + 对数网格 + 黄金分割细化）。
//! - [`gas`]：逐项 Gas 估算（AMM / 转账 / Executor / 闪电贷 / Approve）。
//! - [`sim`]：M1 状态一致性模拟、M3 压力模拟、M2 显式不可用。
//! - [`graph`]：历史选择器（三角 / 跨池）保留给旧代码与回归测试。
//!
//! 使用约束：
//! - 只有通过准入（registry）的池才会进入交易图。
//! - 报价单位是池内**原始单位（raw units）**。
//! - 报价可以宽松，但生成可执行机会必须同时通过 M1 模拟与风控。

pub mod discovery;
pub mod fingerprint;
pub mod gas;
pub mod graph;
pub mod math;
pub mod multigraph;
pub mod optimizer;
pub mod pool;
pub mod registry;
pub mod route_search;
pub mod sim;

pub use gas::{GasParams, amm_swap_units};
pub use graph::{HopEdge, enumerate_cross_pool_cycles, enumerate_triangles};
pub use multigraph::{GraphEdge, TokenGraph};
pub use pool::{PathLegQuote, PathQuote, PoolRuntime, PoolSet, QuoteError, SwapQuote};
pub use registry::{PoolLimits, TokenRegistry};
pub use route_search::{RouteCandidate, SearchLimits, SearchOutcome, SearchStats};
