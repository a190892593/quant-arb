//! REVM 分叉执行（M2）：在报价区块的链上状态上本地执行一次调用。
//!
//! 用途：
//! - 用 QuoterV2 / Router 的真实字节码与真实存储，独立复核 M1 的报价；
//! - 返回执行结果、输出字节、gas 与回滚原因，供策略做 M2 判定。
//!
//! 注意：本模块只读状态、不落盘、不广播；`DatabaseRef` 通过 RPC 按需取数并缓存。

use revm::context::{BlockEnv, TxEnv};
use revm::context_interface::result::ExecutionResult;
use revm::database_interface::WrapDatabaseRef;
use revm::primitives::{Address, Bytes, TxKind, U256};
use revm::{Context, ExecuteEvm, MainBuilder, MainContext};

use super::fork::ForkDb;
use super::RpcClient;

/// 一次分叉调用。
#[derive(Clone, Debug)]
pub struct ForkCall {
    pub from: Address,
    pub to: Address,
    pub data: Vec<u8>,
    pub gas_limit: u64,
    pub value: U256,
}

/// 分叉执行结果。
#[derive(Clone, Debug)]
pub struct ForkResult {
    pub success: bool,
    pub output: Vec<u8>,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
    pub block: u64,
}

/// 在指定区块的链上状态上执行一次只读调用。
pub async fn call_on_fork(
    rpc: &RpcClient,
    chain_id: u64,
    block: u64,
    call: ForkCall,
) -> Result<ForkResult, String> {
    call_on_fork_with(rpc, chain_id, block, call, ForkOverrides::default()).await
}

#[derive(Default, Clone)]
pub struct ForkOverrides {
    pub code: Vec<(Address, Vec<u8>)>,
    pub storage: Vec<(Address, U256, U256)>,
}

pub async fn call_on_fork_with(
    rpc: &RpcClient,
    chain_id: u64,
    block: u64,
    call: ForkCall,
    overrides: ForkOverrides,
) -> Result<ForkResult, String> {
    let (timestamp, base_fee) = rpc.block_header(block).await.unwrap_or((0, 0));
    let mut db = ForkDb::new(rpc.clone(), block).fund(
        call.from,
        U256::from(1_000_000_000_000_000_000u64) * U256::from(10u64),
    );
    for (addr, code) in &overrides.code {
        db.set_code(*addr, code.clone());
    }
    for (addr, slot, value) in &overrides.storage {
        db.set_storage(*addr, *slot, *value);
    }

    let mut ctx = Context::mainnet().with_db(WrapDatabaseRef(db));
    ctx.cfg.chain_id = chain_id;
    // 只读模拟：不校验 nonce；gas 费仍按区块 basefee 计入。
    ctx.cfg.disable_nonce_check = true;
    ctx.block = BlockEnv {
        number: U256::from(block),
        beneficiary: Address::ZERO,
        timestamp: U256::from(timestamp),
        gas_limit: 30_000_000,
        basefee: base_fee as u64,
        difficulty: U256::ZERO,
        prevrandao: Some(revm::primitives::B256::ZERO),
        ..Default::default()
    };
    let mut evm = ctx.build_mainnet();

    let tx = TxEnv {
        caller: call.from,
        kind: TxKind::Call(call.to),
        gas_limit: call.gas_limit,
        gas_price: base_fee.max(1) as u128,
        value: call.value,
        data: Bytes::from(call.data),
        nonce: 0,
        chain_id: Some(chain_id),
        ..Default::default()
    };

    let executed = tokio::task::block_in_place(|| evm.transact(tx))
        .map_err(|e| format!("REVM 执行失败: {e:?}"))?;

    Ok(match executed.result {
        ExecutionResult::Success { output, gas, .. } => ForkResult {
            success: true,
            output: output.into_data().to_vec(),
            gas_used: gas.tx_gas_used(),
            revert_reason: None,
            block,
        },
        ExecutionResult::Revert { output, gas, .. } => ForkResult {
            success: false,
            output: output.to_vec(),
            gas_used: gas.tx_gas_used(),
            revert_reason: Some(format!("revert: {}", super::abi::hex_encode(&output))),
            block,
        },
        ExecutionResult::Halt { reason, gas, .. } => ForkResult {
            success: false,
            output: Vec::new(),
            gas_used: gas.tx_gas_used(),
            revert_reason: Some(format!("halt: {reason:?}")),
            block,
        },
    })
}
