//! 真实 EVM 数据访问：JSON-RPC、最小 ABI，以及 REVM 分叉执行（M2）。
//!
//! - `rpc` / `abi`：只读链上数据（M1 行情与 M2 取数）；
//! - `fork`：按区块号从 RPC 取账号/代码/存储，供 REVM 本地执行；
//! - `revm_sim`：在报价区块的状态上执行 Quoter/Router 调用，复核 M1 报价。

pub mod abi;
pub mod atomic_sim;
pub mod fork;
pub mod revm_sim;
pub mod rpc;

pub use abi::Address;
pub use revm_sim::{ForkCall, ForkResult, call_on_fork};
pub use rpc::{Call, CallResult, Log, RpcClient};
