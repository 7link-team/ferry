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
    dial_with_policy(url, &ferry_protocol::resources::ResourcePolicy::default()).await
}

pub(super) async fn dial_with_policy(
    url: &str,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> anyhow::Result<Socket> {
    let limit = policy
        .websocket_message_bytes
        .map(|value| ferry_protocol::resources::byte_limit(Some(value)));
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(limit)
        .max_frame_size(limit);
    let (socket, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), false)
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    Ok(socket)
}

/// No deadline is scheduled unless the platform explicitly configured one.
pub(crate) async fn deadline(seconds: Option<u64>) {
    match seconds.filter(|value| *value > 0).and_then(|value| {
        tokio::time::Instant::now().checked_add(std::time::Duration::from_secs(value))
    }) {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
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

    #[tokio::test]
    async fn absent_zero_and_huge_deadlines_do_not_expire_or_panic() {
        for seconds in [None, Some(0), Some(9_007_199_254_740_991), Some(u64::MAX)] {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), deadline(seconds))
                    .await
                    .is_err(),
                "unexpected expiration for {seconds:?}"
            );
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), deadline(Some(1)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn websocket_defaults_have_no_implicit_frame_or_message_ceiling() {
        for limit in [None, Some(1024)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let _ = socket
                    .send(Message::Binary(vec![0; 16 * 1024 * 1024 + 1].into()))
                    .await;
            });
            let policy = ferry_protocol::resources::ResourcePolicy {
                websocket_message_bytes: limit,
                ..Default::default()
            };
            let mut socket = dial_with_policy(&url, &policy).await.unwrap();
            assert_eq!(
                socket.get_config().max_message_size,
                limit.map(|value| value as usize)
            );
            assert_eq!(
                socket.get_config().max_frame_size,
                limit.map(|value| value as usize)
            );
            let received = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap();
            if limit.is_none() {
                assert!(
                    matches!(received.unwrap(), Message::Binary(bytes) if bytes.len() > 16 * 1024 * 1024)
                );
            } else {
                assert!(received.is_err());
            }
            drop(socket);
            server.await.unwrap();
        }
    }

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
