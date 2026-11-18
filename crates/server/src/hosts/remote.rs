//! The terminal and file-transfer endpoints.
//!
//! These are deliberately thin: they authorize, open a `HostChannel`, and move
//! bytes. No SSH or runner detail appears here, which is what keeps both ways
//! of reaching a host behaving identically from the browser's point of view.

use crate::snowflake::Id;

use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::hosts,
    error::{ApiError, Result},
    hosts::channel::{self, HostChannel, PtySize},
    state::AppState,
};
use axum::{
    body::Body,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt, TryStreamExt};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

#[derive(Deserialize)]
pub struct PathQuery {
    path: String,
    #[serde(default)]
    show_hidden: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    command: String,
    timeout_seconds: u64,
}

/// Non-interactive counterpart of the terminal, sharing its permission and host channel.
pub async fn execute_command(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<CommandRequest>,
) -> Result<Json<Value>> {
    scope.require("hosts.terminal")?;
    if body.command.trim().is_empty()
        || body.command.len() > 64 * 1024
        || body.command.contains('\0')
        || !(1..=300).contains(&body.timeout_seconds)
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("request.jsonData"),
        ));
    }
    let channel = open(&s, &scope.workspace_id, &id).await?;
    // The approval contains the command; persistent host audit records metadata only.
    crate::hosts::activity(
        &s,
        &scope,
        "command-start",
        "hosts",
        &id,
        json!({"timeout_seconds":body.timeout_seconds}),
    )
    .await?;
    let (output, mut receiver) = tokio::sync::mpsc::channel(16);
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    struct Cancel(tokio::sync::watch::Sender<bool>);
    impl Drop for Cancel {
        fn drop(&mut self) {
            self.0.send_replace(true);
        }
    }
    let guard = Cancel(cancel);
    let execution = channel.exec_stream(
        &body.command,
        Duration::from_secs(body.timeout_seconds),
        output,
        cancelled,
    );
    tokio::pin!(execution);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut truncated = false;
    let code = loop {
        tokio::select! {
            result=&mut execution=>break result?,
            Some((stream,bytes))=receiver.recv()=>{
                let remaining=(128*1024usize).saturating_sub(stdout.len()+stderr.len());
                let count=remaining.min(bytes.len());
                if stream=="stderr" {stderr.extend_from_slice(&bytes[..count]);} else {stdout.extend_from_slice(&bytes[..count]);}
                if count<bytes.len() {truncated=true;guard.0.send_replace(true);}
            }
        }
    };
    while let Ok((stream, bytes)) = receiver.try_recv() {
        let remaining = (128 * 1024usize).saturating_sub(stdout.len() + stderr.len());
        let count = remaining.min(bytes.len());
        if stream == "stderr" {
            stderr.extend_from_slice(&bytes[..count]);
        } else {
            stdout.extend_from_slice(&bytes[..count]);
        }
        truncated |= count < bytes.len();
    }
    crate::hosts::activity(
        &s,
        &scope,
        "command-finish",
        "hosts",
        &id,
        json!({"code":code,"truncated":truncated}),
    )
    .await?;
    Ok(Json(
        json!({"code":code,"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"truncated":truncated}),
    ))
}

/// Discover mounted filesystems through the existing host channel.
pub async fn file_systems(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("hosts.files")?;
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let metrics = open(&s, &scope.workspace_id, &id).await?.monitor().await?;
    let mut disks = metrics.disks;
    disks.sort_by(|a, b| a.mount.cmp(&b.mount));
    disks.dedup_by(|a, b| a.mount == b.mount);
    let windows = disks
        .iter()
        .any(|disk| disk.mount.as_bytes().get(1) == Some(&b':'));
    let items: Vec<_> = disks
        .into_iter()
        .map(|disk| {
            let mut path = if windows {
                disk.mount.replace('\\', "/")
            } else {
                disk.mount
            };
            if windows && path.len() == 2 && path.ends_with(':') {
                path.push('/');
            }
            json!({ "name": disk.name, "path": path, "total": disk.total, "used": disk.used })
        })
        .collect();
    Ok(Json(
        json!({ "platform": if windows { "windows" } else { "unix" }, "items": items, "total": items.len(), "page": 1, "page_size": items.len() }),
    ))
}

fn windows_hidden_command(path: &str) -> String {
    use base64::Engine;
    let script = format!(
        "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); ConvertTo-Json -Compress -InputObject @(Get-ChildItem -Force -LiteralPath '{}' | Where-Object {{ ($_.Attributes -band 6) -ne 0 }} | ForEach-Object {{ $_.Name }})",
        path.replace('\'', "''")
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    // Under the host agent, `powershell -EncodedCommand` never exits (even for
    // `1`), so hidden-file listing always hit the 15s timeout, while -Command
    // returns in milliseconds. The script still travels as base64 (only
    // [A-Za-z0-9+/=], safe inside cmd's quotes and PowerShell's single quotes),
    // so paths containing quotes, % or & never reach either parser.
    format!(
        "powershell.exe -NoProfile -NonInteractive -Command \"& ([scriptblock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{}'))))\"",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Loads a host the caller may reach and opens a channel to it.
pub(crate) async fn open(s: &AppState, workspace_id: &Id, id: &Id) -> Result<Box<dyn HostChannel>> {
    let row = hosts::Entity::find_by_id(i64::from(*id))
        .filter(hosts::Column::WorkspaceId.eq(*workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let host = Host {
        id: Id::from(row.id),
        workspace_id: *workspace_id,
        credential_id: Id::from(row.credential_id),
        name: row.name,
        address: row.address,
        port: row.port as u16,
        username: row.username,
        connection: row.connection,
        host_key: row.host_key,
    };
    crate::channel_open(s, &host).await
}

#[derive(Clone)]
pub struct Host {
    pub id: Id,
    pub workspace_id: Id,
    pub credential_id: Id,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub connection: String,
    pub host_key: String,
}

// ------------------------------------------------------------------ 终端

/// `GET .../nodes/{id}/terminal-sessions` upgraded to a WebSocket.
///
/// Browser -> server: binary frames are keystrokes, text frames are JSON
/// control (`{"type":"resize","cols":120,"rows":40}`).
/// Server -> browser: binary frames are terminal output, text frames are JSON
/// status (`{"type":"error"|"closed"}`).
pub async fn terminal(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(raw): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Result<Response> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.terminal")?;
    let size = PtySize {
        cols: raw.get("cols").and_then(|v| v.parse().ok()).unwrap_or(80),
        rows: raw.get("rows").and_then(|v| v.parse().ok()).unwrap_or(24),
    }
    .sane();

    // The channel is opened before the upgrade so a connection failure is a
    // plain HTTP error with a readable message, not a socket that closes.
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let pty = channel.open_pty(size).await?;

    crate::hosts::activity(&s, &scope, "terminal-open", "hosts", &id, json!({})).await?;
    let (state, who, host) = (s.clone(), scope.user.clone(), id.clone());
    let permissions = scope.permissions.clone();
    let locale = crate::i18n::current_locale();
    Ok(upgrade.on_upgrade(move |socket| {
        crate::i18n::with_locale(locale, async move {
            let opened = db::now();
            pump(socket, pty, channel).await;
            // The close is audited too: how long a session lasted is the part an
            // operator actually asks about afterwards.
            let seconds = (db::now() - opened) / 1000;
            let scope = Scope {
                workspace_id: state_workspace(&state, &host).await.unwrap_or_default(),
                user: who,
                permissions,
            };
            let _ = crate::hosts::activity(
                &state,
                &scope,
                "terminal-close",
                "hosts",
                &host,
                json!({ "seconds": seconds }),
            )
            .await;
        })
    }))
}

/// The scope is gone by the time the socket closes, so the workspace is looked
/// up again rather than captured and trusted.
async fn state_workspace(s: &AppState, host_id: &Id) -> Option<Id> {
    hosts::Entity::find_by_id(i64::from(*host_id))
        .one(&s.db)
        .await
        .ok()
        .flatten()
        .map(|row| Id::from(row.workspace_id))
}

/// Copies bytes both ways until either side stops. The channel is held for the
/// lifetime of the session: dropping it would close the remote connection.
async fn pump(socket: WebSocket, pty: Box<dyn channel::Pty>, _channel: Box<dyn HostChannel>) {
    let (sink, stream) = socket.split();
    pump_io(sink, stream, pty).await;
}

async fn pump_io<S, R>(mut sink: S, mut stream: R, mut pty: Box<dyn channel::Pty>)
where
    S: futures::Sink<Message> + Unpin,
    R: futures::Stream<Item = std::result::Result<Message, axum::Error>> + Unpin,
{
    // Bound output memory without letting a slow browser flush stop receiving
    // Ctrl-C, resize or Pong. Only one pending PTY chunk sits outside this queue.
    let (outgoing, mut queued) = tokio::sync::mpsc::channel(32);
    // One coalesced heartbeat signal has priority over queued logs. A full data
    // queue must not discard every Ping and time out a responsive busy terminal.
    let heartbeat_due = tokio::sync::Notify::new();
    let heartbeat_signal = &heartbeat_due;
    let writer = async {
        loop {
            let message = tokio::select! {
                biased;
                _ = heartbeat_signal.notified() => Message::Ping(bytes::Bytes::new()),
                message = queued.recv() => match message {
                    Some(message) => message,
                    None => break,
                },
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    };
    let relay = async move {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
        let mut last_pong = tokio::time::Instant::now();
        let mut pending = None;
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if last_pong.elapsed() > Duration::from_secs(70) {
                        let _ = outgoing.send(Message::Text(json!({"type":"error","error":crate::i18n::message("terminal.heartbeatTimeout")}).to_string().into())).await;
                        return true;
                    }
                    heartbeat_signal.notify_one();
                },
                permit = outgoing.reserve(), if pending.is_some() => {
                    let Ok(permit) = permit else { return false; };
                    permit.send(pending.take().unwrap());
                },
                output = pty.read(), if pending.is_none() => match output {
                    Ok(Some(bytes)) => { pending = Some(Message::Binary(bytes)); }
                    Ok(None) => {
                        let _ = outgoing.send(Message::Text(json!({"type":"closed"}).to_string().into())).await;
                        return true;
                    }
                    Err(error) => {
                        let _ = outgoing.send(Message::Text(json!({"type":"error","error":error.render(crate::i18n::current_locale())}).to_string().into())).await;
                        return true;
                    }
                },
                incoming = stream.next() => match incoming {
                    Some(Ok(Message::Binary(data))) => {
                        if pty.write(&data).await.is_err() { return false; }
                    }
                    Some(Ok(Message::Text(text))) => {
                        #[derive(Deserialize)]
                        struct Control { r#type: String, cols: Option<u16>, rows: Option<u16> }
                        if let Ok(control) = serde_json::from_str::<Control>(&text)
                            && control.r#type == "resize"
                            && let (Some(cols), Some(rows)) = (control.cols, control.rows)
                            && pty.resize(PtySize { cols, rows }.sane()).await.is_err()
                        {
                            return false;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => { last_pong = tokio::time::Instant::now(); },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return false,
                    Some(Ok(_)) => {}
                },
            }
        }
    };
    tokio::pin!(writer, relay);
    tokio::select! {
        _ = &mut writer => {},
        drain = &mut relay => {
            // Deliver final output/status on shell exit; a browser disconnect
            // instead cancels the writer immediately and drops the remote PTY.
            if drain { writer.await; }
        },
    }
}

// -------------------------------------------------------------- 文件传输

pub async fn list_dir(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.files")?;
    let path = channel::check_path(&query.path)?;
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let mut entries = channel.list_dir(path).await?;
    if !query.show_hidden {
        let host = hosts::Entity::find_by_id(i64::from(id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        let windows = host.os.eq_ignore_ascii_case("windows")
            || path.as_bytes().get(1) == Some(&b':')
            || path.starts_with("\\\\");
        let hidden: Vec<String> = if windows {
            let result = channel
                .exec(&windows_hidden_command(path), Duration::from_secs(15))
                .await?;
            if result.code != 0 {
                return Err(ApiError::localized(
                    StatusCode::BAD_GATEWAY,
                    crate::i18n::Message::new("runner.hiddenRead"),
                ));
            }
            serde_json::from_str(result.stdout.trim_start_matches('\u{feff}').trim()).map_err(
                |_| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("runner.hiddenResponse"),
                    )
                },
            )?
        } else {
            Vec::new()
        };
        entries.retain(|entry| !entry.name.starts_with('.') && !hidden.contains(&entry.name));
    }
    Ok(Json(json!({ "path": path, "items": entries })))
}

#[derive(Deserialize)]
pub struct CreateEntryBody {
    path: String,
    kind: String,
}

pub async fn create_entry(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<CreateEntryBody>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.files")?;
    let path = channel::check_path(&body.path)?;
    if path == "/" || path.ends_with("/.") {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.fileName"),
        ));
    }
    let channel = open(&s, &scope.workspace_id, &id).await?;
    match body.kind.as_str() {
        "file" => channel.create_file(path).await?,
        "directory" => channel.create_dir(path).await?,
        _ => {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.fileKind"),
            ));
        }
    }
    crate::hosts::activity(
        &s,
        &scope,
        "file-create",
        "hosts",
        &id,
        json!({ "path": path, "kind": body.kind }),
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn download(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
) -> Result<Response> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.files")?;
    let path = channel::check_path(&query.path)?;
    let name = path.rsplit('/').next().unwrap_or("download").to_owned();
    let (meta, stream) = open(&s, &scope.workspace_id, &id)
        .await?
        .read_file(path)
        .await?;
    crate::hosts::activity(
        &s,
        &scope,
        "file-download",
        "hosts",
        &id,
        json!({ "path": path, "bytes": meta.size }),
    )
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                // The name is quoted and stripped of quotes/newlines so a
                // crafted filename cannot inject another header.
                format!(
                    "attachment; filename=\"{}\"",
                    name.replace(['"', '\r', '\n'], "")
                ),
            ),
            (header::CONTENT_LENGTH, meta.size.to_string()),
        ],
        Body::from_stream(stream.map_err(std::io::Error::other)),
    )
        .into_response())
}

pub async fn upload(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
    body: Body,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.files")?;
    let path = channel::check_path(&query.path)?;
    let stream = Box::pin(body.into_data_stream().map_err(|error| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.uploadInterrupted").with("detail", error.to_string()),
        )
    }));
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let windows = path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\");
    let split = if windows {
        path.rfind(['/', '\\'])
    } else {
        path.rfind('/')
    };
    let (parent, name) = split
        .map(|cut| {
            (
                &path[..if windows && cut == 2 { 3 } else { cut }],
                &path[cut + 1..],
            )
        })
        .unwrap_or(("/", path));
    let parent = if parent.is_empty() { "/" } else { parent };
    let mode = channel
        .list_dir(parent)
        .await
        .ok()
        .and_then(|entries| entries.into_iter().find(|entry| entry.name == name))
        .map(|entry| entry.mode & 0o777)
        .unwrap_or(0o644);
    channel.write_file(path, mode, stream).await?;
    crate::hosts::activity(
        &s,
        &scope,
        "file-upload",
        "hosts",
        &id,
        json!({ "path": path }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsBody {
    path: String,
    mode: u32,
}
/// POSIX rwx only, non-recursive and without elevation. Existing special bits remain intact.
pub async fn set_file_permissions(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<PermissionsBody>,
) -> Result<Json<Value>> {
    scope.require("hosts.files")?;
    let path = channel::check_path(&body.path)?;
    if body.mode > 0o777 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("files.permissionInvalid"),
        ));
    }
    let host = hosts::Entity::find_by_id(id)
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if host.os.eq_ignore_ascii_case("windows")
        || path.as_bytes().get(1) == Some(&b':')
        || path.starts_with("\\\\")
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("files.permissionUnsupported"),
        ));
    }
    let (parent, name) = path
        .rsplit_once('/')
        .filter(|(_, name)| !name.is_empty())
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.permissionInvalid"),
            )
        })?;
    let parent = if parent.is_empty() { "/" } else { parent };
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let before = channel
        .list_dir(parent)
        .await?
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(ApiError::missing)?;
    if before.kind == "symlink" {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("files.permissionUnsupported"),
        ));
    }
    let mode = (before.mode & 0o7000) | body.mode;
    let command = permissions_command(path, mode);
    let result = channel.exec(&command, Duration::from_secs(15)).await?;
    if result.code != 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_GATEWAY,
            crate::i18n::Message::new("ssh.fileOperation")
                .with("path", path)
                .with(
                    "error",
                    result.stderr.chars().take(2048).collect::<String>(),
                ),
        ));
    }
    crate::hosts::activity(
        &s,
        &scope,
        "file-permissions",
        "hosts",
        &id,
        json!({"path":path,"before":before.mode&0o7777,"after":mode}),
    )
    .await?;
    Ok(Json(json!({"mode":mode})))
}

