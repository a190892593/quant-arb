fn main(){for s in ["getA()","A()","amplification()"]{let x=arb_backend::evm::abi::selector(s);println!("{s} 0x{}",x.iter().map(|b|format!("{b:02x}")).collect::<String>());}}
