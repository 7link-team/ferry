//! The public `/holes/{token}/...` reverse proxy (design sections 5, 6, 8).
//!
//! hyper's pooled client speaks HTTP/1.1; its connector opens a Runner
//! `Forward` session per upstream connection. Each connection is a `pump`
//! task between the session and an in-memory duplex: the pump is the single
//! place that counts the connection, re-checks the hole, and closes both
//! sides, so nothing it owns can outlive it.

use super::tls::{self, Tls};
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
    sync::{Arc, LazyLock, Mutex, MutexGuard, Weak},
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::{Notify, watch};

#[cfg(test)]
const MAX_PER_HOLE: usize = 32;
#[cfg(test)]
const MAX_PER_HOST: usize = 128;
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
#[derive(Clone, Debug, PartialEq)]
struct Target {
    hole: i64,
    workspace: i64,
    host: i64,
    port: u16,
    /// Empty: the Runner's own loopback. Not carried in the pool authority;
    /// a connection reloads it from the hole, which never changes it.
    address: String,
    /// Plain HTTP or HTTPS (verified or not); part of the pool authority.
    tls: Tls,
}

/// Loopback or private-network addresses only; the Runner applies the same
/// rule to whatever a name resolves to.
pub(super) fn private_target(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback() || v4.is_private() || (a == 100 && (64..=127).contains(&b))
        }
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => private_target(std::net::IpAddr::V4(v4)),
            None => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        },
    }
}

/// The upstream authority the previewed service expects to see.
fn upstream_authority(address: &str, port: u16) -> String {
    if address.is_empty() {
        format!("localhost:{port}")
    } else if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

/// Runners before 0.5.0-alpha ignore the forward target and would silently
/// connect to their own loopback instead.
fn target_supported(agent_version: &str) -> bool {
    semver::Version::parse(agent_version.trim_start_matches('v'))
        .is_ok_and(|v| v >= semver::Version::new(0, 5, 0) || (v.major, v.minor) >= (0, 5))
}

impl Target {
    fn of(row: &holes::Model) -> Option<Self> {
        Some(Self {
            hole: row.id,
            workspace: row.workspace_id,
            host: row.host_id,
            port: u16::try_from(row.target_port).ok().filter(|p| *p != 0)?,
            address: row.target_host.clone(),
            tls: Tls::of(&row.target_scheme, row.target_tls_insecure),
        })
    }
    fn authority(&self) -> String {
        format!(
            "h{}-{}-{}-{}-{}.invalid",
            self.hole,
            self.workspace,
            self.host,
            self.port,
            self.tls.code()
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
            address: String::new(),
            tls: Tls::from_code(parts.next()?)?,
        };
        parts.next().is_none().then_some(target)
    }
}

/// The state a hole's connections use, and when a request last needed it.
/// Weak, so a test's state is not kept alive by the process-wide pool.
struct Handle {
    state: Weak<AppStateInner>,
    used: tokio::time::Instant,
    generation: Arc<()>,
}
static STATES: LazyLock<Mutex<HashMap<i64, Handle>>> = LazyLock::new(Default::default);

fn states() -> MutexGuard<'static, HashMap<i64, Handle>> {
    STATES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn remember_state(hole: i64, s: &AppState) {
    let mut states = states();
    let state = Arc::downgrade(s);
    let now = tokio::time::Instant::now();
    if let Some(handle) = states.get_mut(&hole)
        && handle.state.ptr_eq(&state)
    {
        handle.used = now;
    } else {
        states.insert(
            hole,
            Handle {
                state,
                used: now,
                generation: Arc::new(()),
            },
        );
    }
}

fn current_generation(hole: i64, generation: &Arc<()>) -> bool {
    states()
        .get(&hole)
        .is_some_and(|handle| Arc::ptr_eq(&handle.generation, generation))
}

/// Drops handles nothing has used for a while and nothing still holds open;
/// called from the monitor so the map cannot grow without bound. The grace
/// period keeps a handle a request has only just registered.
pub(super) fn forget_stale() {
    let live = registry();
    let now = tokio::time::Instant::now();
    states().retain(|hole, handle| {
        handle.state.strong_count() > 0
            && (live.holes.contains_key(hole)
                || Arc::strong_count(&handle.generation) > 1
                || now.duration_since(handle.used) < POOL_IDLE * 2)
    });
}

// ----------------------------------------------------------- registry

#[derive(Default)]
struct Registry {
    holes: HashMap<i64, (usize, watch::Sender<bool>)>,
    hosts: HashMap<i64, usize>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Default::default);
static CAPACITY_CHANGED: LazyLock<Notify> = LazyLock::new(Notify::new);

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
        drop(registry);
        CAPACITY_CHANGED.notify_waiters();
    }
}

