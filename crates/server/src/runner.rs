//! The reverse-connection agent, server side: enrolment, the registry of live
//! runners, and `RunnerChannel`.
//!
//! A runner dials out, so a host behind NAT needs no inbound route and the
//! server never holds its credentials — the host holds its own key. Three
//! endpoints make that work:
//!
//! * `POST /api/workspaces/{workspace}/runner-tokens` — an operator mints a
//!   single-use, TTL-limited enrolment token for their workspace;
//! * `GET /api/runners/connect` — the runner's long-lived control connection;
//! * `GET /api/runners/sessions/{session}` — one dedicated connection per
//!   terminal, transfer or command, dialled on request.
//!
//! Workspace isolation rests on one fact rather than on a check in every
//! handler: a runner authenticates as a single `host_id`, that host row belongs
//! to exactly one workspace, and `remote::open` only ever reaches a host it
//! already loaded through the `Scope`-filtered query. An enrolment token is
//! tied to its workspace, so enrolling cannot produce a host anywhere else,
//! and a key only matches the hash on its own host's row.

use crate::snowflake::Id;

use crate::{
    channel::{ByteStream, DirEntry, FileMeta, HostChannel, Output, Pty, PtySize},
    db,
    entities::{hosts, runner_tokens},
    error::{ApiError, Result},
    remote::Host,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use ferry_protocol as wire;
use futures::StreamExt;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

/// How often the server probes a quiet control connection.
const HEARTBEAT_SECONDS: u64 = 20;
/// How often a runner reports load/memory/disk. Doubles as its heartbeat.
const METRICS_SECONDS: u64 = 30;
/// Probes allowed to go unanswered before the connection is considered dead.
const MISSED_PROBES: u32 = 3;
/// How long a runner has to dial a session it was asked for.
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a handshake frame may take to arrive.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a small metadata answer (listing, delete, file header) may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Enrolment token lifetime, in minutes, and the bounds an operator may pick.
const TOKEN_MINUTES: i64 = 30;
const TOKEN_MINUTES_RANGE: std::ops::RangeInclusive<i64> = 5..=1440;
/// Upload chunks are re-cut to this, so one oversized body chunk cannot exceed
/// the WebSocket frame limit at the other end.
const CHUNK: usize = 256 * 1024;

// --------------------------------------------------------------- 在线登记

/// A live control connection. Dropping the last handle closes the sender,
/// which is how an evicted connection's own loop learns it was replaced.
struct Live {
    control: mpsc::Sender<wire::Control>,
    managed: bool,
    version: String,
    ready: std::sync::atomic::AtomicBool,
    failed_version: Mutex<Option<String>>,
    update_error: Mutex<Option<(String, String)>>,
}

/// A session the server has asked a runner to dial. `host_id` is what stops a
/// second runner claiming someone else's session id.
struct Pending {
    host_id: String,
    socket: oneshot::Sender<std::result::Result<WebSocket, String>>,
}

// ponytail: process-global, so this is one server process's view of its own
// runners. Running several replicas behind a load balancer needs the session
// hand-off to cross processes — route runners and browsers to the same replica,
// or add a shared broker, before scaling out.
static LIVE: LazyLock<Mutex<HashMap<String, Arc<Live>>>> = LazyLock::new(Default::default);
static PENDING: LazyLock<Mutex<HashMap<String, Pending>>> = LazyLock::new(Default::default);

fn live() -> MutexGuard<'static, HashMap<String, Arc<Live>>> {
    LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
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

// ------------------------------------------------------------------ 凭据

/// A runner key and an enrolment token are both 256 bits of randomness, so the
/// server stores only a SHA-256 of them. Argon2 is for passwords a human
/// chose; stretching a random 256-bit secret buys nothing and would be paid on
/// every reconnect. There is no timing concern in comparing the hashes either:
/// matching one means finding a preimage, not guessing it byte by byte.
pub fn fingerprint(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

pub fn secret() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}

// ------------------------------------------------------------------ 报错

fn offline(name: &str) -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("主机「{name}」的代理未连接，请确认该主机上的 ferry-runner 正在运行。"),
    )
}

fn remote_failed(error: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        format!("主机代理执行失败：{error}"),
    )
}

fn garbled() -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "主机代理返回了无法识别的响应，请确认代理版本与服务一致。".into(),
    )
}

fn timed_out() -> ApiError {
    ApiError(
        StatusCode::GATEWAY_TIMEOUT,
        "主机代理没有在超时时间内响应。".into(),
    )
}

// -------------------------------------------------------------- 帧读写

async fn send_frame<T: Serialize>(socket: &mut WebSocket, frame: &T) -> Result<()> {
    let text = serde_json::to_string(frame).map_err(|_| garbled())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| remote_failed(error))
}

/// Reads frames until a JSON text frame arrives. Binary frames are payload and
/// never control, so they are skipped rather than treated as an error.
async fn next_frame<T: serde::de::DeserializeOwned>(socket: &mut WebSocket) -> Result<T> {
    while let Some(message) = socket.recv().await {
        match message.map_err(remote_failed)? {
            Message::Text(text) => return serde_json::from_str(&text).map_err(|_| garbled()),
            Message::Close(_) => break,
            _ => {}
        }
    }
    Err(garbled())
}

async fn handshake<T: serde::de::DeserializeOwned>(socket: &mut WebSocket) -> Result<T> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, next_frame(socket))
        .await
        .map_err(|_| timed_out())?
}

/// Says no in a way the other end can log, instead of just closing the socket.
async fn reject(socket: &mut WebSocket, reason: &str) {
    tracing::warn!(%reason, "拒绝 runner 连接");
    let _ = send_frame(
        socket,
        &wire::Control::Rejected {
            reason: reason.to_owned(),
        },
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
}

// ------------------------------------------------------------ 注册令牌

pub(crate) async fn bootstrap_token(db: &sea_orm::DatabaseConnection, token: &str) -> Result<()> {
    if token.len() != 64 {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "接入令牌无效，请重新生成安装命令。".into(),
        ));
    }
    let valid = runner_tokens::Entity::find()
        .filter(runner_tokens::Column::TokenHash.eq(fingerprint(token)))
        .filter(runner_tokens::Column::UsedAt.eq(0_i64))
        .filter(runner_tokens::Column::ExpiresAt.gt(db::now()))
        .one(db)
        .await?;
    if valid.is_none() {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "接入令牌已过期或已使用，请重新生成安装命令。".into(),
        ));
    }
    Ok(())
}

