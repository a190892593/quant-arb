//! 诊断用：单次 eth_call 与批量 eth_call 的耗时。
use std::time::Instant;

use arb_backend::evm::abi::{self, Address};
use arb_backend::evm::{Call, RpcClient};

fn addr(s: &str) -> Address {
    let s = s.trim_start_matches("0x");
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

#[tokio::main]
async fn main() {
    let c = RpcClient::new(
        "https://arb1.arbitrum.io/rpc",
        Some("https://arbitrum-one.publicnode.com"),
    );
    let usdc = addr("0xaf88d065e77c8cC2239327C5EDb3A432268e5831");
    let weth = addr("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1");

    let t = Instant::now();
    let one = c
        .batch_eth_call(
            &[Call { to: usdc, data: abi::encode_call("decimals()", &[]) }],
            None,
        )
        .await;
    println!("batch x1: {:?} in {:?}", one.map(|v| v.len()), t.elapsed());

    let t = Instant::now();
    let two = c
        .batch_eth_call(
            &[
                Call { to: usdc, data: abi::encode_call("decimals()", &[]) },
                Call { to: weth, data: abi::encode_call("decimals()", &[]) },
            ],
            None,
        )
        .await;
    println!("batch x2: {:?} in {:?}", two.map(|v| v.len()), t.elapsed());

    let t = Instant::now();
    let mut calls = Vec::new();
    for _ in 0..25 {
        calls.push(Call { to: usdc, data: abi::encode_call("decimals()", &[]) });
    }
    let many = c.batch_eth_call(&calls, None).await;
    println!("batch x25: {:?} in {:?}", many.map(|v| v.len()), t.elapsed());
}
