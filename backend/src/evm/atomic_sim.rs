//! 整笔原子交易回放（M2-TX）：最小 executor 从 Uniswap V3 池闪电借出起始 Token，
//! 在一次事务里完成“逐跳 swap + 归还本息 + 利润回款”，任一跳失败整笔回滚。
//!
//! - `simulate_route`：REVM fork 回放，返回利润/实际 gas；
//! - `estimate_route_gas`：RPC `eth_estimateGas` + state override 注入 executor 代码。
use revm::primitives::{Address, U256};

use super::abi::{self, Arg};
use super::revm_sim::{call_on_fork_with, ForkCall, ForkOverrides, ForkResult};
use super::RpcClient;

const EXECUTOR_BYTECODE: &str = include_str!("executor.hex");

#[derive(Clone, Debug)]
pub struct AtomicHop {
    pub router: Address,
    pub token_in: Address,
    pub token_out: Address,
    /// "UNISWAP_V3" / "UNISWAP_V2"
    pub kind: String,
    pub fee_bps: i64,
    pub amount_in: U256,
}

#[derive(Clone, Debug, Default)]
pub struct AtomicResult {
    pub success: bool,
    /// 本笔交易最终利润（tokenIn 原始单位）。
    pub profit: U256,
    pub gas_used: u64,
    pub revert: Option<String>,
    pub block: u64,
    pub hops: usize,
}

pub fn executor_address() -> Address {
    let mut a = [0u8; 20];
    a[18] = 0xec;
    a[19] = 0x01;
    Address::from(a)
}

fn caller_address() -> Address {
    Address::from([0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbe, 0xef])
}

fn word_addr(a: Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_slice());
    w
}

fn word_u256(v: U256) -> [u8; 32] {
    v.to_be_bytes::<32>()
}

fn v3_call(h: &AtomicHop, executor: Address) -> Vec<u8> {
    let selector =
        abi::selector("exactInputSingle((address,address,uint24,address,uint256,uint256,uint160))");
    let mut out = selector.to_vec();
    out.extend(word_addr(h.token_in));
    out.extend(word_addr(h.token_out));
    out.extend(word_u256(U256::from((h.fee_bps.max(0) as u64) * 100)));
    out.extend(word_addr(executor));
    out.extend(word_u256(h.amount_in));
    out.extend(word_u256(U256::ZERO));
    out.extend(word_u256(U256::ZERO));
    out
}

fn v2_call(h: &AtomicHop, executor: Address, deadline: U256) -> Vec<u8> {
    let selector =
        abi::selector("swapExactTokensForTokens(uint256,uint256,address[],address,uint256)");
    let mut out = selector.to_vec();
    out.extend(word_u256(h.amount_in));
    out.extend(word_u256(U256::ZERO));
    out.extend(word_u256(U256::from(5u64 * 32)));
    out.extend(word_addr(executor));
    out.extend(word_u256(deadline));
    out.extend(word_u256(U256::from(2u64)));
    out.extend(word_addr(h.token_in));
    out.extend(word_addr(h.token_out));
    out
}

fn decode_hex(hex: &str) -> Vec<u8> {
    abi::hex_decode(hex.trim()).unwrap_or_default()
}

/// 构造 executor.execute 的 calldata。
fn prepare(
    flash_pool: Address,
    token_in: Address,
    hops: &[AtomicHop],
    block: u64,
) -> Result<(Address, Vec<u8>), String> {
    if hops.is_empty() || hops.len() > 5 {
        return Err("hops 必须在 1..=5".into());
    }
    let executor = executor_address();
    let deadline = U256::from(block + 300);
    let mut blob = Vec::new();
    let mut lengths = [0u64; 5];
    let mut targets = [Address::ZERO; 5];
    let mut tokens = [Address::ZERO; 5];
    let mut amounts = [U256::ZERO; 5];
    for (i, h) in hops.iter().enumerate() {
        let call = match h.kind.as_str() {
            "UNISWAP_V3" => v3_call(h, executor),
            "UNISWAP_V2" => v2_call(h, executor, deadline),
            other => return Err(format!("原子回放暂不支持 {other}")),
        };
        lengths[i] = call.len() as u64;
        targets[i] = h.router;
        tokens[i] = h.token_in;
        amounts[i] = h.amount_in;
        blob.extend(call);
    }
    let amount_in = hops[0].amount_in;

    let selector = abi::selector(
        "execute(address,address,uint256,address,address[5],address[5],uint256[5],uint256[5],bytes,uint8)",
    );
    let mut data = selector.to_vec();
    data.extend(word_addr(flash_pool));
    data.extend(word_addr(token_in));
    data.extend(word_u256(amount_in));
    data.extend(word_addr(executor));
    for a in targets {
        data.extend(word_addr(a));
    }
    for a in tokens {
        data.extend(word_addr(a));
    }
    for a in amounts {
        data.extend(word_u256(a));
    }
    for l in lengths {
        data.extend(word_u256(U256::from(l)));
    }
    data.extend(word_u256(U256::from(26u64 * 32)));
    data.extend(word_u256(U256::from(hops.len() as u64)));
    data.extend(word_u256(U256::from(blob.len() as u64)));
    data.extend(blob);
    let rem = data.len() % 32;
    if rem != 0 {
        data.extend(vec![0u8; 32 - rem]);
    }
    Ok((executor, data))
}

/// 用闪电池 + executor 执行整条路线，返回利润与 Gas。
pub async fn simulate_route(
    rpc: &RpcClient,
    chain_id: u64,
    block: u64,
    flash_pool: Address,
    token_in: Address,
    _token_out: Address,
    hops: &[AtomicHop],
    min_profit: U256,
) -> Result<AtomicResult, String> {
    let (executor, data) = prepare(flash_pool, token_in, hops, block)?;
    let code = decode_hex(EXECUTOR_BYTECODE);
    if code.is_empty() {
        return Err("executor 字节码缺失".into());
    }
    let overrides = ForkOverrides {
        code: vec![(executor, code)],
        storage: Vec::new(),
    };
    let call = ForkCall {
        from: caller_address(),
        to: executor,
        data,
        gas_limit: 10_000_000,
        value: U256::ZERO,
    };
    let res: ForkResult = call_on_fork_with(rpc, chain_id, block, call, overrides).await?;
    let profit = abi::word_at(&res.output, 0)
        .and_then(|w| {
            let mut b = [0u8; 32];
            b.copy_from_slice(w);
            Some(U256::from_be_bytes(b))
        })
        .unwrap_or(U256::ZERO);
    let success = res.success && profit >= min_profit;
    Ok(AtomicResult {
        success,
        profit,
        gas_used: res.gas_used,
        revert: res.revert_reason,
        block: res.block,
        hops: hops.len(),
    })
}

/// 逐路线 `eth_estimateGas`（state override 注入 executor 代码）。
pub async fn estimate_route_gas(
    rpc: &RpcClient,
    block: u64,
    flash_pool: Address,
    token_in: Address,
    hops: &[AtomicHop],
) -> Result<u64, String> {
    let (executor, data) = prepare(flash_pool, token_in, hops, block)?;
    let exec = format!("{executor:?}");
    let from = format!("{:?}", caller_address());
    rpc.estimate_gas_with_code(
        &from,
        &exec,
        &abi::hex_encode(&data),
        block,
        &format!("0x{}", EXECUTOR_BYTECODE.trim()),
    )
    .await
}

#[allow(dead_code)]
fn _arg_marker(a: Address) -> Arg {
    Arg::Address(a.as_slice().try_into().unwrap_or([0u8; 20]))
}
