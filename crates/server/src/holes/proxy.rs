//! The public `/holes/{token}/...` reverse proxy (design sections 5, 6, 8).
//!
//! hyper's pooled client speaks HTTP/1.1; its connector opens a Runner
//! `Forward` session per upstream connection. Each connection is a `pump`
//! task between the session and an in-memory duplex: the pump is the single
//! place that counts the connection, re-checks the hole, and closes both
//! sides, so nothing it owns can outlive it.

use super::{bounded, holes, state};
use crate::{
    cluster::Socket,
    db,
    error::ApiError,
    i18n::Message,
    snowflake::Id,
    state::{AppState, State as AppStateInner},
};
use axum::{
    body::Body,
    extract::{Path, Request, State, ws::Message as Frame},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, sea_query::Expr};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{LazyLock, Mutex, MutexGuard, Weak},
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::watch;

const MAX_PER_HOLE: usize = 32;
const MAX_PER_HOST: usize = 128;
const BODY_LIMIT: usize = 64 * 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(test))]
const POOL_IDLE: Duration = Duration::from_secs(30);
#[cfg(test)]
const POOL_IDLE: Duration = Duration::from_millis(500);
const CHUNK: usize = 64 * 1024;
#[cfg(not(test))]
const REVALIDATE: Duration = Duration::from_secs(30);
#[cfg(test)]
const REVALIDATE: Duration = Duration::from_millis(300);
/// No `allow-same-origin`: the preview gets an opaque origin, so it cannot
/// reach the console's cookies, storage, DOM or service workers.
pub(super) const SANDBOX: &str =
    "sandbox allow-scripts allow-forms allow-popups allow-modals allow-downloads";

// ------------------------------------------------------------- target

/// Everything a connection needs, carried in the pool key so the connector
/// stays stateless and the pool never mixes two holes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    hole: i64,
    workspace: i64,
    host: i64,
    port: u16,
}

impl Target {
    fn of(row: &holes::Model) -> Option<Self> {
        Some(Self {
            hole: row.id,
            workspace: row.workspace_id,
            host: row.host_id,
            port: u16::try_from(row.target_port).ok().filter(|p| *p != 0)?,
        })
    }
    fn authority(&self) -> String {
        format!(
            "h{}-{}-{}-{}.invalid",
            self.hole, self.workspace, self.host, self.port
        )
    }
    fn parse(uri: &Uri) -> Option<Self> {
        let host = uri.host()?.strip_prefix('h')?.strip_suffix(".invalid")?;
        let mut parts = host.split('-');
        let target = Self {
            hole: parts.next()?.parse().ok()?,
            workspace: parts.next()?.parse().ok()?,
            host: parts.next()?.parse().ok()?,
            port: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(target)
    }
}

/// The state a hole's connections use, and when a request last needed it.
/// Weak, so a test's state is not kept alive by the process-wide pool.
type Handle = (Weak<AppStateInner>, tokio::time::Instant);
static STATES: LazyLock<Mutex<HashMap<i64, Handle>>> = LazyLock::new(Default::default);

fn states() -> MutexGuard<'static, HashMap<i64, Handle>> {
    STATES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Drops handles nothing has used for a while and nothing still holds open;
/// called from the monitor so the map cannot grow without bound. The grace
/// period keeps a handle a request has only just registered.
pub(super) fn forget_stale() {
    let live = registry();
    let now = tokio::time::Instant::now();
    states().retain(|hole, (state, used)| {
        state.strong_count() > 0
            && (live.holes.contains_key(hole) || now.duration_since(*used) < POOL_IDLE * 2)
    });
}

// ----------------------------------------------------------- registry

#[derive(Default)]
struct Registry {
    holes: HashMap<i64, (usize, watch::Sender<bool>)>,
    hosts: HashMap<i64, usize>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Default::default);

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One counted connection. Dropping it is the only way the count goes down,
/// so every exit path of a pump releases it.
struct Held {
    hole: i64,
    host: i64,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut registry = registry();
        if let Some(entry) = registry.holes.get_mut(&self.hole) {
            entry.0 -= 1;
            if entry.0 == 0 {
                registry.holes.remove(&self.hole);
            }
        }
        if let Some(count) = registry.hosts.get_mut(&self.host) {
            *count -= 1;
            if *count == 0 {
                registry.hosts.remove(&self.host);
            }
        }
    }
}

