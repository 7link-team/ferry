//! 授权认证 — the credential store shared by SSH hosts and Git repositories.
//!
//! The secret half never leaves the server. List and detail report only
//! whether one is set; there is no endpoint that returns a stored secret, not
//! even to an owner. Reading one happens inside the server, at the moment it
//! is used, and that use is audited.

use crate::{
    db::{self, execute, one, text},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    state::AppState,
    vault,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::QueryResult;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    table: "credentials",
    search: &["name", "username"],
    filters: &["kind"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};
/// `secret` is deliberately absent: a column that is never selected cannot be
/// returned by accident from the shared list path.
const COLUMNS: &str =
    "id,name,kind,username,description,created_at,updated_at,version,length(secret) AS secret_len";

pub const KINDS: [&str; 3] = ["password", "ssh-key", "token"];

fn to_json(row: &QueryResult) -> Value {
    json!({
        "id": text(row, "id"),
        "name": text(row, "name"),
        "kind": text(row, "kind"),
        "username": text(row, "username"),
        "description": text(row, "description"),
        // Whether a secret exists, never the secret.
        "has_secret": db::int(row, "secret_len") > 0,
        "created_at": db::int(row, "created_at"),
        "updated_at": db::int(row, "updated_at"),
        "version": db::int(row, "version"),
    })
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    description: String,
    /// Absent on an edit means "leave the stored secret alone", which is what
    /// lets the edit form round-trip without ever holding the real value.
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    version: i64,
}

fn check(body: &Body, creating: bool) -> Result<String> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写名称（60 字以内）。"));
    }
    if creating {
        if !KINDS.contains(&body.kind.trim()) {
            return Err(ApiError::bad("请选择凭据类型：密码、SSH 私钥或令牌。"));
        }
        match body.secret.as_deref().map(str::trim) {
            None | Some("") => return Err(ApiError::bad("请填写凭据内容。")),
            Some(secret) if secret.len() > 64 * 1024 => {
                return Err(ApiError::bad("凭据内容过长（上限 64KB）。"));
            }
            Some(_) => {}
        }
    }
    Ok(name.to_owned())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(&s.db, &SPEC, COLUMNS, &scope.workspace_id, &query).await?;
    Ok(Json(json!({
        "items": rows.iter().map(to_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let row = one(
        &s.db,
        &format!("SELECT {COLUMNS} FROM credentials WHERE workspace_id = ? AND id = ?"),
        vec![scope.workspace_id.into(), id.into()],
    )
    .await?
    .ok_or_else(ApiError::missing)?;
    Ok(Json(to_json(&row)))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("credentials.create")?;
    let name = check(&body, true)?;
    let (id, now) = (db::id(), db::now());
    // The row's own identity is the associated data, so the id has to exist
    // before the secret is sealed.
    let sealed = s.vault.seal(
        body.secret.as_deref().unwrap_or_default().trim(),
        &vault::aad(&scope.workspace_id, "credentials", &id),
    )?;
    execute(
        &s.db,
        "INSERT INTO credentials (id,workspace_id,name,kind,username,description,secret,created_at,updated_at,version)
         VALUES (?,?,?,?,?,?,?,?,?,1)",
        vec![
            id.clone().into(),
            scope.workspace_id.clone().into(),
            name.clone().into(),
            body.kind.trim().into(),
            body.username.trim().into(),
            body.description.trim().into(),
            sealed.into(),
            now.into(),
            now.into(),
        ],
    )
    .await?;
    // The details of an audit entry are readable by anyone with activity.read,
    // so they carry the name and never the secret.
    crate::hosts::activity(
        &s,
        &scope,
        "create",
        "credentials",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("credentials.update")?;
    let name = check(&body, false)?;
    let rotated = match body
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(secret) => Some(
            s.vault
                .seal(secret, &vault::aad(&scope.workspace_id, "credentials", &id))?,
        ),
        None => None,
    };
    let affected = match &rotated {
        Some(sealed) => execute(
            &s.db,
            "UPDATE credentials SET name=?,username=?,description=?,secret=?,updated_at=?,version=version+1
             WHERE workspace_id=? AND id=? AND version=?",
            vec![
                name.clone().into(),
                body.username.trim().into(),
                body.description.trim().into(),
                sealed.clone().into(),
                db::now().into(),
                scope.workspace_id.clone().into(),
                id.clone().into(),
                body.version.into(),
            ],
        ),
        None => execute(
            &s.db,
            "UPDATE credentials SET name=?,username=?,description=?,updated_at=?,version=version+1
             WHERE workspace_id=? AND id=? AND version=?",
            vec![
                name.clone().into(),
                body.username.trim().into(),
                body.description.trim().into(),
                db::now().into(),
                scope.workspace_id.clone().into(),
                id.clone().into(),
                body.version.into(),
            ],
        ),
    }
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(crate::hosts::stale(&s, "credentials", &scope.workspace_id, &id).await?);
    }
    crate::hosts::activity(
        &s,
        &scope,
        "update",
        "credentials",
        &id,
        json!({"name": name, "secret_rotated": rotated.is_some()}),
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("credentials.delete")?;
    // Refuse rather than leave anything pointing at a credential that is gone:
    // the failure would surface later as an unexplained connection error, far
    // from the deletion that caused it. Every table that can reference a
    // credential has to be listed here — a new one that is forgotten silently
    // reintroduces the dangling reference.
    for (table, noun) in [("hosts", "台主机"), ("repositories", "个仓库")] {
        let used = one(
            &s.db,
            &format!(
                "SELECT COUNT(*) AS total FROM {table} WHERE workspace_id = ? AND credential_id = ?"
            ),
            vec![scope.workspace_id.clone().into(), id.clone().into()],
        )
        .await?
        .map(|row| db::int(&row, "total"))
        .unwrap_or(0);
        if used > 0 {
            return Err(ApiError::conflict(format!(
                "还有 {used} {noun}在用这条凭据，请先改掉再删除。"
            )));
        }
    }
    let affected = execute(
        &s.db,
        "DELETE FROM credentials WHERE workspace_id = ? AND id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::missing());
    }
    crate::hosts::activity(&s, &scope, "delete", "credentials", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Decrypts a credential for the server's own use. There is no HTTP route to
/// this; the caller is a channel that is about to connect somewhere.
pub async fn unseal(
    s: &AppState,
    workspace_id: &str,
    id: &str,
) -> Result<(String, String, String)> {
    let row = one(
        &s.db,
        "SELECT kind, username, secret FROM credentials WHERE workspace_id = ? AND id = ?",
        vec![workspace_id.into(), id.into()],
    )
    .await?
    .ok_or_else(|| ApiError::bad("这台主机引用的凭据已不存在，请重新选择。"))?;
    let secret = s.vault.open_secret(
        &text(&row, "secret"),
        &vault::aad(workspace_id, "credentials", id),
    )?;
    Ok((text(&row, "kind"), text(&row, "username"), secret))
}
