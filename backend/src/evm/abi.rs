//! 最小 ABI 编解码（只覆盖扫描所需的调用与返回值）。
//!
//! 不引入第三方 ABI 库：选择子用 keccak256(signature) 前 4 字节，参数按 32 字节
//! 大端字左填充编码；返回值按同样规则解码。所有函数都做长度校验，异常返回 None/Err，
//! 绝不在数据不完整时返回猜测值。

use tiny_keccak::{Hasher, Keccak};

/// 20 字节地址。
pub type Address = [u8; 20];

/// 计算函数选择子。
pub fn selector(signature: &str) -> [u8; 4] {
    let mut hasher = Keccak::v256();
    hasher.update(signature.as_bytes());
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    [out[0], out[1], out[2], out[3]]
}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// 计算事件 topic0（keccak256(signature) 全 32 字节）。
pub fn event_topic(signature: &str) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(signature.as_bytes());
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// 解析 0x 前缀（可省略）的 20 字节地址。
pub fn parse_address(s: &str) -> Result<Address, String> {
    let h = s.trim().trim_start_matches("0x");
    if h.len() != 40 {
        return Err(format!("地址长度非法: {s}"));
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("地址含非十六进制字符: {s}"))?;
    }
    Ok(out)
}

/// 0x 小写地址。
pub fn format_address(a: &Address) -> String {
    let mut s = String::with_capacity(42);
    s.push_str("0x");
    for b in a {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn is_zero_address(a: &Address) -> bool {
    a.iter().all(|b| *b == 0)
}

/// ABI 参数。
#[derive(Clone, Debug)]
pub enum Arg {
    Address(Address),
    Uint(u128),
    Int(i128),
    Bool(bool),
    Bytes32([u8; 32]),
    AddressArray(Vec<Address>),
}

/// 编码一次函数调用：selector + 参数。
pub fn encode_call(signature: &str, args: &[Arg]) -> Vec<u8> {
    let mut out = selector(signature).to_vec();
    out.extend(encode_args(args));
    out
}

/// 编码参数列表（支持动态地址数组）。
pub fn encode_args(args: &[Arg]) -> Vec<u8> {
    let head_len = args.len() * 32;
    let mut head = Vec::with_capacity(head_len);
    let mut tail = Vec::new();
    for arg in args {
        match arg {
            Arg::Address(a) => head.extend_from_slice(&encode_address(a)),
            Arg::Uint(v) => head.extend_from_slice(&encode_u128(*v)),
            Arg::Int(v) => head.extend_from_slice(&encode_i128(*v)),
            Arg::Bool(b) => head.extend_from_slice(&encode_u128(u128::from(*b))),
            Arg::Bytes32(b) => head.extend_from_slice(b),
            Arg::AddressArray(items) => {
                let offset = head_len + tail.len();
                head.extend_from_slice(&encode_u128(offset as u128));
                tail.extend_from_slice(&encode_u128(items.len() as u128));
                for a in items {
                    tail.extend_from_slice(&encode_address(a));
                }
            }
        }
    }
    head.extend_from_slice(&tail);
    head
}

pub fn encode_u128(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

pub fn encode_i128(v: i128) -> [u8; 32] {
    let mut w = if v < 0 { [0xffu8; 32] } else { [0u8; 32] };
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

pub fn encode_address(a: &Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// 返回值按 32 字节切分（忽略尾部不足一字）。
pub fn words(data: &[u8]) -> Vec<&[u8]> {
    data.chunks(32).filter(|c| c.len() == 32).collect()
}

pub fn word_at(data: &[u8], index: usize) -> Option<&[u8]> {
    let start = index.checked_mul(32)?;
    let end = start.checked_add(32)?;
    data.get(start..end)
}

pub fn decode_address(word: &[u8]) -> Option<Address> {
    if word.len() != 32 {
        return None;
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&word[12..]);
    Some(a)
}

/// 解出 uint256 的低 128 位。高 128 位非零时返回 None（真实余额不可能溢出）。
pub fn decode_u128(word: &[u8]) -> Option<u128> {
    if word.len() != 32 || word[..16].iter().any(|b| *b != 0) {
        return None;
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&word[16..]);
    Some(u128::from_be_bytes(buf))
}

/// 解出 uint256 的低 128 位，允许高位存在时饱和（用于 uint160）。
pub fn decode_uint_low(word: &[u8]) -> Option<u128> {
    if word.len() != 32 {
        return None;
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&word[16..]);
    Some(u128::from_be_bytes(buf))
}

pub fn decode_i128(word: &[u8]) -> Option<i128> {
    if word.len() != 32 {
        return None;
    }
    let negative = word[0] & 0x80 != 0;
    if negative {
        if !word[..16].iter().all(|b| *b == 0xff) {
            return None;
        }
    } else if word[..16].iter().any(|b| *b != 0) {
        return None;
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&word[16..]);
    Some(i128::from_be_bytes(buf))
}

/// 解出 int24（V3 tick）。按 32 字节符号扩展。
pub fn decode_i24(word: &[u8]) -> Option<i32> {
    if word.len() != 32 {
        return None;
    }
    let v = decode_i128(word)?;
    if !(-8_388_608..=8_388_607).contains(&v) {
        return None;
    }
    Some(v as i32)
}

pub fn decode_bool(word: &[u8]) -> Option<bool> {
    decode_u128(word).map(|v| v != 0)
}

/// 解出动态数组 offset（第 head_index 个字）。
pub fn decode_array_offset(data: &[u8], head_index: usize) -> Option<usize> {
    let w = word_at(data, head_index)?;
    let off = decode_u128(w)?;
    usize::try_from(off).ok()
}

/// 解出 address[]（offset 指向长度字）。
pub fn decode_address_array(data: &[u8], offset: usize) -> Option<Vec<Address>> {
    let len_word = data.get(offset..offset + 32)?;
    let len = decode_u128(len_word)? as usize;
    if len > 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let w = data.get(offset + 32 + i * 32..offset + 64 + i * 32)?;
        out.push(decode_address(w)?);
    }
    Some(out)
}

/// 解出 uint256[]（offset 指向长度字）。
pub fn decode_uint_array(data: &[u8], offset: usize) -> Option<Vec<u128>> {
    let len_word = data.get(offset..offset + 32)?;
    let len = decode_u128(len_word)? as usize;
    if len > 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let w = data.get(offset + 32 + i * 32..offset + 64 + i * 32)?;
        out.push(decode_uint_low(w)?);
    }
    Some(out)
}

/// V2 getReserves() → (reserve0, reserve1, blockTimestampLast)。
pub fn decode_get_reserves(data: &[u8]) -> Option<(u128, u128, u32)> {
    let r0 = decode_u128(word_at(data, 0)?)?;
    let r1 = decode_u128(word_at(data, 1)?)?;
    let ts = decode_u128(word_at(data, 2)?)?;
    Some((r0, r1, u32::try_from(ts).ok()?))
}

/// V3 slot0() → (sqrtPriceX96, tick, ...)。
pub fn decode_slot0(data: &[u8]) -> Option<(u128, i32)> {
    let sqrt = decode_uint_low(word_at(data, 0)?)?;
    let tick = decode_i24(word_at(data, 1)?)?;
    Some((sqrt, tick))
}

/// 解码单个 uint256（低 128 位）。
pub fn decode_single_u128(data: &[u8]) -> Option<u128> {
    decode_u128(word_at(data, 0)?)
}

/// 解码 uint160（低 128 位，允许高 32 位非零的 uint160 值）。
pub fn decode_single_u160(data: &[u8]) -> Option<u128> {
    decode_uint_low(word_at(data, 0)?)
}

/// 解码单个地址。
pub fn decode_single_address(data: &[u8]) -> Option<Address> {
    decode_address(word_at(data, 0)?)
}

/// 解码单个 bool。
pub fn decode_single_bool(data: &[u8]) -> Option<bool> {
    decode_bool(word_at(data, 0)?)
}

/// 将字节转为 0x 十六进制。
pub fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(2 + data.len() * 2);
    s.push_str("0x");
    for b in data {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 解析 0x 十六进制。
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let h = s.trim().trim_start_matches("0x");
    if h.len() % 2 != 0 {
        return Err(format!("十六进制长度非法: {}", h.len()));
    }
    let mut out = Vec::with_capacity(h.len() / 2);
    for i in (0..h.len()).step_by(2) {
        out.push(
            u8::from_str_radix(&h[i..i + 2], 16)
                .map_err(|_| format!("非法十六进制: {}", &h[i..i + 2]))?,
        );
    }
    Ok(out)
}

/// 解析 `string` 返回值（offset + length + bytes）；也兼容 bytes32 形式的 symbol。
pub fn decode_string(data: &[u8]) -> Option<String> {
    let off = decode_uint_low(word_at(data, 0)?)? as usize;
    if off + 32 <= data.len() {
        if let Some(len) = data
            .get(off..off + 32)
            .and_then(decode_uint_low)
            .map(|v| v as usize)
        {
            if let Some(bytes) = data.get(off + 32..off + 32 + len) {
                if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                    let s = s.trim_matches('\0').trim().to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
        }
    }
    // bytes32 fallback
    let raw = word_at(data, 0)?;
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    let s = String::from_utf8(raw[..end].to_vec()).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_match_known_values() {
        assert_eq!(selector("transfer(address,uint256)"), [0xa9, 0x05, 0x9c, 0xbb]);
        assert_eq!(selector("getReserves()"), [0x09, 0x02, 0xf1, 0xac]);
        assert_eq!(selector("slot0()"), [0x38, 0x50, 0xc7, 0xbd]);
        assert_eq!(selector("decimals()"), [0x31, 0x3c, 0xe5, 0x67]);
    }

    #[test]
    fn address_roundtrip() {
        let a = parse_address("0xaf88d065e77c8cC2239327C5EDb3A432268e5831").unwrap();
        assert_eq!(format_address(&a), "0xaf88d065e77c8cc2239327c5edb3a432268e5831");
        assert!(parse_address("0x1234").is_err());
        assert!(!is_zero_address(&a));
        assert!(is_zero_address(&[0u8; 20]));
    }

    #[test]
    fn encodes_static_and_dynamic_args() {
        let a = [1u8; 20];
        let b = [2u8; 20];
        let enc = encode_call("getAmountsOut(uint256,address[])", &[
            Arg::Uint(1_000),
            Arg::AddressArray(vec![a, b]),
        ]);
        assert_eq!(&enc[..4], &selector("getAmountsOut(uint256,address[])"));
        let body = &enc[4..];
        assert_eq!(decode_u128(word_at(body, 0).unwrap()), Some(1_000));
        let off = decode_array_offset(body, 1).unwrap();
        assert_eq!(off, 64);
        let arr = decode_address_array(body, off).unwrap();
        assert_eq!(arr, vec![a, b]);
    }

    #[test]
    fn encodes_negative_ints() {
        let enc = encode_args(&[Arg::Int(-887272)]);
        assert_eq!(word_at(&enc, 0).unwrap()[0], 0xff);
        assert_eq!(decode_i24(word_at(&enc, 0).unwrap()), Some(-887272));
        assert_eq!(decode_i24(&encode_i128(-1)), Some(-1));
    }

    #[test]
    fn decodes_reserves_slot0_and_arrays() {
        let mut data = Vec::new();
        data.extend_from_slice(&encode_u128(1_000_000));
        data.extend_from_slice(&encode_u128(2_000_000));
        data.extend_from_slice(&encode_u128(123));
        assert_eq!(decode_get_reserves(&data), Some((1_000_000, 2_000_000, 123)));

        let mut slot = Vec::new();
        slot.extend_from_slice(&encode_u128(79_228_162_514_264_337_593_543_950_336));
        slot.extend_from_slice(&encode_i128(-193_380));
        slot.extend_from_slice(&encode_u128(0));
        assert_eq!(
            decode_slot0(&slot),
            Some((79_228_162_514_264_337_593_543_950_336, -193_380))
        );

        let mut arr = Vec::new();
        arr.extend_from_slice(&encode_u128(32));
        arr.extend_from_slice(&encode_u128(2));
        arr.extend_from_slice(&encode_u128(7));
        arr.extend_from_slice(&encode_u128(9));
        assert_eq!(decode_uint_array(&arr, 32), Some(vec![7, 9]));
    }

    #[test]
    fn hex_roundtrip() {
        let b = vec![0u8, 1, 255, 16];
        assert_eq!(hex_decode(&hex_encode(&b)).unwrap(), b);
        assert!(hex_decode("0x123").is_err());
    }
}
