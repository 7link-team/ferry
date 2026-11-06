//! Workflow draft resources. Publication and execution use separate entry points;
//! a draft write never grants production authority or changes an in-flight run.
pub(crate) mod ai;
mod condition;
pub(crate) mod expression;
pub(crate) mod extensions;
pub(crate) mod graph;
mod host;
pub(crate) mod integration;
pub(crate) mod journal;
pub(crate) mod media;
pub(crate) mod plan;
pub(crate) mod publication;
pub(crate) mod run;
mod transform;

use crate::definitions::access::{self, Operation};
use crate::{
    accounts::{workspace::Scope, workspace_admin},
    db, definitions,
    entities::{definition_revisions, workflow_executions, workflows as rows},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    ExprTrait, QuerySelect, QueryTrait, sea_query::Expr,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &["group_id", "lifecycle"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-updated_at",
};

#[derive(Deserialize)]
pub struct Body {
    name: Option<String>,
    group_id: Option<String>,
    description: Option<String>,
    definition: Option<String>,
    message: Option<String>,
    lifecycle: Option<String>,
    #[serde(default)]
    version: i64,
}

#[derive(Deserialize)]
pub struct Version {
    version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    owner_id: Id,
    version: i64,
}

pub async fn transfer(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Transfer>,
) -> Result<Json<Value>> {
    scope.require("workflows.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let current = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &current, Operation::Edit).await?;
    if current.owner_id != Some(scope.user.id.into()) {
        return Err(ApiError::forbidden());
    }
    if current.version != body.version {
        return Err(conflict("execution.versionConflict"));
    }
    let member = crate::entities::workspace_members::Entity::find()
        .filter(crate::entities::workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::workspace_members::Column::UserId.eq(body.owner_id))
        .filter(crate::entities::workspace_members::Column::State.eq("active"))
        .count(&tx)
        .await?;
    if member == 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.aclMember"),
        ));
    }
    let target = Scope::for_actor_in(&tx, scope.workspace_id, body.owner_id).await?;
    access::require(&tx, &target, &current, Operation::Edit).await?;
    let mut row = current.clone();
    row.owner_id = Some(body.owner_id.into());
    row.updated_by = scope.user.id.into();
    row.updated_at = db::now();
    row.version = row
        .version
        .checked_add(1)
        .ok_or_else(|| conflict("execution.versionConflict"))?;
    let changed = rows::Entity::update_many()
        .col_expr(rows::Column::OwnerId, Expr::value(row.owner_id))
        .col_expr(rows::Column::UpdatedBy, Expr::value(row.updated_by))
        .col_expr(rows::Column::UpdatedAt, Expr::value(row.updated_at))
        .col_expr(rows::Column::Version, Expr::value(row.version))
        .filter(rows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(rows::Column::Id.eq(id))
        .filter(rows::Column::OwnerId.eq(scope.user.id))
        .filter(rows::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed != 1 {
        return Err(conflict("execution.versionConflict"));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "transfer",
        "workflows",
        &id,
        json!({"old_owner_id":current.owner_id.map(Id::from),
            "new_owner_id":body.owner_id,"version":row.version}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json(row)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    version: i64,
    node_path: String,
    /// Optional unsaved parameter edits; never persisted by this operation.
    params: Option<Value>,
    data: ferry_protocol::sandbox::InputData,
    selections: Vec<Option<usize>>,
}

/// Pure editor preview over caller-supplied samples. No node executor, asset
/// downloader or credential client is invoked, and no revision/run is created.
pub async fn preview(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Preview>,
) -> Result<Json<Value>> {
    scope.require("workflows.update")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::Edit).await?;
    if row.version != body.version {
        return Err(conflict("execution.versionConflict"));
    }
    let definition: Value =
        serde_json::from_str(&row.definition).map_err(|_| ApiError::missing())?;
    let mut node = graph::node_at(&definition, &body.node_path)
        .ok_or_else(ApiError::missing)?
        .clone();
    if let Some(params) = body.params {
        node["params"] = params;
    }
    let vars = crate::variables::snapshot_for_workflow(&s, &tx, scope.workspace_id).await?;
    let (now, today) = expression::clock(&definition["settings"], db::now())?;
    let context = json!({"$vars":vars,"$node":{"id":node["id"],"name":node["name"],"type":node["type"]},
        "$workflow":{"id":id,"name":row.name},"$execution":{"mode":"preview"},"$now":now,"$today":today});
    tx.commit().await?;
    let items = graph::resolve(
        &node,
        &body.node_path,
        body.data,
        &body.selections,
        context,
        true,
    )
    .await?;
    Ok(Json(
        json!({"total":items.len(),"page":1,"page_size":items.len(),"items":items,"version":row.version}),
    ))
}

fn conflict(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new(key))
}

pub(crate) async fn scoped(
    conn: &impl ConnectionTrait,
    workspace: Id,
    id: Id,
) -> Result<rows::Model> {
    rows::Entity::find_by_id(i64::from(id))
        .filter(rows::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

fn json(row: rows::Model) -> Value {
    json!({"id":Id::from(row.id), "name":row.name, "group_id":Id::from(row.group_id),
        "description":row.description, "definition":row.definition,
        "draft_revision_id":Id::from(row.draft_revision_id),
        "published_revision_id":row.published_revision_id.map(Id::from),
        "owner_id":row.owner_id.map(Id::from), "trigger_run_as":row.trigger_run_as.map(Id::from),
        "lifecycle":row.lifecycle, "archived_at":row.archived_at, "archived_by":Id::from(row.archived_by),
        "created_by":Id::from(row.created_by), "created_at":row.created_at,
        "updated_by":Id::from(row.updated_by), "updated_at":row.updated_at, "version":row.version})
}

fn name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.nameRequired"),
        ));
    }
    Ok(name.into())
}

