//! Preview hole tests: see docs/preview-holes-design.md section 14.
//!
//! The proxy tests run the real control and session endpoints, the real
//! request guard, and a fake Runner that implements `Forward` the same way the
//! Runner binary does, against a local service that reports what it received.
use super::proxy::{self, SANDBOX};
use super::*;
use crate::agents::tests::{admin, setup};
use axum::{
    extract::Request,
    middleware,
    response::{IntoResponse, Response},
    routing::any,
};
use ferry_protocol as wire;
use futures::{SinkExt, StreamExt};
use sea_orm::{ActiveValue::Set, EntityTrait, QueryFilter};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

const KEY: &str = "fixture-runner-key";
const OLD_KEY: &str = "fixture-old-runner-key";

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

async fn add_host(s: &AppState, id: i64, workspace: i64, key: &str) {
    hosts::Entity::insert(hosts::ActiveModel {
        id: Set(id),
        workspace_id: Set(workspace),
        name: Set(format!("Runner {id}")),
        address: Set(String::new()),
        port: Set(0),
        username: Set(String::new()),
        connection: Set("runner".into()),
        state: Set("offline".into()),
        group_id: Set(0),
        description: Set(String::new()),
        host_key: Set(String::new()),
        runner_key_hash: Set(crate::hosts::runner::fingerprint(key)),
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

async fn create_hole(s: &AppState, host: i64, port: u16) -> Value {
    let (status, Json(created)) = create(
        State(s.clone()),
        admin(1),
        Json(parse(json!({"name":"Preview","host_id":host.to_string(),
                          "target_port":port,"ttl_hours":24}))),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    created
}

fn hole_id(value: &Value) -> Id {
    value["id"].as_str().unwrap().parse().unwrap()
}

// ------------------------------------------------------------- API

#[tokio::test]
async fn resource_lifecycle_validation_and_isolation() {
    let (_dir, s) = setup().await;
    add_host(&s, 11, 1, KEY).await;
    let attempt = |body: Value| {
        let s = s.clone();
        async move { create(State(s), admin(1), Json(parse(body))).await }
    };
    for body in [
        json!({"name":"  ","host_id":"11","target_port":80,"ttl_hours":24}),
        json!({"name":"x".repeat(129),"host_id":"11","target_port":80,"ttl_hours":24}),
        json!({"name":"a","host_id":"11","target_port":0,"ttl_hours":24}),
        json!({"name":"a","host_id":"11","target_port":65536,"ttl_hours":24}),
        json!({"name":"a","host_id":"11","target_port":80,"ttl_hours":5}),
        // Host 2 belongs to workspace 2.
        json!({"name":"a","host_id":"2","target_port":80,"ttl_hours":24}),
    ] {
        let error = attempt(body).await.unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }
    let mut readonly = admin(1);
    readonly.permissions.retain(|p| p != "holes.create");
    let denied = create(
        State(s.clone()),
        readonly,
        Json(parse(
            json!({"name":"a","host_id":"11","target_port":80,"ttl_hours":24}),
        )),
    )
    .await
    .unwrap_err();
    assert_eq!(denied.0, StatusCode::FORBIDDEN);

    let before = db::now();
    let created =
        attempt(json!({"name":"  Web  ","host_id":"11","target_port":5173,"ttl_hours":24}))
            .await
            .unwrap()
            .1
            .0;
    let id = hole_id(&created);
    assert_eq!(created["name"], "Web");
    assert_eq!(created["state"], "active");
    assert_eq!(created["strip_prefix"], true);
    assert_eq!(created["probe_state"], "unknown");
    assert_eq!(created["created_by_name"], "Owner 1");
    assert_eq!(created["host_name"], "Runner 11");
    let url = created["url"].as_str().unwrap();
    let token = url
        .strip_prefix("/holes/")
        .unwrap()
        .strip_suffix('/')
        .unwrap();
    assert_eq!(token.len(), 64);
    let expires = created["expires_at"].as_i64().unwrap();
    assert!(expires >= before + 24 * HOUR_MS && expires <= db::now() + 24 * HOUR_MS);
    // Only the hash and a sealed copy are stored.
    let row = holes::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.token_hash, crate::hosts::runner::fingerprint(token));
    assert!(!row.token_sealed.contains(token));

    let list_of = |state: &str| {
        let s = s.clone();
        let state = state.to_owned();
        async move {
            list(
                State(s),
                admin(1),
                Query(HashMap::from([("state".to_owned(), state)])),
            )
            .await
        }
    };
    assert_eq!(list_of("active").await.unwrap().0["total"], 1);
    assert_eq!(list_of("revoked").await.unwrap().0["total"], 0);
    assert_eq!(
        list_of("bogus").await.unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let other = detail(State(s.clone()), admin(2), Path(("2".into(), id)))
        .await
        .unwrap_err();
    assert_eq!(other.0, StatusCode::NOT_FOUND);

    // The shared form sends the whole record back; extra fields are ignored.
    let mut edit = created.clone();
    edit["name"] = json!("Renamed");
    edit["ttl_hours"] = json!(1);
    edit["version"] = json!(0);
    let stale = update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(edit.clone())),
    )
    .await
    .unwrap_err();
    assert_eq!(stale.0, StatusCode::CONFLICT);
    edit["version"] = json!(1);
    let edited = update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(edit)),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(edited["name"], "Renamed");
    assert_eq!(edited["version"], 2);
    assert!(edited["expires_at"].as_i64().unwrap() <= db::now() + HOUR_MS);
    assert_eq!(edited["url"], created["url"], "editing keeps the link");

    let revoked_view = revoke(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap()
        .0;
    assert_eq!(revoked_view["state"], "revoked");
    for error in [
        revoke(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap_err(),
        update(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(parse(json!({"version":3,"name":"x"}))),
        )
        .await
        .unwrap_err(),
        probe_now(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.0, StatusCode::CONFLICT);
    }
    assert_eq!(list_of("revoked").await.unwrap().0["total"], 1);
    assert_eq!(
        remove(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );

    // Deleting the host takes its holes with it.
    create_hole(&s, 11, 80).await;
    crate::hosts::delete_host(State(s.clone()), admin(1), Path(("1".into(), "11".into())))
        .await
        .unwrap();
    let left = holes::Entity::find()
        .filter(holes::Column::HostId.eq(11))
        .all(&s.db)
        .await
        .unwrap();
    assert!(left.is_empty());
}

#[test]
fn every_permission_and_message_is_translated() {
    for locale in crate::i18n::Locale::ALL {
        for key in [
            "permission.group.holes",
            "holes.invalid",
            "holes.changed",
            "holes.revoked",
            "holes.notActive",
            "holes.runnerUpgrade",
            "holes.connectionUnsupported",
            "holes.notFound",
            "holes.unavailable",
            "holes.busy",
            "holes.timeout",
            "holes.tooLarge",
        ] {
            let text = crate::i18n::render(locale, &Message::new(key));
            assert_ne!(text, key, "{key} missing for {locale:?}");
        }
    }
    for permission in ["holes.read", "holes.create", "holes.update", "holes.delete"] {
        assert!(
            crate::accounts::workspace::PERMISSIONS
                .iter()
                .any(|p| p.0 == permission)
        );
    }
}

// ---------------------------------------------------------- helpers

#[test]
fn tokens_never_reach_request_logs() {
    assert_eq!(
        redact("/holes/abc123/assets/x.js"),
        "/holes/***/assets/x.js"
    );
    assert_eq!(redact("/holes/abc123"), "/holes/***");
    assert_eq!(redact("/api/status"), "/api/status");
}

#[test]
fn redirects_from_the_local_service_stay_under_the_hole() {
    let prefix = "/holes/t";
    assert_eq!(
        proxy::location("http://localhost:5173/login?x=1", prefix, 5173).as_deref(),
        Some("/holes/t/login?x=1")
    );
    assert_eq!(
        proxy::location("http://127.0.0.1:5173", prefix, 5173).as_deref(),
        Some("/holes/t/")
    );
    assert_eq!(
        proxy::location("/next", prefix, 5173).as_deref(),
        Some("/holes/t/next")
    );
    assert_eq!(
        proxy::location("/holes/t/next", prefix, 5173).as_deref(),
        Some("/holes/t/next"),
        "an app configured with the base path is left as it is"
    );
    assert_eq!(proxy::location("https://example.com/", prefix, 5173), None);
    assert_eq!(proxy::location("//evil.example/", prefix, 5173), None);
    assert_eq!(
        proxy::location("http://localhost:9999/x", prefix, 5173),
        None
    );
    assert_eq!(proxy::location("relative", prefix, 5173), None);
}

#[test]
fn connection_limits_are_enforced_and_fully_released() {
    // Negative ids never collide with snowflake ids used by other tests.
    let host = -900;
    let mut held = Vec::new();
    for hole in [-901, -902, -903, -904] {
        for _ in 0..32 {
            held.push(proxy::tests_hold(hole, host).expect("under the limit"));
        }
        assert!(
            proxy::tests_hold(hole, host).is_none(),
            "per-hole limit is 32"
        );
    }
    assert!(
        proxy::tests_hold(-905, host).is_none(),
        "per-host limit is 128"
    );
    assert!(
        proxy::tests_hold(-905, -906).is_some(),
        "other hosts unaffected"
    );
    drop(held);
    for hole in [-901, -902, -903, -904, -905] {
        assert_eq!(proxy::active(hole), 0);
    }
    assert!(proxy::tests_hold(-901, host).is_some());
}

#[tokio::test]
async fn state_handles_outlive_a_fresh_request_but_not_idleness() {
    let (_dir, s) = setup().await;
    tokio::time::pause();
    // A handle a request has just registered survives a cleanup tick...
    proxy::tests_register(-950, &s);
    proxy::forget_stale();
    assert!(proxy::tests_registered(-950));
    // ...and is dropped once idle with no live connection.
    tokio::time::advance(Duration::from_secs(5)).await;
    proxy::forget_stale();
    assert!(!proxy::tests_registered(-950));
    // A handle with a live connection stays however long it is idle.
    proxy::tests_register(-951, &s);
    let _held = proxy::tests_hold(-951, -952).unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    proxy::forget_stale();
    assert!(proxy::tests_registered(-951));
}

// ------------------------------------------------------------ proxy

/// A local service that reports what it received and returns every header
/// the proxy must strip.
async fn upstream() -> u16 {
    let app = axum::Router::new().fallback(|mut request: Request| async move {
        let path = request.uri().path().to_owned();
        let host = request
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if path.ends_with("/redirect") {
            return Response::builder()
                .status(302)
                .header("location", format!("http://{host}/target"))
                .body(axum::body::Body::empty())
                .unwrap();
        }
        if path.ends_with("/upgrade") {
            let upgrade = hyper::upgrade::on(&mut request);
            tokio::spawn(async move {
                if let Ok(io) = upgrade.await {
                    let mut io = hyper_util::rt::TokioIo::new(io);
                    let mut buffer = [0u8; 1024];
                    while let Ok(n) = io.read(&mut buffer).await {
                        if n == 0 || io.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            });
            return Response::builder()
                .status(101)
                .header("connection", "upgrade")
                .header("upgrade", "echo")
                .body(axum::body::Body::empty())
                .unwrap();
        }
        let headers = request
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
            .collect::<HashMap<_, _>>();
        let method = request.method().to_string();
        let query = request.uri().query().unwrap_or("").to_owned();
        let body = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            [
                ("set-cookie", "upstream=1; Path=/"),
                ("service-worker-allowed", "/"),
                ("clear-site-data", "\"*\""),
                ("content-security-policy", "default-src *"),
                ("access-control-allow-origin", "https://other.example"),
                ("access-control-allow-credentials", "true"),
            ],
            axum::Json(json!({"method":method,"path":path,"query":query,
                              "headers":headers,"body":body.len()})),
        )
            .into_response()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

async fn server(s: AppState) -> SocketAddr {
    let allowed = vec!["127.0.0.1".to_owned()];
    let app = axum::Router::new()
        .route("/api/runners/connect", get(crate::hosts::runner::connect))
        .route(
            "/api/runners/sessions/{session}",
            get(crate::hosts::runner::session),
        )
        .route("/holes/{token}", any(serve_root))
        .route("/holes/{token}/", any(serve))
        .route("/holes/{token}/{*rest}", any(serve))
        .route("/other", post(|| async { "ok" }))
        .layer(middleware::from_fn(
            move |request: Request, next: middleware::Next| {
                let allowed = allowed.clone();
                async move { crate::guard(request, next, &allowed).await }
            },
        ))
        .layer(middleware::from_fn(crate::i18n::middleware))
        .with_state(s);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

async fn ws_send<T: serde::Serialize>(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    frame: &T,
) {
    socket
        .send(WsMessage::Text(
            serde_json::to_string(frame).unwrap().into(),
        ))
        .await
        .unwrap();
}

/// A Runner that serves `Forward` sessions exactly as the real one does.
async fn fake_runner(s: &AppState, address: SocketAddr, host: i64, key: &str, forward: bool) {
    let (mut control, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/runners/connect"))
            .await
            .unwrap();
    ws_send(
        &mut control,
        &wire::Hello {
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
            supports_forward: forward,
        },
    )
    .await;
    let key = key.to_owned();
    tokio::spawn(async move {
        while let Some(Ok(message)) = control.next().await {
            let WsMessage::Text(text) = message else {
                continue;
            };
            match serde_json::from_str::<wire::Control>(&text) {
                Ok(wire::Control::OpenSession { session, .. }) => {
                    tokio::spawn(forward_session(address, session, host, key.clone()));
                }
                Ok(wire::Control::Ping) => ws_send(&mut control, &wire::Report::Pong).await,
                _ => {}
            }
        }
    });
    for _ in 0..100 {
        let row = hosts::Entity::find_by_id(host)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        if row.state == "online" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fake runner never came online");
}

async fn forward_session(address: SocketAddr, session: String, host: i64, key: String) {
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/runners/sessions/{session}"))
            .await
            .unwrap();
    ws_send(
        &mut socket,
        &wire::SessionHello {
            version: wire::VERSION,
            host_id: host.to_string(),
            key,
        },
    )
    .await;
    let Some(Ok(WsMessage::Text(text))) = socket.next().await else {
        return;
    };
    let Ok(wire::Session::ForwardStart { port }) = serde_json::from_str(&text) else {
        return;
    };
    let tcp = match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
        Ok(tcp) => tcp,
        Err(error) => {
            ws_send(
                &mut socket,
                &wire::Session::Failed {
                    error: format!("无法连接本机端口 {port}：{error}"),
                },
            )
            .await;
            return;
        }
    };
    ws_send(&mut socket, &wire::Session::ForwardReady).await;
    let (mut reader, mut writer) = tcp.into_split();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => if socket.send(WsMessage::Binary(buffer[..n].to_vec().into())).await.is_err() { break },
            },
            message = socket.next() => match message {
                Some(Ok(WsMessage::Binary(bytes))) => if writer.write_all(&bytes).await.is_err() { break },
                Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {}
                _ => break,
            },
        }
    }
    let _ = socket.close(None).await;
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn until(what: &str, mut check: impl AsyncFnMut() -> bool) {
    for _ in 0..100 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Sends a raw HTTP/1.1 upgrade and returns the stream once `101` arrives.
async fn upgraded(address: SocketAddr, path: &str) -> tokio::net::TcpStream {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: upgrade\r\nUpgrade: echo\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.to_ascii_lowercase().contains(SANDBOX), "{head}");
    stream
}

async fn closed_within(stream: &mut tokio::net::TcpStream, limit: Duration) {
    let mut buffer = [0u8; 16];
    let read = tokio::time::timeout(limit, stream.read(&mut buffer))
        .await
        .expect("connection was not closed in time");
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_isolates_the_console_and_releases_every_connection() {
    let (_dir, s) = own_ids(setup().await);
    add_host(&s, 21, 1, KEY).await; // online, supports forward
    add_host(&s, 22, 1, "never-connects").await; // offline
    add_host(&s, 23, 1, OLD_KEY).await; // online, predates forward
    let address = server(s.clone()).await;
    fake_runner(&s, address, 21, KEY, true).await;
    fake_runner(&s, address, 23, OLD_KEY, false).await;
    let port = upstream().await;
    let client = http();
    let base = format!("http://{address}");

    let hole = create_hole(&s, 21, port).await;
    let id = i64::from(hole_id(&hole));
    let path = hole["url"].as_str().unwrap().to_owned();
    let prefix = path.trim_end_matches('/').to_owned();

    // The root without its slash redirects so relative URLs resolve.
    let root = client
        .get(format!("{base}{prefix}?a=1"))
        .send()
        .await
        .unwrap();
    assert_eq!(root.status(), 308);
    assert_eq!(root.headers()["location"], format!("{prefix}/?a=1"));

    // A request from a sandboxed preview: console cookie, Origin null, no
    // console header, and a body.
    let response = client
        .post(format!("{base}{path}api/items?x=1"))
        .header("cookie", "ferry_session=console-secret")
        .header("origin", "null")
        .body("payload")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let headers = response.headers().clone();
    assert_eq!(headers["content-security-policy"], SANDBOX);
    assert_eq!(headers["access-control-allow-origin"], "*");
    for stripped in [
        "set-cookie",
        "service-worker-allowed",
        "clear-site-data",
        "access-control-allow-credentials",
    ] {
        assert!(!headers.contains_key(stripped), "{stripped} leaked");
    }
    let seen: Value = response.json().await.unwrap();
    assert_eq!(seen["method"], "POST");
    assert_eq!(seen["path"], "/api/items", "prefix stripped by default");
    assert_eq!(seen["query"], "x=1");
    assert_eq!(seen["body"], 7);
    let upstream_headers = seen["headers"].as_object().unwrap();
    assert!(
        !upstream_headers.contains_key("cookie"),
        "console cookie reached the previewed service"
    );
    assert!(!upstream_headers.contains_key("x-ferry-request"));
    assert_eq!(upstream_headers["host"], format!("localhost:{port}"));
    assert_eq!(
        upstream_headers["origin"],
        format!("http://localhost:{port}")
    );
    assert_eq!(upstream_headers["x-forwarded-prefix"], prefix);
    assert_eq!(upstream_headers["x-forwarded-host"], address.to_string());

    // The exemption is scoped to holes: the console still demands its header.
    let other = client.post(format!("{base}/other")).send().await.unwrap();
    assert_eq!(other.status(), 403);

    // Redirects from the local origin stay under the hole.
    let redirect = client
        .get(format!("{base}{path}redirect"))
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.status(), 302);
    assert_eq!(redirect.headers()["location"], format!("{prefix}/target"));

    // Preflights are answered here, never forwarded.
    let preflight = client
        .request(reqwest::Method::OPTIONS, format!("{base}{path}api"))
        .header("origin", "null")
        .header("access-control-request-method", "PUT")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 204);
    assert_eq!(preflight.headers()["access-control-allow-origin"], "*");
    assert_eq!(preflight.headers()["access-control-allow-methods"], "PUT");
    assert_eq!(
        preflight.headers()["access-control-allow-headers"],
        "content-type"
    );

    // Keep-prefix mode forwards the full path.
    holes::Entity::update_many()
        .col_expr(
            holes::Column::StripPrefix,
            sea_orm::sea_query::Expr::value(false),
        )
        .filter(holes::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let kept: Value = client
        .get(format!("{base}{path}assets/app.js"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(kept["path"], format!("{path}assets/app.js"));

    // Oversized bodies are refused before anything is forwarded.
    let mut raw = tokio::net::TcpStream::connect(address).await.unwrap();
    raw.write_all(
        format!(
            "POST {path}upload HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n",
            64 * 1024 * 1024 + 1
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut reply = [0u8; 12];
    raw.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"HTTP/1.1 413");
    drop(raw);

    // Traffic recorded the service as reachable.
    let row = holes::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.probe_state, "reachable");
    assert!(proxy::active(id) > 0, "pooled connection is counted");

    // Idle pooled connections are released: nothing leaks after traffic stops.
    until("idle connections to be released", async || {
        proxy::active(id) == 0
    })
    .await;

    // Upgrades (WebSocket and friends) are relayed both ways...
    let mut stream = upgraded(address, &format!("{path}upgrade")).await;
    stream.write_all(b"hello").await.unwrap();
    let mut echoed = [0u8; 5];
    stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"hello");
    assert_eq!(proxy::active(id), 1);
    // ...and cut as soon as the hole is revoked.
    revoke(State(s.clone()), admin(1), Path(("1".into(), Id::from(id))))
        .await
        .unwrap();
    closed_within(&mut stream, Duration::from_secs(5)).await;
    until("revoked connections to be released", async || {
        proxy::active(id) == 0
    })
    .await;

    // Unknown, revoked and expired links are indistinguishable.
    let body_of = |url: String| {
        let client = client.clone();
        async move {
            let response = client.get(url).send().await.unwrap();
            assert_eq!(response.status(), 404);
            assert_eq!(response.headers()["content-security-policy"], SANDBOX);
            response.text().await.unwrap()
        }
    };
    let revoked_body = body_of(format!("{base}{path}")).await;
    let unknown_body = body_of(format!("{base}/holes/{}/", "0".repeat(64))).await;
    let expired = create_hole(&s, 21, port).await;
    let expired_id = i64::from(hole_id(&expired));
    let expired_path = expired["url"].as_str().unwrap().to_owned();
    // An expiry noticed by the periodic re-check, not by a revoke: covers
    // other replicas, which never see the local close signal.
    let mut stream = upgraded(address, &format!("{expired_path}upgrade")).await;
    holes::Entity::update_many()
        .col_expr(
            holes::Column::ExpiresAt,
            sea_orm::sea_query::Expr::value(1_i64),
        )
        .filter(holes::Column::Id.eq(expired_id))
        .exec(&s.db)
        .await
        .unwrap();
    closed_within(&mut stream, Duration::from_secs(5)).await;
    until("expired connections to be released", async || {
        proxy::active(expired_id) == 0
    })
    .await;
    let expired_body = body_of(format!("{base}{expired_path}")).await;
    assert_eq!(revoked_body, unknown_body);
    assert_eq!(expired_body, unknown_body);

    // Failures are reported to the visitor and recorded for the operator.
    let cases = [
        (22, port, "runner_offline"),
        (23, port, "unsupported"),
        (21, closed_port().await, "unreachable"),
    ];
    for (host, target, expected) in cases {
        let hole = create_hole(&s, host, target).await;
        let id = i64::from(hole_id(&hole));
        let response = client
            .get(format!("{base}{}", hole["url"].as_str().unwrap()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502, "{expected}");
        let row = holes::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.probe_state, expected);
        if expected == "unreachable" {
            assert!(
                row.probe_error.contains(&target.to_string()),
                "{}",
                row.probe_error
            );
        }
        assert_eq!(proxy::active(id), 0, "failed opens leave nothing behind");
    }

    // The explicit probe and the background round record reachability.
    let live = create_hole(&s, 21, port).await;
    let live_id = hole_id(&live);
    let probed = probe_now(State(s.clone()), admin(1), Path(("1".into(), live_id)))
        .await
        .unwrap()
        .0;
    assert_eq!(probed["probe_state"], "reachable");
    assert!(probed["probed_at"].as_i64().unwrap() > 0);
    holes::Entity::update_many()
        .col_expr(
            holes::Column::ProbeState,
            sea_orm::sea_query::Expr::value("unknown"),
        )
        .exec(&s.db)
        .await
        .unwrap();
    probe_round(&s, "test-owner").await.unwrap();
    let row = holes::Entity::find_by_id(i64::from(live_id))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.probe_state, "reachable");
    assert_eq!(
        proxy::active(i64::from(live_id)),
        0,
        "probes hold no connection"
    );
}

/// The shared fixture always uses snowflake node 1, so two tests creating a
/// hole in the same millisecond get the same id. The proxy keeps per-hole
/// state process-wide (as it must in production, where ids are unique), so
/// this test draws its ids from another node.
fn own_ids((dir, s): (tempfile::TempDir, AppState)) -> (tempfile::TempDir, AppState) {
    let state = std::sync::Arc::new(crate::state::State {
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(9, tokio::time::Instant::now().into_std()).unwrap(),
        ),
        db: s.db.clone(),
        data_dir: s.data_dir.clone(),
        instance_id: "holes-proxy-test".into(),
        server_mode: s.server_mode,
        secure_cookie: s.secure_cookie,
        bind: s.bind,
        vault: s.vault.clone(),
        cluster: None,
        host_monitor: Default::default(),
        monitor: Default::default(),
    });
    (dir, state)
}

async fn closed_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}
