use arb_backend::evm::abi;
use arb_backend::evm::atomic_sim::{simulate_route, AtomicHop};
use arb_backend::evm::rpc::RpcClient;
use revm::primitives::{Address, U256};

#[tokio::main]
async fn main() {
    let rpc = RpcClient::new("https://arb1.arbitrum.io/rpc", None);
    let block = rpc.block_number().await.unwrap();
    let weth = abi::parse_address("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1").unwrap();
    let usdc = abi::parse_address("0xaf88d065e77c8cC2239327C5EDb3A432268e5831").unwrap();
    let router = abi::parse_address("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45").unwrap();
    let flash = abi::parse_address("0xbe3ad6a5669dc0b8b12febc03608860c31e2eef6").unwrap();
    let hops = vec![
        AtomicHop { router: Address::from(router), token_in: Address::from(usdc), token_out: Address::from(weth), kind: "UNISWAP_V3".into(), fee_bps: 5, amount_in: U256::from(100_000_000u64) },
        AtomicHop { router: Address::from(router), token_in: Address::from(weth), token_out: Address::from(usdc), kind: "UNISWAP_V3".into(), fee_bps: 5, amount_in: U256::from(57_000_000_000_000_000u64) },
    ];
    match simulate_route(&rpc, 42161, block, Address::from(flash), Address::from(usdc), Address::from(usdc), &hops, U256::ZERO).await {
        Ok(r) => println!("success={} profit={} gas={} revert={:?}", r.success, r.profit, r.gas_used, r.revert),
        Err(e) => println!("ERR {e}"),
    }
}

