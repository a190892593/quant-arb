//! 最小 JSON-RPC 客户端：单调用、批量 eth_call、日志与区块头。
//!
//! 设计约束：
//! - 优先主 RPC，失败（网络/HTTP/JSON-RPC 错误）自动切备用 RPC；
//! - 批量请求按 id 对齐结果，单条调用失败不会让整批失败；
//! - 所有返回值保留原始错误文本，便于在 /api/markets/data-health 里展示；
//! - 不做静默重试循环，避免在链上数据过期时伪装成功。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::abi::{self, Address};

/// 单次 eth_call。
#[derive(Clone, Debug)]
pub struct Call {
    pub to: Address,
    pub data: Vec<u8>,
}

/// 批量 eth_call 的单条结果。
#[derive(Clone, Debug)]
pub struct CallResult {
    pub data: Vec<u8>,
    pub error: Option<String>,
}

impl CallResult {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// 日志条目。
#[derive(Clone, Debug)]
pub struct Log {
    pub address: Address,
    pub topics: Vec<Vec<u8>>,
    pub data: Vec<u8>,
    pub block_number: u64,
    pub log_index: u64,
    pub transaction_hash: String,
}

/// RPC 客户端。
#[derive(Clone)]
pub struct RpcClient {
    primary: String,
    fallback: Option<String>,
    endpoints: Vec<String>,
    current: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    http: reqwest::Client,
    next_id: std::sync::Arc<AtomicU64>,
}

impl RpcClient {
    pub fn new(primary: &str, fallback: Option<&str>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("arbx-scan/0.2")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let mut endpoints = vec![primary.to_string()];
        if let Some(f) = fallback {
            if !f.is_empty() && !endpoints.iter().any(|x| x == f) {
                endpoints.push(f.to_string());
            }
        }
        Self {
            primary: primary.to_string(),
            fallback: endpoints.get(1).cloned(),
            endpoints,
            current: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            http,
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    /// 多个免费 RPC 端点：失败自动切换并记住可用节点。
    pub fn new_pool(primary: &str, pool: &[String]) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent("arbx-scan/0.2")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let mut endpoints = vec![primary.to_string()];
        for u in pool {
            let u = u.trim();
            if !u.is_empty() && !endpoints.iter().any(|x| x == u) {
                endpoints.push(u.to_string());
            }
        }
        Self {
            primary: primary.to_string(),
            fallback: endpoints.get(1).cloned(),
            endpoints,
            current: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            http,
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn endpoints(&self) -> &[String] {
        &self.endpoints
    }

    pub fn primary(&self) -> &str {
        &self.primary
    }

    pub fn fallback(&self) -> Option<&str> {
        self.fallback.as_deref()
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn post(&self, url: &str, body: &Value) -> Result<Value, String> {
        let resp = self
            .http
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("响应不是合法 JSON: {e}; {}", text.chars().take(160).collect::<String>()))
    }

    /// 发送原始 JSON（单对象或批次），主节点失败后切换备用节点。
    pub async fn raw(&self, body: Value) -> Result<Value, String> {
        let n = self.endpoints.len();
        if n == 0 {
            return Err("无可用 RPC 端点".into());
        }
        let start = self.current.load(Ordering::Relaxed) % n;
        let mut last_err = String::new();
        for i in 0..n {
            let idx = (start + i) % n;
            let url = &self.endpoints[idx];
            match self.post(url, &body).await {
                Ok(v) => {
                    // JSON-RPC 错误（例如额度耗尽）也要切换节点，不能当成成功。
                    if v.get("error").is_some() && v.get("result").is_none() {
                        let msg = v
                            .get("error")
                            .map(|e| e.to_string())
                            .unwrap_or_default();
                        tracing::warn!("RPC 返回错误（{msg}），切换下一个节点 {url}");
                        last_err = msg;
                        continue;
                    }
                    self.current.store(idx, Ordering::Relaxed);
                    return Ok(v);
                }
                Err(e) => {
                    tracing::warn!("RPC 失败（{e}），切换下一个节点 {url}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    /// 单次调用，返回 result 字段。
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.id();
        let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let resp = self.raw(body).await?;
        if let Some(err) = resp.get("error") {
            return Err(format!("{method} 返回错误: {err}"));
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| format!("{method} 响应缺少 result"))
    }


    /// eth_estimateGas with state override (inject executor code at target).
    pub async fn estimate_gas_with_code(
        &self,
        from: &str,
        to: &str,
        data: &str,
        block: u64,
        code: &str,
    ) -> Result<u64, String> {
        let overrides = serde_json::json!({ to: { "code": code } });
        let params = serde_json::json!([
            { "from": from, "to": to, "data": data, "value": "0x0" },
            format!("0x{block:x}"),
            overrides
        ]);
        let v = self.call("eth_estimateGas", params).await?;
        let hex = v
            .as_str()
            .ok_or_else(|| format!("eth_estimateGas returned non-string: {v}"))?;
        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| format!("eth_estimateGas parse {hex}: {e}"))
    }

    pub async fn block_number(&self) -> Result<u64, String> {
        let v = self.call("eth_blockNumber", json!([])).await?;
        let s = v
            .as_str()
            .ok_or_else(|| format!("eth_blockNumber 非字符串: {v}"))?;
        u64::from_str_radix(s.trim_start_matches("0x"), 16)
            .map_err(|e| format!("区块号解析失败: {e}"))
    }

    /// eth_getBalance（REVM fork 数据库用）。
    pub async fn balance_wei(&self, address: &str, block: Option<u64>) -> Result<u128, String> {
        let tag = block.map(|b| format!("0x{b:x}")).unwrap_or_else(|| "latest".into());
        let v = self.call("eth_getBalance", json!([address, tag])).await?;
        let s = v.as_str().ok_or_else(|| format!("eth_getBalance 非字符串: {v}"))?;
        u128::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("余额解析失败: {e}"))
    }

    /// eth_getTransactionCount。
    pub async fn nonce(&self, address: &str, block: Option<u64>) -> Result<u64, String> {
        let tag = block.map(|b| format!("0x{b:x}")).unwrap_or_else(|| "latest".into());
        let v = self.call("eth_getTransactionCount", json!([address, tag])).await?;
        let s = v.as_str().ok_or_else(|| format!("eth_getTransactionCount 非字符串: {v}"))?;
        u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("nonce 解析失败: {e}"))
    }

    /// eth_getCode。
    pub async fn code_at(&self, address: &str, block: Option<u64>) -> Result<Vec<u8>, String> {
        let tag = block.map(|b| format!("0x{b:x}")).unwrap_or_else(|| "latest".into());
        let v = self.call("eth_getCode", json!([address, tag])).await?;
        let s = v.as_str().ok_or_else(|| format!("eth_getCode 非字符串: {v}"))?;
        abi::hex_decode(s).map_err(|e| format!("eth_getCode 解码失败: {e}"))
    }

    /// eth_getStorageAt（slot 为 32 字节十六进制）。
    pub async fn storage_at(&self, address: &str, slot: &str, block: Option<u64>) -> Result<[u8; 32], String> {
        let tag = block.map(|b| format!("0x{b:x}")).unwrap_or_else(|| "latest".into());
        let v = self.call("eth_getStorageAt", json!([address, slot, tag])).await?;
        let s = v.as_str().ok_or_else(|| format!("eth_getStorageAt 非字符串: {v}"))?;
        let raw = abi::hex_decode(s).map_err(|e| format!("storage 解码失败: {e}"))?;
        let mut out = [0u8; 32];
        let start = 32usize.saturating_sub(raw.len());
        out[start..].copy_from_slice(&raw[raw.len().min(32) - (32 - start)..]);
        Ok(out)
    }
    pub async fn gas_price(&self) -> Result<u128, String> {
        let v = self.call("eth_gasPrice", json!([])).await?;
        let s = v.as_str().ok_or_else(|| format!("eth_gasPrice 非字符串: {v}"))?;
        u128::from_str_radix(s.trim_start_matches("0x"), 16)
            .map_err(|e| format!("gasPrice 解析失败: {e}"))
    }

    /// 区块头时间戳（秒）与 baseFee（wei）。
    pub async fn block_header(&self, block: u64) -> Result<(u64, u128), String> {
        let v = self
            .call(
                "eth_getBlockByNumber",
                json!([format!("0x{block:x}"), false]),
            )
            .await?;
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| "区块缺少 timestamp".to_string())?;
        let ts = u64::from_str_radix(ts.trim_start_matches("0x"), 16)
            .map_err(|e| format!("timestamp 解析失败: {e}"))?;
        let base = v
            .get("baseFeePerGas")
            .and_then(Value::as_str)
            .map(|s| u128::from_str_radix(s.trim_start_matches("0x"), 16))
            .transpose()
            .map_err(|e| format!("baseFee 解析失败: {e}"))?
            .unwrap_or(0);
        Ok((ts, base))
    }

    /// 批量 eth_call（自动分块、并发分片，单条失败不影响整批）。
    pub async fn batch_eth_call(
        &self,
        calls: &[Call],
        block: Option<u64>,
    ) -> Result<Vec<CallResult>, String> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let tag = block
            .map(|b| format!("0x{b:x}"))
            .unwrap_or_else(|| "latest".to_string());
        let chunks: Vec<Vec<Call>> = calls
            .chunks(40)
            .map(|chunk| chunk.to_vec())
            .collect();
        let mut out: Vec<CallResult> = Vec::with_capacity(calls.len());
        // 并发 2 个分片：既降低总延迟，又避免把公共 RPC 打出 429。
        for group in chunks.chunks(2) {
            let mut set = tokio::task::JoinSet::new();
            for (offset, chunk) in group.iter().enumerate() {
                let client = self.clone();
                let tag = tag.clone();
                let chunk = chunk.clone();
                set.spawn(async move {
                    let mut items = Vec::with_capacity(chunk.len());
                    let mut ids = Vec::with_capacity(chunk.len());
                    for call in &chunk {
                        let id = client.id();
                        ids.push(id);
                        items.push(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "method": "eth_call",
                            "params": [{
                                "to": abi::format_address(&call.to),
                                "data": abi::hex_encode(&call.data)
                            }, tag]
                        }));
                    }
                    let resp = client.raw(Value::Array(items)).await?;
                    Ok::<(usize, Vec<CallResult>), String>((offset, parse_batch(&resp, &ids)?))
                });
            }
            let mut slots: Vec<Option<Vec<CallResult>>> = vec![None; group.len()];
            while let Some(joined) = set.join_next().await {
                let (idx, results) = joined
                    .map_err(|e| format!("批量任务失败: {e}"))?
                    .map_err(|e| e)?;
                slots[idx] = Some(results);
            }
            for slot in slots {
                out.extend(slot.ok_or_else(|| "批量结果缺失".to_string())?);
            }
        }
        Ok(out)
    }

