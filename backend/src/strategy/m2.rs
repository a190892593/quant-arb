//! M2 判定：用 REVM 在报价区块的链上状态上重放 V3 路径，复核 M1 报价。
//!
//! - 只支持全 V3 路径（QuoterV2 `quoteExactInput(bytes,uint256)`）；
//! - 输出与 M1 的偏差在 `tolerance_bps` 内 → 通过，机会可升级为 M2 + VERIFIED；
//! - 回滚/偏差过大 → 不通过，保留 M1 并记录证据。

use std::collections::HashMap;
use std::time::Instant;

use revm::primitives::{Address, U256};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use std::str::FromStr;

use crate::config::M2Config;
use crate::domain::market::{DexRoute, M2Evidence, Token};
use crate::evm::RpcClient;
use crate::evm::abi;
use crate::evm::revm_sim::{ForkCall, call_on_fork};
use crate::evm::atomic_sim::{estimate_route_gas, simulate_route, AtomicHop};

/// 全 V3 路径才可 M2 复核。
pub fn is_v3_route(route: &DexRoute) -> bool {
    !route.legs.is_empty()
        && route
            .legs
            .iter()
            .all(|l| l.protocol_id.starts_with("uniswap-v3"))
}

fn dec_u128(v: u128) -> Decimal {
    Decimal::from_str(&v.to_string()).unwrap_or(Decimal::MAX)
}

fn token_address(token: &Token) -> Result<[u8; 20], String> {
    abi::parse_address(&token.address).map_err(|e| format!("Token {} 地址非法: {e}", token.symbol))
}

fn scale(decimals: u32) -> Decimal {
    let mut v = Decimal::ONE;
    for _ in 0..decimals.min(36) {
        v *= Decimal::from(10);
    }
    v
}

/// 对一条全 V3 路径执行 M2 复核。
pub async fn verify_route(
    rpc: &RpcClient,
    route: &DexRoute,
    tokens: &HashMap<String, Token>,
    cfg: &M2Config,
) -> Result<M2Evidence, String> {
    // 以 WETH 开头的路线可以走“原生 ETH 包装 → 逐跳 swap”的整笔原子回放。
    if flash_pool_for(route.chain_id, &route.start_token).is_some() && !route.legs.is_empty() {
        if let Ok(ev) = verify_atomic_route(rpc, route, tokens, cfg).await {
            return Ok(ev);
        }
    }
    if !is_v3_route(route) {
        return Err("M2 仅支持全 UniswapV3 路径".into());
    }
    let in_token = tokens
        .get(&route.start_token)
        .ok_or_else(|| format!("缺少 Token {}", route.start_token))?;
    let out_token = tokens
        .get(&route.end_token)
        .ok_or_else(|| format!("缺少 Token {}", route.end_token))?;

    // 构造 QuoterV2 path: tokenIn + fee(3B) + tokenOut + fee + ...
    let mut path: Vec<u8> = Vec::new();
    for leg in &route.legs {
        let t = tokens
            .get(&leg.token_in)
            .ok_or_else(|| format!("缺少 Token {}", leg.token_in))?;
        path.extend_from_slice(&token_address(t)?);
        let fee_units = (leg.fee_bps.max(0) as u32) * 100; // 5bps -> 500
        path.extend_from_slice(&fee_units.to_be_bytes()[1..]);
    }
    path.extend_from_slice(&token_address(out_token)?);

    let amount_raw = (route.amount_in * scale(in_token.decimals))
        .to_u128()
        .ok_or_else(|| "输入金额无法转换为原始单位".to_string())?;

    // quoteExactInput(bytes,uint256)
    let mut data = abi::selector("quoteExactInput(bytes,uint256)").to_vec();
    let mut offset = [0u8; 32];
    offset[31] = 0x40;
    data.extend_from_slice(&offset);
    data.extend_from_slice(&abi::encode_u128(amount_raw));
    data.extend_from_slice(&abi::encode_u128(path.len() as u128));
    let mut padded = path.clone();
    while padded.len() % 32 != 0 {
        padded.push(0);
    }
    data.extend_from_slice(&padded);

    let quoter = Address::from(
        abi::parse_address(&cfg.quoter).map_err(|e| format!("Quoter 地址非法: {e}"))?,
    );
    let started = Instant::now();
    let out = call_on_fork(
        rpc,
        route.chain_id,
        route.block_number,
        ForkCall {
            from: Address::ZERO,
            to: quoter,
            data,
            gas_limit: cfg.gas_limit,
            value: U256::ZERO,
        },
    )
    .await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let base = M2Evidence {
        passed: false,
        block: route.block_number,
        amount_in: route.amount_in,
        m1_out: route.expected_out,
        m2_out: Decimal::ZERO,
        diff_bps: Decimal::ZERO,
        gas_used: out.gas_used,
        gas_source: "quoter".into(),
        elapsed_ms,
        detail: String::new(),
    };

    if !out.success {
        return Ok(M2Evidence {
            detail: out.revert_reason.unwrap_or_else(|| "REVM 执行失败".into()),
            ..base
        });
    }
    let word = abi::word_at(&out.output, 0).ok_or_else(|| "QuoterV2 返回为空".to_string())?;
    let m2_raw = abi::decode_u128(word).unwrap_or(0);
    let m2_out = dec_u128(m2_raw) / scale(out_token.decimals);
    if route.expected_out <= Decimal::ZERO {
        return Ok(M2Evidence {
            m2_out,
            detail: "M1 输出为零，无法比较".into(),
            ..base
        });
    }
    let tolerance = Decimal::from_f64(cfg.tolerance_bps).unwrap_or(Decimal::from(50));
    let diff_bps = ((m2_out - route.expected_out).abs() / route.expected_out)
        * Decimal::from(10_000);
    let passed = diff_bps <= tolerance;
    Ok(M2Evidence {
        passed,
        m2_out,
        diff_bps,
        detail: format!(
            "QuoterV2 分叉复核：M1 {} / M2 {}（偏差 {:.2} bps，阈值 {} bps，gas {}）",
            route.expected_out,
            m2_out,
            diff_bps.to_f64().unwrap_or(0.0),
            tolerance,
            out.gas_used
        ),
        ..base
    })
}


