use arb_backend::evm::{abi, atomic_sim, revm_sim::{call_on_fork_with, ForkCall, ForkOverrides}};
use arb_backend::evm::rpc::RpcClient;
use revm::primitives::{Address, U256};

fn mapping_slot(addr: Address, slot: u64) -> U256 {
    let mut buf = [0u8; 64];
    buf[12..32].copy_from_slice(addr.as_slice());
    buf[32 + 24..].copy_from_slice(&slot.to_be_bytes());
    U256::from_be_bytes(abi::keccak256(&buf))
}

#[tokio::main]
async fn main() {
    let rpc = RpcClient::new("https://arb1.arbitrum.io/rpc", None);
    let block = rpc.block_number().await.unwrap();
    let weth = Address::from(abi::parse_address("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1").unwrap());
    let executor = atomic_sim::executor_address();
    let caller = Address::from([0u8,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0xbe,0xef]);
    let amount = U256::from(1_000_000_000_000_000_000u64);
    let mut overrides = ForkOverrides::default();
    for slot in 0..=20u64 {
        overrides.storage.push((weth, mapping_slot(executor, slot), amount));
    }
    let mut data = abi::selector("balanceOf(address)").to_vec();
    let mut w = [0u8;32]; w[12..].copy_from_slice(executor.as_slice());
    data.extend(w);
    let call = ForkCall { from: caller, to: weth, data, gas_limit: 1_000_000 };
    let res = call_on_fork_with(&rpc, 42161, block, call, overrides).await.unwrap();
    let out = abi::word_at(&res.output,0).unwrap();
    let mut b=[0u8;32]; b.copy_from_slice(out);
    println!("success={} balance={}", res.success, U256::from_be_bytes(b));
}
