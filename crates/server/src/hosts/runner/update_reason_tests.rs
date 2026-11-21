//! Why an update rolled back: the runner reports the supervisor's reason with
//! UpdateHealth, and the server turns it into a per-reader message.
use super::*;
use crate::agents::tests::setup;
use futures::SinkExt;
use tokio_tungstenite::tungstenite::Message as WsMessage;

async fn add_host(s: &AppState, id: i64, key: &str) {
    hosts::Entity::insert(hosts::ActiveModel {
        id: Set(id),
        workspace_id: Set(1),
        name: Set(format!("Runner {id}")),
        address: Set(String::new()),
        port: Set(0),
        username: Set(String::new()),
        connection: Set("runner".into()),
        state: Set("offline".into()),
        group_id: Set(0),
        description: Set(String::new()),
        host_key: Set(String::new()),
        runner_key_hash: Set(fingerprint(key)),
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
}

#[tokio::test]
async fn a_reported_rollback_reason_is_kept_with_the_failed_version() {
    let (_dir, s) = setup().await;
    add_host(&s, 9201, "reason-key").await;
    let app = axum::Router::new()
        .route("/api/runners/connect", axum::routing::get(connect))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/runners/connect"))
            .await
            .unwrap();
    let hello = wire::Hello {
        version: wire::VERSION,
        auth: wire::Auth::Key {
            host_id: "9201".into(),
            key: "reason-key".into(),
        },
        hostname: "fixture".into(),
        ip_address: String::new(),
        os: "windows".into(),
        arch: "x86_64".into(),
        agent_version: "0.8.2-alpha".into(),
        managed_updates: true,
        installation_scope: Some("user".into()),
        supports_monitor: false,
        supports_agent: false,
        supports_agent_probe: false,
        supports_forward: false,
        supports_timezone: true,
        supports_resource_policy: false,
        timezone: String::new(),
    };
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&hello).unwrap().into(),
        ))
        .await
        .unwrap();
    // Wait for Welcome before reporting health.
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no welcome")
            .unwrap()
            .unwrap();
        if let WsMessage::Text(text) = message {
            if matches!(
                serde_json::from_str::<wire::Control>(&text),
                Ok(wire::Control::Welcome { .. })
            ) {
                break;
            }
        }
    }
    let health = wire::Report::UpdateHealth {
        version: "0.8.2-alpha".into(),
        failed_version: Some("0.9.0-alpha".into()),
        failed_reason: Some("timeout:60".into()),
    };
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&health).unwrap().into(),
        ))
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let live_reason = loop {
        let peer = live().get("9201").cloned();
        if let Some(peer) = peer {
            if peer.ready.load(std::sync::atomic::Ordering::Acquire) {
                break peer.failed_reason.lock().unwrap().clone();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "runner never became ready"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(live_reason.as_deref(), Some("timeout:60"));
    let peer = live().get("9201").cloned().unwrap();
    assert_eq!(
        peer.failed_version.lock().unwrap().as_deref(),
        Some("0.9.0-alpha")
    );
    server.abort();
}

#[test]
fn rollback_reasons_read_as_sentences_and_unknown_ones_keep_their_text() {
    use crate::i18n::{Locale, render_stored};
    let zh = |raw: &str| render_stored(Locale::ZhCn, &updates::rollback_reason(Some(raw)).unwrap());
    assert_eq!(zh("timeout:60"), "新版本 60 秒内未连上平台");
    assert_eq!(zh("exit:3"), "新版本启动后退出（退出码 3）");
    assert_eq!(zh("signal:9"), "新版本启动时被信号 9 终止");
    assert_eq!(zh("invalid_event"), "新版本输出了无法识别的控制消息");
    assert_eq!(
        zh("version_mismatch:1.0.0"),
        "新版本报告的版本 1.0.0 与升级包不一致"
    );
    assert_eq!(zh("spawn:access denied"), "新版本无法启动（access denied）");
    assert_eq!(zh("missing_binary"), "新版本文件缺失或校验失败");
    assert_eq!(zh("interrupted"), "激活完成前服务已重启");
    // A malformed known code, and an unknown one, keep their text rather than inventing a cause.
    assert_eq!(zh("timeout:soon"), "timeout:soon");
    assert_eq!(zh("error:worker crashed"), "error:worker crashed");
    assert!(updates::rollback_reason(Some("  ")).is_none());
    assert!(updates::rollback_reason(None).is_none());
    let en = render_stored(
        Locale::EnUs,
        &updates::rollback_reason(Some("timeout:60")).unwrap(),
    );
    assert_eq!(en, "it did not connect to the platform within 60 s");
}
