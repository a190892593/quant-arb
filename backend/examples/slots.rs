fn main(){
 let addr=arb_backend::evm::abi::parse_address("0xc6962004f452be9203591991d15f6b388e09e8d0").unwrap();
 for slot in 0..=20u8 {
  let mut buf=[0u8;64]; buf[12..32].copy_from_slice(&addr); buf[63]=slot;
  let h=arb_backend::evm::abi::keccak256(&buf);
  println!("{} 0x{}",slot,h.iter().map(|b|format!("{b:02x}")).collect::<String>());
 }
}
