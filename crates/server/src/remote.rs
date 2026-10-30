//! The terminal and file-transfer endpoints.
//!
//! These are deliberately thin: they authorize, open a `HostChannel`, and move
//! bytes. No SSH or runner detail appears here, which is what keeps both ways
//! of reaching a host behaving identically from the browser's point of view.

use crate::snowflake::Id;

use crate::{
    channel::{self, HostChannel, PtySize},
    db,
    entities::hosts,
    error::{ApiError, Result},
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    body::Body,
    extract::{
        Path, Query, State,
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

/// `GET .../hosts/{id}/terminal` upgraded to a WebSocket.
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
    Ok(upgrade.on_upgrade(move |socket| async move {
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
async fn pump(socket: WebSocket, mut pty: Box<dyn channel::Pty>, _channel: Box<dyn HostChannel>) {
    let (mut sink, mut stream) = socket.split();
    // Protocol-level heartbeats never reach the PTY or the user's shell.
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    let mut last_pong = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_pong.elapsed() > Duration::from_secs(70) {
                    let _ = sink.send(Message::Text(json!({"type":"error","error":"终端连接心跳超时，请重新连接。"}).to_string().into())).await;
                    break;
                }
                if sink.send(Message::Ping(bytes::Bytes::new())).await.is_err() {
                    break;
                }
            },
            // Terminal output.
            output = pty.read() => match output {
                Ok(Some(bytes)) => {
                    if sink.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = sink.send(Message::Text(json!({"type":"closed"}).to_string().into())).await;
                    break;
                }
                Err(error) => {
                    let _ = sink
                        .send(Message::Text(json!({"type":"error","error":error.1}).to_string().into()))
                        .await;
                    break;
                }
            },
            // Keystrokes and control frames.
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Binary(data))) => {
                    if pty.write(&data).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    #[derive(Deserialize)]
                    struct Control { r#type: String, cols: Option<u16>, rows: Option<u16> }
                    if let Ok(control) = serde_json::from_str::<Control>(&text)
                        && control.r#type == "resize"
                        && let (Some(cols), Some(rows)) = (control.cols, control.rows)
                        && pty.resize(PtySize { cols, rows }.sane()).await.is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Pong(_))) => { last_pong = tokio::time::Instant::now(); },
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
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
    let entries = open(&s, &scope.workspace_id, &id)
        .await?
        .list_dir(path)
        .await?;
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
        return Err(ApiError::bad("请填写文件或文件夹名称。"));
    }
    let channel = open(&s, &scope.workspace_id, &id).await?;
    match body.kind.as_str() {
        "file" => channel.create_file(path).await?,
        "directory" => channel.create_dir(path).await?,
        _ => return Err(ApiError::bad("类型必须是文件或文件夹。")),
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
    let stream = Box::pin(
        body.into_data_stream()
            .map_err(|error| ApiError(StatusCode::BAD_REQUEST, format!("上传中断：{error}"))),
    );
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let (parent, name) = path.rsplit_once('/').unwrap_or(("/", path));
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

/// `POST .../hosts/{id}/actions/test-connection` — proves the stored details
/// actually reach the machine, before anyone relies on them.
pub async fn test_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.read")?;
    let channel = open(&s, &scope.workspace_id, &id).await?;
    let output = channel
        .exec("echo ferry-ok", Duration::from_secs(15))
        .await?;
    let ok = output.code == 0 && output.stdout.contains("ferry-ok");
    if ok {
        hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Id.eq(id))
            .exec(&s.db)
            .await?;
    }
    Ok(Json(
        json!({ "ok": ok, "code": output.code, "stderr": output.stderr }),
    ))
}
