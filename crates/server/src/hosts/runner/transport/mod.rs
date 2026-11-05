//! Session rendezvous, WebSocket framing, and transport error identities.
use super::*;
mod channel;
mod control;
mod pty;
pub(crate) use channel::cluster_open;
pub use channel::open;
pub use control::connect;
use pty::{RunnerPty, pump_pty};

/// How long a runner has to dial a session it was asked for.
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a handshake frame may take to arrive.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a small metadata answer (listing, delete, file header) may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Upload chunks are re-cut to this, so one oversized body chunk cannot exceed
/// the Socket frame limit at the other end.
const CHUNK: usize = 256 * 1024;

/// A session the server has asked a runner to dial. `host_id` is what stops a
/// second runner claiming someone else's session id.
struct Pending {
    host_id: String,
    socket: oneshot::Sender<Result<Socket>>,
}

static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);

fn pending() -> MutexGuard<'static, HashMap<String, Pending>> {
    PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Hands a dialled session to whoever asked for it, but only to the runner it
/// was asked of. Separate from the handler so the binding rule is testable
/// without a socket.
fn claim_pending(session: &str, host_id: &str) -> Option<Pending> {
    let mut map = pending();
    match map.get(session) {
        Some(entry) if entry.host_id == host_id => map.remove(session),
        _ => None,
    }
}

fn offline(name: &str) -> ApiError {
    ApiError::localized(
        StatusCode::SERVICE_UNAVAILABLE,
        crate::i18n::Message::new("runner.offline").with("name", name),
    )
}

fn remote_failed(error: impl std::fmt::Display) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_GATEWAY,
        crate::i18n::Message::new("runner.remoteFailed").with("detail", error.to_string()),
    )
}

fn connection_failed(error: ApiError) -> ApiError {
    let message = crate::i18n::Message::new("runner.remoteFailed");
    let message = match error.2 {
        Some(identity) => message.with_message("detail", identity),
        None => message.with("detail", error.1),
    };
    ApiError::localized(StatusCode::BAD_GATEWAY, message)
}

fn garbled() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_GATEWAY,
        crate::i18n::Message::new("runner.garbled"),
    )
}

fn timed_out() -> ApiError {
    ApiError::localized(
        StatusCode::GATEWAY_TIMEOUT,
        crate::i18n::Message::new("runner.timeout"),
    )
}

fn runner_extra(key: &str) -> ApiError {
    ApiError::localized(StatusCode::BAD_GATEWAY, crate::i18n::Message::new(key))
}

fn session_failed(error: String, identity: Option<wire::SystemMessage>) -> ApiError {
    if identity.is_some_and(|message| {
        message.key == "runnerExtra.preparingUpgrade" && message.params.is_empty()
    }) {
        runner_extra("runnerExtra.preparingUpgrade")
    } else {
        remote_failed(error)
    }
}

// -------------------------------------------------------------- 帧读写

async fn send_frame<T: Serialize>(socket: &mut Socket, frame: &T) -> Result<()> {
    let text = serde_json::to_string(frame).map_err(|_| garbled())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(connection_failed)
}

/// Reads frames until a JSON text frame arrives. Binary frames are payload and
/// never control, so they are skipped rather than treated as an error.
async fn next_frame<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> Result<T> {
    while let Some(message) = socket.recv().await {
        match message.map_err(connection_failed)? {
            Message::Text(text) => return serde_json::from_str(&text).map_err(|_| garbled()),
            Message::Close(_) => break,
            _ => {}
        }
    }
    Err(garbled())
}

async fn handshake<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> Result<T> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, next_frame(socket))
        .await
        .map_err(|_| timed_out())?
}

/// Says no in a way the other end can log, instead of just closing the socket.
async fn reject(socket: &mut Socket, error: &ApiError) {
    let reason = error.render(crate::i18n::current_locale());
    tracing::warn!(%reason, "拒绝 runner 连接");
    let _ = send_frame(socket, &wire::Control::Rejected { reason }).await;
    let _ = socket.send(Message::Close(None)).await;
}

/// `GET /api/runners/sessions/{session}` — one connection for one session,
/// dialled by the runner after `OpenSession`.
pub async fn session(
    State(s): State<AppState>,
    Path(session): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let locale = crate::i18n::current_locale();
    upgrade.on_upgrade(move |socket| {
        crate::i18n::with_locale(locale, hand_over(s, session, socket.into()))
    })
}