fn hold(target: &Target) -> Option<(Held, watch::Receiver<bool>)> {
    let mut registry = registry();
    let hole_count = registry.holes.get(&target.hole).map_or(0, |entry| entry.0);
    let host_count = registry.hosts.get(&target.host).copied().unwrap_or(0);
    if hole_count >= MAX_PER_HOLE || host_count >= MAX_PER_HOST {
        return None;
    }
    let entry = registry
        .holes
        .entry(target.hole)
        .or_insert_with(|| (0, watch::channel(false).0));
    entry.0 += 1;
    let closing = entry.1.subscribe();
    *registry.hosts.entry(target.host).or_insert(0) += 1;
    Some((
        Held {
            hole: target.hole,
            host: target.host,
        },
        closing,
    ))
}

/// Live upstream connections this replica holds for a hole.
pub(super) fn active(hole: i64) -> usize {
    registry().holes.get(&hole).map_or(0, |entry| entry.0)
}

/// Cuts every live connection of a hole on this replica now. Other replicas
/// notice within `REVALIDATE`.
pub(super) fn close(hole: i64) {
    if let Some(entry) = registry().holes.get(&hole) {
        entry.1.send_replace(true);
    }
    states().remove(&hole);
}

#[cfg(test)]
pub(super) fn tests_register(hole: i64, s: &AppState) {
    states().insert(
        hole,
        (std::sync::Arc::downgrade(s), tokio::time::Instant::now()),
    );
}

#[cfg(test)]
pub(super) fn tests_registered(hole: i64) -> bool {
    states().contains_key(&hole)
}

#[cfg(test)]
pub(super) fn tests_hold(hole: i64, host: i64) -> Option<impl Drop> {
    hold(&Target {
        hole,
        workspace: 0,
        host,
        port: 1,
    })
    .map(|(held, _)| held)
}

// ------------------------------------------------------- probe & open

/// Maps a failure to open a forward onto the stored probe state.
fn classify(error: &ApiError) -> (&'static str, String) {
    let key = error.2.as_ref().map(|message| message.key.as_str());
    if matches!(
        key,
        Some("holes.runnerUpgrade" | "holes.connectionUnsupported")
    ) {
        ("unsupported", String::new())
    } else if error.0 == StatusCode::SERVICE_UNAVAILABLE {
        ("runner_offline", String::new())
    } else {
        ("unreachable", stored_error(error))
    }
}

/// The error as a stored, viewer-localized message that fits the column.
pub(super) fn stored_error(error: &ApiError) -> String {
    let stored = match &error.2 {
        Some(message) => crate::i18n::store_message(message),
        None => crate::i18n::store_raw(&error.to_string()),
    };
    if stored.chars().count() <= super::ERROR_LIMIT {
        stored
    } else {
        crate::i18n::store_raw(
            &bounded(&error.to_string())
                .chars()
                .take(400)
                .collect::<String>(),
        )
    }
}

/// Writes the probe outcome. A passive update (from real traffic) only writes
/// when the outcome changed, so serving requests does not write every time.
async fn record(s: &AppStateInner, hole: i64, outcome: (&str, String), always: bool) {
    let (probe_state, probe_error) = outcome;
    let mut update = holes::Entity::update_many()
        .col_expr(holes::Column::ProbeState, Expr::value(probe_state))
        .col_expr(holes::Column::ProbeError, Expr::value(probe_error.clone()))
        .col_expr(holes::Column::ProbedAt, Expr::value(db::now()))
        .col_expr(holes::Column::ProbedBy, Expr::value(0_i64))
        .filter(holes::Column::Id.eq(hole));
    if !always {
        update = update.filter(
            Condition::any()
                .add(holes::Column::ProbeState.ne(probe_state))
                .add(holes::Column::ProbeError.ne(probe_error)),
        );
    }
    if let Err(error) = update.exec(&s.db).await {
        tracing::warn!(%error, hole, "预览穿透探测结果写入失败");
    }
}

/// Opens one forward session. The channel, and with it the Runner upgrade
/// reservation, is dropped as soon as the session exists.
async fn open(s: &AppState, target: &Target) -> Result<Socket, ApiError> {
    let channel =
        crate::hosts::remote::open(s, &Id::from(target.workspace), &Id::from(target.host)).await?;
    channel.forward(target.port).await
}

/// Checks that the local service accepts a connection and records the result.
pub(super) async fn probe(s: &AppState, row: &holes::Model) {
    let outcome = match reach(s, row).await {
        Ok(()) => ("reachable", String::new()),
        Err(error) => classify(&error),
    };
    record(s, row.id, outcome, true).await;
}

/// Opens and immediately closes one forward: the local service accepts.
pub(super) async fn reach(s: &AppState, row: &holes::Model) -> Result<(), ApiError> {
    let target = Target::of(row).ok_or_else(ApiError::missing)?;
    let mut socket = open(s, &target).await?;
    let _ = socket.send(Frame::Close(None)).await;
    Ok(())
}