fn permissions_command(path: &str, mode: u32) -> String {
    format!("chmod {mode:o} -- '{}'", path.replace('\'', "'\\''"))
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn permissions_quote_the_complete_literal_path() {
        assert_eq!(
            permissions_command("/tmp/a'b; $(id)", 0o6750),
            "chmod 6750 -- '/tmp/a'\\''b; $(id)'"
        );
        assert!(
            serde_json::from_value::<PermissionsBody>(json!({"path":"/tmp/a","mode":-1})).is_err()
        );
        assert!(
            serde_json::from_value::<PermissionsBody>(
                json!({"path":"/tmp/a","mode":420,"recursive":true})
            )
            .is_err()
        );
    }
}

pub async fn delete_file(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.files")?;
    let path = channel::check_path(&query.path)?;
    open(&s, &scope.workspace_id, &id)
        .await?
        .delete_file(path)
        .await?;
    crate::hosts::activity(
        &s,
        &scope,
        "file-delete",
        "hosts",
        &id,
        json!({ "path": path }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST .../nodes/{id}/connection-tests` — proves the stored details
/// actually reach the machine, before anyone relies on them.
pub async fn test_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.read")?;
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let owner = match &s.cluster {
        Some(cluster) => cluster.owner(&id.to_string()).await?,
        None => None,
    };
    let output = channel
        .exec("echo ferry-ok", Duration::from_secs(15))
        .await?;
    let ok = output.code == 0 && output.stdout.contains("ferry-ok");
    if ok {
        let mut update = hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Id.eq(id));
        if let Some(owner) = owner {
            update = update.filter(crate::cluster::fence_condition(
                &id.to_string(),
                &owner.fence,
            ));
        }
        update.exec(&s.db).await?;
    }
    Ok(Json(
        json!({ "ok": ok, "code": output.code, "stderr": output.stderr }),
    ))
}

#[cfg(test)]
mod terminal_transport_tests {
    use super::*;
    use std::{
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
    };

    struct StalledSink(Arc<tokio::sync::Notify>);
    impl futures::Sink<Message> for StalledSink {
        type Error = ();
        fn poll_ready(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            self.0.notify_one();
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> std::result::Result<(), ()> {
            unreachable!()
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Pending
        }
        fn poll_close(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
    }
    struct TestPty(tokio::sync::mpsc::UnboundedSender<Vec<u8>>);
    struct ThrottledSink {
        ready: Pin<Box<tokio::time::Sleep>>,
        pings: Arc<std::sync::atomic::AtomicUsize>,
        output: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl futures::Sink<Message> for ThrottledSink {
        type Error = ();
        fn poll_ready(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            use std::future::Future;
            self.get_mut().ready.as_mut().poll(cx).map(|_| Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, message: Message) -> std::result::Result<(), ()> {
            use std::sync::atomic::Ordering;
            let this = self.get_mut();
            match message {
                Message::Ping(_) => {
                    this.pings.fetch_add(1, Ordering::Relaxed);
                }
                Message::Binary(_) => {
                    this.output.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            this.ready
                .as_mut()
                .reset(tokio::time::Instant::now() + Duration::from_millis(50));
            Ok(())
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), ()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn terminal_heartbeats_are_not_starved_by_continuous_backpressure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let pings = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(AtomicUsize::new(0));
        let (written, _writes) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(pump_io(
            ThrottledSink {
                ready: Box::pin(tokio::time::sleep(Duration::ZERO)),
                pings: pings.clone(),
                output: output.clone(),
            },
            futures::stream::pending(),
            Box::new(TestPty(written)),
        ));
        // Exercise the real twenty-second heartbeat interval with output that
        // continuously fills the bounded queue; no clock or policy changes.
        tokio::time::sleep(Duration::from_secs(25)).await;
        task.abort();
        let _ = task.await;
        assert!(
            output.load(Ordering::Relaxed) > 100,
            "the throttled writer must keep making progress"
        );
        assert!(
            pings.load(Ordering::Relaxed) >= 2,
            "busy output must not discard heartbeat frames"
        );
    }

    #[async_trait::async_trait]
    impl channel::Pty for TestPty {
        async fn write(&mut self, data: &[u8]) -> Result<()> {
            self.0.send(data.to_vec()).unwrap();
            Ok(())
        }
        async fn read(&mut self) -> Result<Option<bytes::Bytes>> {
            Ok(Some(bytes::Bytes::from_static(b"log")))
        }
        async fn resize(&mut self, _: PtySize) -> Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn terminal_stalled_browser_output_does_not_block_interrupt_input() {
        let blocked = Arc::new(tokio::sync::Notify::new());
        let ready = blocked.clone();
        let incoming = futures::stream::once(async move {
            ready.notified().await;
            Ok(Message::Binary(bytes::Bytes::from_static(b"\x03")))
        })
        .chain(futures::stream::pending())
        .boxed();
        let (written, mut writes) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(pump_io(
            StalledSink(blocked),
            incoming,
            Box::new(TestPty(written)),
        ));
        let result = tokio::time::timeout(Duration::from_millis(500), writes.recv()).await;
        task.abort();
        let _ = task.await;
        assert_eq!(
            result.expect("a slow browser writer must not stall Ctrl-C"),
            Some(vec![3])
        );
    }

    #[tokio::test]
    async fn terminal_delivers_final_output_before_closed_status() {
        struct FinitePty(bool);
        #[async_trait::async_trait]
        impl channel::Pty for FinitePty {
            async fn write(&mut self, _: &[u8]) -> Result<()> {
                Ok(())
            }
            async fn read(&mut self) -> Result<Option<bytes::Bytes>> {
                if std::mem::replace(&mut self.0, false) {
                    Ok(Some(bytes::Bytes::from_static(b"last line")))
                } else {
                    Ok(None)
                }
            }
            async fn resize(&mut self, _: PtySize) -> Result<()> {
                Ok(())
            }
        }
        let (sink, mut messages) = futures::channel::mpsc::unbounded::<Message>();
        tokio::time::timeout(
            Duration::from_secs(1),
            pump_io(sink, futures::stream::pending(), Box::new(FinitePty(true))),
        )
        .await
        .expect("shell exit must drain and close the writer");
        let mut observed = Vec::new();
        while let Some(message) = messages.next().await {
            match message {
                Message::Binary(data) => observed.push(String::from_utf8(data.to_vec()).unwrap()),
                Message::Text(text) => observed.push(text.to_string()),
                _ => {}
            }
        }
        assert_eq!(observed, ["last line", "{\"type\":\"closed\"}"]);
    }
}
