//! 全链池/Token 自动发现：从 GeckoTerminal 公共 API 拉取 Arbitrum 高 TVL 池，
//! 再回链上校验 token0/token1/fee/TVL，过滤黑名单与低流动性后登记。
//!
//! 选择 API 而不是工厂事件的原因：Arbitrum 上 `PoolCreated` 极稀疏，工厂
//! `allPoolsLength` 在该部署不可用；聚合器能直接给出跨 DEX 的高 TVL 池集合。
//! 所有最终入池数据都以链上读取为准，聚合器只作为候选来源。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::engine::AppState;
use crate::evm::abi::{self, Address};
use crate::evm::{Call, RpcClient};

const GECKO_BASE: &str = "https://api.geckoterminal.com/api/v2/networks/arbitrum/pools";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredPool {
    pub pool: String,
    pub protocol_id: String,
    pub kind: String,
    pub fee_bps: i64,
    pub tick_spacing: i64,
    pub token0: String,
    pub token1: String,
    pub address0: String,
    pub address1: String,
    pub decimals0: u32,
    pub decimals1: u32,
    pub tvl_usd: f64,
    #[serde(default)]
    pub price0_usd: f64,
    #[serde(default)]
    pub price1_usd: f64,
    pub block: u64,
    pub discovered_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoveryFile {
    #[serde(default)]
    pub pools: Vec<DiscoveredPool>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoveryState {
    #[serde(default)]
    pub last_block: u64,
    #[serde(default)]
    pub source_cursor: usize,
    #[serde(default)]
    pub last_run_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoveryStatus {
    pub enabled: bool,
    pub running: bool,
    pub last_block: u64,
    pub latest_block: u64,
    /// 本轮候选池数（沿用旧字段名）。
    pub scanned_logs: usize,
    pub new_pools: usize,
    pub new_tokens: usize,
    pub total_pools: usize,
    pub last_run_at: String,
    pub message: String,
    #[serde(default)]
    pub source: String,
}

fn pools_path(workspace: &std::path::Path) -> PathBuf {
    workspace.join("data").join("discovered_pools.json")
}
fn state_path(workspace: &std::path::Path) -> PathBuf {
    workspace.join("data").join("discovery_state.json")
}

fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &std::path::Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_json<T: Serialize>(path: &std::path::Path, v: &T) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(path, s);
    }
}

pub fn load_pools(workspace: &std::path::Path) -> Vec<DiscoveredPool> {
    load_json::<DiscoveryFile>(&pools_path(workspace)).pools
}

fn addr_str(a: &Address) -> String {
    abi::format_address(a)
}


fn with_network(url: &str, network: &str) -> String {
    url.replace("/arbitrum/", &format!("/{network}/"))
        .replace("/arbitrum?", &format!("/{network}?"))
}

fn token_price(sym: &str, prices: &HashMap<String, f64>) -> Option<f64> {
    let s = sym.to_ascii_uppercase();
    if matches!(s.as_str(), "USDC" | "USDT" | "DAI" | "USDC.E") {
        return Some(1.0);
    }
    prices
        .get(&s)
        .copied()
        .or_else(|| prices.get("WETH").copied().filter(|_| s == "WSTETH"))
}

fn protocol_of(dex: &str) -> (&'static str, &'static str) {
    let d = dex.to_ascii_lowercase();
    if d.contains("uniswap-v3") || d.contains("pancakeswap-v3") || d.contains("sushiswap-v3") {
        ("uniswap-v3", "UNISWAP_V3")
    } else if d.contains("uniswap-v2") {
        ("uniswap-v2", "UNISWAP_V2")
    } else if d.contains("sushi") {
        ("sushi-v2", "UNISWAP_V2")
    } else if d.contains("camelot") {
        ("camelot-v2", "UNISWAP_V2")
    } else if d.contains("pancake") {
        ("pancake-v2", "UNISWAP_V2")
    } else {
        // 未识别 DEX 暂不纳入，避免把假 Token/异常池登记进白名单。
        ("skip", "")
    }
}

/// 执行一次发现扫描。
pub async fn scan_once(state: &std::sync::Arc<AppState>) -> Result<DiscoveryStatus, String> {
    let cfg = state.config.discovery.clone();
    let mut status = DiscoveryStatus {
        enabled: cfg.enabled,
        ..Default::default()
    };
    if !cfg.enabled {
        status.message = "discovery disabled".into();
        return Ok(status);
    }
    let source_cfg = state.sources.lock().await.clone();
    // 若配置了带 Key 的 RPC 数据源，优先用专用 RPC，避免公共节点 429。
    let keyed_rpc = source_cfg
        .sources
        .iter()
        .filter(|s| s.enabled && s.kind == "rpc" && !s.api_key.trim().is_empty())
        .min_by_key(|s| s.priority)
        .map(|s| {
            let base = s.url.trim_end_matches('/');
            if s.id == "quicknode_rpc" {
                base.to_string()
            } else {
                format!("{base}/{}", s.api_key.trim())
            }
        });
    let rpc = match &keyed_rpc {
        Some(url) => RpcClient::new(url, None),
        None => RpcClient::new_pool(
            &state.config.live.rpc_primary,
            &state.config.live.rpc_pool,
        ),
    };
    let latest = rpc.block_number().await?;
    status.latest_block = latest;

    // 1) 数据源轮询：读取 source_config（启用/优先级/Key/策略）
    let sc = source_cfg.clone();
    let entries = sc.discovery_sources();
    if entries.is_empty() {
        status.message = "没有启用的发现数据源（见数据源配置页）".into();
        status.total_pools = load_pools(&state.workspace).len();
        return Ok(status);
    }
    let client = reqwest::Client::new();
    let sp = state_path(&state.workspace);
    let mut st = load_json::<DiscoveryState>(&sp);
    let len = entries.len();
    let start_idx = match sc.strategy.as_str() {
        "priority" => 0,
        "random" => {
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0);
            n % len
        }
        _ => st.source_cursor % len,
    };
    let mut candidates: Vec<(Address, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut used_source = String::new();
    let mut tried: Vec<(String, bool, String)> = Vec::new();
    for off in 0..len {
        let idx = (start_idx + off) % len;
        let entry = &entries[idx];
        let list = if entry.kind == "dexscreener" {
            fetch_dexscreener(&client, &cfg.network).await
        } else {
            let key: Option<String> = if entry.kind == "coingecko" {
                if !entry.api_key.is_empty() {
                    Some(entry.api_key.clone())
                } else {
                    std::env::var("COINGECKO_API_KEY").ok()
                }
            } else {
                None
            };
            fetch_pools(
                &client,
                &with_network(&entry.url, &cfg.network),
                key.as_deref(),
            )
            .await
        };
        match list {
            Ok(l) if !l.is_empty() => {
                for (a, dex) in l {
                    let k = addr_str(&a).to_ascii_lowercase();
                    if seen.insert(k) {
                        candidates.push((a, dex));
                    }
                }
                used_source = entry.id.clone();
                tried.push((entry.id.clone(), true, format!("{} candidates", candidates.len())));
                st.source_cursor = idx + 1;
                break;
            }
            Ok(_) => tried.push((entry.id.clone(), false, "empty".into())),
            Err(e) => tried.push((entry.id.clone(), false, e)),
        }
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    }
    if candidates.is_empty() && sc.sources.iter().any(|s| s.id == "dexscreener" && s.enabled) {
        if let Ok(list) = fetch_dexscreener(&client, &cfg.network).await {
            for (a, dex) in list {
                let k = addr_str(&a).to_ascii_lowercase();
                if seen.insert(k) {
                    candidates.push((a, dex));
                }
            }
            if !candidates.is_empty() {
                used_source = "dexscreener".into();
                tried.push(("dexscreener".into(), true, "fallback".into()));
            }
        }
    }
    status.source = used_source;
    {
        let mut cfg = state.sources.lock().await;
        for (id, ok, msg) in &tried {
            cfg.mark_used(id, if *ok { "ok" } else { msg.as_str() });
        }
        cfg.save(&state.workspace);
    }

    status.scanned_logs = candidates.len();
    if candidates.is_empty() {
        status.total_pools = load_pools(&state.workspace).len();
        status.message = format!("no candidates from geckoterminal; {}", status.message);
        return Ok(status);
    }

    // 2) 链上校验 token0/token1/fee
    let mut meta_calls = Vec::new();
    for (pool, _) in &candidates {
        meta_calls.push(Call {
            to: *pool,
            data: abi::encode_call("token0()", &[]),
        });
        meta_calls.push(Call {
            to: *pool,
            data: abi::encode_call("token1()", &[]),
        });
        meta_calls.push(Call {
            to: *pool,
            data: abi::encode_call("fee()", &[]),
        });
    }
    let meta_res = rpc.batch_eth_call(&meta_calls, Some(latest)).await?;

    let existing = load_pools(&state.workspace);
    let known: HashSet<String> = existing
        .iter()
        .map(|p| p.pool.to_ascii_lowercase())
        .collect();
    let mut pools: Vec<(Address, Address, Address, i64, String, String)> = Vec::new();
    for (i, (pool, dex)) in candidates.iter().enumerate() {
        let a0 = meta_res
            .get(i * 3)
            .filter(|r| r.ok())
            .and_then(|r| abi::decode_single_address(&r.data));
        let a1 = meta_res
            .get(i * 3 + 1)
            .filter(|r| r.ok())
            .and_then(|r| abi::decode_single_address(&r.data));
        let fee = meta_res
            .get(i * 3 + 2)
            .filter(|r| r.ok())
            .and_then(|r| abi::decode_single_u128(&r.data))
            .unwrap_or(0) as i64
            / 100;
        let (Some(a0), Some(a1)) = (a0, a1) else {
            continue;
        };
        let (proto, kind) = protocol_of(dex);
        if kind.is_empty() || known.contains(&addr_str(pool).to_ascii_lowercase()) {
            continue;
        }
        pools.push((*pool, a0, a1, fee, proto.into(), kind.into()));
    }

    // 3) Token 元数据
    let mut token_set: HashSet<Address> = HashSet::new();
    for (_, t0, t1, ..) in &pools {
        token_set.insert(*t0);
        token_set.insert(*t1);
    }
    let tokens: Vec<Address> = token_set.into_iter().collect();
    let mut t_calls = Vec::new();
    for a in &tokens {
        t_calls.push(Call {
            to: *a,
            data: abi::encode_call("symbol()", &[]),
        });
        t_calls.push(Call {
            to: *a,
            data: abi::encode_call("decimals()", &[]),
        });
    }
    let t_res = rpc.batch_eth_call(&t_calls, Some(latest)).await?;
    let mut meta: HashMap<Address, (String, u32)> = HashMap::new();
    for (i, a) in tokens.iter().enumerate() {
        let sym = t_res
            .get(i * 2)
            .filter(|r| r.ok())
            .and_then(|r| abi::decode_string(&r.data));
        let dec = t_res
            .get(i * 2 + 1)
            .filter(|r| r.ok())
            .and_then(|r| abi::decode_single_u128(&r.data))
            .map(|v| v as u32);
        if let (Some(s), Some(d)) = (sym, dec) {
            meta.insert(*a, (s, d));
        }
    }

    // 4) 价格 + 余额 + slot0
    let prices: HashMap<String, f64> = {
        let m = state.market.read().await;
        m.prices
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string().parse::<f64>().unwrap_or(0.0)))
            .collect()
    };
    let mut s_calls = Vec::new();
    for (pool, t0, t1, _, _, _) in &pools {
        s_calls.push(Call {
            to: *t0,
            data: abi::encode_call("balanceOf(address)", &[abi::Arg::Address(*pool)]),
        });
        s_calls.push(Call {
            to: *t1,
            data: abi::encode_call("balanceOf(address)", &[abi::Arg::Address(*pool)]),
        });
        s_calls.push(Call {
            to: *pool,
            data: abi::encode_call("slot0()", &[]),
        });
    }
    let s_res = rpc.batch_eth_call(&s_calls, Some(latest)).await?;

    let mut added = Vec::new();
    let mut new_tokens = 0usize;
    let price_path = state.workspace.join("data").join("discovered_prices.json");
    let mut discovered_prices: HashMap<String, f64> = load_json(&price_path);
    {
        let mut sc = state.search.lock().await;
        for (i, (pool, t0, t1, fee, proto, kind)) in pools.iter().enumerate() {
            let Some((s0, d0)) = meta.get(t0).cloned() else { continue };
            let Some((s1, d1)) = meta.get(t1).cloned() else { continue };
            if sc.is_blacklisted(&s0) || sc.is_blacklisted(&s1) {
                continue;
            }
            let b0 = s_res
                .get(i * 3)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_u128(&r.data))
                .unwrap_or(0) as f64;
            let b1 = s_res
                .get(i * 3 + 1)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_u128(&r.data))
                .unwrap_or(0) as f64;
            let sqrt = s_res
                .get(i * 3 + 2)
                .filter(|r| r.ok())
                .and_then(|r| abi::decode_single_u160(&r.data))
                .unwrap_or(0) as f64;
            if b0 <= 0.0 || b1 <= 0.0 || sqrt <= 0.0 {
                continue;
            }
            let scale0 = 10f64.powi(d0 as i32);
            let scale1 = 10f64.powi(d1 as i32);
            let h0 = b0 / scale0;
            let h1 = b1 / scale1;
            let p = sqrt / 2f64.powi(96);
            let ratio = p * p * scale0 / scale1;
            let mut p0 = token_price(&s0, &prices);
            let mut p1 = token_price(&s1, &prices);
            if p0.is_none() && p1.is_some() && ratio > 0.0 {
                p0 = Some(p1.unwrap() / ratio);
            }
            if p1.is_none() && p0.is_some() && ratio > 0.0 {
                p1 = Some(p0.unwrap() * ratio);
            }
            let (Some(p0), Some(p1)) = (p0, p1) else { continue };
            let tvl = h0 * p0 + h1 * p1;
            // 异常价格/精度会导致 TVL 明显失真，做上界保护。
            if !tvl.is_finite() || tvl < cfg.min_tvl_usd || tvl > 1.0e9 {
                continue;
            }
            discovered_prices.insert(s0.clone(), p0);
            discovered_prices.insert(s1.clone(), p1);
            if !sc.tokens.iter().any(|t| t.address.eq_ignore_ascii_case(&addr_str(t0))) {
                sc.upsert(&s0, &addr_str(t0), d0, crate::search_config::WHITELIST, "discovered");
                new_tokens += 1;
            }
            if !sc.tokens.iter().any(|t| t.address.eq_ignore_ascii_case(&addr_str(t1))) {
                sc.upsert(&s1, &addr_str(t1), d1, crate::search_config::WHITELIST, "discovered");
                new_tokens += 1;
            }
            added.push(DiscoveredPool {
                pool: addr_str(pool),
                protocol_id: proto.clone(),
                kind: kind.clone(),
                fee_bps: *fee,
                tick_spacing: 0,
                token0: s0,
                token1: s1,
                address0: addr_str(t0),
                address1: addr_str(t1),
                decimals0: d0,
                decimals1: d1,
                tvl_usd: tvl,
                price0_usd: p0,
                price1_usd: p1,
                block: latest,
                discovered_at: Utc::now().to_rfc3339(),
            });
        }
        if !added.is_empty() || new_tokens > 0 {
            sc.save(&state.workspace);
        }
    }
    if !discovered_prices.is_empty() {
        save_json(&price_path, &discovered_prices);
    }

    status.new_pools = added.len();
    status.new_tokens = new_tokens;
    if !added.is_empty() {
        let mut file = load_json::<DiscoveryFile>(&pools_path(&state.workspace));
        file.pools.extend(added);
        file.pools.sort_by(|a, b| {
            b.tvl_usd
                .partial_cmp(&a.tvl_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        file.pools.truncate(cfg.max_total_pools);
        file.updated_at = Utc::now().to_rfc3339();
        save_json(&pools_path(&state.workspace), &file);
    }

    st.last_block = latest;
    st.last_run_at = Utc::now().to_rfc3339();
    save_json(&sp, &st);
    status.last_block = latest;
    status.last_run_at = st.last_run_at.clone();
    status.total_pools = load_pools(&state.workspace).len();
    status.message = format!(
        "candidates={}, verified_pools={}, new_pools={}, new_tokens={}",
        status.scanned_logs,
        pools.len(),
        status.new_pools,
        status.new_tokens
    );
    Ok(status)
}

async fn fetch_pools(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
) -> Result<Vec<(Address, String)>, String> {
    let mut req = client
        .get(url)
        .header("accept", "application/json")
        .timeout(std::time::Duration::from_secs(20));
    if let Some(k) = key {
        req = req.header("x-cg-demo-api-key", k);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "no data".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        let addr = item
            .get("attributes")
            .and_then(|a| a.get("address"))
            .and_then(|a| a.as_str())
            .unwrap_or("");
        let dex = item
            .get("relationships")
            .and_then(|r| r.get("dex"))
            .and_then(|d| d.get("data"))
            .and_then(|d| d.get("id"))
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if let Ok(a) = abi::parse_address(addr) {
            out.push((a, dex.to_string()));
        }
    }
    Ok(out)
}

/// DexScreener 免 Key：boost 列表 → 每个 Token 的全部交易对。
async fn fetch_dexscreener(
    client: &reqwest::Client,
    network: &str,
) -> Result<Vec<(Address, String)>, String> {
    let resp = client
        .get("https://api.dexscreener.com/token-boosts/latest/v1")
        .header("accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let Some(arr) = v.as_array() else {
        return Ok(Vec::new());
    };
    let mut tokens: Vec<String> = Vec::new();
    for item in arr {
        let chain = item.get("chainId").and_then(|c| c.as_str()).unwrap_or("");
        let token = item
            .get("tokenAddress")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if chain.eq_ignore_ascii_case(network) && !token.is_empty() && tokens.len() < 10 {
            tokens.push(token.to_string());
        }
    }
    let mut out = Vec::new();
    for tok in tokens {
        let url = format!("https://api.dexscreener.com/token-pairs/v1/{network}/{tok}");
        let Ok(resp) = client
            .get(&url)
            .header("accept", "application/json")
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
        else {
            continue;
        };
        let Ok(v) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        let Some(pairs) = v.as_array() else { continue };
        for pair in pairs {
            let addr = pair
                .get("pairAddress")
                .and_then(|a| a.as_str())
                .unwrap_or("");
            let dex = pair.get("dexId").and_then(|d| d.as_str()).unwrap_or("");
            if let Ok(a) = abi::parse_address(addr) {
                out.push((a, dex.to_string()));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(out)
}