fn hold_with_policy(
    target: &Target,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Option<(Held, watch::Receiver<bool>)> {
    let mut registry = registry();
    let hole_count = registry.holes.get(&target.hole).map_or(0, |entry| entry.0);
    let host_count = registry.hosts.get(&target.host).copied().unwrap_or(0);
    if !ferry_protocol::resources::allows(
        policy.preview_connections_per_entry,
        hole_count.saturating_add(1),
    ) || !ferry_protocol::resources::allows(
        policy.preview_connections_per_host,
        host_count.saturating_add(1),
    ) {
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

/// A browser can request more modules than there are upstream connections.
/// Keep the active bounds, but wait inside the existing response deadline.
async fn wait_for_capacity(
    target: &Target,
    generation: &Arc<()>,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<(Held, watch::Receiver<bool>), Refused> {
    loop {
        let changed = CAPACITY_CHANGED.notified();
        tokio::pin!(changed);
        // Register before checking capacity so a concurrent release is not lost.
        changed.as_mut().enable();
        if !current_generation(target.hole, generation) {
            return Err(Refused::Unavailable);
        }
        if let Some(held) = hold_with_policy(target, policy) {
            return if current_generation(target.hole, generation) {
                Ok(held)
            } else {
                Err(Refused::Unavailable)
            };
        }
        changed.await;
    }
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
    CAPACITY_CHANGED.notify_waiters();
}

#[cfg(test)]
pub(super) fn tests_register(hole: i64, s: &AppState) {
    remember_state(hole, s);
}

#[cfg(test)]
pub(super) fn tests_registered(hole: i64) -> bool {
    states().contains_key(&hole)
}

#[cfg(test)]
pub(super) fn tests_upstream_authority(address: &str, port: u16) -> String {
    upstream_authority(address, port)
}

#[cfg(test)]
pub(super) fn tests_target_supported(version: &str) -> bool {
    target_supported(version)
}

#[cfg(test)]
pub(super) fn tests_hold(hole: i64, host: i64) -> Option<impl Drop> {
    hold_with_policy(
        &Target {
            hole,
            workspace: 0,
            host,
            port: 1,
            address: String::new(),
            tls: Tls::Plain,
        },
        &test_policy(),
    )
    .map(|(held, _)| held)
}

// ------------------------------------------------------- probe & open

#[cfg(test)]
fn test_policy() -> ferry_protocol::resources::ResourcePolicy {
    ferry_protocol::resources::ResourcePolicy {
        preview_connections_per_entry: Some(MAX_PER_HOLE as u64),
        preview_connections_per_host: Some(MAX_PER_HOST as u64),
        task_timeout_seconds: Some(60),
        ..Default::default()
    }
}
#[cfg(test)]
pub(super) async fn tests_configure_limits(s: &AppState) {
    use sea_orm::sea_query::Expr;
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ResourceConfig,
            Expr::value(serde_json::to_string(&test_policy()).unwrap()),
        )
        .exec(&s.db)
        .await
        .unwrap();
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    #[test]
    fn default_preview_capacity_has_no_legacy_ceiling() {
        let target = Target {
            hole: 995_101,
            workspace: 1,
            host: 995_102,
            port: 8091,
            address: String::new(),
            tls: Tls::Plain,
        };
        let held: Vec<_> = (0..129)
            .map(|_| {
                hold_with_policy(
                    &target,
                    &ferry_protocol::resources::ResourcePolicy::default(),
                )
                .expect("default resource policy must not reject a normal connection burst")
            })
            .collect();
        assert_eq!(active(target.hole), 129);
        drop(held);
        assert_eq!(active(target.hole), 0);
    }

    #[tokio::test]
    async fn cancelled_and_timed_out_waiters_never_reserve_capacity() {
        let (_dir, s) = crate::agents::tests::setup().await;
        tests_configure_limits(&s).await;
        let mut policy = test_policy();
        policy.task_timeout_seconds = Some(1);
        use sea_orm::{EntityTrait, sea_query::Expr};
        crate::entities::platform_state::Entity::update_many()
            .col_expr(
                crate::entities::platform_state::Column::ResourceConfig,
                Expr::value(serde_json::to_string(&policy).unwrap()),
            )
            .exec(&s.db)
            .await
            .unwrap();
        let target = Target {
            hole: 995_091,
            workspace: 1,
            host: 995_092,
            port: 8091,
            address: String::new(),
            tls: Tls::Plain,
        };
        remember_state(target.hole, &s);
        let generation = states().get(&target.hole).unwrap().generation.clone();
        let held: Vec<_> = (0..MAX_PER_HOLE)
            .map(|_| hold_with_policy(&target, &policy).unwrap())
            .collect();
        let mut cancelled = Box::pin(wait_for_capacity(&target, &generation, &policy));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut cancelled)
                .await
                .is_err()
        );
        drop(cancelled);
        assert_eq!(active(target.hole), MAX_PER_HOLE);
        let mut connection = Box::pin(connect(
            format!("http://{}/", target.authority()).parse().unwrap(),
        ));
        let result = tokio::time::timeout(Duration::from_secs(3), &mut connection)
            .await
            .unwrap();
        assert!(
            matches!(result, Err(Refused::Busy)),
            "a background pool connector must have a bounded wait"
        );
        assert_eq!(active(target.hole), MAX_PER_HOLE);
        drop(held);
        assert_eq!(active(target.hole), 0);
        assert!(
            hold_with_policy(&target, &policy).is_some(),
            "cancelled waiters must not consume a released slot"
        );
        close(target.hole);
    }

    #[tokio::test]
    async fn closed_entry_waiter_cannot_join_a_new_generation() {
        let (_dir, s) = crate::agents::tests::setup().await;
        tests_configure_limits(&s).await;
        let policy = test_policy();
        let target = Target {
            hole: 995_093,
            workspace: 1,
            host: 995_094,
            port: 8091,
            address: String::new(),
            tls: Tls::Plain,
        };
        remember_state(target.hole, &s);
        let generation = states().get(&target.hole).unwrap().generation.clone();
        let mut held: Vec<_> = (0..MAX_PER_HOLE)
            .map(|_| hold_with_policy(&target, &policy).unwrap())
            .collect();
        let mut waiting = Box::pin(wait_for_capacity(&target, &generation, &policy));
        assert!(futures::poll!(&mut waiting).is_pending());
        close(target.hole);
        remember_state(target.hole, &s);
        drop(held.pop());
        assert!(matches!(
            futures::poll!(&mut waiting),
            std::task::Poll::Ready(Err(Refused::Unavailable))
        ));
        assert!(!current_generation(target.hole, &generation));
        let current = states().get(&target.hole).unwrap().generation.clone();
        assert!(
            wait_for_capacity(&target, &current, &policy).await.is_ok(),
            "the new generation remains usable"
        );
        drop(held);
        close(target.hole);
    }

    #[tokio::test]
    async fn maintenance_keeps_a_queued_connector_until_it_is_cancelled() {
        let (_dir, s) = crate::agents::tests::setup().await;
        tests_configure_limits(&s).await;
        let policy = test_policy();
        tokio::time::pause();
        let target = Target {
            hole: 995_095,
            workspace: 1,
            host: 995_096,
            port: 8091,
            address: String::new(),
            tls: Tls::Plain,
        };
        remember_state(target.hole, &s);
        let mut held = Vec::new();
        for hole in [995_097, 995_098, 995_099, 995_100] {
            let occupied = Target {
                hole,
                ..target.clone()
            };
            held.extend((0..MAX_PER_HOLE).map(|_| hold_with_policy(&occupied, &policy).unwrap()));
        }
        let mut waiting = Box::pin(connect(
            format!("http://{}/", target.authority()).parse().unwrap(),
        ));
        assert!(futures::poll!(&mut waiting).is_pending());
        tokio::time::advance(POOL_IDLE * 2 + Duration::from_millis(1)).await;
        forget_stale();
        assert!(
            tests_registered(target.hole),
            "queued work is not an idle pool entry"
        );
        drop(waiting);
        forget_stale();
        assert!(
            !tests_registered(target.hole),
            "cancelling releases the pending state reference"
        );
        drop(held);
    }
}

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
    if !target.address.is_empty() {
        let host = crate::entities::hosts::Entity::find_by_id(target.host)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if !target_supported(&host.agent_version) {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("holes.targetRunnerUpgrade"),
            ));
        }
    }
    let channel =
        crate::hosts::remote::open(s, &Id::from(target.workspace), &Id::from(target.host)).await?;
    channel.forward(&target.address, target.port).await
}