/// Records the outcome of a readiness check the same way a probe does.
pub(super) async fn record_reach(s: &AppState, hole: i64, outcome: &Result<(), ApiError>) {
    let outcome = match outcome {
        Ok(()) => ("reachable", String::new()),
        Err(error) => classify(error),
    };
    record(s, hole, outcome, true).await;
}

// --------------------------------------------------------- connection

/// Why the connector could not produce a connection.
#[derive(Debug)]
enum Refused {
    Busy,
    Unavailable,
}
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "preview connection limit reached",
            Self::Unavailable => "preview upstream unavailable",
        })
    }
}
impl std::error::Error for Refused {}

/// The upstream end of a pump, as hyper sees it.
struct Pipe(TokioIo<DuplexStream>);

impl Connection for Pipe {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}
impl hyper::rt::Read for Pipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}
impl hyper::rt::Write for Pipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[derive(Clone)]
struct Connector;

impl tower_service::Service<Uri> for Connector {
    type Response = Pipe;
    type Error = Refused;
    type Future = Pin<Box<dyn Future<Output = Result<Pipe, Refused>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Refused>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(connect(uri))
    }
}

async fn connect(uri: Uri) -> Result<Pipe, Refused> {
    let target = Target::parse(&uri).ok_or(Refused::Unavailable)?;
    let s = states()
        .get(&target.hole)
        .and_then(|(state, _)| state.upgrade())
        .ok_or(Refused::Unavailable)?;
    let (held, closing) = hold(&target).ok_or(Refused::Busy)?;
    let socket = match open(&s, &target).await {
        Ok(socket) => socket,
        Err(error) => {
            record(&s, target.hole, classify(&error), false).await;
            return Err(Refused::Unavailable);
        }
    };
    record(&s, target.hole, ("reachable", String::new()), false).await;
    let (near, far) = tokio::io::duplex(CHUNK * 2);
    tokio::spawn(pump(s, target.hole, socket, far, held, closing));
    Ok(Pipe(TokioIo::new(near)))
}

/// Still servable: exists, not revoked, not expired. A database error keeps
/// the connection; the next check decides.
async fn valid(s: &AppState, hole: i64, domain: &str) -> bool {
    if let Ok(config) = crate::accounts::addresses::Configuration::load(s, &s.db).await
        && (config.preview_domain.is_empty() || config.preview_domain != domain)
    {
        return false;
    }
    match holes::Entity::find_by_id(hole).one(&s.db).await {
        Ok(Some(row)) => state(&row, db::now()) == "active",
        Ok(None) => false,
        Err(_) => true,
    }
}

async fn pump(
    s: AppState,
    hole: i64,
    mut socket: Socket,
    io: DuplexStream,
    _held: Held,
    mut closing: watch::Receiver<bool>,
) {
    let (mut reader, mut writer) = tokio::io::split(io);
    let domain = crate::accounts::addresses::Configuration::load(&s, &s.db)
        .await
        .map(|c| c.preview_domain)
        .unwrap_or_default();
    let mut buffer = vec![0u8; CHUNK];
    let mut check = tokio::time::interval_at(tokio::time::Instant::now() + REVALIDATE, REVALIDATE);
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let frame = Frame::Binary(bytes::Bytes::copy_from_slice(&buffer[..n]));
                    if socket.send(frame).await.is_err() { break }
                }
            },
            frame = socket.recv() => match frame {
                Some(Ok(Frame::Binary(bytes))) => {
                    if writer.write_all(&bytes).await.is_err() { break }
                }
                Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => {}
                // Text after `ForwardReady` can only be `Failed`; close either way.
                _ => break,
            },
            _ = closing.changed() => break,
            _ = check.tick() => if !valid(&s, hole, &domain).await { break },
        }
    }
    let _ = writer.shutdown().await;
    let _ = socket.send(Frame::Close(None)).await;
}

// ------------------------------------------------------------- client

type Client = hyper_util::client::legacy::Client<Connector, http_body_util::Limited<Body>>;

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    hyper_util::client::legacy::Client::builder(TokioExecutor::new())
        .pool_idle_timeout(POOL_IDLE)
        .pool_timer(TokioTimer::new())
        .pool_max_idle_per_host(8)
        .build(Connector)
});

// -------------------------------------------------------------- routes

fn refuse(status: StatusCode, key: &str) -> Response {
    let text = crate::i18n::render(crate::i18n::current_locale(), &Message::new(key));
    let mut response = (status, text).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(SANDBOX),
    );
    response
}