fn flash_pool_for(chain_id: u64, start: &str) -> Option<&'static str> {
    let s = start.to_ascii_uppercase();
    match (chain_id, s.as_str()) {
        (42161, "USDC") => Some("0xbe3ad6a5669dc0b8b12febc03608860c31e2eef6"),
        (42161, "USDT") => Some("0xbe3ad6a5669dc0b8b12febc03608860c31e2eef6"),
        (42161, "WETH") => Some("0xc6962004f452be9203591991d15f6b388e09e8d0"),
        (42161, "DAI") => Some("0xa635415f18cda471f5384465a9bc2097755e137f"),
        (8453, "USDC") => Some("0x06959273E9A65433De71F5a452D529544e07Ddd0"),
        (8453, "USDbC") => Some("0x06959273E9A65433De71F5a452D529544e07Ddd0"),
        (8453, "WETH") => Some("0xd0b53D9277642d899DF5C87A3966A349A798F224"),
        (8453, "DAI") => Some("0xc18F50D6a832F12F6DCaaEEe8D0c87a65B96787E"),
        (8453, "AERO") => Some("0x6cdcb1c4a4d1c3c6d054b27ac5b77e89eafb971d"),
        _ => None,
    }
}

fn router_for(chain_id: u64, kind: &str) -> Option<[u8; 20]> {
    let (v3, v2) = match chain_id {
        42161 => (
            "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45",
            "0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24",
        ),
        8453 => (
            "0x2626664c2603336E57B271c5C0b26F421741e481",
            "0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24",
        ),
        _ => return None,
    };
    let raw = if kind.contains("V3") { v3 } else { v2 };
    abi::parse_address(raw).ok()
}

