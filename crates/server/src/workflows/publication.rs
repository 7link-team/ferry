//! Immutable publication metadata and production identity, separate from drafts.
use super::{conflict, graph, json as workflow_json, scoped};
use crate::{
    accounts::{workspace::Scope, workspace_admin},
    db,
    definitions::access::{self, Operation},
    entities::{definition_publications as publications, definition_revisions, workflows},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, sea_query::Expr,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    version: i64,
    revision_id: Id,
    name: String,
    change_note: Option<String>,
    request_key: Option<String>,
    previous_run_as: Option<Id>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unpublish {
    version: i64,
}

fn invalid(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new(key))
}

fn json(row: publications::Model) -> Value {
    json!({"id":Id::from(row.id),"subject_kind":row.subject_kind,
        "subject_id":Id::from(row.subject_id),"revision_id":Id::from(row.revision_id),
        "name":row.name,"change_note":row.change_note,"run_as":Id::from(row.run_as),
        "created_at":row.created_at,"created_by":Id::from(row.created_by),"version":row.version})
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let tx = access::snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.read").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::View).await?;
    const SPEC: Spec = Spec {
        search: &["name", "change_note"],
        filters: &["revision_id", "run_as"],
        sortable: &["created_at", "name"],
        default_sort: "-created_at",
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(
        &tx,
        &SPEC,
        publications::Entity::find()
            .filter(publications::Column::SubjectKind.eq("workflow"))
            .filter(publications::Column::SubjectId.eq(id)),
        &scope.workspace_id,
        &query,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(json).collect::<Vec<_>>(),
        "total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Publish>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("workflows.update")?;
    scope.require("workflows.run")?;
    let name = body.name.trim();
    if !(1..=64).contains(&name.chars().count()) {
        return Err(invalid("workflow.publicationName"));
    }
    if body
        .change_note
        .as_ref()
        .is_some_and(|note| note.chars().count() > 200)
    {
        return Err(invalid("execution.revisionMessageLong"));
    }
    if body
        .request_key
        .as_ref()
        .is_some_and(|key| !(1..=64).contains(&key.chars().count()))
    {
        return Err(invalid("workflow.publicationRequest"));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::Edit).await?;
    access::require(&tx, &scope, &row, Operation::Run).await?;
    if let Some(key) = &body.request_key {
        if let Some(previous) = publications::Entity::find()
            .filter(publications::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(publications::Column::SubjectKind.eq("workflow"))
            .filter(publications::Column::SubjectId.eq(id))
            .filter(publications::Column::RequestKey.eq(key))
            .one(&tx)
            .await?
        {
            if previous.revision_id != i64::from(body.revision_id)
                || previous.name_key != name
                || previous.change_note != body.change_note
                || previous.run_as != i64::from(scope.user.id)
            {
                return Err(conflict("workflow.publicationConflict"));
            }
            tx.commit().await?;
            return Ok((StatusCode::OK, Json(json(previous))));
        }
    }
    if row.version != body.version || row.draft_revision_id != i64::from(body.revision_id) {
        return Err(conflict("execution.versionConflict"));
    }
    if row.lifecycle != "active" {
        return Err(conflict("workflow.archived"));
    }
    if row
        .trigger_run_as
        .is_some_and(|previous| previous != i64::from(scope.user.id))
        && row.trigger_run_as != body.previous_run_as.map(i64::from)
    {
        return Err(conflict("workflow.publicationIdentity"));
    }
    if publications::Entity::find()
        .filter(publications::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(publications::Column::SubjectKind.eq("workflow"))
        .filter(publications::Column::SubjectId.eq(id))
        .filter(publications::Column::NameKey.eq(name))
        .count(&tx)
        .await?
        != 0
    {
        return Err(conflict("workflow.publicationConflict"));
    }
    let revision = definition_revisions::Entity::find_by_id(i64::from(body.revision_id))
        .filter(definition_revisions::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(definition_revisions::Column::SubjectKind.eq("workflow"))
        .filter(definition_revisions::Column::SubjectId.eq(id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let checked = graph::checked(&revision.definition).await?;
    let graph: Value = serde_json::from_str(&checked).map_err(|_| ApiError::missing())?;
    super::run::publication_ready(&graph)?;
    let version = row
        .version
        .checked_add(1)
        .ok_or_else(|| conflict("execution.versionConflict"))?;
    let publication = publications::Model {
        id: s.next_id()?.into(),
        workspace_id: row.workspace_id,
        subject_kind: "workflow".into(),
        subject_id: row.id,
        revision_id: revision.id,
        name: name.into(),
        name_key: name.into(),
        change_note: body.change_note,
        run_as: scope.user.id.into(),
        request_key: body.request_key,
        created_at: db::now(),
        created_by: scope.user.id.into(),
        version: 1,
    };
    publications::Entity::insert(publication.clone().into_active_model())
        .exec(&tx)
        .await?;
    let changed = workflows::Entity::update_many()
        .col_expr(
            workflows::Column::PublishedRevisionId,
            Expr::value(Some(revision.id)),
        )
        .col_expr(
            workflows::Column::TriggerRunAs,
            Expr::value(Some(publication.run_as)),
        )
        .col_expr(
            workflows::Column::UpdatedAt,
            Expr::value(publication.created_at),
        )
        .col_expr(
            workflows::Column::UpdatedBy,
            Expr::value(publication.created_by),
        )
        .col_expr(workflows::Column::Version, Expr::value(version))
        .filter(workflows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed != 1 {
        return Err(conflict("execution.versionConflict"));
    }
    if row.trigger_run_as != Some(publication.run_as) {
        crate::hosts::activity_in(
            &s,
            &tx,
            &scope,
            "confirm_identity",
            "definition_revisions",
            &id,
            json!({"revision_id":body.revision_id,"old_run_as":row.trigger_run_as.map(Id::from),
                "new_run_as":scope.user.id}),
        )
        .await?;
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "publish",
        "definition_revisions",
        &id,
        json!({"publication_id":Id::from(publication.id),"revision_id":body.revision_id,
            "version":version}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json(publication))))
}

pub async fn unpublish(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Unpublish>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let mut row = scoped(&tx, scope.workspace_id, id).await?;
    access::require(&tx, &scope, &row, Operation::Edit).await?;
    if row.version != body.version {
        return Err(conflict("execution.versionConflict"));
    }
    if row.published_revision_id.is_none() {
        tx.commit().await?;
        return Ok(Json(workflow_json(row)));
    }
    let previous = row.published_revision_id;
    row.published_revision_id = None;
    row.version = row
        .version
        .checked_add(1)
        .ok_or_else(|| conflict("execution.versionConflict"))?;
    row.updated_at = db::now();
    row.updated_by = scope.user.id.into();
    if workflows::Entity::update_many()
        .col_expr(
            workflows::Column::PublishedRevisionId,
            Expr::value(None::<i64>),
        )
        .col_expr(workflows::Column::UpdatedAt, Expr::value(row.updated_at))
        .col_expr(workflows::Column::UpdatedBy, Expr::value(row.updated_by))
        .col_expr(workflows::Column::Version, Expr::value(row.version))
        .filter(workflows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected
        != 1
    {
        return Err(conflict("execution.versionConflict"));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "unpublish",
        "definition_revisions",
        &id,
        json!({"old_revision_id":previous.map(Id::from),"version":row.version}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(workflow_json(row)))
}