pub(super) fn not_found() -> Response {
    refuse(StatusCode::NOT_FOUND, "holes.notFound")
}

/// Request logs never carry a hole token.
pub fn redact(path: &str) -> std::borrow::Cow<'_, str> {
    match path.strip_prefix("/holes/") {
        Some(rest) => {
            let tail = rest.find('/').map_or("", |at| &rest[at..]);
            format!("/holes/***{tail}").into()
        }
        None => path.into(),
    }
}

/// `/holes/{token}`: relative URLs only resolve under the trailing slash.
pub async fn serve_root(
    State(s): State<AppState>,
    Path(_token): Path<String>,
    request: Request,
) -> Response {
    serve(State(s), request).await
}

/// `/holes/{token}/` and everything below it.
pub async fn serve(State(s): State<AppState>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let Some(rest) = path.strip_prefix("/holes/") else {
        return not_found();
    };
    let (token, tail) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let row = match holes::Entity::find()
        .filter(holes::Column::TokenHash.eq(crate::hosts::runner::fingerprint(token)))
        .one(&s.db)
        .await
    {
        Ok(Some(row)) if state(&row, db::now()) == "active" => row,
        _ => return not_found(),
    };
    let url = match super::link(&s, &row).await {
        Ok(url) if !url.is_empty() => url,
        _ => return not_found(),
    };
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let location = format!(
        "{}{}{query}",
        url.trim_end_matches('/'),
        if tail.is_empty() { "/" } else { tail }
    );
    let Ok(location) = HeaderValue::from_str(&location) else {
        return not_found();
    };
    let mut response = StatusCode::PERMANENT_REDIRECT.into_response();
    response.headers_mut().insert(header::LOCATION, location);
    response
}

pub(super) async fn serve_domain(s: AppState, request: Request, token: String) -> Response {
    serve_entry(s, request, token, "domain").await
}

pub(super) async fn serve_entry(
    s: AppState,
    request: Request,
    token: String,
    expected: &str,
) -> Response {
    let path = request.uri().path().to_owned();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return not_found();
    }
    let row = match holes::Entity::find()
        .filter(holes::Column::TokenHash.eq(crate::hosts::runner::fingerprint(&token)))
        .one(&s.db)
        .await
    {
        Ok(Some(row)) if state(&row, db::now()) == "active" => row,
        // Unknown, expired and revoked are indistinguishable from outside.
        Ok(_) => return not_found(),
        Err(error) => {
            tracing::warn!(%error, "预览穿透查询失败");
            return refuse(StatusCode::BAD_GATEWAY, "holes.unavailable");
        }
    };
    match super::ingress::Entry::read(&s, &row) {
        Ok(Some(entry)) if entry.mode != expected => return not_found(),
        Ok(None) if expected != "domain" => return not_found(),
        Err(_) => return not_found(),
        _ => {}
    }
    let Some(target) = Target::of(&row) else {
        return not_found();
    };
    if request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
    {
        return preflight(request.headers());
    }
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_some_and(|length| length > BODY_LIMIT) {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "holes.tooLarge");
    }
    states().insert(
        target.hole,
        (std::sync::Arc::downgrade(&s), tokio::time::Instant::now()),
    );
    let prefix = String::new();
    let forwarded_path = path;
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let Ok(uri) = format!("http://{}{forwarded_path}{query}", target.authority()).parse::<Uri>()
    else {
        return not_found();
    };
    let external_host = request
        .headers()
        .get(header::HOST)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static(""));
    let (mut parts, body) = request.into_parts();
    let upgrade = upgrade_requested(&parts.headers);
    // Taken before the parts go upstream; hyper resolves it after our 101.
    let client_upgrade = if upgrade {
        parts.extensions.remove::<hyper::upgrade::OnUpgrade>()
    } else {
        None
    };
    rewrite_request(
        &mut parts.headers,
        target.port,
        &external_host,
        &prefix,
        upgrade,
    );
    parts.uri = uri;
    let upstream_request =
        Request::from_parts(parts, http_body_util::Limited::new(body, BODY_LIMIT));
    let mut response =
        match tokio::time::timeout(RESPONSE_TIMEOUT, CLIENT.request(upstream_request)).await {
            Err(_) => return refuse(StatusCode::GATEWAY_TIMEOUT, "holes.timeout"),
            Ok(Err(error)) => return failure(&error),
            Ok(Ok(response)) => response,
        };
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        match client_upgrade {
            Some(client_upgrade) => {
                tokio::spawn(splice(client_upgrade, upstream_upgrade));
            }
            // A 101 nobody asked for cannot be relayed.
            None => return refuse(StatusCode::BAD_GATEWAY, "holes.unavailable"),
        }
    }
    let (mut head, body) = response.into_parts();
    rewrite_response(&mut head.headers, &prefix, target.port);
    Response::from_parts(head, Body::new(body))
}

