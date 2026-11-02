//! WebSocket connection and protocol frame transport shared by control and sessions.

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio_tungstenite::tungstenite::Message;

/// `http://host:port` -> `ws://host:port/path`, `https://` -> `wss://`.
/// Taking a base URL rather than a WebSocket URL keeps the one-command install
/// honest: the operator pastes the address they already use in a browser.
pub(super) fn ws_url(base: &str, path: &str) -> anyhow::Result<String> {
    let base = base.trim().trim_end_matches('/');
    let rest = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_owned()
    } else {
        anyhow::bail!("服务地址需要以 http:// 或 https:// 开头，当前为「{base}」。");
    };
    anyhow::ensure!(
        !rest.ends_with("://") && rest.split("://").nth(1).is_some_and(|h| !h.is_empty()),
        "服务地址缺少主机名：「{base}」。"
    );
    Ok(format!("{rest}{path}"))
}

pub(super) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn dial(url: &str) -> anyhow::Result<Socket> {
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    Ok(socket)
}

pub(super) async fn send<T: Serialize>(socket: &mut Socket, frame: &T) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(frame)?.into()))
        .await?;
    Ok(())
}

/// Reads frames until a JSON text frame arrives, which is the only shape
/// control information ever takes.
pub(super) async fn next_frame<T: serde::de::DeserializeOwned>(
    socket: &mut Socket,
) -> anyhow::Result<T> {
    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .with_context(|| format!("无法解析服务下发的帧：{text}"));
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    anyhow::bail!("连接已关闭，未收到预期的帧。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_becomes_the_matching_websocket_url() {
        assert_eq!(
            ws_url("http://127.0.0.1:4290", "/api/runners/connect").unwrap(),
            "ws://127.0.0.1:4290/api/runners/connect"
        );
        // A trailing slash is what an operator pastes out of a browser.
        assert_eq!(
            ws_url("https://ops.example.com/", "/api/runners/connect").unwrap(),
            "wss://ops.example.com/api/runners/connect"
        );
        assert_eq!(
            ws_url("ws://127.0.0.1:9/", "/x").unwrap(),
            "ws://127.0.0.1:9/x"
        );
        for bad in ["ops.example.com", "ftp://ops", "http://", ""] {
            assert!(ws_url(bad, "/x").is_err(), "{bad} should be refused");
        }
    }
}