/// Checks that the local service accepts a connection and records the result.
pub(super) async fn probe(s: &AppState, row: &holes::Model) {
    let outcome = match reach(s, row).await {
        Ok(()) => ("reachable", String::new()),
        Err(error) => classify(&error),
    };
    record(s, row.id, outcome, true).await;
}

/// Opens and closes one forward: the local service accepts. An HTTPS target
/// must also complete a TLS handshake under its certificate policy.
pub(super) async fn reach(s: &AppState, row: &holes::Model) -> Result<(), ApiError> {
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let target = Target::of(row).ok_or_else(ApiError::missing)?;
    let mut socket = open(s, &target).await?;
    if target.tls == Tls::Plain {
        let _ = socket.send(Frame::Close(None)).await;
        return Ok(());
    }
    // A full preview already holds every slot: it is plainly reachable.
    let Some((held, closing)) = hold_with_policy(&target, &policy) else {
        let _ = socket.send(Frame::Close(None)).await;
        return Ok(());
    };
    let (near, far) = tokio::io::duplex(CHUNK * 2);
    tokio::spawn(pump(s.clone(), target.hole, socket, far, held, closing));
    // Dropping the TLS stream closes the pipe, and the pump closes the session.
    tls::connect(target.tls, &target.address, near)
        .await
        .map(drop)
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

/// The upstream end of a pump, as hyper sees it: the pipe itself, or TLS on it.
struct Pipe(TokioIo<Box<dyn tls::Io>>);

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
    let mut target = Target::parse(&uri).ok_or(Refused::Unavailable)?;
    let (s, generation) = {
        let states = states();
        let handle = states.get(&target.hole).ok_or(Refused::Unavailable)?;
        (
            handle.state.upgrade().ok_or(Refused::Unavailable)?,
            handle.generation.clone(),
        )
    };
    let policy = crate::accounts::resource_policy::load(&s.db)
        .await
        .map_err(|_| Refused::Unavailable)?;
    let wait = wait_for_capacity(&target, &generation, &policy);
    let (held, closing) = match policy.task_timeout_seconds {
        Some(seconds) => tokio::time::timeout(Duration::from_secs(seconds), wait)
            .await
            .map_err(|_| Refused::Busy)??,
        None => wait.await?,
    };
    let row = holes::Entity::find_by_id(target.hole)
        .one(&s.db)
        .await
        .ok()
        .flatten()
        .ok_or(Refused::Unavailable)?;
    // The pool key was made under a setting that has since changed.
    if !current_generation(target.hole, &generation)
        || state(&row, db::now()) != "active"
        || Tls::of(&row.target_scheme, row.target_tls_insecure) != target.tls
    {
        return Err(Refused::Unavailable);
    }
    target.address = row.target_host;
    let socket = match open(&s, &target).await {
        Ok(socket) => socket,
        Err(error) => {
            record(&s, target.hole, classify(&error), false).await;
            return Err(Refused::Unavailable);
        }
    };
    let (near, far) = tokio::io::duplex(CHUNK * 2);
    tokio::spawn(pump(s.clone(), target.hole, socket, far, held, closing));
    match tls::connect(target.tls, &target.address, near).await {
        Ok(io) => {
            record(&s, target.hole, ("reachable", String::new()), false).await;
            Ok(Pipe(TokioIo::new(io)))
        }
        Err(error) => {
            record(&s, target.hole, classify(&error), false).await;
            Err(Refused::Unavailable)
        }
    }
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

fn build_client() -> Client {
    hyper_util::client::legacy::Client::builder(TokioExecutor::new())
        .pool_idle_timeout(POOL_IDLE)
        .pool_timer(TokioTimer::new())
        .pool_max_idle_per_host(8)
        .build(Connector)
}

/// One pool for the process: its idle reaper runs on the server's runtime.
#[cfg(not(test))]
static CLIENT: LazyLock<Client> = LazyLock::new(build_client);
#[cfg(not(test))]
fn client() -> Client {
    CLIENT.clone()
}

// Each test has its own runtime. A process-wide pool would leave its idle
// reaper on whichever runtime used it first, and stop reaping once that test
// ended. Per-thread pools keep every reaper on the runtime that uses it.
#[cfg(test)]
thread_local! {
    static CLIENT: Client = build_client();
}
#[cfg(test)]
fn client() -> Client {
    CLIENT.with(Client::clone)
}

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
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "holes.unavailable");
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
    let cookies = Cookies::for_entry(&s, expected).await;
    if request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
    {
        return preflight(request.headers());
    }
    let policy = match crate::accounts::resource_policy::load(&s.db).await {
        Ok(policy) => policy,
        Err(_) => return refuse(StatusCode::SERVICE_UNAVAILABLE, "holes.unavailable"),
    };
    let body_limit = ferry_protocol::resources::byte_limit(policy.preview_body_bytes);
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_some_and(|length| length > body_limit) {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "holes.tooLarge");
    }
    remember_state(target.hole, &s);
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
        &target.address,
        target.port,
        target.tls.scheme(),
        &external_host,
        &prefix,
        upgrade,
        cookies,
    );
    parts.uri = uri;
    let upstream_request =
        Request::from_parts(parts, http_body_util::Limited::new(body, body_limit));
    let response = client().request(upstream_request);
    let result = match policy.task_timeout_seconds {
        Some(seconds) => tokio::time::timeout(Duration::from_secs(seconds), response).await,
        None => Ok(response.await),
    };
    let mut response = match result {
        // 503, not 502/504: Cloudflare replaces origin 502/504 pages with
        // its own, which would hide why the preview failed.
        Err(_) => return refuse_with(StatusCode::SERVICE_UNAVAILABLE, "holes.timeout", &row),
        Ok(Err(error)) => return failure(&error, &row),
        Ok(Ok(response)) => response,
    };
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        match client_upgrade {
            Some(client_upgrade) => {
                tokio::spawn(splice(client_upgrade, upstream_upgrade));
            }
            // A 101 nobody asked for cannot be relayed.
            None => return refuse_with(StatusCode::SERVICE_UNAVAILABLE, "holes.unavailable", &row),
        }
    }
    let (mut head, body) = response.into_parts();
    rewrite_response(
        &mut head.headers,
        &prefix,
        &target.address,
        target.port,
        cookies,
    );
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