async fn verify_atomic_route(
    rpc: &RpcClient,
    route: &DexRoute,
    tokens: &HashMap<String, Token>,
    cfg: &M2Config,
) -> Result<M2Evidence, String> {
    let in_token = tokens
        .get(&route.start_token)
        .ok_or_else(|| format!("缺少 Token {}", route.start_token))?;
    let out_token = tokens
        .get(&route.end_token)
        .ok_or_else(|| format!("缺少 Token {}", route.end_token))?;
    let mut hops = Vec::new();
    for leg in &route.legs {
        let tin = tokens
            .get(&leg.token_in)
            .ok_or_else(|| format!("缺少 Token {}", leg.token_in))?;
        let tout = tokens
            .get(&leg.token_out)
            .ok_or_else(|| format!("缺少 Token {}", leg.token_out))?;
        let kind = leg.adapter.clone();
        let router = router_for(route.chain_id, &kind)
            .ok_or_else(|| format!("链 {} 未配置 {} router", route.chain_id, kind))?;
        let amount_in = (leg.amount_in * scale(tin.decimals))
            .to_u128()
            .ok_or_else(|| "leg amount_in 无法转换为原始单位".to_string())?;
        hops.push(AtomicHop {
            router: Address::from(router),
            token_in: Address::from(token_address(tin)?),
            token_out: Address::from(token_address(tout)?),
            kind,
            fee_bps: leg.fee_bps,
            amount_in: U256::from(amount_in),
        });
    }
    let flash_pool = match route.chain_id {
        // Arbitrum: WETH/USDC 5bps；Base: WETH/USDC 5bps
        // 优先用稳定池作为闪电源，避免与路线内 pool 冲突（同池 swap 会 LOK）。
        42161 => "0xbe3ad6a5669dc0b8b12febc03608860c31e2eef6",
        8453 => "0x06959273E9A65433De71F5a452D529544e07Ddd0",
        other => return Err(format!("链 {other} 未配置闪电池")),
    };
    let flash_pool = Address::from(abi::parse_address(flash_pool)?);
    let amount_raw = (route.amount_in * scale(in_token.decimals))
        .to_u128()
        .ok_or_else(|| "输入金额无法转换为原始单位".to_string())?;
    let started = Instant::now();
    let gas_estimate = estimate_route_gas(
        rpc,
        route.block_number,
        flash_pool,
        Address::from(token_address(in_token)?),
        &hops,
    )
    .await
    .ok();
    let res = simulate_route(
        rpc,
        route.chain_id,
        route.block_number,
        flash_pool,
        Address::from(token_address(in_token)?),
        Address::from(token_address(out_token)?),
        &hops,
        U256::ZERO,
    )
    .await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let final_raw = U256::from(amount_raw).saturating_add(res.profit);
    let m2_out = Decimal::from_str(&final_raw.to_string()).unwrap_or(Decimal::ZERO)
        / scale(in_token.decimals);
    let diff_bps = if route.expected_out > Decimal::ZERO {
        ((m2_out - route.expected_out).abs() / route.expected_out) * Decimal::from(10_000)
    } else {
        Decimal::ZERO
    };
    let passed = res.success && diff_bps <= Decimal::from_f64(cfg.tolerance_bps).unwrap_or(Decimal::from(50));
    Ok(M2Evidence {
        passed,
        block: route.block_number,
        amount_in: route.amount_in,
        m1_out: route.expected_out,
        m2_out,
        diff_bps,
        gas_used: gas_estimate.unwrap_or(res.gas_used),
        gas_source: if gas_estimate.is_some() {
            "eth_estimateGas".into()
        } else {
            "revm".into()
        },
        elapsed_ms,
        detail: format!(
            "原子回放（executor，{} 跳）：M1 {} / 原子 {}（偏差 {:.2} bps，gas {} / {}）{}",
            res.hops,
            route.expected_out,
            m2_out,
            diff_bps.to_f64().unwrap_or(0.0),
            res.gas_used,
            gas_estimate.map(|g| g.to_string()).unwrap_or_else(|| "revm".into()),
            res.revert.unwrap_or_default()
        ),
    })
}
