//! WS 订阅（可选）：`newHeads` → 触发提前同步（V22-107/108 的传输层）。
//!
//! 设计要点：
//! - 免费公共端点多数不支持 `eth_subscribe` 或限流；**连接/订阅失败不阻塞主流程**，
//!   调用方（`engine::spawn_feed`）会继续按轮询间隔同步；
//! - 接入付费 WS 只需在配置里填 `[live] ws_endpoints`，代码不变；
//! - 只解析订阅推送里的区块号，不信任任何其他字段。

use futures_util::{SinkExt, StreamExt};

/// 订阅 `newHeads`，把区块号推入 `tx`；连接断开时返回。
pub async fn watch_heads(url: &str, tx: tokio::sync::mpsc::Sender<u64>) -> Result<(), String> {
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("WS 连接失败: {e}"))?;

    let sub = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["newHeads"]
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        sub.to_string().into(),
    ))
    .await
    .map_err(|e| format!("WS 发送订阅失败: {e}"))?;

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| format!("WS 消息错误: {e}"))?;
        let text = match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                return Err("WS 连接关闭".into())
            }
            _ => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(err) = v.get("error") {
            return Err(format!("WS 订阅错误: {err}"));
        }
        // 只取订阅推送的区块号（十六进制）。
        if let Some(num) = v.pointer("/params/result/number").and_then(|x| x.as_str()) {
            let hex = num.trim_start_matches("0x");
            if let Ok(block) = u64::from_str_radix(hex, 16) {
                if tx.send(block).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}
