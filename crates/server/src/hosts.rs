//! 主机组 / 主机 / 标签. Three resources, one shape: the list goes through
//! `resource::list`, writes go through `version` optimistic locking, and every
//! mutation lands in `activity`.

use crate::{
    db::{self, execute, one, text},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{QueryResult, TransactionTrait};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

// ---------------------------------------------------------------- 主机组

const GROUP_SPEC: Spec = Spec {
    table: "host_groups",
    search: &["name", "description"],
    filters: &["parent_id"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};
const GROUP_COLUMNS: &str = "id,name,description,parent_id,created_at,updated_at,version";

fn group_json(row: &QueryResult) -> Value {
    json!({
        "id": text(row, "id"),
        "name": text(row, "name"),
        "description": text(row, "description"),
        "parent_id": text(row, "parent_id"),
        "created_at": db::int(row, "created_at"),
        "updated_at": db::int(row, "updated_at"),
        "version": db::int(row, "version"),
    })
}

#[derive(Deserialize)]
pub struct GroupBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parent_id: String,
    #[serde(default)]
    version: i64,
}

pub async fn list_groups(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("host-groups.read")?;
    let query = ListQuery::parse(&GROUP_SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &GROUP_SPEC,
        GROUP_COLUMNS,
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(json!({
        "items": rows.iter().map(group_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get_group(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("host-groups.read")?;
    let row = one(
        &s.db,
        &format!("SELECT {GROUP_COLUMNS} FROM host_groups WHERE workspace_id = ? AND id = ?"),
        vec![scope.workspace_id.into(), id.into()],
    )
    .await?
    .ok_or_else(ApiError::missing)?;
    Ok(Json(group_json(&row)))
}

fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写名称（60 字以内）。"));
    }
    Ok(name.to_owned())
}

pub async fn create_group(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<GroupBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("host-groups.create")?;
    let name = check_name(&body.name)?;
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO host_groups (id,workspace_id,name,description,parent_id,created_at,updated_at,version)
         VALUES (?,?,?,?,?,?,?,1)",
        vec![
            id.clone().into(),
            scope.workspace_id.clone().into(),
            name.clone().into(),
            body.description.trim().into(),
            body.parent_id.clone().into(),
            now.into(),
            now.into(),
        ],
    )
    .await?;
    activity(
        &s,
        &scope,
        "create",
        "host-groups",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update_group(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<GroupBody>,
) -> Result<Json<Value>> {
    scope.require("host-groups.update")?;
    let name = check_name(&body.name)?;
    if body.parent_id == id {
        return Err(ApiError::bad("主机组不能把自己作为上级。"));
    }
    let affected = execute(
        &s.db,
        "UPDATE host_groups SET name=?,description=?,parent_id=?,updated_at=?,version=version+1
         WHERE workspace_id=? AND id=? AND version=?",
        vec![
            name.clone().into(),
            body.description.trim().into(),
            body.parent_id.into(),
            db::now().into(),
            scope.workspace_id.clone().into(),
            id.clone().into(),
            body.version.into(),
        ],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(stale(&s, "host_groups", &scope.workspace_id, &id).await?);
    }
    activity(
        &s,
        &scope,
        "update",
        "host-groups",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_group(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("host-groups.delete")?;
    // Refuse rather than orphan: a host pointing at a deleted group would
    // disappear from every group filter without being gone.
    let used = one(
        &s.db,
        "SELECT COUNT(*) AS total FROM hosts WHERE workspace_id = ? AND group_id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .map(|row| db::int(&row, "total"))
    .unwrap_or(0);
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "该主机组下还有 {used} 台主机，请先移出再删除。"
        )));
    }
    let affected = execute(
        &s.db,
        "DELETE FROM host_groups WHERE workspace_id = ? AND id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "host-groups", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ 主机

const HOST_SPEC: Spec = Spec {
    table: "hosts",
    search: &["name", "address", "username"],
    filters: &["group_id", "connection", "state"],
    sortable: &["name", "address", "state", "seen_at", "created_at"],
    default_sort: "name",
};
const HOST_COLUMNS: &str = "id,name,address,port,username,connection,state,group_id,description,seen_at,created_at,updated_at,version";

pub const CONNECTIONS: [&str; 2] = ["runner", "ssh"];
pub const STATES: [&str; 3] = ["online", "offline", "unknown"];

fn host_json(row: &QueryResult) -> Value {
    json!({
        "id": text(row, "id"),
        "name": text(row, "name"),
        "address": text(row, "address"),
        "port": db::int(row, "port"),
        "username": text(row, "username"),
        "connection": text(row, "connection"),
        "state": text(row, "state"),
        "group_id": text(row, "group_id"),
        "description": text(row, "description"),
        "seen_at": db::int(row, "seen_at"),
        "created_at": db::int(row, "created_at"),
        "updated_at": db::int(row, "updated_at"),
        "version": db::int(row, "version"),
    })
}

#[derive(Deserialize)]
pub struct HostBody {
    name: String,
    #[serde(default)]
    connection: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    port: i64,
    #[serde(default)]
    username: String,
    #[serde(default)]
    group_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: i64,
}

/// SSH needs somewhere to dial; a runner host reports its own address when it
/// enrolls, so requiring one up front would block the one-command flow.
fn check_host(body: &HostBody, creating: bool) -> Result<(String, String)> {
    let name = check_name(&body.name)?;
    let connection = if creating { body.connection.trim() } else { "" };
    if creating {
        if !CONNECTIONS.contains(&connection) {
            return Err(ApiError::bad("请选择接入方式：代理或 SSH。"));
        }
        if connection == "ssh" {
            if body.address.trim().is_empty() {
                return Err(ApiError::bad("SSH 接入需要填写主机地址。"));
            }
            if !(1..=65535).contains(&body.port) {
                return Err(ApiError::bad("端口需要在 1–65535 之间。"));
            }
            if body.username.trim().is_empty() {
                return Err(ApiError::bad("SSH 接入需要填写登录账号。"));
            }
        }
    }
    Ok((name, connection.to_owned()))
}

pub async fn list_hosts(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    let query = ListQuery::parse(&HOST_SPEC, &raw)?;
    let (rows, total) =
        resource::list(&s.db, &HOST_SPEC, HOST_COLUMNS, &scope.workspace_id, &query).await?;
    Ok(Json(json!({
        "items": rows.iter().map(host_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get_host(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    let row = one(
        &s.db,
        &format!("SELECT {HOST_COLUMNS} FROM hosts WHERE workspace_id = ? AND id = ?"),
        vec![scope.workspace_id.into(), id.into()],
    )
    .await?
    .ok_or_else(ApiError::missing)?;
    Ok(Json(host_json(&row)))
}

pub async fn create_host(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<HostBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.create")?;
    let (name, connection) = check_host(&body, true)?;
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO hosts (id,workspace_id,name,address,port,username,connection,state,group_id,description,seen_at,created_at,updated_at,version)
         VALUES (?,?,?,?,?,?,?,'unknown',?,?,0,?,?,1)",
        vec![
            id.clone().into(),
            scope.workspace_id.clone().into(),
            name.clone().into(),
            body.address.trim().into(),
            body.port.into(),
            body.username.trim().into(),
            connection.clone().into(),
            body.group_id.into(),
            body.description.trim().into(),
            now.into(),
            now.into(),
        ],
    )
    .await?;
    activity(
        &s,
        &scope,
        "create",
        "hosts",
        &id,
        json!({"name": name, "connection": connection}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update_host(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<HostBody>,
) -> Result<Json<Value>> {
    scope.require("hosts.update")?;
    let (name, _) = check_host(&body, false)?;
    // `connection` is deliberately absent: changing how an enrolled host is
    // reached would invalidate its key or its stored credential silently.
    let affected = execute(
        &s.db,
        "UPDATE hosts SET name=?,address=?,port=?,username=?,group_id=?,description=?,updated_at=?,version=version+1
         WHERE workspace_id=? AND id=? AND version=?",
        vec![
            name.clone().into(),
            body.address.trim().into(),
            body.port.into(),
            body.username.trim().into(),
            body.group_id.into(),
            body.description.trim().into(),
            db::now().into(),
            scope.workspace_id.clone().into(),
            id.clone().into(),
            body.version.into(),
        ],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(stale(&s, "hosts", &scope.workspace_id, &id).await?);
    }
    activity(&s, &scope, "update", "hosts", &id, json!({"name": name})).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_host(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("hosts.delete")?;
    let affected = execute(
        &s.db,
        "DELETE FROM hosts WHERE workspace_id = ? AND id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "hosts", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct BulkBody {
    ids: Vec<String>,
}

/// Batch delete as an action, not a `DELETE` with a body — proxies drop the
/// body of a DELETE, which turns "delete these three" into "delete nothing".
pub async fn bulk_delete_hosts(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<BulkBody>,
) -> Result<Json<Value>> {
    scope.require("hosts.delete")?;
    if body.ids.is_empty() || body.ids.len() > 200 {
        return Err(ApiError::bad("一次最多删除 200 项，且至少选择一项。"));
    }
    let transaction = s.db.begin().await?;
    let mut deleted = 0;
    for id in &body.ids {
        deleted += crate::db::execute(
            &transaction,
            "DELETE FROM hosts WHERE workspace_id = ? AND id = ?",
            vec![scope.workspace_id.clone().into(), id.clone().into()],
        )
        .await?
        .rows_affected();
    }
    transaction.commit().await?;
    activity(
        &s,
        &scope,
        "bulk-delete",
        "hosts",
        "",
        json!({"ids": body.ids, "deleted": deleted}),
    )
    .await?;
    Ok(Json(json!({"deleted": deleted})))
}

// ------------------------------------------------------------------ 标签

const TAG_SPEC: Spec = Spec {
    table: "tags",
    search: &["name"],
    filters: &["color"],
    sortable: &["name", "created_at"],
    default_sort: "name",
};
const TAG_COLUMNS: &str = "id,name,color,created_at,updated_at,version";

fn tag_json(row: &QueryResult) -> Value {
    json!({
        "id": text(row, "id"),
        "name": text(row, "name"),
        "color": text(row, "color"),
        "created_at": db::int(row, "created_at"),
        "updated_at": db::int(row, "updated_at"),
        "version": db::int(row, "version"),
    })
}

#[derive(Deserialize)]
pub struct TagBody {
    name: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    version: i64,
}

pub async fn list_tags(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("tags.read")?;
    let query = ListQuery::parse(&TAG_SPEC, &raw)?;
    let (rows, total) =
        resource::list(&s.db, &TAG_SPEC, TAG_COLUMNS, &scope.workspace_id, &query).await?;
    Ok(Json(json!({
        "items": rows.iter().map(tag_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get_tag(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("tags.read")?;
    let row = one(
        &s.db,
        &format!("SELECT {TAG_COLUMNS} FROM tags WHERE workspace_id = ? AND id = ?"),
        vec![scope.workspace_id.into(), id.into()],
    )
    .await?
    .ok_or_else(ApiError::missing)?;
    Ok(Json(tag_json(&row)))
}

pub async fn create_tag(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<TagBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("tags.create")?;
    let name = check_name(&body.name)?;
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO tags (id,workspace_id,name,color,created_at,updated_at,version) VALUES (?,?,?,?,?,?,1)",
        vec![
            id.clone().into(),
            scope.workspace_id.clone().into(),
            name.clone().into(),
            body.color.trim().into(),
            now.into(),
            now.into(),
        ],
    )
    .await?;
    activity(&s, &scope, "create", "tags", &id, json!({"name": name})).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update_tag(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<TagBody>,
) -> Result<Json<Value>> {
    scope.require("tags.update")?;
    let name = check_name(&body.name)?;
    let affected = execute(
        &s.db,
        "UPDATE tags SET name=?,color=?,updated_at=?,version=version+1
         WHERE workspace_id=? AND id=? AND version=?",
        vec![
            name.clone().into(),
            body.color.trim().into(),
            db::now().into(),
            scope.workspace_id.clone().into(),
            id.clone().into(),
            body.version.into(),
        ],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(stale(&s, "tags", &scope.workspace_id, &id).await?);
    }
    activity(&s, &scope, "update", "tags", &id, json!({"name": name})).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_tag(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("tags.delete")?;
    let affected = execute(
        &s.db,
        "DELETE FROM tags WHERE workspace_id = ? AND id = ?",
        vec![scope.workspace_id.clone().into(), id.clone().into()],
    )
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "tags", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- 共用

/// A zero-row update means the row is gone or someone else saved first. Tell
/// those apart, because "refresh and retry" is useless advice for a deleted row.
async fn stale(s: &AppState, table: &str, workspace_id: &str, id: &str) -> Result<ApiError> {
    let exists = one(
        s.db(),
        &format!("SELECT id FROM {table} WHERE workspace_id = ? AND id = ?"),
        vec![workspace_id.into(), id.into()],
    )
    .await?
    .is_some();
    Ok(if exists {
        ApiError::conflict("这条记录已被他人修改，请刷新后重试。")
    } else {
        ApiError::missing()
    })
}

pub async fn activity(
    s: &AppState,
    scope: &Scope,
    action: &str,
    resource: &str,
    resource_id: &str,
    details: Value,
) -> Result<()> {
    execute(
        &s.db,
        "INSERT INTO activity (id,workspace_id,actor_id,actor_name,action,resource,resource_id,details,created_at)
         VALUES (?,?,?,?,?,?,?,?,?)",
        vec![
            db::id().into(),
            scope.workspace_id.clone().into(),
            scope.user.id.clone().into(),
            scope.user.name.clone().into(),
            action.into(),
            resource.into(),
            resource_id.into(),
            details.to_string().into(),
            db::now().into(),
        ],
    )
    .await?;
    Ok(())
}

pub async fn list_activity(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("activity.read")?;
    const SPEC: Spec = Spec {
        table: "activity",
        search: &["actor_name", "resource", "action"],
        filters: &["resource", "action", "actor_id"],
        sortable: &["created_at"],
        default_sort: "-created_at",
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &SPEC,
        "id,actor_name,action,resource,resource_id,details,created_at",
        &scope.workspace_id,
        &query,
    )
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": text(row, "id"),
                "actor_name": text(row, "actor_name"),
                "action": text(row, "action"),
                "resource": text(row, "resource"),
                "resource_id": text(row, "resource_id"),
                "details": serde_json::from_str::<Value>(&text(row, "details")).unwrap_or(json!({})),
                "created_at": db::int(row, "created_at"),
            })
        })
        .collect();
    Ok(Json(
        json!({"items": items, "total": total, "page": query.page, "page_size": query.page_size}),
    ))
}

/// Resource counts and state distribution for the dashboard, in one round trip.
pub async fn dashboard(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("dashboard.read")?;
    let mut counts = serde_json::Map::new();
    for table in ["hosts", "host_groups", "tags"] {
        let total = one(
            &s.db,
            &format!("SELECT COUNT(*) AS total FROM {table} WHERE workspace_id = ?"),
            vec![scope.workspace_id.clone().into()],
        )
        .await?
        .map(|row| db::int(&row, "total"))
        .unwrap_or(0);
        counts.insert(table.replace('_', "-"), json!(total));
    }
    let rows = db::all(
        &s.db,
        "SELECT state, COUNT(*) AS total FROM hosts WHERE workspace_id = ? GROUP BY state",
        vec![scope.workspace_id.clone().into()],
    )
    .await?;
    let mut states = serde_json::Map::new();
    for state in STATES {
        states.insert(state.into(), json!(0));
    }
    for row in &rows {
        states.insert(text(row, "state"), json!(db::int(row, "total")));
    }
    Ok(Json(json!({"counts": counts, "states": {"hosts": states}})))
}