/// Joins an upgraded browser connection to the upgraded upstream one. Both
/// halves are already bounded by the pump behind the upstream side.
async fn splice(client: hyper::upgrade::OnUpgrade, upstream: hyper::upgrade::OnUpgrade) {
    let (Ok(client), Ok(upstream)) = tokio::join!(client, upstream) else {
        return;
    };
    let mut client = TokioIo::new(client);
    let mut upstream = TokioIo::new(upstream);
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

fn upgrade_requested(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE)
        && headers
            .get_all(header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
}

fn failure(error: &hyper_util::client::legacy::Error) -> Response {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(refused) = current.downcast_ref::<Refused>() {
            return match refused {
                Refused::Busy => refuse(StatusCode::SERVICE_UNAVAILABLE, "holes.busy"),
                Refused::Unavailable => refuse(StatusCode::BAD_GATEWAY, "holes.unavailable"),
            };
        }
        if current
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
        {
            return refuse(StatusCode::PAYLOAD_TOO_LARGE, "holes.tooLarge");
        }
        source = current.source();
    }
    refuse(StatusCode::BAD_GATEWAY, "holes.unavailable")
}

fn preflight(request: &HeaderMap) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    for (from, to) in [
        (
            header::ACCESS_CONTROL_REQUEST_METHOD,
            header::ACCESS_CONTROL_ALLOW_METHODS,
        ),
        (
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            header::ACCESS_CONTROL_ALLOW_HEADERS,
        ),
    ] {
        if let Some(value) = request.get(from) {
            headers.insert(to, value.clone());
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(SANDBOX),
    );
    response
}

const HOP_BY_HOP: [&str; 6] = [
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
];

/// Section 5: what the local service must and must not see.
fn rewrite_request(
    headers: &mut HeaderMap,
    port: u16,
    external_host: &HeaderValue,
    prefix: &str,
    upgrade: bool,
) {
    // The console session cookie must never reach the previewed service.
    headers.remove(header::COOKIE);
    headers.remove("x-ferry-request");
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
    if !upgrade {
        headers.remove(header::CONNECTION);
        headers.remove(header::UPGRADE);
    }
    let local = format!("localhost:{port}");
    if let Ok(value) = HeaderValue::from_str(&local) {
        headers.insert(header::HOST, value);
    }
    if headers.contains_key(header::ORIGIN)
        && let Ok(value) = HeaderValue::from_str(&format!("http://{local}"))
    {
        headers.insert(header::ORIGIN, value);
    }
    headers.remove(header::REFERER);
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        external_host.clone(),
    );
    headers.remove("x-forwarded-prefix");
    if !prefix.is_empty()
        && let Ok(value) = HeaderValue::from_str(prefix)
    {
        headers.insert(HeaderName::from_static("x-forwarded-prefix"), value);
    }
}

/// Section 6: nothing the local service returns may act on the console origin.
fn rewrite_response(headers: &mut HeaderMap, prefix: &str, port: u16) {
    for name in [
        header::SET_COOKIE,
        header::CONTENT_SECURITY_POLICY,
        header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
    ] {
        headers.remove(name);
    }
    headers.remove("service-worker-allowed");
    headers.remove("clear-site-data");
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(SANDBOX),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    if let Some(location) = headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| location(value, prefix, port))
        && let Ok(value) = HeaderValue::from_str(&location)
    {
        headers.insert(header::LOCATION, value);
    }
}

/// Points a redirect from the local service back under the hole's prefix.
/// Anything that is not the local origin or a root-relative path is left alone.
pub(super) fn location(value: &str, prefix: &str, port: u16) -> Option<String> {
    let local = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ];
    let path = if let Some(rest) = local
        .iter()
        .find_map(|origin| value.strip_prefix(origin.as_str()))
    {
        if rest.is_empty() {
            "/".to_owned()
        } else if rest.starts_with('/') || rest.starts_with('?') {
            if rest.starts_with('?') {
                format!("/{rest}")
            } else {
                rest.to_owned()
            }
        } else {
            return None;
        }
    } else if value.starts_with('/') && !value.starts_with("//") {
        value.to_owned()
    } else {
        return None;
    };
    if path == prefix || path.starts_with(&format!("{prefix}/")) {
        Some(path)
    } else {
        Some(format!("{prefix}{path}"))
    }
}
