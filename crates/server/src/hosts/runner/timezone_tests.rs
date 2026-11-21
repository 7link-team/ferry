//! Host time zone delivery: only to runners that understand it, on connect and
//! on change, with the detected zone and capability stored on the host row.
use super::*;
use crate::agents::tests::setup;
use futures::SinkExt;
use tokio_tungstenite::tungstenite::Message as WsMessage;

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn add_host(s: &AppState, id: i64, key: &str, timezone: &str) {
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
        timezone: Set(timezone.into()),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
}

async fn connect_runner(
    address: std::net::SocketAddr,
    host: i64,
    key: &str,
    supports: bool,
) -> Client {
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/runners/connect"))
            .await
            .unwrap();
    let hello = wire::Hello {
        version: wire::VERSION,
        auth: wire::Auth::Key {
            host_id: host.to_string(),
            key: key.into(),
        },
        hostname: "fixture".into(),
        ip_address: String::new(),
        os: "linux".into(),
        arch: "x86_64".into(),
        agent_version: "fixture".into(),
        managed_updates: false,
        installation_scope: None,
        supports_monitor: false,
        supports_agent: false,
        supports_agent_probe: false,
        supports_forward: false,
        supports_timezone: supports,
        supports_resource_policy: false,
        timezone: "Asia/Tokyo".into(),
    };
    socket
        .send(WsMessage::Text(
            serde_json::to_string(&hello).unwrap().into(),
        ))
        .await
        .unwrap();
    socket
}

/// Control frames until (and including) the first Ping.
async fn frames_until_ping(socket: &mut Client) -> Vec<wire::Control> {
    let mut frames = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no control frame")
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let frame: wire::Control = serde_json::from_str(&text).unwrap();
        let ping = matches!(frame, wire::Control::Ping);
        frames.push(frame);
        if ping {
            return frames;
        }
    }
}

async fn next_settings(socket: &mut Client) -> Option<String> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no settings frame")
            .unwrap()
            .unwrap();
        if let WsMessage::Text(text) = message {
            if let Ok(wire::Control::Settings { timezone, .. }) = serde_json::from_str(&text) {
                return timezone;
            }
        }
    }
}

#[tokio::test]
async fn settings_reach_only_runners_that_understand_them() {
    let (_dir, s) = setup().await;
    add_host(&s, 9101, "tz-new-key", "Europe/Berlin").await;
    add_host(&s, 9102, "tz-old-key", "Europe/Berlin").await;
    let app = axum::Router::new()
        .route("/api/runners/connect", axum::routing::get(connect))
        .with_state(s.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // A current runner: Welcome, then its pinned zone before the first Ping.
    let mut current = connect_runner(address, 9101, "tz-new-key", true).await;
    let frames = frames_until_ping(&mut current).await;
    assert!(matches!(frames[0], wire::Control::Welcome { .. }));
    assert!(frames.iter().any(|frame| matches!(
        frame,
        wire::Control::Settings { timezone: Some(zone), .. } if zone == "Europe/Berlin"
    )));
    let row = hosts::Entity::find_by_id(9101_i64)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.detected_timezone, "Asia/Tokyo");
    assert_eq!(row.timezone_supported, 1);
    // A change made here is pushed at once; "" means the host's own zone.
    push_timezone(Id::from(9101_i64), "");
    assert_eq!(next_settings(&mut current).await, None);

    // An older runner would reject the unknown frame: it never gets one.
    let mut old = connect_runner(address, 9102, "tz-old-key", false).await;
    let frames = frames_until_ping(&mut old).await;
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, wire::Control::Settings { .. })),
        "{frames:?}"
    );
    push_timezone(Id::from(9102_i64), "Asia/Shanghai");
    let row = hosts::Entity::find_by_id(9102_i64)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.timezone_supported, 0);
    server.abort();
}

#[test]
fn only_known_iana_zones_are_accepted() {
    assert_eq!(check_timezone(" Asia/Shanghai ").unwrap(), "Asia/Shanghai");
    assert_eq!(check_timezone("").unwrap(), "");
    for bad in ["Mars/Olympus", "Asia/Shanghai; rm", "UTC+8"] {
        let error = check_timezone(bad).unwrap_err();
        assert!(
            format!("{error:?}").contains("runner.timezoneInvalid"),
            "{bad}"
        );
    }
    assert_eq!(timezone_setting(""), None);
    assert_eq!(timezone_setting("UTC").as_deref(), Some("UTC"));
}