fn settings(definition: &str) -> String {
    let value: Value = serde_json::from_str(definition).expect("validated graph JSON");
    serde_json::to_string(value.get("settings").unwrap_or(&json!({}))).expect("graph settings JSON")
}

async fn retain_pins(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    workspace: Id,
    revision: &definition_revisions::Model,
) -> Result<()> {
    let definition: Value =
        serde_json::from_str(&revision.definition).map_err(|_| ApiError::missing())?;
    let mut references = Vec::new();
    let mut scopes = vec![&definition];
    while let Some(scope) = scopes.pop() {
        if let Some(pins) = scope.get("pin_data").and_then(Value::as_object) {
            for pin in pins.values() {
                for item in graph::pin_items(pin)? {
                    references.extend(item.binary.into_values());
                }
            }
        }
        if let Some(nodes) = scope.get("nodes").and_then(Value::as_array) {
            for node in nodes.iter().filter(|node| node["type"] == "core.loop") {
                scopes.push(&node["params"]["body"]);
            }
        }
    }
    if references.is_empty() {
        return Ok(());
    }
    references.sort_by(|a, b| a.asset.cmp(&b.asset));
    let backend = crate::artifact_store::ArtifactStore::for_connection(s, tx).await?;
    let store = crate::content_assets::Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    for reference in references {
        store
            .attach_binary(tx, workspace.into(), &reference, "pin", revision.id)
            .await?;
    }
    Ok(())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let (items, total) = resource::list(
        &tx,
        &SPEC,
        rows::Entity::find().filter(access::visible(&scope, Operation::View)?),
        &scope.workspace_id,
        &query,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"items":items.into_iter().map(json).collect::<Vec<_>>(),"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::View).await?;
    tx.commit().await?;
    Ok(Json(json(row)))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("workflows.create")?;
    let name = name(body.name.as_deref().unwrap_or_default())?;
    let definition = graph::checked(body.definition.as_deref().unwrap_or_default()).await?;
    if body
        .lifecycle
        .as_deref()
        .is_some_and(|lifecycle| lifecycle != "active")
    {
        return Err(conflict("workflow.archived"));
    }
    let group = Id::reference(body.group_id.as_deref().unwrap_or_default())?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.create").await?;
    crate::pipelines::contexts::check_single(&tx, scope.workspace_id, "workflow_group", group)
        .await?;
    let id = s.next_id()?;
    let revision = definitions::record(
        &s,
        &tx,
        scope.workspace_id,
        "workflow",
        id,
        &definition,
        scope.user.id,
        body.message.as_deref(),
    )
    .await?;
    retain_pins(&s, &tx, scope.workspace_id, &revision).await?;
    let now = db::now();
    let row = rows::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        name: name.clone(),
        group_id: group.into(),
        description: body.description.unwrap_or_default(),
        settings: settings(&definition),
        definition,
        draft_revision_id: revision.id,
        published_revision_id: None,
        owner_id: Some(scope.user.id.into()),
        trigger_run_as: None,
        lifecycle: "active".into(),
        archived_at: None,
        archived_by: 0,
        created_by: scope.user.id.into(),
        created_at: now,
        updated_at: now,
        updated_by: scope.user.id.into(),
        version: 1,
    };
    rows::Entity::insert(row.clone().into_active_model())
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "workflows",
        &id,
        json!({"name":name,"revision_id":Id::from(revision.id)}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json(row))))
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("workflows.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let current = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &current, Operation::Edit).await?;
    if current.version != body.version {
        return Err(conflict("execution.versionConflict"));
    }
    let mut row = current.clone();
    if let Some(lifecycle) = body.lifecycle {
        if !matches!(lifecycle.as_str(), "active" | "archived") {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("workflow.lifecycleInvalid"),
            ));
        }
        row.lifecycle = lifecycle;
    }
    if current.lifecycle == "archived" && row.lifecycle == "archived" {
        return Err(conflict("workflow.archived"));
    }
    if let Some(value) = body.name {
        row.name = name(&value)?;
    }
    if let Some(value) = body.description {
        row.description = value;
    }
    if let Some(value) = body.group_id {
        let group = Id::reference(&value)?;
        crate::pipelines::contexts::check_single(&tx, scope.workspace_id, "workflow_group", group)
            .await?;
        row.group_id = group.into();
    }
    if let Some(value) = body.definition {
        row.definition = graph::checked(&value).await?;
    }
    let revision = definitions::record(
        &s,
        &tx,
        scope.workspace_id,
        "workflow",
        id,
        &row.definition,
        scope.user.id,
        body.message.as_deref(),
    )
    .await?;
    retain_pins(&s, &tx, scope.workspace_id, &revision).await?;
    row.draft_revision_id = revision.id;
    row.settings = settings(&row.definition);
    row.updated_at = db::now();
    row.updated_by = scope.user.id.into();
    row.version = row
        .version
        .checked_add(1)
        .ok_or_else(|| conflict("execution.versionConflict"))?;
    let action = if current.lifecycle != row.lifecycle {
        // Both archive and restore revoke the current production pointer.
        row.published_revision_id = None;
        if row.lifecycle == "archived" {
            row.archived_at = Some(row.updated_at);
            row.archived_by = scope.user.id.into();
            "archive"
        } else {
            row.archived_at = None;
            row.archived_by = 0;
            "restore"
        }
    } else {
        "update"
    };
    let affected = rows::Entity::update_many()
        .col_expr(rows::Column::Name, Expr::value(row.name.clone()))
        .col_expr(rows::Column::GroupId, Expr::value(row.group_id))
        .col_expr(
            rows::Column::Description,
            Expr::value(row.description.clone()),
        )
        .col_expr(
            rows::Column::Definition,
            Expr::value(row.definition.clone()),
        )
        .col_expr(rows::Column::Settings, Expr::value(row.settings.clone()))
        .col_expr(
            rows::Column::DraftRevisionId,
            Expr::value(row.draft_revision_id),
        )
        .col_expr(
            rows::Column::PublishedRevisionId,
            Expr::value(row.published_revision_id),
        )
        .col_expr(rows::Column::Lifecycle, Expr::value(row.lifecycle.clone()))
        .col_expr(rows::Column::ArchivedAt, Expr::value(row.archived_at))
        .col_expr(rows::Column::ArchivedBy, Expr::value(row.archived_by))
        .col_expr(rows::Column::UpdatedAt, Expr::value(row.updated_at))
        .col_expr(rows::Column::UpdatedBy, Expr::value(row.updated_by))
        .col_expr(rows::Column::Version, Expr::value(row.version))
        .filter(rows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(rows::Column::Id.eq(id))
        .filter(rows::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected != 1 {
        return Err(conflict("execution.versionConflict"));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        action,
        "workflows",
        &id,
        json!({"revision_id":Id::from(revision.id),"version":row.version}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json(row)))
}

fn references(
    graph: &Value,
    subject: &str,
    revisions: &std::collections::BTreeSet<String>,
) -> bool {
    if graph["settings"]["error_workflow_id"] == subject {
        return true;
    }
    graph["nodes"].as_array().is_some_and(|nodes| {
        nodes.iter().any(|node| {
            (node["type"] == "core.subworkflow"
                && (node["params"]["subject_id"] == subject
                    || node["params"]["revision_id"]
                        .as_str()
                        .is_some_and(|id| revisions.contains(id))))
                || (node["type"] == "core.loop"
                    && references(&node["params"]["body"], subject, revisions))
        })
    })
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(body): Query<Version>,
) -> Result<StatusCode> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.delete").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::Delete).await?;
    if row.version != body.version {
        return Err(conflict("execution.versionConflict"));
    }
    if row.lifecycle != "archived" {
        return Err(conflict("workflow.archiveFirst"));
    }
    if workflow_executions::Entity::find()
        .filter(workflow_executions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workflow_executions::Column::WorkflowId.eq(id))
        .count(&tx)
        .await?
        != 0
    {
        return Err(conflict("workflow.referenced"));
    }
    let revision_rows = definition_revisions::Entity::find()
        .filter(definition_revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
        .all(&tx)
        .await?;
    let revisions = revision_rows
        .iter()
        .filter(|row| row.subject_id == i64::from(id))
        .map(|row| row.id.to_string())
        .collect();
    for revision in revision_rows
        .iter()
        .filter(|row| row.subject_id != i64::from(id))
    {
        let graph: Value = serde_json::from_str(&revision.definition)
            .map_err(|_| conflict("workflow.referenced"))?;
        if references(&graph, &id.to_string(), &revisions) {
            return Err(conflict("workflow.referenced"));
        }
    }
    rows::Entity::delete_many()
        .filter(rows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(rows::Column::Id.eq(id))
        .filter(rows::Column::Version.eq(body.version))
        .exec(&tx)
        .await?;
    // Immutable revisions stay retained. No assets or other definitions cascade.
    access::delete_grants(&tx, scope.workspace_id, "workflow", id).await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "workflows", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revisions(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::View).await?;
    let mut result = definitions::list(&tx, scope.workspace_id, "workflow", id, raw).await?;
    if let Some(items) = result.0["items"].as_array_mut() {
        let ids: Vec<i64> = items.iter().filter_map(|item| item["id"].as_str()?.parse().ok()).collect();
        if !ids.is_empty() {
            let latest = workflow_executions::Entity::find()
                .filter(workflow_executions::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(workflow_executions::Column::WorkflowId.eq(id))
                .filter(workflow_executions::Column::Id.in_subquery(
                    workflow_executions::Entity::find().select_only()
                        .column_as(Expr::col(workflow_executions::Column::Id).max(), "latest_id")
                        .filter(workflow_executions::Column::WorkspaceId.eq(scope.workspace_id))
                        .filter(workflow_executions::Column::WorkflowId.eq(id))
                        .filter(workflow_executions::Column::RevisionId.is_in(ids))
                        .group_by(workflow_executions::Column::RevisionId).into_query()
                )).all(&tx).await?;
            let latest: HashMap<_, _> = latest.iter().map(|run| (run.revision_id, run)).collect();
            for item in items {
                let revision_id = item["id"].as_str().and_then(|value| value.parse::<i64>().ok());
                item["node_count"] = item["definition"].as_str()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .and_then(|value| value["nodes"].as_array().map(|nodes| json!(nodes.len())))
                    .unwrap_or(Value::Null);
                item["is_draft"] = json!(revision_id == Some(row.draft_revision_id));
                item["is_published"] = json!(revision_id.is_some() && revision_id == row.published_revision_id);
                item["latest_execution"] = revision_id.and_then(|id| latest.get(&id))
                    .map(|run| json!({"id":Id::from(run.id),"number":run.number,"state":run.state,"created_at":run.created_at}))
                    .unwrap_or(Value::Null);
            }
        }
    }
    tx.commit().await?;
    Ok(result)
}

pub async fn revision(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, revision)): Path<(String, Id, String)>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::View).await?;
    let result = definitions::get(&tx, scope.workspace_id, "workflow", id, &revision).await?;
    tx.commit().await?;
    Ok(result)
}