fn failure(error: &hyper_util::client::legacy::Error, row: &holes::Model) -> Response {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(refused) = current.downcast_ref::<Refused>() {
            return match refused {
                Refused::Busy => refuse(StatusCode::SERVICE_UNAVAILABLE, "holes.busy"),
                Refused::Unavailable => {
                    refuse_with(StatusCode::SERVICE_UNAVAILABLE, "holes.unavailable", row)
                }
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
    refuse_with(StatusCode::SERVICE_UNAVAILABLE, "holes.unavailable", row)
}

/// A refusal that also says what the latest check of this preview found
/// (e.g. which address the Runner could not reach and why).
fn refuse_with(status: StatusCode, key: &str, row: &holes::Model) -> Response {
    let locale = crate::i18n::current_locale();
    let mut text = crate::i18n::render(locale, &Message::new(key));
    if !row.probe_error.is_empty() {
        let detail = crate::holes::explain_probe_error(locale, &row.probe_error);
        let label = crate::i18n::render(locale, &Message::new("holes.lastCheck"));
        text = format!(
            "{text}

{label}{detail}"
        );
    }
    let mut response = refuse(status, key);
    *response.body_mut() = Body::from(text);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
    response
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
    address: &str,
    port: u16,
    scheme: &str,
    external_host: &HeaderValue,
    prefix: &str,
    upgrade: bool,
    cookies: Cookies,
) {
    // The console session cookie must never reach the previewed service. On a
    // separate site the browser cannot send console cookies here at all, so the
    // preview's own cookies pass through.
    if cookies == Cookies::Isolated {
        headers.remove(header::COOKIE);
    }
    headers.remove("x-ferry-request");
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
    if !upgrade {
        headers.remove(header::CONNECTION);
        headers.remove(header::UPGRADE);
    }
    let local = upstream_authority(address, port);
    if let Ok(value) = HeaderValue::from_str(&local) {
        headers.insert(header::HOST, value);
    }
    if headers.contains_key(header::ORIGIN)
        && let Ok(value) = HeaderValue::from_str(&format!("{scheme}://{local}"))
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
fn rewrite_response(
    headers: &mut HeaderMap,
    prefix: &str,
    address: &str,
    port: u16,
    cookies: Cookies,
) {
    match cookies {
        Cookies::Isolated => {
            for name in [
                header::SET_COOKIE,
                header::CONTENT_SECURITY_POLICY,
                header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
            ] {
                headers.remove(name);
            }
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(SANDBOX),
            );
        }
        // Its own site: a real origin, so the app's scripts, storage, wss and
        // login work. Cookies stay host-only on this preview's host, the app's
        // own CSP is kept, and only same-origin pages may frame it.
        Cookies::Separate { secure } => {
            let rewritten: Vec<HeaderValue> = headers
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .filter_map(|value| set_cookie(value, secure))
                .filter_map(|value| HeaderValue::from_str(&value).ok())
                .collect();
            headers.remove(header::SET_COOKIE);
            for value in rewritten {
                headers.append(header::SET_COOKIE, value);
            }
            headers.append(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("frame-ancestors 'self'"),
            );
        }
    }
    for name in [
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
    ] {
        headers.remove(name);
    }
    headers.remove("service-worker-allowed");
    headers.remove("clear-site-data");
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    for name in [header::LOCATION, header::CONTENT_LOCATION] {
        if let Some(location) = headers
            .get(&name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| location(value, prefix, address, port))
            && let Ok(value) = HeaderValue::from_str(&location)
        {
            headers.insert(name, value);
        }
    }
    // `Refresh: 0; url=https://192.168.0.2:8006/` works like a redirect.
    if let Some(refresh) = headers
        .get(header::REFRESH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| refresh(value, prefix, address, port))
        && let Ok(value) = HeaderValue::from_str(&refresh)
    {
        headers.insert(header::REFRESH, value);
    }
}

/// Rewrites the URL part of a `Refresh` header the same way as `Location`.
pub(super) fn refresh(value: &str, prefix: &str, address: &str, port: u16) -> Option<String> {
    let at = value.to_ascii_lowercase().find("url=")?;
    let (head, url) = value.split_at(at + 4);
    let url = url.trim().trim_matches(|c| c == '\'' || c == '"');
    Some(format!("{head}{}", location(url, prefix, address, port)?))
}

/// Points a redirect from the local service back under the hole's prefix.
/// Anything that is not the local origin or a root-relative path is left alone.
pub(super) fn location(value: &str, prefix: &str, address: &str, port: u16) -> Option<String> {
    let mut hosts = vec![
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ];
    if !address.is_empty() {
        hosts.push(upstream_authority(address, port));
    }
    // Either scheme: an HTTPS-only device (PVE, a NAS) redirects to https://.
    let mut local: Vec<String> = hosts
        .iter()
        .flat_map(|host| [format!("http://{host}"), format!("https://{host}")])
        .collect();
    // Default ports are often left out of an absolute URL.
    if !address.is_empty() && (port == 443 || port == 80) {
        let authority = upstream_authority(address, port);
        let bare = authority
            .rsplit_once(':')
            .map_or(authority.as_str(), |(host, _)| host);
        local.push(format!(
            "{}://{bare}",
            if port == 443 { "https" } else { "http" }
        ));
    }
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

// ------------------------------------------------------------- cookies

/// Whether a preview may keep cookies and a real origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cookies {
    /// Cookies stripped both ways and the response sandboxed (opaque origin).
    Isolated,
    /// Served on its own site: host-only cookies and a real origin.
    Separate { secure: bool },
}

impl Cookies {
    /// Only the separate-domain entry, and only when the preview domain is a
    /// different site from the console; everything else stays isolated.
    async fn for_entry(s: &AppState, mode: &str) -> Self {
        if mode != "domain" {
            return Self::Isolated;
        }
        match crate::accounts::addresses::Configuration::load(s, &s.db).await {
            Ok(config) => Self::of(&config),
            Err(_) => Self::Isolated,
        }
    }
    pub(super) fn of(config: &crate::accounts::addresses::Configuration) -> Self {
        let Ok(console) = reqwest::Url::parse(&config.public_url) else {
            return Self::Isolated;
        };
        let console_site = site(console.host_str().unwrap_or_default());
        let preview_site = site(&config.preview_domain);
        if console_site.is_empty() || preview_site.is_empty() || console_site == preview_site {
            return Self::Isolated;
        }
        // Previews are served with the console's scheme.
        Self::Separate {
            secure: console.scheme() == "https",
        }
    }
}

/// The registrable part of a host, approximated by its last two DNS labels
/// (no public-suffix list is available): "lh64….debugs.top" → "debugs.top".
/// Under a shared suffix such as "co.uk" two different sites compare equal and
/// so stay isolated, which errs on the safe side. An IP literal (or a single
/// label) cannot be compared this way and counts as unknown, which isolates.
pub(super) fn site(host: &str) -> String {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return String::new();
    }
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    if labels.len() < 2 {
        return String::new();
    }
    labels[labels.len() - 2..].join(".")
}

/// A `Set-Cookie` the preview may keep: any Domain attribute is dropped so the
/// cookie is host-only (one preview host can never set or read another's),
/// Secure is added when the preview is served over HTTPS, and a `__Host-` or
/// `__Secure-` cookie that could not be valid without Secure is dropped.
pub(super) fn set_cookie(value: &str, secure: bool) -> Option<String> {
    let mut parts = value.split(';');
    let pair = parts.next()?.trim();
    let name = pair.split('=').next().unwrap_or_default().trim();
    if name.is_empty() {
        return None;
    }
    let mut attributes: Vec<String> = Vec::new();
    let mut has_secure = false;
    for attribute in parts {
        let attribute = attribute.trim();
        let key = attribute
            .split('=')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match key.as_str() {
            "" | "domain" => continue,
            "secure" => has_secure = true,
            _ => {}
        }
        attributes.push(attribute.to_owned());
    }
    if secure && !has_secure {
        attributes.push("Secure".into());
        has_secure = true;
    }
    let prefixed = name.starts_with("__Host-") || name.starts_with("__Secure-");
    if prefixed && !has_secure {
        return None;
    }
    Some(
        std::iter::once(pair.to_owned())
            .chain(attributes)
            .collect::<Vec<_>>()
            .join("; "),
    )
}
