//! 主机组 / 主机 / 标签. Three resources, one shape: the list goes through
//! `resource::list`, writes go through `version` optimistic locking, and every
//! mutation lands in `activity`.

use crate::entities::{activity, host_groups, host_tags, hosts, pipelines, tags};
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
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------- 主机组

const GROUP_SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &["parent_id"],
    sortable: &["name", "position", "created_at", "updated_at"],
    default_sort: "position",
};

fn group_json(row: &host_groups::Model) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "description": row.description,
        "parent_id": Id::from(row.parent_id),
        "position": row.position,
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_group_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_group_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: GroupBody,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("host-groups.create")?;
    let name = check_name(&body.name)?;
    let (id, now) = (s.next_id()?, db::now());
    validate_parent(tx, scope, id, &body.parent_id).await?;
    let parent_id = Id::reference(&body.parent_id)?;
    let position = host_groups::Entity::find()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::ParentId.eq(parent_id))
        .order_by_desc(host_groups::Column::Position)
        .one(tx)
        .await?
        .map(|group| group.position + 1)
        .unwrap_or(0);
    host_groups::Entity::insert(host_groups::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        description: Set(body.description.trim().to_owned()),
        parent_id: Set(i64::from(parent_id)),
        position: Set(position),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(tx)
    .await?;
    activity_in(
        &s,
        tx,
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    validate_parent(&tx, &scope, id, &body.parent_id).await?;
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
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        let exists = host_groups::Entity::find_by_id(i64::from(id))
            .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&tx)
            .await?
            .is_some();
        return Err(if exists {
            ApiError::conflict("这条记录已被他人修改，请刷新后重试。")
        } else {
            ApiError::missing()
        });
    }
    activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "host-groups",
        &id,
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct ReorderGroupsBody {
    parent_id: String,
    ids: Vec<String>,
}

