use arb_backend::dex::math;
use arb_backend::evm::{abi, abi::Arg, rpc::RpcClient};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::str::FromStr;

fn dec_u128(v: u128) -> Decimal { Decimal::from_str(&v.to_string()).unwrap() }

#[tokio::main]
async fn main() {
    let rpc = RpcClient::new("https://base.publicnode.com", None);
    let pool = "0x3548029694fbb241d45fb24ba0cd9c9d4e745f16";
    let weth = abi::parse_address("0x4200000000000000000000000000000000000006").unwrap();
    let res = rpc.call("eth_call", serde_json::json!([{"to": pool, "data": format!("0x{}", hex(&abi::encode_call("getReserves()", &[])))}, "latest"])).await.unwrap();
    let data = abi::hex_decode(res.as_str().unwrap()).unwrap();
    let (r0, r1, _) = abi::decode_get_reserves(&data).unwrap();
    let amount = 1_000_000_000_000_000_000u128;
    let out = rpc.call("eth_call", serde_json::json!([{"to": pool, "data": format!("0x{}", hex(&abi::encode_call("getAmountOut(uint256,address)", &[Arg::Uint(amount), Arg::Address(weth)])))}, "latest"])).await.unwrap();
    let odata = abi::hex_decode(out.as_str().unwrap()).unwrap();
    let onchain = abi::word_at(&odata, 0).and_then(abi::decode_u128).unwrap_or(0);
    let d0 = dec_u128(r0) / Decimal::from(1_000_000_000_000_000_000u64);
    let d1 = dec_u128(r1) / Decimal::from(1_000_000u64);
    let ours = math::aerodrome_stable_amount_out(Decimal::ONE, d0, d1, 5);
    let ours_raw = (ours * Decimal::from(1_000_000u64)).to_u128().unwrap_or(0);
    println!("reserves WETH={d0} USDC={d1}");
    println!("onchain={onchain} ours={ours_raw} ours_dec={ours} diff_bps={}", if onchain>0 { ((ours_raw as i128 - onchain as i128).abs() as f64 / onchain as f64)*10000.0 } else {0.0});
}

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
