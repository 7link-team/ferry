//! 主机组 / 主机 / 标签. Three resources, one shape: the list goes through
//! `resource::list`, writes go through `version` optimistic locking, and every
//! mutation lands in `activity`.

use crate::entities::{activity, host_groups, hosts, pipelines, tags};
use crate::snowflake::Id;

use crate::{
    db,
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
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

// ---------------------------------------------------------------- 主机组

const GROUP_SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &["parent_id"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

fn group_json(row: &host_groups::Model) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "description": row.description,
        "parent_id": Id::from(row.parent_id),
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
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
        host_groups::Entity::find(),
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("host-groups.read")?;
    let row = host_groups::Entity::find()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::Id.eq(id))
        .one(&s.db)
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
    let (id, now) = (s.next_id()?, db::now());
    host_groups::Entity::insert(host_groups::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        description: Set(body.description.trim().to_owned()),
        parent_id: Set(i64::from(Id::reference(&body.parent_id)?)),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("host-groups.update")?;
    let name = check_name(&body.name)?;
    if Id::reference(&body.parent_id)? == id {
        return Err(ApiError::bad("主机组不能把自己作为上级。"));
    }
    let affected = host_groups::Entity::update_many()
        .col_expr(host_groups::Column::Name, Expr::value(name.clone()))
        .col_expr(
            host_groups::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(
            host_groups::Column::ParentId,
            Expr::value(Id::reference(&body.parent_id)?),
        )
        .col_expr(host_groups::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            host_groups::Column::Version,
            Expr::col(host_groups::Column::Version).add(1),
        )
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::Id.eq(id))
        .filter(host_groups::Column::Version.eq(body.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(stale(
            &s,
            host_groups::Entity::find()
                .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(host_groups::Column::Id.eq(id)),
        )
        .await?);
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("host-groups.delete")?;
    // Refuse rather than orphan: a host pointing at a deleted group would
    // disappear from every group filter without being gone.
    let used = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::GroupId.eq(id))
        .count(&s.db)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "该主机组下还有 {used} 台主机，请先移出再删除。"
        )));
    }
    let affected = host_groups::Entity::delete_many()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::Id.eq(id))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "host-groups", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ 主机

const HOST_SPEC: Spec = Spec {
    search: &["name", "address", "username"],
    filters: &["group_id", "connection", "state"],
    sortable: &["name", "address", "state", "seen_at", "created_at"],
    default_sort: "name",
};

pub const CONNECTIONS: [&str; 2] = ["runner", "ssh"];
pub const STATES: [&str; 3] = ["online", "offline", "unknown"];

fn host_json(row: &hosts::Model) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "address": row.address,
        "port": row.port,
        "username": row.username,
        "connection": row.connection,
        "state": row.state,
        "group_id": Id::from(row.group_id),
        "credential_id": Id::from(row.credential_id),
        "description": row.description,
        "seen_at": row.seen_at,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
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
    credential_id: String,
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
    let (rows, total) = resource::list(
        &s.db,
        &HOST_SPEC,
        hosts::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.read")?;
    let row = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.eq(id))
        .one(&s.db)
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
    let (id, now) = (s.next_id()?, db::now());
    hosts::Entity::insert(hosts::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        address: Set(body.address.trim().to_owned()),
        port: Set(body.port),
        username: Set(body.username.trim().to_owned()),
        connection: Set(connection.clone()),
        state: Set("unknown".to_owned()),
        group_id: Set(i64::from(Id::reference(&body.group_id)?)),
        credential_id: Set(i64::from(Id::reference(&body.credential_id)?)),
        description: Set(body.description.trim().to_owned()),
        seen_at: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.update")?;
    let (name, _) = check_host(&body, false)?;
    // `connection` is deliberately absent: changing how an enrolled host is
    // reached would invalidate its key or its stored credential silently.
    let affected = hosts::Entity::update_many()
        .col_expr(hosts::Column::Name, Expr::value(name.clone()))
        .col_expr(hosts::Column::Address, Expr::value(body.address.trim()))
        .col_expr(hosts::Column::Port, Expr::value(body.port))
        .col_expr(hosts::Column::Username, Expr::value(body.username.trim()))
        .col_expr(
            hosts::Column::GroupId,
            Expr::value(Id::reference(&body.group_id)?),
        )
        .col_expr(
            hosts::Column::CredentialId,
            Expr::value(Id::reference(&body.credential_id)?),
        )
        .col_expr(
            hosts::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(hosts::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            hosts::Column::Version,
            Expr::col(hosts::Column::Version).add(1),
        )
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.eq(id))
        .filter(hosts::Column::Version.eq(body.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(stale(
            &s,
            hosts::Entity::find()
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(hosts::Column::Id.eq(id)),
        )
        .await?);
    }
    activity(&s, &scope, "update", "hosts", &id, json!({"name": name})).await?;
    Ok(Json(json!({"ok": true})))
}

/// Refuses to delete a host that something still points at.
///
/// Both the single and the batch delete route through here: putting the guard
/// only on the one the report mentions leaves the sibling path still able to
/// orphan a pipeline's target.
async fn host_is_free(
    conn: &impl sea_orm::ConnectionTrait,
    workspace_id: &Id,
    id: &Id,
) -> Result<()> {
    let used = pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(*workspace_id))
        .filter(pipelines::Column::TargetKind.eq("host"))
        .filter(pipelines::Column::TargetRef.eq(*id))
        .count(conn)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "还有 {used} 条流水线以这台主机为目标，请先改掉再删除。"
        )));
    }
    Ok(())
}

