//! Workspace pipeline groups/environments and their classification references.
use crate::{
    db,
    entities::{
        deployments, host_groups, hosts, pipeline_contexts as rows, pipeline_links as links,
        pipelines, tags,
    },
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, QueryTrait, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &[],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    target_kind: String,
    #[serde(default)]
    target_ref: String,
    #[serde(default)]
    version: i64,
}
fn to_json(row: &rows::Model) -> Value {
    let mut value = json!({"id":Id::from(row.id),"name":row.name,"description":row.description,"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at});
    if row.kind == "environment" {
        value["target_kind"] = json!(row.target_kind);
        value["target_ref"] = json!(Id::from(row.target_ref));
    }
    value
}
pub async fn scoped(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    id: Id,
) -> Result<rows::Model> {
    rows::Entity::find_by_id(id)
        .filter(rows::Column::WorkspaceId.eq(workspace))
        .filter(rows::Column::Kind.eq(kind))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn check(conn: &impl ConnectionTrait, scope: &Scope, kind: &str, body: &Body) -> Result<()> {
    if body.name.trim().is_empty() || body.name.trim().chars().count() > 60 {
        return Err(ApiError::bad("请填写名称（60 字以内）。"));
    }
    if kind == "environment" {
        let id = Id::reference(&body.target_ref)?;
        let count = match body.target_kind.as_str() {
            "host" => {
                hosts::Entity::find_by_id(id)
                    .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                    .count(conn)
                    .await?
            }
            "host-group" => {
                host_groups::Entity::find_by_id(id)
                    .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                    .count(conn)
                    .await?
            }
            _ => return Err(ApiError::bad("环境执行目标必须是主机或主机组。")),
        };
        if count == 0 {
            return Err(ApiError::bad("环境执行目标不存在于当前工作空间。"));
        }
    }
    Ok(())
}
async fn list(
    s: AppState,
    scope: Scope,
    raw: HashMap<String, String>,
    kind: &str,
) -> Result<Json<Value>> {
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (items, total) = resource::list(
        &s.db,
        &SPEC,
        rows::Entity::find().filter(rows::Column::Kind.eq(kind)),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":items.iter().map(to_json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
async fn create_in(
    s: &AppState,
    tx: &DatabaseTransaction,
    scope: &Scope,
    body: Body,
    kind: &str,
    resource: &str,
) -> Result<(StatusCode, Json<Value>)> {
    check(tx, scope, kind, &body).await?;
    let id = s.next_id()?;
    let now = db::now();
    rows::Entity::insert(rows::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        kind: Set(kind.into()),
        name: Set(body.name.trim().into()),
        description: Set(body.description),
        target_kind: Set(if kind == "environment" {
            body.target_kind
        } else {
            String::new()
        }),
        target_ref: Set(if kind == "environment" {
            Id::reference(&body.target_ref)?.into()
        } else {
            0
        }),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(tx)
    .await?;
    crate::hosts::activity_in(
        s,
        tx,
        scope,
        "create",
        resource,
        &id,
        json!({"name":body.name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
async fn update(
    s: AppState,
    scope: Scope,
    id: Id,
    body: Body,
    kind: &str,
    resource: &str,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let current = scoped(&tx, scope.workspace_id, kind, id).await?;
    if current.version != body.version {
        return Err(ApiError::conflict("记录已被修改，请刷新后重试。"));
    }
    check(&tx, &scope, kind, &body).await?;
    rows::Entity::update_many()
        .col_expr(rows::Column::Name, Expr::value(body.name.trim()))
        .col_expr(rows::Column::Description, Expr::value(body.description))
        .col_expr(
            rows::Column::TargetKind,
            Expr::value(if kind == "environment" {
                body.target_kind
            } else {
                String::new()
            }),
        )
        .col_expr(
            rows::Column::TargetRef,
            Expr::value(if kind == "environment" {
                i64::from(Id::reference(&body.target_ref)?)
            } else {
                0
            }),
        )
        .col_expr(rows::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            rows::Column::Version,
            Expr::col(rows::Column::Version).add(1),
        )
        .filter(rows::Column::Id.eq(id))
        .filter(rows::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        resource,
        &id,
        json!({"name":body.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn remove(
    s: AppState,
    scope: Scope,
    id: Id,
    kind: &str,
    resource: &str,
) -> Result<StatusCode> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    scoped(&tx, scope.workspace_id, kind, id).await?;
    ensure_unreferenced(&tx, scope.workspace_id, kind, &[id]).await?;
    if kind == "environment" {
        crate::variables::check_object_removal(&tx, scope.workspace_id, "environment", id).await?;
    }
    rows::Entity::delete_by_id(id).exec(&tx).await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", resource, &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
// Both context resources have the same scoped/versioned API, following templates' API macro.
macro_rules! context_api {
    ($module:ident,$kind:literal,$resource:literal) => {
        pub mod $module {
            use super::*;
            pub async fn list(
                State(s): State<AppState>,
                scope: Scope,
                Query(raw): Query<HashMap<String, String>>,
            ) -> Result<Json<Value>> {
                scope.require(concat!($resource, ".read"))?;
                super::list(s, scope, raw, $kind).await
            }
            pub async fn get(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id)): Path<(String, Id)>,
            ) -> Result<Json<Value>> {
                scope.require(concat!($resource, ".read"))?;
                Ok(Json(to_json(
                    &scoped(&s.db, scope.workspace_id, $kind, id).await?,
                )))
            }
            pub async fn create(
                State(s): State<AppState>,
                scope: Scope,
                Json(body): Json<Body>,
            ) -> Result<(StatusCode, Json<Value>)> {
                let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
                let result = create_in(&s, &tx, &scope, body).await?;
                tx.commit().await?;
                Ok(result)
            }
            pub(crate) async fn create_in(
                s: &AppState,
                tx: &DatabaseTransaction,
                scope: &Scope,
                body: Body,
            ) -> Result<(StatusCode, Json<Value>)> {
                scope.require(concat!($resource, ".create"))?;
                super::create_in(s, tx, scope, body, $kind, $resource).await
            }
            pub async fn update(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id)): Path<(String, Id)>,
                Json(body): Json<Body>,
            ) -> Result<Json<Value>> {
                scope.require(concat!($resource, ".update"))?;
                super::update(s, scope, id, body, $kind, $resource).await
            }
            pub async fn remove(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id)): Path<(String, Id)>,
            ) -> Result<StatusCode> {
                scope.require(concat!($resource, ".delete"))?;
                super::remove(s, scope, id, $kind, $resource).await
            }
        }
    };
}
context_api!(groups, "group", "pipeline-groups");
context_api!(environments, "environment", "pipeline-environments");

pub async fn check_single(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    id: Id,
) -> Result<()> {
    if i64::from(id) == 0 {
        return Ok(());
    }
    let count = match kind {
        "application" => {
            deployments::Entity::find_by_id(id)
                .filter(deployments::Column::WorkspaceId.eq(workspace))
                .count(conn)
                .await?
        }
        "tag" => {
            tags::Entity::find_by_id(id)
                .filter(tags::Column::WorkspaceId.eq(workspace))
                .count(conn)
                .await?
        }
        _ => {
            rows::Entity::find_by_id(id)
                .filter(rows::Column::WorkspaceId.eq(workspace))
                .filter(rows::Column::Kind.eq(kind))
                .count(conn)
                .await?
        }
    };
    if count == 0 {
        return Err(ApiError::bad("分类对象不存在于当前工作空间。"));
    }
    Ok(())
}
pub async fn replace_links(
    tx: &DatabaseTransaction,
    workspace: Id,
    pipeline: Id,
    kind: &str,
    ids: Option<&[Id]>,
) -> Result<()> {
    let Some(ids) = ids else { return Ok(()) };
    let ids: BTreeSet<Id> = ids.iter().copied().collect();
    if kind == "environment" {
        crate::cron_jobs::check_pipeline_environments(tx, workspace, pipeline, &ids).await?;
    }
    for id in &ids {
        if i64::from(*id) == 0 {
            return Err(ApiError::bad("请选择有效分类对象。"));
        }
        check_single(tx, workspace, kind, *id).await?;
    }
    links::Entity::delete_many()
        .filter(links::Column::PipelineId.eq(pipeline))
        .filter(links::Column::Kind.eq(kind))
        .exec(tx)
        .await?;
    for id in ids {
        links::Entity::insert(links::ActiveModel {
            pipeline_id: Set(pipeline.into()),
            kind: Set(kind.into()),
            reference_id: Set(id.into()),
        })
        .exec(tx)
        .await?;
    }
    Ok(())
}
pub async fn ids(conn: &impl ConnectionTrait, pipeline: Id, kind: &str) -> Result<Vec<Id>> {
    let mut ids = links::Entity::find()
        .filter(links::Column::PipelineId.eq(pipeline))
        .filter(links::Column::Kind.eq(kind))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| Id::from(row.reference_id))
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}
pub async fn selected(
    conn: &impl ConnectionTrait,
    workspace: Id,
    pipeline: Id,
    environment: Option<&str>,
) -> Result<Option<rows::Model>> {
    let allowed = ids(conn, pipeline, "environment").await?;
    let selected = Id::reference(environment.unwrap_or(""))?;
    if allowed.is_empty() && i64::from(selected) == 0 {
        return Ok(None);
    }
    if !allowed.contains(&selected) {
        return Err(ApiError::bad("请选择该流水线绑定的执行环境。"));
    }
    Ok(Some(
        scoped(conn, workspace, "environment", selected).await?,
    ))
}
pub async fn ensure_unreferenced(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    ids: &[Id],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let count = match kind {
        "group" => {
            pipelines::Entity::find()
                .filter(pipelines::Column::WorkspaceId.eq(workspace))
                .filter(pipelines::Column::GroupId.is_in(ids.iter().copied()))
                .count(conn)
                .await?
        }
        "application" => {
            pipelines::Entity::find()
                .filter(pipelines::Column::WorkspaceId.eq(workspace))
                .filter(pipelines::Column::ApplicationId.is_in(ids.iter().copied()))
                .count(conn)
                .await?
        }
        "host" | "host-group" => {
            rows::Entity::find()
                .filter(rows::Column::WorkspaceId.eq(workspace))
                .filter(rows::Column::TargetKind.eq(kind))
                .filter(rows::Column::TargetRef.is_in(ids.iter().copied()))
                .count(conn)
                .await?
        }
        _ => {
            links::Entity::find()
                .filter(links::Column::Kind.eq(kind))
                .filter(links::Column::ReferenceId.is_in(ids.iter().copied()))
                .filter(
                    links::Column::PipelineId.in_subquery(
                        pipelines::Entity::find()
                            .select_only()
                            .column(pipelines::Column::Id)
                            .filter(pipelines::Column::WorkspaceId.eq(workspace))
                            .into_query(),
                    ),
                )
                .count(conn)
                .await?
        }
    };
    if count > 0 {
        return Err(ApiError::conflict(
            "对象仍被流水线或执行环境引用，请先解除关联。",
        ));
    }
    Ok(())
}
pub async fn decorate(
    conn: &impl ConnectionTrait,
    workspace: Id,
    mut values: Vec<Value>,
) -> Result<Vec<Value>> {
    let pipeline_ids = values
        .iter()
        .filter_map(|row| row["id"].as_str())
        .map(Id::reference)
        .collect::<Result<Vec<_>>>()?;
    if pipeline_ids.is_empty() {
        return Ok(values);
    }
    let all = links::Entity::find()
        .filter(links::Column::PipelineId.is_in(pipeline_ids))
        .all(conn)
        .await?;
    let contexts = rows::Entity::find()
        .filter(rows::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?;
    let labels = tags::Entity::find()
        .filter(tags::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?;
    let applications = deployments::Entity::find()
        .filter(deployments::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?;
    for value in &mut values {
        let pipeline = Id::reference(value["id"].as_str().unwrap_or(""))?;
        for (kind, key, list_key) in [
            ("tag", "tag_ids", "tags"),
            ("environment", "environment_ids", "environments"),
        ] {
            let mut ids: Vec<_> = all
                .iter()
                .filter(|link| link.pipeline_id == i64::from(pipeline) && link.kind == kind)
                .map(|link| link.reference_id)
                .collect();
            ids.sort();
            value[key] = json!(ids.iter().map(|id| Id::from(*id)).collect::<Vec<_>>());
            value[list_key] = json!(
                ids.iter()
                    .filter_map(|id| if kind == "tag" {
                        labels
                            .iter()
                            .find(|r| r.id == *id)
                            .map(|r| json!({"id":Id::from(r.id),"name":r.name,"color":r.color}))
                    } else {
                        contexts
                            .iter()
                            .find(|r| r.id == *id && r.kind == kind)
                            .map(to_json)
                    })
                    .collect::<Vec<_>>()
            );
        }
        let group = Id::reference(value["group_id"].as_str().unwrap_or(""))?;
        value["group_name"] = json!(
            contexts
                .iter()
                .find(|r| r.id == i64::from(group) && r.kind == "group")
                .map(|r| r.name.as_str())
                .unwrap_or("")
        );
        let app = Id::reference(value["application_id"].as_str().unwrap_or(""))?;
        value["application_name"] = json!(
            applications
                .iter()
                .find(|r| r.id == i64::from(app))
                .map(|r| r.name.as_str())
                .unwrap_or("")
        );
    }
    Ok(values)
}
