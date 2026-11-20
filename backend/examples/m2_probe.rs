//! REVM 分叉探针：在最新区块上用 QuoterV2 复核 1 WETH → USDC。
//!
//! 运行：`cargo run --example m2_probe`
//! 只读链上状态，不广播交易。

use std::time::Instant;

use arb_backend::config::Config;
use arb_backend::evm::abi::{self, Arg};
use arb_backend::evm::revm_sim::{call_on_fork, ForkCall};
use arb_backend::evm::RpcClient;
use revm::primitives::Address;

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let cfg = Config::load(&std::env::current_dir()?)?;
        let rpc = RpcClient::new(&cfg.live.rpc_primary, Some(&cfg.live.rpc_fallback));
        let block = rpc.block_number().await.map_err(anyhow::Error::msg)?;
        println!("block={block}");

        let quoter = Address::from(
            abi::parse_address("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
                .map_err(anyhow::Error::msg)?,
        );
        let weth = Address::from(
            abi::parse_address("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1")
                .map_err(anyhow::Error::msg)?,
        );
        let usdc = Address::from(
            abi::parse_address("0xaf88d065e77c8cC2239327C5EDb3A432268e5831")
                .map_err(anyhow::Error::msg)?,
        );

        let sig = "quoteExactInputSingle((address,address,uint256,uint24,uint160))";
        let sel = abi::selector(sig);
        println!("selector={}", abi::hex_encode(&sel));
        let mut data = sel.to_vec();
        data.extend(abi::encode_args(&[
            Arg::Address(weth.into_array()),
            Arg::Address(usdc.into_array()),
            Arg::Uint(1_000_000_000_000_000_000u128),
            Arg::Uint(500),
            Arg::Uint(0),
        ]));

        let started = Instant::now();
        let out = call_on_fork(
            &rpc,
            42161,
            block,
            ForkCall {
                from: Address::ZERO,
                to: quoter,
                data,
                gas_limit: 3_000_000,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let elapsed = started.elapsed();
        println!(
            "success={} gas={} elapsed={:?} revert={:?}",
            out.success, out.gas_used, elapsed, out.revert_reason
        );
        if out.success {
            if let Some(w) = abi::words(&out.output).first() {
                let v = abi::decode_u128(w).unwrap_or(0);
                println!("1 WETH -> {} USDC (raw={v})", v as f64 / 1e6);
            }
        } else {
            println!("output={}", abi::hex_encode(&out.output));
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}