async fn hand_over(s: AppState, session: String, mut socket: Socket) {
    let hello: wire::SessionHello = match handshake(&mut socket).await {
        Ok(hello) => hello,
        Err(error) => return reject(&mut socket, &error).await,
    };
    if hello.version != wire::VERSION {
        return reject(
            &mut socket,
            &runner_extra("runnerExtra.sessionProtocolMismatch"),
        )
        .await;
    }
    if let Err(error) = verify_key(&s, &hello.host_id, &hello.key).await {
        return reject(&mut socket, &error).await;
    }
    if let Some(cluster) = &s.cluster {
        let route = match cluster.claim_session(&session, &hello.host_id).await {
            Ok(route) => route,
            Err(error) => return reject(&mut socket, &error).await,
        };
        match cluster.socket(&session, true, &route).await {
            Ok(forwarded) => crate::cluster::bridge(socket, forwarded).await,
            Err(error) => reject(&mut socket, &error).await,
        }
        return;
    }
    // The session id is only ever sent to one runner, and this is the check
    // that makes that binding enforced rather than merely true.
    let Some(waiting) = claim_pending(&session, &hello.host_id) else {
        return reject(&mut socket, &runner_extra("runnerExtra.sessionUnavailable")).await;
    };
    // The socket now lives in the request that asked for it; this task is done.
    if let Err(Ok(mut socket)) = waiting.socket.send(Ok(socket)) {
        let _ = socket.send(Message::Close(None)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::runner::test_support::scratch_db;

    #[test]
    fn connection_errors_preserve_local_and_cluster_classification_in_six_languages() {
        for key in ["runner.transportFailed", "runner.clusterUnavailable"] {
            let identity = crate::i18n::Message::new(key);
            let error = connection_failed(ApiError::localized(
                StatusCode::BAD_GATEWAY,
                identity.clone(),
            ));
            for locale in crate::i18n::Locale::ALL {
                assert_eq!(
                    error.render(locale),
                    crate::i18n::render(
                        locale,
                        &crate::i18n::Message::new("runner.remoteFailed")
                            .with_message("detail", identity.clone())
                    )
                );
            }
        }
        let raw = "literal provider {runner.transportFailed}";
        let error = connection_failed(ApiError(StatusCode::BAD_GATEWAY, raw.into(), None));
        for locale in crate::i18n::Locale::ALL {
            assert!(error.render(locale).contains(raw));
        }
    }

    #[test]
    fn session_failure_identity_is_allowlisted_and_preserves_unknown_raw_reasons() {
        let raw = "provider 原文 {runnerExtra.preparingUpgrade}";
        let known = session_failed(
            raw.into(),
            Some(wire::SystemMessage::new("runnerExtra.preparingUpgrade")),
        );
        for locale in crate::i18n::Locale::ALL {
            assert_eq!(
                known.render(locale),
                crate::i18n::render(
                    locale,
                    &crate::i18n::Message::new("runnerExtra.preparingUpgrade")
                )
            );
            for identity in [
                None,
                Some(wire::SystemMessage::new("error.forbidden")),
                Some(wire::SystemMessage {
                    key: "runnerExtra.preparingUpgrade".into(),
                    params: std::collections::BTreeMap::from([(
                        "unexpected".into(),
                        "value".into(),
                    )]),
                }),
            ] {
                let error = session_failed(raw.into(), identity);
                assert!(error.render(locale).contains(raw));
                assert_eq!(error.2.unwrap().key, "runner.remoteFailed");
            }
        }
    }

    #[tokio::test]
    async fn websocket_rejections_and_terminal_errors_follow_each_readers_locale() {
        use crate::i18n::{Locale, render};
        use axum::{Router, middleware, routing::get};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Exercise the actual HTTP upgrade and masked WebSocket text frames.
        async fn rejected(
            address: std::net::SocketAddr,
            path: &str,
            locale: Locale,
            payload: Value,
        ) -> String {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAccept-Language: {}\r\n\r\n",
                locale.tag()
            );
            socket.write_all(request.as_bytes()).await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
                assert!(headers.len() < 8192);
            }
            assert!(
                headers.starts_with(b"HTTP/1.1 101"),
                "{}",
                String::from_utf8_lossy(&headers)
            );
            let payload = serde_json::to_vec(&payload).unwrap();
            assert!(payload.len() < 65536);
            let mut frame = vec![0x81];
            if payload.len() < 126 {
                frame.push(0x80 | payload.len() as u8);
            } else {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            let mask = [1, 2, 3, 4];
            frame.extend_from_slice(&mask);
            frame.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| byte ^ mask[index % 4]),
            );
            socket.write_all(&frame).await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 0x81);
            let length = socket.read_u8().await.unwrap();
            assert_eq!(length & 0x80, 0);
            let length = match length {
                126 => socket.read_u16().await.unwrap() as usize,
                127 => socket.read_u64().await.unwrap() as usize,
                n => n as usize,
            };
            assert!(length < 65536);
            let mut message = vec![0; length];
            socket.read_exact(&mut message).await.unwrap();
            let message: Value = serde_json::from_slice(&message).unwrap();
            assert_eq!(message["type"], "rejected");
            assert_eq!(
                socket.read_u8().await.unwrap() & 0x0f,
                8,
                "rejection still closes the connection"
            );
            message["reason"].as_str().unwrap().to_owned()
        }

        let dir = tempfile::tempdir().unwrap();
        let scratch = scratch_db().await;
        let s = Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db: scratch.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "runner-locale-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let key = secret();
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(7),
            workspace_id: Set(1),
            name: Set("host".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("offline".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(fingerprint(&key)),
            enroll_token: Set(String::new()),
            enroll_token_hash: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            metrics: Set(String::new()),
            credential_id: Set(0),
            ..Default::default()
        })
        .exec(&s.db)
        .await
        .unwrap();
        let app = Router::new()
            .route("/connect", get(connect))
            .route("/sessions/{session}", get(session))
            .route(
                "/raw",
                get(|upgrade: WebSocketUpgrade| async move {
                    let locale = crate::i18n::current_locale();
                    upgrade.on_upgrade(move |socket| {
                        crate::i18n::with_locale(locale, async move {
                            let mut socket: Socket = socket.into();
                            let _: Value = next_frame(&mut socket).await.unwrap();
                            reject(
                                &mut socket,
                                &ApiError::bad("provider 原文 {runnerExtra.hostIdInvalid}"),
                            )
                            .await;
                        })
                    })
                }),
            )
            .layer(middleware::from_fn(crate::i18n::middleware))
            .with_state(s);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for locale in Locale::ALL {
            // Serializing the actual protocol type keeps this fixture aligned with its auth tag.
            let hello: Value = serde_json::to_value(wire::Hello {
                version: wire::VERSION + 1,
                auth: wire::Auth::Enroll {
                    token: "unused".into(),
                },
                hostname: "literal hostname".into(),
                ip_address: String::new(),
                os: "linux".into(),
                arch: "amd64".into(),
                agent_version: "fixture".into(),
                managed_updates: false,
                supports_monitor: false,
            })
            .unwrap();
            let expected = crate::i18n::Message::new("runnerExtra.protocolMismatch")
                .with("expected", wire::VERSION.to_string())
                .with("actual", (wire::VERSION + 1).to_string());
            assert_eq!(
                rejected(address, "/connect", locale, hello.clone()).await,
                render(locale, &expected)
            );
            let session_hello = json!({"version":wire::VERSION,"host_id":"7","key":key});
            assert_eq!(
                rejected(address, "/sessions/unknown", locale, session_hello.clone()).await,
                render(
                    locale,
                    &crate::i18n::Message::new("runnerExtra.sessionUnavailable")
                )
            );
            let mut wrong_version = session_hello;
            wrong_version["version"] = json!(wire::VERSION + 1);
            assert_eq!(
                rejected(address, "/sessions/unknown", locale, wrong_version).await,
                render(
                    locale,
                    &crate::i18n::Message::new("runnerExtra.sessionProtocolMismatch")
                )
            );
            let mut invalid_credentials = hello;
            invalid_credentials["version"] = json!(wire::VERSION);
            invalid_credentials["auth"] = serde_json::to_value(wire::Auth::Key {
                host_id: "7".into(),
                key: "wrong-secret".into(),
            })
            .unwrap();
            assert_eq!(
                rejected(address, "/connect", locale, invalid_credentials).await,
                render(locale, &crate::i18n::Message::new("runner.credentials"))
            );
            assert_eq!(
                rejected(address, "/raw", locale, json!({})).await,
                "provider 原文 {runnerExtra.hostIdInvalid}"
            );
            let (input, receiver) = mpsc::channel(1);
            drop(receiver);
            let (_, output) = mpsc::channel(1);
            let mut pty = RunnerPty { input, output };
            for error in [
                pty.write(b"literal input").await.unwrap_err(),
                pty.resize(PtySize { cols: 80, rows: 24 })
                    .await
                    .unwrap_err(),
            ] {
                assert_eq!(error.0, StatusCode::BAD_GATEWAY);
                assert_eq!(
                    error.render(locale),
                    render(
                        locale,
                        &crate::i18n::Message::new("runnerExtra.terminalEnded")
                    )
                );
            }
            let provider = remote_failed("provider 原文 {runnerExtra.terminalEnded}");
            assert!(
                provider
                    .render(locale)
                    .contains("provider 原文 {runnerExtra.terminalEnded}")
            );
        }
        server.abort();
    }

    #[test]
    fn a_session_can_only_be_claimed_by_the_runner_it_was_asked_of() {
        let (socket, _waiting) = oneshot::channel();
        pending().insert(
            "session-a".into(),
            Pending {
                host_id: "host-a".into(),
                socket,
            },
        );

        assert!(
            claim_pending("session-a", "host-b").is_none(),
            "another host must not claim it"
        );
        assert!(
            claim_pending("session-nope", "host-a").is_none(),
            "an unknown session must not be claimable"
        );
        // Refusing must not consume it: the rightful runner is still coming.
        assert!(claim_pending("session-a", "host-a").is_some());
        assert!(
            claim_pending("session-a", "host-a").is_none(),
            "a session is claimable once"
        );
    }
}