    /// 通用批量 JSON-RPC：一次 HTTP 往返执行多个方法（REVM 分叉读取用）。
    ///
    /// 返回与入参同序的结果；单条失败以 `Err` 表示，不影响其他条目。
    pub async fn batch_raw(
        &self,
        calls: Vec<(&str, Value)>,
    ) -> Result<Vec<Result<Value, String>>, String> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::with_capacity(calls.len());
        let mut items = Vec::with_capacity(calls.len());
        for (method, params) in calls {
            let id = self.id();
            ids.push(id);
            items.push(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }));
        }
        let resp = self.raw(Value::Array(items)).await?;
        if let Some(err) = resp.get("error") {
            return Err(format!("批量请求失败: {err}"));
        }
        let arr = resp
            .as_array()
            .ok_or_else(|| format!("批量响应不是数组: {resp}"))?;
        let mut out: Vec<Result<Value, String>> = vec![Err("批量响应缺失 id".into()); ids.len()];
        for item in arr {
            let Some(id) = item.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let Some(pos) = ids.iter().position(|x| *x == id) else {
                continue;
            };
            if let Some(error) = item.get("error") {
                out[pos] = Err(error.to_string());
            } else {
                out[pos] = Ok(item.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        Ok(out)
    }

    /// 拉取事件日志（topics[0] 为事件签名集合的 OR 过滤）。
    pub async fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        addresses: &[Address],
        topic0: &[[u8; 32]],
    ) -> Result<Vec<Log>, String> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let topic_hex: Vec<String> = topic0.iter().map(|t| abi::hex_encode(t)).collect();
        let filter = json!({
            "fromBlock": format!("0x{from_block:x}"),
            "toBlock": format!("0x{to_block:x}"),
            "address": addresses.iter().map(abi::format_address).collect::<Vec<_>>(),
            "topics": [topic_hex],
        });
        let v = self.call("eth_getLogs", json!([filter])).await?;
        let arr = v.as_array().ok_or_else(|| format!("eth_getLogs 返回非数组: {v}"))?;
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let address = item
                .get("address")
                .and_then(Value::as_str)
                .ok_or_else(|| "日志缺少 address".to_string())?;
            let block = item
                .get("blockNumber")
                .and_then(Value::as_str)
                .ok_or_else(|| "日志缺少 blockNumber".to_string())?;
            let block_number = u64::from_str_radix(block.trim_start_matches("0x"), 16)
                .map_err(|e| format!("日志区块号解析失败: {e}"))?;
            let log_index = item
                .get("logIndex")
                .and_then(Value::as_str)
                .map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0))
                .unwrap_or(0);
            let topics: Vec<Vec<u8>> = item
                .get("topics")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .filter_map(|s| abi::hex_decode(s).ok())
                        .collect()
                })
                .unwrap_or_default();
            let data = item
                .get("data")
                .and_then(Value::as_str)
                .map(|s| abi::hex_decode(s).unwrap_or_default())
                .unwrap_or_default();
            let tx = item
                .get("transactionHash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            out.push(Log {
                address: abi::parse_address(address).map_err(|e| format!("日志地址非法: {e}"))?,
                topics,
                data,
                block_number,
                log_index,
                transaction_hash: tx,
            });
        }
        out.sort_by_key(|l| (l.block_number, l.log_index));
        Ok(out)
    }
}

