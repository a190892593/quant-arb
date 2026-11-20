fn main() {
    for s in ["allPoolsLength()","allPools(uint256)","getPool(address,address,uint24)","token0()","fee()","slot0()","factory()"] {
        let sel = arb_backend::evm::abi::selector(s);
        println!("{s} 0x{}", sel.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
}
