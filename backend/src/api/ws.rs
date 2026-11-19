//! WebSocket 推送：机会、执行、风控、扫描状态、盘口与账本事件。
//!
//! 客户端只需连接 `/ws`，服务端在连接时推送一次快照，随后推送增量事件。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

use crate::engine::{AppState, WsEvent};

pub async fn ws_handler(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn handle(mut socket: WebSocket, state: Arc<AppState>) {
    // 连接即推送扫描状态与当前机会快照，避免前端空等
    let status = state.scanner.read().await.clone();
    let snapshot = serde_json::to_string(&WsEvent::Scanner {
        status: Box::new(status),
    })
    .unwrap_or_default();
    if socket.send(Message::Text(snapshot.into())).await.is_err() {
        return;
    }
    {
        let opps = state.opportunities.read().await;
        for o in opps.iter().take(30).rev() {
            if let Ok(s) = serde_json::to_string(&WsEvent::Opportunity {
                opportunity: Box::new(o.clone()),
            }) {
                if socket.send(Message::Text(s.into())).await.is_err() {
                    return;
                }
            }
        }
    }

    let mut rx = state.events.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        if let Ok(s) = serde_json::to_string(&ev) {
                            if socket.send(Message::Text(s.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let msg = serde_json::json!({"type":"lagged","missed":n});
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(p))) => {
                        if socket.send(Message::Pong(p)).await.is_err() { break; }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}
