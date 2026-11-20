use arb_backend::evm::abi;
use arb_backend::evm::atomic_sim::{estimate_route_gas, AtomicHop};
use arb_backend::evm::rpc::RpcClient;
use revm::primitives::{Address, U256};

#[tokio::main]
async fn main() {
    let rpc = RpcClient::new("https://arbitrum-one.publicnode.com", None);
    let block = rpc.block_number().await.unwrap();
    let weth = Address::from(abi::parse_address("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1").unwrap());
    let usdc = Address::from(abi::parse_address("0xaf88d065e77c8cC2239327C5EDb3A432268e5831").unwrap());
    let router = Address::from(abi::parse_address("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45").unwrap());
    let flash = Address::from(abi::parse_address("0xbe3ad6a5669dc0b8b12febc03608860c31e2eef6").unwrap());
    let hop = AtomicHop { router, token_in: usdc, token_out: weth, kind: "UNISWAP_V3".into(), fee_bps: 5, amount_in: U256::from(100_000_000u64) };
    match estimate_route_gas(&rpc, block, flash, usdc, &[hop]).await {
        Ok(g) => println!("eth_estimateGas={g}"),
        Err(e) => println!("ERR {e}"),
    }
}