pub async fn token_status(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("hosts.enroll")?;
    let token = runner_tokens::Entity::find_by_id(i64::from(Id::reference(&id)?))
        .filter(runner_tokens::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let host = if token.used_by != 0 {
        hosts::Entity::find_by_id(token.used_by)
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
    } else {
        None
    };
    Ok(Json(json!({
        "id": Id::from(token.id), "expires_at": token.expires_at, "used_at": token.used_at,
        "host_id": if token.used_by != 0 { token.used_by.to_string() } else { String::new() },
        "host_state": host.map(|h| h.state).unwrap_or_default(),
        "state": if token.used_by != 0 { "connected" } else if token.expires_at <= db::now() { "expired" } else { "pending" },
    })))
}

#[derive(Deserialize)]
pub struct TokenBody {
    /// Bind the token to a host the operator already created. Empty means the
    /// runner's own hostname becomes a new host row when it enrols.
    #[serde(default)]
    host_id: String,
    #[serde(default)]
    ttl_minutes: i64,
}

/// `POST .../runner-tokens` — mints the token that goes in the one command the
/// operator runs on the host.
pub async fn create_token(
    State(s): State<AppState>,
    scope: Scope,
    headers: HeaderMap,
    Json(body): Json<TokenBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.enroll")?;
    let minutes = if body.ttl_minutes == 0 {
        TOKEN_MINUTES
    } else if TOKEN_MINUTES_RANGE.contains(&body.ttl_minutes) {
        body.ttl_minutes
    } else {
        return Err(ApiError::bad("有效期需要在 5 分钟到 24 小时之间。"));
    };
    let host_id = Id::reference(&body.host_id)?;
    if host_id != Id::from(0) {
        // Refuse now rather than burning a token on a host that cannot use it.
        hosts::Entity::find_by_id(i64::from(host_id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Connection.eq("runner"))
            .one(&s.db)
            .await?
            .ok_or_else(|| ApiError::bad("该主机不存在，或接入方式不是代理。"))?;
    }

    let (id, now, token) = (s.next_id()?, db::now(), secret());
    let expires_at = now + minutes * 60 * 1000;
    runner_tokens::Entity::insert(runner_tokens::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        host_id: Set(host_id.into()),
        token_hash: Set(fingerprint(&token)),
        expires_at: Set(expires_at),
        used_at: Set(0),
        used_by: Set(0),
        created_by: Set(scope.user.id.into()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(&s.db)
    .await?;
    // The token itself is never stored, so it is audited by id only.
    crate::hosts::activity(
        &s,
        &scope,
        "enroll-token",
        "hosts",
        &host_id,
        json!({"token_id": id, "expires_at": expires_at}),
    )
    .await?;

    // Built from the Host header, which the request guard has already checked
    // against the allowed-hosts list, so it cannot be pointed somewhere else.
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if s.secure_cookie { "https" } else { "http" };
    let server = format!("{scheme}://{authority}");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "token": token,
            "expires_at": expires_at,
            "server": server,
            "command": format!("ferry-runner --server {server} --token {token}"),
        })),
    ))
}

/// Burns the token and reports which workspace and host it was for.
///
/// One conditional UPDATE does the whole check: unknown, expired and
/// already-used tokens all affect zero rows, and two runners racing with the
/// same token cannot both win because only one UPDATE sees `used_at = 0`.
async fn consume_token(db: &impl sea_orm::ConnectionTrait, token: &str) -> Result<(Id, Id, Id)> {
    let (hash, now) = (fingerprint(token), db::now());
    let claimed = runner_tokens::Entity::update_many()
        .col_expr(runner_tokens::Column::UsedAt, Expr::value(now))
        .col_expr(runner_tokens::Column::UpdatedAt, Expr::value(now))
        .filter(runner_tokens::Column::TokenHash.eq(hash.clone()))
        .filter(runner_tokens::Column::UsedAt.eq(0_i64))
        .filter(runner_tokens::Column::ExpiresAt.gt(now))
        .exec(db)
        .await?
        .rows_affected;
    if claimed == 0 {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "注册令牌无效、已过期或已被使用，请在主机页重新生成。".into(),
        ));
    }
    let row = runner_tokens::Entity::find()
        .filter(runner_tokens::Column::TokenHash.eq(hash))
        .one(db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        Id::from(row.id),
        Id::from(row.workspace_id),
        Id::from(row.host_id),
    ))
}

/// A name for a host row the runner created itself.
fn reported_name(hostname: &str) -> String {
    let name: String = hostname.trim().chars().take(60).collect();
    if name.is_empty() {
        "未命名主机".into()
    } else {
        name
    }
}

/// Exchanges a token for the host row it owns and the long-lived key it keeps.
async fn enrol(s: &AppState, hello: &wire::Hello, token: &str) -> Result<(String, String)> {
    let (token_id, workspace_id, bound_host) = consume_token(&s.db, token).await?;
    let (key, now) = (secret(), db::now());
    // The key rotates the host's credential, so the row's version moves with
    // it: an operator's open edit form is genuinely stale afterwards.
    let host_id = if bound_host == Id::from(0) {
        let id = s.next_id()?;
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(workspace_id.into()),
            name: Set(reported_name(&hello.hostname)),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(fingerprint(&key)),
            os: Set(hello.os.clone()),
            arch: Set(hello.arch.clone()),
            agent_version: Set(hello.agent_version.clone()),
            metrics: Set(String::new()),
            seen_at: Set(now),
            created_at: Set(now),
            updated_at: Set(now),
            version: Set(1),
            ..Default::default()
        })
        .exec(&s.db)
        .await?;
        id
    } else {
        let affected = hosts::Entity::update_many()
            .col_expr(hosts::Column::RunnerKeyHash, Expr::value(fingerprint(&key)))
            .col_expr(hosts::Column::Os, Expr::value(hello.os.clone()))
            .col_expr(hosts::Column::Arch, Expr::value(hello.arch.clone()))
            .col_expr(
                hosts::Column::AgentVersion,
                Expr::value(hello.agent_version.clone()),
            )
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(now))
            .col_expr(hosts::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                hosts::Column::Version,
                sea_orm::ExprTrait::add(Expr::col(hosts::Column::Version), 1_i64),
            )
            .filter(hosts::Column::WorkspaceId.eq(workspace_id))
            .filter(hosts::Column::Id.eq(bound_host))
            .filter(hosts::Column::Connection.eq("runner"))
            .exec(&s.db)
            .await?
            .rows_affected;
        if affected == 0 {
            // The token is spent either way: a token that survives a failed
            // binding would be a token that can be retried.
            return Err(ApiError::bad(
                "注册令牌绑定的主机已不存在，或接入方式不是代理，请重新生成令牌。",
            ));
        }
        bound_host
    };
    runner_tokens::Entity::update_many()
        .col_expr(runner_tokens::Column::UsedBy, Expr::value(host_id))
        .filter(runner_tokens::Column::Id.eq(token_id))
        .exec(&s.db)
        .await?;
    tracing::info!(%host_id, %workspace_id, hostname = %hello.hostname, "runner 注册成功");
    Ok((host_id.to_string(), key))
}