pub async fn delete_host(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("hosts.delete")?;
    host_is_free(&s.db, &scope.workspace_id, &id).await?;
    let affected = hosts::Entity::delete_many()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.eq(id))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "hosts", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct BulkBody {
    ids: Vec<Id>,
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
        // Inside the transaction, so a pipeline created concurrently either
        // blocks this delete or is not yet visible to it — and a refusal rolls
        // back every delete in the batch rather than leaving it half applied.
        host_is_free(&transaction, &scope.workspace_id, id).await?;
        deleted += hosts::Entity::delete_many()
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Id.eq(id))
            .exec(&transaction)
            .await?
            .rows_affected;
    }
    transaction.commit().await?;
    activity(
        &s,
        &scope,
        "bulk-delete",
        "hosts",
        &Id::from(0),
        json!({"ids": body.ids, "deleted": deleted}),
    )
    .await?;
    Ok(Json(json!({"deleted": deleted})))
}

// ------------------------------------------------------------------ 标签

const TAG_SPEC: Spec = Spec {
    search: &["name"],
    filters: &["color"],
    sortable: &["name", "created_at"],
    default_sort: "name",
};

fn tag_json(row: &tags::Model) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "color": row.color,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
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
    let (rows, total) = resource::list(
        &s.db,
        &TAG_SPEC,
        tags::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("tags.read")?;
    let row = tags::Entity::find()
        .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(tags::Column::Id.eq(id))
        .one(&s.db)
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
    let (id, now) = (s.next_id()?, db::now());
    tags::Entity::insert(tags::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        color: Set(body.color.trim().to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
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
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("tags.update")?;
    let name = check_name(&body.name)?;
    let affected = tags::Entity::update_many()
        .col_expr(tags::Column::Name, Expr::value(name.clone()))
        .col_expr(tags::Column::Color, Expr::value(body.color.trim()))
        .col_expr(tags::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            tags::Column::Version,
            Expr::col(tags::Column::Version).add(1),
        )
        .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(tags::Column::Id.eq(id))
        .filter(tags::Column::Version.eq(body.version))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(stale(
            &s,
            tags::Entity::find()
                .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(tags::Column::Id.eq(id)),
        )
        .await?);
    }
    activity(&s, &scope, "update", "tags", &id, json!({"name": name})).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_tag(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("tags.delete")?;
    let affected = tags::Entity::delete_many()
        .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(tags::Column::Id.eq(id))
        .exec(&s.db)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity(&s, &scope, "delete", "tags", &id, json!({})).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- 共用

/// A zero-row update means the row is gone or someone else saved first. Tell
/// those apart, because "refresh and retry" is useless advice for a deleted row.
pub async fn stale<E: EntityTrait>(s: &AppState, query: sea_orm::Select<E>) -> Result<ApiError>
where
    E::Model: Sync,
{
    let exists = query.count(&s.db).await? > 0;
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
    resource_id: &Id,
    details: Value,
) -> Result<()> {
    activity_in(s, &s.db, scope, action, resource, resource_id, details).await
}

pub(crate) async fn activity_in(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    action: &str,
    resource: &str,
    resource_id: &Id,
    details: Value,
) -> Result<()> {
    activity::Entity::insert(activity::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        actor_id: Set(i64::from(scope.user.id)),
        actor_name: Set(scope.user.name.clone()),
        action: Set(action.to_owned()),
        resource: Set(resource.to_owned()),
        resource_id: Set(i64::from(*resource_id)),
        details: Set(details.to_string()),
        created_at: Set(db::now()),
        ..Default::default()
    })
    .exec(conn)
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
        search: &["actor_name", "resource", "action"],
        filters: &["resource", "action", "actor_id"],
        sortable: &["created_at"],
        default_sort: "-created_at",
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<activity::Entity, activity::Model>(
        &s.db,
        &SPEC,
        activity::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": Id::from(row.id),
                "actor_name": row.actor_name,
                "action": row.action,
                "resource": row.resource,
                "resource_id": Id::from(row.resource_id),
                "details": serde_json::from_str::<Value>(&row.details).unwrap_or(json!({})),
                "created_at": row.created_at,
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
    for (name, total) in [
        (
            "hosts",
            hosts::Entity::find()
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .count(&s.db)
                .await?,
        ),
        (
            "host-groups",
            host_groups::Entity::find()
                .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                .count(&s.db)
                .await?,
        ),
        (
            "tags",
            tags::Entity::find()
                .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
                .count(&s.db)
                .await?,
        ),
    ] {
        counts.insert(name.into(), json!(total));
    }
    let rows = hosts::Entity::find()
        .select_only()
        .column(hosts::Column::State)
        .column_as(hosts::Column::Id.count(), "total")
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .group_by(hosts::Column::State)
        .into_tuple::<(String, i64)>()
        .all(&s.db)
        .await?;
    let mut states = serde_json::Map::new();
    for state in STATES {
        states.insert(state.into(), json!(0));
    }
    for (state, total) in rows {
        states.insert(state, json!(total));
    }
    Ok(Json(json!({"counts": counts, "states": {"hosts": states}})))
}