pub async fn reorder_groups(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<ReorderGroupsBody>,
) -> Result<Json<Value>> {
    scope.require("host-groups.update")?;
    let parent_id = Id::reference(&body.parent_id)?;
    let ids = body
        .ids
        .iter()
        .map(|id| Id::reference(id))
        .collect::<Result<Vec<_>>>()?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let siblings = host_groups::Entity::find()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::ParentId.eq(parent_id))
        .all(&tx)
        .await?;
    let expected: HashSet<i64> = siblings.iter().map(|group| group.id).collect();
    let actual: HashSet<i64> = ids.iter().map(|id| i64::from(*id)).collect();
    if actual != expected || ids.len() != siblings.len() {
        return Err(ApiError::bad("分组顺序已变化，请刷新后重试。"));
    }
    for (position, id) in ids.iter().enumerate() {
        host_groups::Entity::update_many()
            .col_expr(host_groups::Column::Position, Expr::value(position as i64))
            .col_expr(
                host_groups::Column::Version,
                Expr::col(host_groups::Column::Version).add(1),
            )
            .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(host_groups::Column::Id.eq(*id))
            .exec(&tx)
            .await?;
    }
    activity_in(
        &s,
        &tx,
        &scope,
        "reorder",
        "host-groups",
        &parent_id,
        json!({"ids": body.ids}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_group(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("host-groups.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    crate::cron_jobs::ensure_unreferenced(&tx, scope.workspace_id, "host-group", &[id]).await?;
    if crate::entities::deployments::Entity::find()
        .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::deployments::Column::TargetKind.eq("host-group"))
        .filter(crate::entities::deployments::Column::TargetRef.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::conflict("主机组仍被应用部署引用。"));
    }

    crate::pipeline_contexts::ensure_unreferenced(&tx, scope.workspace_id, "host-group", &[id])
        .await?;
    crate::actions::ensure_target_unreferenced(&tx, scope.workspace_id, "host-group", id).await?;
    if pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipelines::Column::TargetKind.eq("host-group"))
        .filter(pipelines::Column::TargetRef.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::conflict(
            "主机组仍被流水线引用，请先修改流水线目标。",
        ));
    }
    // Refuse rather than orphan: a host pointing at a deleted group would
    // disappear from every group filter without being gone.
    let used = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::GroupId.eq(id))
        .count(&tx)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "该主机组下还有 {used} 台主机，请先移出再删除。"
        )));
    }
    if host_groups::Entity::find()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::ParentId.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::conflict("主机组下还有子组，请先移动或删除子组。"));
    }
    let affected = host_groups::Entity::delete_many()
        .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(host_groups::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    tx.commit().await?;
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
    tag_ids: Option<Vec<Id>>,
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
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    let tag_filter = raw.remove("tag_id");
    let query = ListQuery::parse(&HOST_SPEC, &raw)?;
    let mut select = hosts::Entity::find();
    if let Some(tag) = tag_filter.as_ref().filter(|v| !v.is_empty()) {
        let tag = Id::reference(tag)?;
        select = select.filter(
            hosts::Column::Id.in_subquery(
                sea_orm::sea_query::Query::select()
                    .column(host_tags::Column::HostId)
                    .from(host_tags::Entity)
                    .and_where(Expr::col(host_tags::Column::TagId).eq(i64::from(tag)))
                    .to_owned(),
            ),
        );
    }
    let (rows, total) =
        resource::list(&s.db, &HOST_SPEC, select, &scope.workspace_id, &query).await?;
    Ok(Json(json!({
        "items": hosts_with_tags(&s.db,scope.workspace_id,&rows).await?,
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
    Ok(Json(
        hosts_with_tags(&s.db, scope.workspace_id, &[row])
            .await?
            .remove(0),
    ))
}

async fn membership_transaction(
    s: &AppState,
    scope: &Scope,
    group: &str,
) -> Result<sea_orm::DatabaseTransaction> {
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    validate_membership(&tx, scope, group).await?;
    Ok(tx)
}
async fn validate_membership(
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    group: &str,
) -> Result<()> {
    let id = Id::reference(group)?;
    if i64::from(id) != 0 {
        host_groups::Entity::find_by_id(i64::from(id))
            .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
            .one(tx)
            .await?
            .ok_or_else(|| ApiError::bad("主机组不存在于当前工作空间。"))?;
    }
    Ok(())
}

pub async fn create_host(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<HostBody>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_host_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_host_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: HostBody,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.create")?;
    let (name, connection) = check_host(&body, true)?;
    let (id, now) = (s.next_id()?, db::now());
    validate_membership(tx, scope, &body.group_id).await?;
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
    .exec(tx)
    .await?;
    replace_tags(tx, scope, id, body.tag_ids.as_deref()).await?;
    activity_in(
        &s,
        tx,
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
    let tx = membership_transaction(&s, &scope, &body.group_id).await?;
    let existing = hosts::Entity::find_by_id(i64::from(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut update = hosts::Entity::update_many()
        .col_expr(hosts::Column::Name, Expr::value(name.clone()))
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
        );
    if existing.connection == "ssh" {
        update = update.col_expr(hosts::Column::Address, Expr::value(body.address.trim()));
    }
    let affected = update
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.eq(id))
        .filter(hosts::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        let exists = hosts::Entity::find_by_id(i64::from(id))
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&tx)
            .await?
            .is_some();
        return Err(if exists {
            ApiError::conflict("这条记录已被他人修改，请刷新后重试。")
        } else {
            ApiError::missing()
        });
    }
    replace_tags(&tx, &scope, id, body.tag_ids.as_deref()).await?;
    activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "hosts",
        &id,
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
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
    crate::cron_jobs::ensure_unreferenced(conn, *workspace_id, "host", &[*id]).await?;
    crate::alerts::ensure_unreferenced(conn, *workspace_id, "host", &[*id]).await?;
    use crate::entities::{deployment_revision_targets, deployment_target_states, deployments};
    if deployments::Entity::find()
        .filter(deployments::Column::WorkspaceId.eq(*workspace_id))
        .filter(deployments::Column::TargetKind.eq("host"))
        .filter(deployments::Column::TargetRef.eq(*id))
        .count(conn)
        .await?
        > 0
        || deployment_target_states::Entity::find()
            .filter(deployment_target_states::Column::WorkspaceId.eq(*workspace_id))
            .filter(deployment_target_states::Column::HostId.eq(*id))
            .count(conn)
            .await?
            > 0
        || deployment_revision_targets::Entity::find()
            .filter(deployment_revision_targets::Column::WorkspaceId.eq(*workspace_id))
            .filter(deployment_revision_targets::Column::HostId.eq(*id))
            .filter(deployment_revision_targets::Column::State.is_in([
                "pending",
                "running",
                "needs-review",
            ]))
            .count(conn)
            .await?
            > 0
    {
        return Err(ApiError::conflict(
            "主机仍被应用或待执行/未确认的部署目标引用。",
        ));
    }

    crate::pipeline_contexts::ensure_unreferenced(conn, *workspace_id, "host", &[*id]).await?;
    crate::actions::ensure_target_unreferenced(conn, *workspace_id, "host", *id).await?;
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    host_is_free(&tx, &scope.workspace_id, &id).await?;
    let affected = hosts::Entity::delete_many()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    host_tags::Entity::delete_many()
        .filter(host_tags::Column::HostId.eq(id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
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
    let transaction = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut deleted = 0;
    for id in &body.ids {
        // Inside the transaction, so a pipeline created concurrently either
        // blocks this delete or is not yet visible to it — and a refusal rolls
        // back every delete in the batch rather than leaving it half applied.
        host_is_free(&transaction, &scope.workspace_id, id).await?;
        let affected = hosts::Entity::delete_many()
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Id.eq(id))
            .exec(&transaction)
            .await?
            .rows_affected;
        if affected > 0 {
            host_tags::Entity::delete_many()
                .filter(host_tags::Column::HostId.eq(*id))
                .exec(&transaction)
                .await?;
        }
        deleted += affected;
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_tag_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_tag_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: TagBody,
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
    .exec(tx)
    .await?;
    activity_in(&s, tx, &scope, "create", "tags", &id, json!({"name": name})).await?;
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
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    tags::Entity::find_by_id(i64::from(id))
        .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    crate::pipeline_contexts::ensure_unreferenced(&tx, scope.workspace_id, "tag", &[id]).await?;
    if host_tags::Entity::find()
        .filter(host_tags::Column::TagId.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::conflict(
            "标签仍被主机引用，请先在主机中移除标签。",
        ));
    }
    let affected = tags::Entity::delete_many()
        .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(tags::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    activity_in(&s, &tx, &scope, "delete", "tags", &id, json!({})).await?;
    tx.commit().await?;
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
    activity_record(
        s,
        conn,
        scope.workspace_id,
        (scope.user.id.into(), &scope.user.name),
        action,
        resource,
        *resource_id,
        details,
    )
    .await
}

pub(crate) async fn activity_record(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    actor: (Id, &str),
    action: &str,
    resource: &str,
    resource_id: Id,
    details: Value,
) -> Result<()> {
    activity::Entity::insert(activity::ActiveModel {
        id: Set(i64::from(s.next_id()?)),
        workspace_id: Set(i64::from(workspace)),
        actor_id: Set(i64::from(actor.0)),
        actor_name: Set(actor.1.to_owned()),
        action: Set(action.to_owned()),
        resource: Set(resource.to_owned()),
        resource_id: Set(i64::from(resource_id)),
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
    use crate::entities::{
        alert_events, alert_rules, credentials, cron_jobs, cron_runs, deployment_revisions,
        deployments, pipeline_runs, repositories, workspace_members, workspace_roles,
    };
    use sea_orm::QueryOrder;
    let mut counts = serde_json::Map::new();
    let mut states = serde_json::Map::new();
    let mut failures = serde_json::Map::new();
    macro_rules! count {
        ($entity:ident, $key:literal, $permission:literal) => {
            if scope.permissions.contains($permission) {
                counts.insert(
                    $key.into(),
                    json!(
                        $entity::Entity::find()
                            .filter($entity::Column::WorkspaceId.eq(scope.workspace_id))
                            .count(&s.db)
                            .await?
                    ),
                );
            }
        };
    }
    count!(alert_rules, "alert-rules", "alerts.read");
    count!(cron_jobs, "cron-jobs", "cron-jobs.read");
    count!(hosts, "hosts", "hosts.read");
    count!(host_groups, "host-groups", "host-groups.read");
    count!(tags, "tags", "tags.read");
    count!(credentials, "credentials", "credentials.read");
    count!(repositories, "repositories", "repositories.read");
    count!(pipelines, "pipelines", "pipelines.read");
    count!(deployments, "deployments", "deployments.read");
    count!(workspace_members, "members", "members.read");
    count!(workspace_roles, "roles", "roles.read");
    macro_rules! distribution {
        ($entity:ident, $key:literal, $permission:literal) => {
            if scope.permissions.contains($permission) {
                let rows = $entity::Entity::find()
                    .select_only()
                    .column($entity::Column::State)
                    .column_as($entity::Column::Id.count(), "total")
                    .filter($entity::Column::WorkspaceId.eq(scope.workspace_id))
                    .group_by($entity::Column::State)
                    .into_tuple::<(String, i64)>()
                    .all(&s.db)
                    .await?;
                states.insert(
                    $key.into(),
                    json!(
                        rows.into_iter()
                            .collect::<std::collections::BTreeMap<_, _>>()
                    ),
                );
            }
        };
    }
    distribution!(alert_events, "alert-events", "alerts.read");
    distribution!(cron_runs, "cron-runs", "cron-jobs.read");
    distribution!(hosts, "hosts", "hosts.read");
    if let Some(Value::Object(host_states)) = states.get_mut("hosts") {
        for state in STATES {
            host_states.entry(state).or_insert(json!(0));
        }
    }
    distribution!(deployments, "deployments", "deployments.read");
    distribution!(pipeline_runs, "pipeline-runs", "pipelines.read");
    let now = db::now();
    macro_rules! failed {
        ($entity:ident, $key:literal, $permission:literal) => {
            if scope.permissions.contains($permission) {
                failures.insert(
                    $key.into(),
                    json!(
                        $entity::Entity::find()
                            .filter($entity::Column::WorkspaceId.eq(scope.workspace_id))
                            .filter($entity::Column::State.eq("failed"))
                            .filter($entity::Column::CreatedAt.gte(now - 86_400_000))
                            .count(&s.db)
                            .await?
                    ),
                );
            }
        };
    }
    failed!(pipeline_runs, "pipeline-runs", "pipelines.read");
    failed!(
        deployment_revisions,
        "deployment-revisions",
        "deployments.read"
    );
    let runners = if scope.permissions.contains("hosts.read") {
        Some(hosts::Entity::find().select_only().columns([hosts::Column::Id,hosts::Column::Name,hosts::Column::SeenAt])
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id)).filter(hosts::Column::Connection.eq("runner"))
            .order_by_asc(hosts::Column::SeenAt).limit(10).into_tuple::<(i64,String,i64)>().all(&s.db).await?
            .into_iter().map(|(id,name,seen)| json!({"id":Id::from(id),"name":name,"seen_at": if seen > 0 {Some(seen)} else {None}, "lag_ms": if seen > 0 {Some(std::cmp::max(now-seen,0))} else {None}})).collect::<Vec<_>>())
    } else {
        None
    };
    let recent = if scope.permissions.contains("activity.read") {
        Some(activity::Entity::find().filter(activity::Column::WorkspaceId.eq(scope.workspace_id))
            .order_by_desc(activity::Column::CreatedAt).order_by_desc(activity::Column::Id).limit(10).all(&s.db).await?
            .into_iter().map(|a| json!({"id":Id::from(a.id),"actor_name":a.actor_name,"action":a.action,"resource":a.resource,"created_at":a.created_at})).collect::<Vec<_>>())
    } else {
        None
    };
    Ok(Json(
        json!({"counts":counts,"states":states,"failures_24h":failures,"runners":runners,"recent":recent,"as_of":now}),
    ))
}

async fn validate_parent(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    id: Id,
    parent: &str,
) -> Result<()> {
    let mut cursor = Id::reference(parent)?;
    let mut seen = HashSet::new();
    while cursor != Id::from(0) {
        if cursor == id || !seen.insert(i64::from(cursor)) {
            return Err(ApiError::bad("上级主机组不能是自己或自己的子组。"));
        }
        let row = host_groups::Entity::find_by_id(i64::from(cursor))
            .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| ApiError::bad("上级主机组不存在于当前工作空间。"))?;
        cursor = row.parent_id.into();
    }
    Ok(())
}
async fn replace_tags(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    host: Id,
    values: Option<&[Id]>,
) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };
    let ids: HashSet<i64> = values.iter().map(|id| i64::from(*id)).collect();
    if ids.len() != values.len() {
        return Err(ApiError::bad("标签不能重复。"));
    }
    if !ids.is_empty()
        && tags::Entity::find()
            .filter(tags::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(tags::Column::Id.is_in(ids.iter().copied()))
            .count(conn)
            .await?
            != ids.len() as u64
    {
        return Err(ApiError::bad("所选标签不存在于当前工作空间。"));
    }
    host_tags::Entity::delete_many()
        .filter(host_tags::Column::HostId.eq(host))
        .exec(conn)
        .await?;
    if !ids.is_empty() {
        host_tags::Entity::insert_many(ids.into_iter().map(|tag| host_tags::ActiveModel {
            host_id: Set(host.into()),
            tag_id: Set(tag),
        }))
        .exec(conn)
        .await?;
    }
    Ok(())
}
async fn hosts_with_tags(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    rows: &[hosts::Model],
) -> Result<Vec<Value>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let links = host_tags::Entity::find()
        .filter(host_tags::Column::HostId.is_in(rows.iter().map(|r| r.id)))
        .all(conn)
        .await?;
    let ids: HashSet<i64> = links.iter().map(|r| r.tag_id).collect();
    let labels = if ids.is_empty() {
        Vec::new()
    } else {
        tags::Entity::find()
            .filter(tags::Column::WorkspaceId.eq(workspace))
            .filter(tags::Column::Id.is_in(ids))
            .all(conn)
            .await?
    };
    let labels: HashMap<i64, Value> = labels
        .iter()
        .map(|r| {
            (
                r.id,
                json!({"id":Id::from(r.id),"name":r.name,"color":r.color}),
            )
        })
        .collect();
    Ok(rows
        .iter()
        .map(|row| {
            let mut result = host_json(row);
            let mut ids: Vec<i64> = links
                .iter()
                .filter(|r| r.host_id == row.id && labels.contains_key(&r.tag_id))
                .map(|r| r.tag_id)
                .collect();
            ids.sort_unstable();
            result["tags"] = json!(
                ids.iter()
                    .filter_map(|id| labels.get(id))
                    .collect::<Vec<_>>()
            );
            result["tag_ids"] = json!(ids.into_iter().map(Id::from).collect::<Vec<_>>());
            result
        })
        .collect())
}