/// Checks a stored key against the one host it could belong to.
async fn verify_key(s: &AppState, host_id: &str, key: &str) -> Result<()> {
    hosts::Entity::find_by_id(i64::from(Id::reference(host_id)?))
        .filter(hosts::Column::Connection.eq("runner"))
        .filter(hosts::Column::RunnerKeyHash.ne(""))
        .filter(hosts::Column::RunnerKeyHash.eq(fingerprint(key)))
        .one(&s.db)
        .await?
        .ok_or_else(|| {
            ApiError(
                StatusCode::UNAUTHORIZED,
                "代理凭据无效，请在主机页重新生成接入命令。".into(),
            )
        })
        .map(|_| ())
}

// ------------------------------------------------------------ 控制连接

/// `GET /api/runners/connect` — the runner's control connection. Deliberately
/// not behind the session cookie: a runner is not a browser, and it proves
/// itself with the `Hello` frame instead.
pub async fn connect(State(s): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| control(s, socket))
}

async fn control(s: AppState, mut socket: WebSocket) {
    let hello: wire::Hello = match handshake(&mut socket).await {
        Ok(hello) => hello,
        Err(error) => return reject(&mut socket, &error.1).await,
    };
    if hello.version != wire::VERSION {
        return reject(
            &mut socket,
            &format!(
                "协议版本不匹配：服务要求 {}，代理为 {}。请升级 ferry-runner。",
                wire::VERSION,
                hello.version
            ),
        )
        .await;
    }

    let authenticated = match &hello.auth {
        wire::Auth::Enroll { token } => enrol(&s, &hello, token)
            .await
            .map(|(id, key)| (id, Some(key))),
        wire::Auth::Key { host_id, key } => verify_key(&s, host_id, key)
            .await
            .map(|()| (host_id.clone(), None)),
    };
    let (host_id, key) = match authenticated {
        Ok(value) => value,
        Err(error) => return reject(&mut socket, &error.1).await,
    };

    let row_id = match host_id.parse::<Id>() {
        Ok(id) => id,
        Err(_) => return reject(&mut socket, "代理主机 ID 无效。").await,
    };
    if send_frame(
        &mut socket,
        &wire::Control::Welcome {
            host_id: host_id.clone(),
            key,
            heartbeat_seconds: HEARTBEAT_SECONDS,
            metrics_seconds: METRICS_SECONDS,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    let (control, inbox) = mpsc::channel(16);
    let registration = Arc::new(Live {
        control,
        managed: hello.managed_updates,
        version: hello.agent_version.clone(),
        ready: std::sync::atomic::AtomicBool::new(!hello.managed_updates),
        failed_version: Mutex::new(None),
        update_error: Mutex::new(None),
    });
    // A replaced connection's own loop ends when its sender is dropped here,
    // so a half-dead socket cannot keep a host looking online.
    live().insert(host_id.clone(), registration.clone());
    // Enrolment already wrote the facts; a reconnect refreshes them.
    if matches!(hello.auth, wire::Auth::Key { .. }) {
        let _ = hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
            .col_expr(hosts::Column::Os, Expr::value(hello.os.clone()))
            .col_expr(hosts::Column::Arch, Expr::value(hello.arch.clone()))
            .col_expr(
                hosts::Column::AgentVersion,
                Expr::value(hello.agent_version.clone()),
            )
            .filter(hosts::Column::Id.eq(row_id))
            .exec(&s.db)
            .await;
    }
    tracing::info!(%host_id, hostname = %hello.hostname, "runner 控制连接已建立");

    pump_control(&s, &host_id, socket, inbox, registration.clone()).await;

    // Only tear down the registration if it is still ours: a reconnect that
    // replaced us must not be marked offline by our own exit.
    let still_ours = {
        let mut map = live();
        let ours = map
            .get(&host_id)
            .is_some_and(|current| Arc::ptr_eq(current, &registration));
        if ours {
            map.remove(&host_id);
        }
        ours
    };
    if still_ours {
        let _ = hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("offline"))
            .filter(hosts::Column::Id.eq(row_id))
            .exec(&s.db)
            .await;
        tracing::info!(%host_id, "runner 控制连接已断开");
    }
}

/// The last heartbeat's numbers, small enough for the `hosts.metrics` column.
/// Short keys on purpose: MySQL stores that column as `VARCHAR(255)`.
fn metrics_json(
    load1: f64,
    mem_used: u64,
    mem_total: u64,
    disk_used: u64,
    disk_total: u64,
) -> String {
    json!({
        "load1": (load1 * 100.0).round() / 100.0,
        "mem_used": mem_used,
        "mem_total": mem_total,
        "disk_used": disk_used,
        "disk_total": disk_total,
    })
    .to_string()
}

async fn pump_control(
    s: &AppState,
    host_id: &str,
    mut socket: WebSocket,
    mut inbox: mpsc::Receiver<wire::Control>,
    registration: Arc<Live>,
) {
    let Ok(row_id) = host_id.parse::<Id>() else {
        return;
    };
    let mut probe = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
    probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let workspace = hosts::Entity::find_by_id(i64::from(row_id))
        .one(&s.db)
        .await
        .ok()
        .flatten()
        .map(|h| Id::from(h.workspace_id));
    let mut unanswered = 0;
    loop {
        tokio::select! {
            outgoing = inbox.recv() => match outgoing {
                Some(frame) => if send_frame(&mut socket, &frame).await.is_err() { break },
                // Dropped sender: this connection was replaced.
                None => break,
            },
            incoming = socket.recv() => {
                let Some(message) = incoming else { break };
                let Ok(message) = message else { break };
                unanswered = 0;
                match message {
                    Message::Text(text) => match serde_json::from_str::<wire::Report>(&text) {
                        Ok(wire::Report::Pong) => {}
                        Ok(wire::Report::UpdateHealth { version, failed_version }) => {
                            if registration.managed && version == registration.version {
                                *registration.failed_version.lock().unwrap_or_else(|e|e.into_inner()) = failed_version;
                                registration.ready.store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        Ok(wire::Report::UpdateFailed { request, error }) => {
                            *registration.update_error.lock().unwrap_or_else(|e|e.into_inner()) = Some((request, error.chars().take(512).collect()));
                        }
                        Ok(wire::Report::Metrics { load1, memory_used, memory_total, disk_used, disk_total }) => {
                            // No version bump: a heartbeat is not an edit, and
                            // bumping it would invalidate an operator's open form
                            // every thirty seconds.
                            let _ = hosts::Entity::update_many()
                                .col_expr(hosts::Column::State, Expr::value("online"))
                                .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
                                .col_expr(hosts::Column::Metrics, Expr::value(metrics_json(load1, memory_used, memory_total, disk_used, disk_total)))
                                .filter(hosts::Column::Id.eq(row_id))
                                .exec(&s.db).await;
                        }
                        Ok(wire::Report::SessionFailed { session, error }) => {
                            if let Some(waiting) = claim_pending(&session, host_id) {
                                let _ = waiting.socket.send(Err(error));
                            }
                        }
                        Err(error) => tracing::warn!(%host_id, %error, "无法解析 runner 上报帧"),
                    },
                    // The control connection carries no bulk data, by design.
                    Message::Binary(_) => tracing::warn!(%host_id, "控制连接收到二进制帧，已忽略"),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = probe.tick() => {
                let allowed = if let Some(workspace) = workspace {
                    crate::maintenance::setting(&s.db,workspace,"heartbeat_timeout_seconds").await.map(|n|(n as u64).div_ceil(HEARTBEAT_SECONDS) as u32).unwrap_or(MISSED_PROBES)
                } else { MISSED_PROBES };
                if unanswered >= allowed {
                    tracing::warn!(%host_id, "runner 连续 {allowed} 次心跳无响应，断开连接");
                    break;
                }
                unanswered += 1;
                if send_frame(&mut socket, &wire::Control::Ping).await.is_err() { break }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

// ------------------------------------------------------------ 会话连接

/// `GET /api/runners/sessions/{session}` — one connection for one session,
/// dialled by the runner after `OpenSession`.
pub async fn session(
    State(s): State<AppState>,
    Path(session): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| hand_over(s, session, socket))
}

async fn hand_over(s: AppState, session: String, mut socket: WebSocket) {
    let hello: wire::SessionHello = match handshake(&mut socket).await {
        Ok(hello) => hello,
        Err(error) => return reject(&mut socket, &error.1).await,
    };
    if hello.version != wire::VERSION {
        return reject(&mut socket, "协议版本不匹配，请升级 ferry-runner。").await;
    }
    if let Err(error) = verify_key(&s, &hello.host_id, &hello.key).await {
        return reject(&mut socket, &error.1).await;
    }
    // The session id is only ever sent to one runner, and this is the check
    // that makes that binding enforced rather than merely true.
    let Some(waiting) = claim_pending(&session, &hello.host_id) else {
        return reject(&mut socket, "该会话不存在、已超时，或不属于这台主机。").await;
    };
    // The socket now lives in the request that asked for it; this task is done.
    if let Err(Ok(mut socket)) = waiting.socket.send(Ok(socket)) {
        let _ = socket.send(Message::Close(None)).await;
    }
}

// ----------------------------------------------------------- HostChannel

pub struct RunnerChannel {
    _lease: crate::runner_updates::Lease,
    host_id: String,
    /// Only for error messages: an operator reads names, not ids.
    name: String,
}

/// The factory arm for `connection = 'runner'`. Fails fast when the agent is
/// not connected, so the operator gets a readable HTTP error instead of a
/// socket that opens and immediately closes.
pub async fn open(_s: &AppState, host: &Host) -> Result<Box<dyn HostChannel>> {
    let lease = crate::runner_updates::lease(&host.id.to_string()).await;
    if !live()
        .get(&host.id.to_string())
        .is_some_and(|r| r.ready.load(std::sync::atomic::Ordering::Acquire))
    {
        return Err(offline(&host.name));
    }
    Ok(Box::new(RunnerChannel {
        _lease: lease,
        host_id: host.id.to_string(),
        name: host.name.clone(),
    }))
}

impl RunnerChannel {
    /// Asks the runner to dial a dedicated connection and waits for it.
    async fn session(&self, kind: wire::SessionKind) -> Result<WebSocket> {
        let registration = live()
            .get(&self.host_id)
            .cloned()
            .ok_or_else(|| offline(&self.name))?;
        let id = secret();
        let (socket, dialled) = oneshot::channel();
        pending().insert(
            id.clone(),
            Pending {
                host_id: self.host_id.clone(),
                socket,
            },
        );
        let asked = registration
            .control
            .send(wire::Control::OpenSession {
                session: id.clone(),
                kind,
            })
            .await;
        if asked.is_err() {
            pending().remove(&id);
            return Err(offline(&self.name));
        }
        match tokio::time::timeout(DIAL_TIMEOUT, dialled).await {
            Ok(Ok(Ok(socket))) => Ok(socket),
            Ok(Ok(Err(error))) => {
                pending().remove(&id);
                Err(remote_failed(error))
            }
            // Dropped sender or timeout: either way nothing will arrive.
            Ok(Err(_)) | Err(_) => {
                pending().remove(&id);
                Err(offline(&self.name))
            }
        }
    }

    /// Opens a session, sends one request and returns its first answer, with
    /// the socket in case the answer is a header followed by bytes.
    async fn ask(
        &self,
        kind: wire::SessionKind,
        request: wire::Session,
        within: Duration,
    ) -> Result<(wire::Session, WebSocket)> {
        let mut socket = self.session(kind).await?;
        send_frame(&mut socket, &request).await?;
        let answer = tokio::time::timeout(within, next_frame(&mut socket))
            .await
            .map_err(|_| timed_out())??;
        if let wire::Session::Failed { error } = answer {
            return Err(remote_failed(error));
        }
        Ok((answer, socket))
    }
}

#[async_trait::async_trait]
impl HostChannel for RunnerChannel {
    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output> {
        let seconds = timeout.as_secs().max(1);
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Exec,
                wire::Session::ExecStart {
                    command: command.to_owned(),
                    timeout_seconds: seconds,
                },
                // The runner enforces the timeout; this is the backstop for a
                // runner that dies holding the command.
                timeout + REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::ExecDone {
                code,
                stdout,
                stderr,
            } => Ok(Output {
                code,
                stdout,
                stderr,
            }),
            _ => Err(garbled()),
        }
    }

    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        let mut socket = self.session(wire::SessionKind::Exec).await?;
        send_frame(
            &mut socket,
            &wire::Session::ExecStreamStart {
                command: command.into(),
                timeout_seconds: timeout.as_secs().max(1),
            },
        )
        .await?;
        let deadline = tokio::time::sleep(timeout + Duration::from_secs(85));
        tokio::pin!(deadline);
        let mut stopping = false;
        loop {
            if *cancel.borrow() && !stopping {
                send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                stopping = true;
            }
            tokio::select! {
                _ = cancel.changed(), if !stopping => {
                    send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                    stopping = true;
                }
                _ = &mut deadline => {
                    let _ = socket.send(Message::Close(None)).await;
                    return Err(ApiError(StatusCode::GATEWAY_TIMEOUT, "执行主机未在超时后完成清理。".into()));
                }
                frame = next_frame(&mut socket) => match frame? {
                    wire::Session::ExecOutput {stream, data} => {
                        if !["stdout", "stderr"].contains(&stream.as_str()) || data.len() > 65536 {return Err(garbled());}
                        if !stopping {
                            let should_stop = tokio::select! {
                                sent = output.send((stream, data)) => sent.is_err(),
                                _ = cancel.changed() => true,
                                _ = &mut deadline => return Err(ApiError(StatusCode::GATEWAY_TIMEOUT, "执行主机未在超时后完成清理。".into())),
                            };
                            if should_stop {
                                send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                                stopping = true;
                            }
                        }
                    }
                    wire::Session::ExecExit {code} => return Ok(code),
                    wire::Session::Failed {error} => return Err(remote_failed(error)),
                    _ => return Err(garbled()),
                }
            }
        }
    }

    async fn open_pty(&self, size: PtySize) -> Result<Box<dyn Pty>> {
        let size = size.sane();
        let mut socket = self.session(wire::SessionKind::Terminal).await?;
        send_frame(
            &mut socket,
            &wire::Session::TerminalStart {
                cols: size.cols,
                rows: size.rows,
            },
        )
        .await?;
        // One task owns the socket and the two directions meet it through
        // channels: `Pty::read` is selected over in `remote::pump`, and an
        // mpsc receiver is cancel-safe where a half-read frame would not be.
        let (output, from_host) = mpsc::channel(32);
        let (to_host, input) = mpsc::channel(32);
        let lease = self._lease.clone();
        tokio::spawn(async move {
            let _lease = lease;
            pump_pty(socket, output, input).await;
        });
        Ok(Box::new(RunnerPty {
            output: from_host,
            input: to_host,
        }))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::ListDir {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::DirEntries { entries } => Ok(entries
                .into_iter()
                .map(|entry| DirEntry {
                    name: entry.name,
                    kind: entry.kind,
                    size: entry.size,
                    mode: entry.mode,
                    modified_at: entry.modified_at,
                })
                .collect()),
            _ => Err(garbled()),
        }
    }

    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)> {
        let (answer, socket) = self
            .ask(
                wire::SessionKind::Download,
                wire::Session::ReadFile {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        let wire::Session::FileMeta {
            size,
            mode,
            modified_at,
        } = answer
        else {
            return Err(garbled());
        };
        // One frame at a time: the server's memory stays at one chunk no
        // matter how large the file is.
        let stream =
            futures::stream::unfold(Some((socket, self._lease.clone())), |state| async move {
                let (mut socket, lease) = state?;
                loop {
                    match socket.recv().await {
                        Some(Ok(Message::Binary(bytes))) => {
                            return Some((Ok(bytes), Some((socket, lease))));
                        }
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<wire::Session>(&text) {
                                Ok(wire::Session::Eof) => return None,
                                Ok(wire::Session::Failed { error }) => {
                                    return Some((Err(remote_failed(error)), None));
                                }
                                _ => continue,
                            }
                        }
                        Some(Ok(_)) => continue,
                        // Closing without `Eof` means a truncated file. Reporting
                        // it as an error is the difference between a failed
                        // download and a silently short one.
                        Some(Err(_)) | None => {
                            return Some((
                                Err(ApiError(
                                    StatusCode::BAD_GATEWAY,
                                    "下载中断：主机代理连接提前关闭。".into(),
                                )),
                                None,
                            ));
                        }
                    }
                }
            });
        Ok((
            FileMeta {
                size,
                mode,
                modified_at,
            },
            Box::pin(stream),
        ))
    }

    async fn write_file(&self, path: &str, mode: u32, mut body: ByteStream) -> Result<()> {
        let mut socket = self.session(wire::SessionKind::Upload).await?;
        send_frame(
            &mut socket,
            &wire::Session::WriteFile {
                path: path.to_owned(),
                mode,
            },
        )
        .await?;
        while let Some(chunk) = body.next().await {
            let mut chunk = chunk?;
            while !chunk.is_empty() {
                // Awaiting each send is the backpressure: the uploader cannot
                // run ahead of the host, so nothing queues up here.
                let piece = chunk.split_to(chunk.len().min(CHUNK));
                socket
                    .send(Message::Binary(piece))
                    .await
                    .map_err(remote_failed)?;
            }
        }
        send_frame(&mut socket, &wire::Session::Eof).await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, next_frame(&mut socket))
            .await
            .map_err(|_| timed_out())??
        {
            wire::Session::Done => Ok(()),
            wire::Session::Failed { error } => Err(remote_failed(error)),
            _ => Err(garbled()),
        }
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::DeleteFile {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::Done => Ok(()),
            _ => Err(garbled()),
        }
    }
}

// ------------------------------------------------------------------ 终端

enum Keystrokes {
    Data(Bytes),
    Control(wire::Session),
}

struct RunnerPty {
    output: mpsc::Receiver<Result<Bytes>>,
    input: mpsc::Sender<Keystrokes>,
}

#[async_trait::async_trait]
impl Pty for RunnerPty {
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.input
            .send(Keystrokes::Data(Bytes::copy_from_slice(data)))
            .await
            .map_err(|_| remote_failed("终端会话已结束"))
    }

    async fn read(&mut self) -> Result<Option<Bytes>> {
        match self.output.recv().await {
            Some(Ok(bytes)) => Ok(Some(bytes)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }

    async fn resize(&mut self, size: PtySize) -> Result<()> {
        let size = size.sane();
        self.input
            .send(Keystrokes::Control(wire::Session::TerminalResize {
                cols: size.cols,
                rows: size.rows,
            }))
            .await
            .map_err(|_| remote_failed("终端会话已结束"))
    }
}

async fn pump_pty(
    mut socket: WebSocket,
    output: mpsc::Sender<Result<Bytes>>,
    mut input: mpsc::Receiver<Keystrokes>,
) {
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Binary(bytes) => if output.send(Ok(bytes)).await.is_err() { break },
                    Message::Text(text) => match serde_json::from_str::<wire::Session>(&text) {
                        Ok(wire::Session::Eof) | Ok(wire::Session::Done) => break,
                        Ok(wire::Session::Failed { error }) => {
                            let _ = output.send(Err(remote_failed(error))).await;
                            break;
                        }
                        _ => {}
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            request = input.recv() => match request {
                Some(Keystrokes::Data(bytes)) => {
                    if socket.send(Message::Binary(bytes)).await.is_err() { break }
                }
                Some(Keystrokes::Control(frame)) => {
                    if send_frame(&mut socket, &frame).await.is_err() { break }
                }
                // The `Pty` was dropped, which closes the remote session.
                None => break,
            },
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    #[test]
    fn a_secret_is_hashed_before_it_is_stored() {
        let (first, second) = (secret(), secret());
        assert_eq!(first.len(), 64, "256 bits of hex");
        assert_ne!(first, second, "secrets must not repeat");
        let hash = fingerprint(&first);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, fingerprint(&first), "same input, same hash");
        assert_ne!(hash, first, "the stored value is not the secret");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }

    /// The rule that stops a second runner stealing a session it was never
    /// asked to open.
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

    /// A private database per test, on disk, deleted when the test ends.
    ///
    /// Not `sqlite::memory:`: sqlx gives in-memory databases a shared cache, so
    /// every test in the process ends up on one connection — and
    /// `rows_affected()` reads SQLite's connection-global `changes()`. Two
    /// tests interleaving on that connection make the single-use token check
    /// report a row it did not touch, which is a false pass on the one
    /// assertion here that is load-bearing for security.
    struct Scratch {
        db: DatabaseConnection,
        path: std::path::PathBuf,
    }

    impl std::ops::Deref for Scratch {
        type Target = DatabaseConnection;
        fn deref(&self) -> &DatabaseConnection {
            &self.db
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    async fn scratch_db() -> Scratch {
        let path =
            std::env::temp_dir().join(format!("ferry-runner-test-{}.sqlite", uuid::Uuid::new_v4()));
        let mut options = ConnectOptions::new("sqlite://ferry");
        let filename = path.clone();
        options.sqlx_logging(false).map_sqlx_sqlite_opts(move |o| {
            o.filename(&filename)
                .create_if_missing(true)
                .foreign_keys(true)
        });
        let db = Database::connect(options).await.expect("scratch sqlite");
        use sea_orm_migration::MigratorTrait;
        crate::migration::Migrator::up(&db, None)
            .await
            .expect("migrations apply");
        Scratch { db, path }
    }

    async fn mint(db: &DatabaseConnection, token: &str, expires_at: i64) {
        runner_tokens::Entity::insert(runner_tokens::ActiveModel {
            id: Set(1),
            workspace_id: Set(1),
            host_id: Set(0),
            token_hash: Set(fingerprint(token)),
            expires_at: Set(expires_at),
            used_at: Set(0),
            used_by: Set(0),
            created_by: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
        })
        .exec(db)
        .await
        .expect("insert token");
    }

    #[tokio::test]
    async fn bootstrap_probe_does_not_consume_and_rejects_expired_or_used_tokens() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let token = secret();
        mint(db, &token, db::now() + 60_000).await;
        bootstrap_token(db, &token).await.unwrap();
        bootstrap_token(db, &token).await.unwrap();
        consume_token(db, &token).await.unwrap();
        assert_eq!(
            bootstrap_token(db, &token).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        runner_tokens::Entity::update_many()
            .col_expr(runner_tokens::Column::UsedAt, Expr::value(0_i64))
            .col_expr(runner_tokens::Column::ExpiresAt, Expr::value(db::now() - 1))
            .exec(db)
            .await
            .unwrap();
        assert_eq!(
            bootstrap_token(db, &token).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            bootstrap_token(db, &secret()).await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            bootstrap_token(db, "bad").await.unwrap_err().0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn an_enrolment_token_works_once_and_never_again() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let token = secret();
        mint(db, &token, db::now() + 60_000).await;

        let (_id, workspace_id, host_id) = consume_token(db, &token).await.expect("first use");
        assert_eq!(workspace_id, Id::from(1));
        assert_eq!(host_id, Id::from(0), "unbound token");

        let again = consume_token(db, &token).await;
        assert!(again.is_err(), "a token must not be reusable");
        assert_eq!(again.unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_expired_or_unknown_token_is_refused() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let expired = secret();
        mint(db, &expired, db::now() - 1).await;
        assert!(
            consume_token(db, &expired).await.is_err(),
            "an expired token must be refused"
        );
        assert!(
            consume_token(db, "not-a-token").await.is_err(),
            "garbage must be refused"
        );
        // A refused token must stay unused, not be quietly burned.
        let row = runner_tokens::Entity::find()
            .filter(runner_tokens::Column::TokenHash.eq(fingerprint(&expired)))
            .one(db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.used_at, 0);
    }

    #[tokio::test]
    async fn a_key_only_matches_its_own_host() {
        let scratch = scratch_db().await;
        let db = &*scratch;
        let (key_a, key_b) = (secret(), secret());
        for (id, workspace, key) in [(1_i64, 11_i64, &key_a), (2_i64, 12_i64, &key_b)] {
            hosts::Entity::insert(hosts::ActiveModel {
                id: Set(id),
                workspace_id: Set(workspace),
                name: Set(id.to_string()),
                address: Set(String::new()),
                port: Set(0),
                username: Set(String::new()),
                connection: Set("runner".into()),
                state: Set("online".into()),
                group_id: Set(0),
                description: Set(String::new()),
                host_key: Set(String::new()),
                runner_key_hash: Set(fingerprint(key)),
                os: Set(String::new()),
                arch: Set(String::new()),
                agent_version: Set(String::new()),
                metrics: Set(String::new()),
                seen_at: Set(0),
                created_at: Set(0),
                updated_at: Set(0),
                version: Set(1),
                ..Default::default()
            })
            .exec(db)
            .await
            .expect("insert host");
        }
        // Reaching across workspaces means presenting another host's id with
        // your own key, which has to fail.
        assert!(matches(db, 1, &key_a).await);
        assert!(!matches(db, 2, &key_a).await, "cross-host reuse");
        assert!(!matches(db, 1, &key_b).await, "cross-host reuse");
        assert!(!matches(db, 1, "").await, "an empty key matches nothing");
    }

    /// `verify_key` needs an `AppState`; this is the same query it runs.
    async fn matches(db: &DatabaseConnection, host_id: i64, key: &str) -> bool {
        hosts::Entity::find_by_id(host_id)
            .filter(hosts::Column::Connection.eq("runner"))
            .filter(hosts::Column::RunnerKeyHash.ne(""))
            .filter(hosts::Column::RunnerKeyHash.eq(fingerprint(key)))
            .one(db)
            .await
            .unwrap()
            .is_some()
    }

    #[test]
    fn metrics_stay_inside_the_column_they_are_stored_in() {
        let worst = metrics_json(u64::MAX as f64, u64::MAX, u64::MAX, u64::MAX, u64::MAX);
        assert!(
            worst.len() <= 255,
            "MySQL stores hosts.metrics as VARCHAR(255), got {} chars: {worst}",
            worst.len()
        );
        let normal = metrics_json(0.567, 1 << 30, 1 << 34, 1 << 36, 1 << 38);
        assert!(normal.contains("\"load1\":0.57"), "{normal}");
    }

    #[tokio::test]
    async fn audit_failure_recovers_after_failed_status_write_becomes_available() {
        use sea_orm::ConnectionTrait;
        let scratch = scratch_db().await;
        let dir = tempfile::tempdir().unwrap();
        let id = Id::from(9107);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("update recovery".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set("1.0.0".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&scratch.db)
        .await
        .unwrap();
        // Real database failures at the two distinct writes, without mocking
        // admission or the persistence code being exercised.
        scratch.db.execute_unprepared("CREATE TRIGGER fail_audit BEFORE INSERT ON activity BEGIN SELECT RAISE(FAIL, 'audit unavailable'); END").await.unwrap();
        scratch.db.execute_unprepared("CREATE TRIGGER fail_update_status BEFORE UPDATE OF agent_update ON hosts WHEN NEW.agent_update LIKE '%failed%' BEGIN SELECT RAISE(FAIL, 'status unavailable'); END").await.unwrap();
        let s = Arc::new(crate::state::State {
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db: scratch.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "update-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let (control, mut receiver) = mpsc::channel(1);
        live().insert(
            id.to_string(),
            Arc::new(Live {
                control,
                managed: true,
                version: "1.0.0".into(),
                ready: std::sync::atomic::AtomicBool::new(true),
                failed_version: Mutex::new(None),
                update_error: Mutex::new(None),
            }),
        );
        let scope = Scope {
            workspace_id: Id::from(1),
            user: crate::auth::User {
                id: Id::from(1),
                email: "test@example.invalid".into(),
                name: "test".into(),
                is_superadmin: false,
            },
            permissions: ["hosts.update".into()].into_iter().collect(),
        };
        let mut request = tokio::spawn(start_update(
            State(s),
            scope,
            Path(("test".into(), id)),
            Json(UpdateBody {
                version: "2.0.0".into(),
                manifest: "{}".into(),
                signature: "fixture".into(),
            }),
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut request)
                .await
                .is_err(),
            "admission must keep recovering after audit and status writes both fail"
        );
        assert!(
            receiver.try_recv().is_err(),
            "an unaudited update must never reach the runner"
        );
        scratch
            .db
            .execute_unprepared("DROP TRIGGER fail_update_status")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), request)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<UpdateJob>(row.agent_update.as_deref().unwrap())
                .unwrap()
                .state,
            "failed"
        );
        assert!(
            !UPDATES
                .lock()
                .unwrap()
                .get(&id.to_string())
                .is_some_and(|job| job.state == "draining")
        );
        live().remove(&id.to_string());
        UPDATES.lock().unwrap().remove(&id.to_string());
    }

    #[tokio::test]
    async fn recovery_blocks_tasks_until_health_and_durable_terminal_status() {
        use sea_orm::ConnectionTrait;
        let scratch = scratch_db().await;
        let dir = tempfile::tempdir().unwrap();
        let id = Id::from(9108);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("update recovery".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set("1.0.0".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&scratch.db)
        .await
        .unwrap();
        let s = Arc::new(crate::state::State {
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db: scratch.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "update-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let job = UpdateJob {
            request: "recovery".into(),
            version: "2.0.0".into(),
            state: "updating".into(),
            message: String::new(),
        };
        persist_update(&s, id, &job).await.unwrap();
        recover_updates(&s).await.unwrap();
        let mut task =
            tokio::spawn(async move { crate::runner_updates::lease(&id.to_string()).await });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut task)
                .await
                .is_err(),
            "offline recovery must exclude new work"
        );
        let (control, _receiver) = mpsc::channel(1);
        let peer = Arc::new(Live {
            control,
            managed: true,
            version: "2.0.0".into(),
            ready: std::sync::atomic::AtomicBool::new(false),
            failed_version: Mutex::new(None),
            update_error: Mutex::new(None),
        });
        live().insert(id.to_string(), peer.clone());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut task)
                .await
                .is_err(),
            "unconfirmed connection must not release recovery lock"
        );
        scratch.db.execute_unprepared("CREATE TRIGGER fail_terminal_status BEFORE UPDATE OF agent_update ON hosts WHEN NEW.agent_update LIKE '%succeeded%' BEGIN SELECT RAISE(FAIL, 'status unavailable'); END").await.unwrap();
        peer.ready.store(true, std::sync::atomic::Ordering::Release);
        assert!(
            tokio::time::timeout(Duration::from_millis(500), &mut task)
                .await
                .is_err(),
            "health without durable status must not release recovery lock"
        );
        scratch
            .db
            .execute_unprepared("DROP TRIGGER fail_terminal_status")
            .await
            .unwrap();
        drop(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap(),
        );
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<UpdateJob>(row.agent_update.as_deref().unwrap())
                .unwrap()
                .state,
            "succeeded"
        );
        live().remove(&id.to_string());
        UPDATES.lock().unwrap().remove(&id.to_string());
    }

    #[test]
    fn a_reported_hostname_becomes_a_usable_name() {
        assert_eq!(reported_name("  web-01 "), "web-01");
        assert_eq!(reported_name(""), "未命名主机");
        assert_eq!(reported_name(&"x".repeat(200)).chars().count(), 60);
    }
}

// Upgrade tasks outlive their HTTP request so closing the page cannot release
// the exclusive host reservation while a remote version switch is in flight.
#[derive(Clone, Serialize, Deserialize)]
struct UpdateJob {
    request: String,
    version: String,
    state: String,
    message: String,
}
static UPDATES: LazyLock<Mutex<HashMap<String, UpdateJob>>> = LazyLock::new(Default::default);

#[derive(Deserialize)]
pub struct UpdateBody {
    version: String,
    manifest: String,
    signature: String,
}

async fn update_host(s: &AppState, scope: &Scope, id: Id) -> Result<hosts::Model> {
    hosts::Entity::find_by_id(i64::from(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}

pub async fn update_status(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    let host = update_host(&s, &scope, id).await?;
    let registration = live().get(&id.to_string()).cloned();
    let status = host
        .agent_update
        .as_deref()
        .map(serde_json::from_str::<UpdateJob>)
        .transpose()
        .map_err(|_| ApiError::bad("代理升级状态无法读取"))?;
    Ok(Json(
        json!({"agent_version":host.agent_version,"supported":registration.as_ref().is_some_and(|r|r.managed),"ready":registration.as_ref().is_some_and(|r|r.ready.load(std::sync::atomic::Ordering::Acquire)),"update":status}),
    ))
}

pub async fn start_update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, Id)>,
    Json(body): Json<UpdateBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.update")?;
    update_host(&s, &scope, id).await?;
    if body.version.len() > 128
        || body.manifest.len() > 65536
        || body.signature.len() > 8192
        || body.version.is_empty()
    {
        return Err(ApiError::bad("升级清单或版本无效。"));
    }
    let registration = live()
        .get(&id.to_string())
        .cloned()
        .ok_or_else(|| ApiError::bad("代理离线，不能升级。"))?;
    if !registration.managed {
        return Err(ApiError::bad(
            "当前代理不支持服务升级，请先安装为新版托管服务。",
        ));
    }
    if registration.version == body.version {
        return Err(ApiError::bad("代理已是该版本。"));
    }
    let job = UpdateJob {
        request: secret(),
        version: body.version.clone(),
        state: "draining".into(),
        message: "等待进行中的完整任务结束".into(),
    };
    {
        let mut jobs = UPDATES.lock().unwrap_or_else(|e| e.into_inner());
        if jobs
            .get(&id.to_string())
            .is_some_and(|j| matches!(j.state.as_str(), "draining" | "updating" | "unknown"))
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "已有代理升级进行中。".into(),
            ));
        }
        jobs.insert(id.to_string(), job.clone());
    }
    // Persist/audit admission also outlives a disconnected HTTP caller.
    let admission = tokio::spawn(async move {
        if let Err(error) = persist_update(&s, id, &job).await {
            UPDATES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id.to_string());
            return Err(error);
        }
        if let Err(error) = crate::hosts::activity(
            &s,
            &scope,
            "runner-update",
            "hosts",
            &id,
            json!({"version":body.version,"request":job.request}),
        )
        .await
        {
            let mut failed = job.clone();
            transition_update(
                &s,
                id,
                &mut failed,
                "failed",
                "升级审计写入失败，未发送更新",
            )
            .await;
            UPDATES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id.to_string());
            return Err(error);
        }
        let response = serde_json::to_value(&job).map_err(|_| ApiError::bad("升级状态编码失败"))?;
        tokio::spawn(async move {
            let host = id.to_string();
            let _exclusive = crate::runner_updates::exclusive(&host).await;
            let mut job = job;
            let peer = live().get(&host).cloned();
            let Some(peer) = peer else {
                transition_update(&s, id, &mut job, "failed", "等待任务结束期间代理离线").await;
                return;
            };
            if !peer.managed {
                transition_update(&s, id, &mut job, "failed", "重连后的代理不支持托管升级").await;
                return;
            }
            transition_update(&s, id, &mut job, "updating", "正在验证更新并切换版本").await;
            if peer
                .control
                .send(wire::Control::Update {
                    request: job.request.clone(),
                    version: body.version.clone(),
                    manifest: body.manifest,
                    signature: body.signature,
                })
                .await
                .is_err()
            {
                transition_update(&s, id, &mut job, "failed", "无法发送升级请求").await;
                return;
            }
            let started = std::time::Instant::now();
            loop {
                let failure = peer
                    .update_error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some((request, message)) = failure {
                    if request == job.request {
                        transition_update(&s, id, &mut job, "failed", &message).await;
                        return;
                    }
                }
                let current = live().get(&host).cloned();
                if let Some(current) = current {
                    if !Arc::ptr_eq(&peer, &current)
                        && current.ready.load(std::sync::atomic::Ordering::Acquire)
                    {
                        if current.version == body.version {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "succeeded",
                                "代理升级完成，已通过连接健康检查",
                            )
                            .await;
                        } else {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "rolled_back",
                                "新版本未完成激活，已恢复原版本",
                            )
                            .await;
                        }
                        return;
                    }
                }
                if started.elapsed() > Duration::from_secs(420) {
                    // Never admit new work merely because a response timed out.
                    if job.state != "unknown" {
                        transition_update(
                            &s,
                            id,
                            &mut job,
                            "unknown",
                            "等待代理重新连接确认，继续暂停向该主机派发新任务",
                        )
                        .await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        Ok((StatusCode::ACCEPTED, Json(response)))
    });
    admission
        .await
        .map_err(|_| ApiError::bad("升级任务初始化失败"))?
}

async fn persist_update(s: &AppState, id: Id, job: &UpdateJob) -> Result<()> {
    let value = serde_json::to_string(job).map_err(|_| ApiError::bad("升级状态编码失败"))?;
    hosts::Entity::update_many()
        .col_expr(hosts::Column::AgentUpdate, Expr::value(value))
        .filter(hosts::Column::Id.eq(id))
        .exec(&s.db)
        .await?;
    Ok(())
}

// The caller retains its exclusive reservation throughout persistence retries.
// A failed state write must never release a host with an uncertain update.
async fn transition_update(s: &AppState, id: Id, job: &mut UpdateJob, state: &str, message: &str) {
    job.state = state.into();
    job.message = message.into();
    loop {
        match persist_update(s, id, job).await {
            Ok(()) => break,
            Err(error) => {
                tracing::error!(host=%id,error=%error.1,"无法保存代理升级状态，继续保留主机升级锁")
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    UPDATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), job.clone());
}

/// Restore exclusion before the schedulers or HTTP listener can admit new work.
pub(crate) async fn recover_updates(s: &AppState) -> Result<()> {
    let rows = hosts::Entity::find()
        .filter(hosts::Column::AgentUpdate.is_not_null())
        .all(&s.db)
        .await?;
    for row in rows {
        let mut job: UpdateJob = serde_json::from_str(row.agent_update.as_deref().unwrap())
            .map_err(|_| ApiError::bad("持久化的代理升级状态无法读取"))?;
        let id = Id::from(row.id);
        if job.state == "draining" {
            job.state = "failed".into();
            job.message = "服务重启中断了等待，尚未发送升级请求".into();
            persist_update(s, id, &job).await?;
        }
        UPDATES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), job.clone());
        if !matches!(job.state.as_str(), "updating" | "unknown") {
            continue;
        }
        let exclusion = crate::runner_updates::exclusive(&id.to_string()).await;
        let s = s.clone();
        tokio::spawn(async move {
            let _exclusion = exclusion;
            loop {
                let peer = live().get(&id.to_string()).cloned();
                if let Some(peer) = peer {
                    if peer.managed && peer.ready.load(std::sync::atomic::Ordering::Acquire) {
                        let failed = peer
                            .failed_version
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        let (state, message) = if peer.version == job.version {
                            ("succeeded", "代理升级完成，重启后已确认连接健康")
                        } else if failed.as_deref() == Some(job.version.as_str()) {
                            ("rolled_back", "新版本未完成激活，重启后已确认回退")
                        } else {
                            ("failed", "升级准备被服务中断，代理已恢复健康连接")
                        };
                        transition_update(&s, id, &mut job, state, message).await;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    }
    Ok(())
}
