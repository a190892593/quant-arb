fn main(){
 let addr=arb_backend::evm::abi::parse_address("0xc6962004f452be9203591991d15f6b388e09e8d0").unwrap();
 let mut buf=[0u8;64]; buf[12..32].copy_from_slice(&addr); // slot 3
 buf[63]=3;
 let h=arb_backend::evm::abi::keccak256(&buf);
 println!("0x{}", h.iter().map(|b|format!("{b:02x}")).collect::<String>());
}