/// 按 id 对齐批量响应。
fn parse_batch(resp: &Value, ids: &[u64]) -> Result<Vec<CallResult>, String> {
    if let Some(err) = resp.get("error") {
        return Err(format!("批量请求失败: {err}"));
    }
    let arr = resp
        .as_array()
        .ok_or_else(|| format!("批量响应不是数组: {resp}"))?;
    let mut out: Vec<CallResult> = vec![
        CallResult {
            data: Vec::new(),
            error: Some("批量响应缺少该 id".into()),
        };
        ids.len()
    ];
    for item in arr {
        let Some(id) = item.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let Some(pos) = ids.iter().position(|x| *x == id) else {
            continue;
        };
        if let Some(error) = item.get("error") {
            out[pos] = CallResult {
                data: Vec::new(),
                error: Some(error.to_string()),
            };
        } else if let Some(result) = item.get("result").and_then(Value::as_str) {
            out[pos] = CallResult {
                data: abi::hex_decode(result).unwrap_or_default(),
                error: None,
            };
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_alignment_and_errors() {
        let ids = vec![1, 2, 3];
        let resp = json!([
            {"jsonrpc":"2.0","id":2,"result":"0x00"},
            {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"revert"}},
            {"jsonrpc":"2.0","id":3,"result":"0x1234"}
        ]);
        let out = parse_batch(&resp, &ids).unwrap();
        assert_eq!(out[0].error.as_deref(), Some(r#"{"code":-32000,"message":"revert"}"#));
        assert_eq!(out[1].data, vec![0x00]);
        assert_eq!(out[2].data, vec![0x12, 0x34]);
    }

    #[test]
    fn client_constructs() {
        let c = RpcClient::new("https://example.invalid", Some("https://fallback.invalid"));
        assert_eq!(c.primary(), "https://example.invalid");
        assert_eq!(c.fallback(), Some("https://fallback.invalid"));
        assert!(RpcClient::new("x", None).fallback().is_none());
    }

    #[test]
    fn batch_response_missing_id_is_error() {
        let ids = vec![9];
        let ok = parse_batch(&json!([{"id":1,"result":"0x"}]), &ids).unwrap();
        assert!(!ok[0].ok());
    }
}
