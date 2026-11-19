//! 风控模块：参数基线目录、交易前评估引擎与熔断器。
//!
//! - [`catalog`]：v1.0 参数基线、分组与 §10 熔断规则表；
//! - [`engine`]：覆盖解析（只允许更严格）与交易前硬限制评估；
//! - [`breakers`]：DEX 回滚 / 对冲异常 / 资金类滑动窗口熔断。

pub mod breakers;
pub mod catalog;
pub mod engine;

pub use breakers::{BreakerTrigger, CircuitBreakers};
pub use catalog::{baseline, circuit_rules, groups, CircuitRule, ParamGroup};
pub use engine::RiskEngine;
