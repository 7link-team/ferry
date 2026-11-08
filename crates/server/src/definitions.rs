//! Immutable definition history. Callers hold the workspace mutation lock so
//! numbering, definition changes and execution provenance commit together.
pub(crate) mod access;
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{definition_revisions as revisions, pipelines},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) fn canonical(input: &str) -> Result<String> {
    let mut value: Value = serde_json::from_str(input).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.saveDefinition"),
        )
    })?;
    value.sort_all_objects();
    serde_json::to_string(&value).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.saveDefinition"),
        )
    })
}

pub(crate) async fn record(
    s: &AppState,
    tx: &DatabaseTransaction,
    workspace: Id,
    kind: &str,
    subject: Id,
    definition: &str,
    author: Id,
    message: Option<&str>,
) -> Result<revisions::Model> {
    if message.is_some_and(|text| text.chars().count() > 200) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.revisionMessageLong"),
        ));
    }
    if !matches!(kind, "pipeline" | "workflow" | "agent" | "model") {
        return Err(ApiError::missing());
    }
    let definition = canonical(definition)?;
    let previous = revisions::Entity::find()
        .filter(revisions::Column::WorkspaceId.eq(workspace))
        .filter(revisions::Column::SubjectKind.eq(kind))
        .filter(revisions::Column::SubjectId.eq(subject))
        .order_by_desc(revisions::Column::Number)
        .one(tx)
        .await?;
    if let Some(previous) = &previous {
        if canonical(&previous.definition).ok().as_deref() == Some(&definition) {
            return Ok(previous.clone());
        }
    }
    let number = previous
        .map_or(Some(1), |r| r.number.checked_add(1))
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::CONFLICT,
                Message::new("execution.versionConflict"),
            )
        })?;
    let row = revisions::Model {
        id: s.next_id()?.into(),
        workspace_id: workspace.into(),
        subject_kind: kind.into(),
        subject_id: subject.into(),
        number,
        definition,
        message: message.map(str::to_owned),
        created_by: author.into(),
        created_at: db::now(),
        version: 1,
    };
    use sea_orm::IntoActiveModel;
    revisions::Entity::insert(row.clone().into_active_model())
        .exec(tx)
        .await?;
    Ok(row)
}

fn json(row: revisions::Model) -> Value {
    json!({"id": Id::from(row.id), "subject_kind":row.subject_kind,
        "subject_id":Id::from(row.subject_id), "number":row.number,
        "definition":row.definition, "message":row.message,
        "created_at":row.created_at, "created_by":Id::from(row.created_by), "version":row.version})
}

async fn pipeline(s: &AppState, scope: &Scope, id: &str) -> Result<Id> {
    scope.require("pipelines.read")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    access::require_pipeline(&s.db, scope, id, access::Operation::View).await?;
    pipelines::Entity::find_by_id(i64::from(id))
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(id)
}

pub(crate) async fn pipeline_list(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let id = pipeline(&s, &scope, &id).await?;
    list(&s.db, scope.workspace_id, "pipeline", id, raw).await
}

pub(crate) async fn list(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    subject: Id,
    raw: HashMap<String, String>,
) -> Result<Json<Value>> {
    let spec = Spec {
        search: &[],
        filters: &[],
        sortable: &["number", "created_at"],
        default_sort: "-number",
    };
    let query = ListQuery::parse(&spec, &raw)?;
    let (rows, total) = resource::list::<revisions::Entity, revisions::Model>(
        conn,
        &spec,
        revisions::Entity::find()
            .filter(revisions::Column::SubjectKind.eq(kind))
            .filter(revisions::Column::SubjectId.eq(subject)),
        &workspace,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(json).collect::<Vec<_>>(),
        "total":total, "page":query.page, "page_size":query.page_size}),
    ))
}

pub(crate) async fn pipeline_get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, revision)): Path<(String, String, String)>,
) -> Result<Json<Value>> {
    let id = pipeline(&s, &scope, &id).await?;
    get(&s.db, scope.workspace_id, "pipeline", id, &revision).await
}

pub(crate) async fn get(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    subject: Id,
    revision: &str,
) -> Result<Json<Value>> {
    let revision: Id = revision.parse().map_err(|_| ApiError::missing())?;
    let row = revisions::Entity::find_by_id(i64::from(revision))
        .filter(revisions::Column::WorkspaceId.eq(workspace))
        .filter(revisions::Column::SubjectKind.eq(kind))
        .filter(revisions::Column::SubjectId.eq(subject))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(json(row)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, PaginatorTrait, Schema};

    async fn fixture() -> (tempfile::TempDir, AppState, AppState) {
        let (dir, a, b) = crate::pipelines::runs::lease_tests::fixture(0).await;
        let schema = Schema::new(a.db.get_database_backend());
        a.db.execute(&schema.create_table_from_entity(revisions::Entity))
            .await
            .unwrap();
        (dir, a, b)
    }

    async fn save(s: &AppState, definition: &str) -> revisions::Model {
        let tx = crate::variables::transaction(s, Id::from(11))
            .await
            .unwrap();
        let row = record(
            s,
            &tx,
            11.into(),
            "pipeline",
            7.into(),
            definition,
            3.into(),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        row
    }

    #[tokio::test]
    async fn immutable_history_compares_json_but_preserves_yaml_bytes_and_restore_lineage() {
        let (_dir, a, _) = fixture().await;
        let first = save(
            &a,
            r##"{"engine":"github-actions","workflow":"# first\njobs: {}\n"}"##,
        )
        .await;
        let same = save(
            &a,
            r##"{ "workflow":"# first\njobs: {}\n", "engine":"github-actions" }"##,
        )
        .await;
        assert_eq!(first.id, same.id);
        let second = save(
            &a,
            r##"{"engine":"github-actions","workflow":"# second\njobs: {}\n"}"##,
        )
        .await;
        assert_eq!(second.number, 2);
        let restored = save(&a, &first.definition).await;
        assert_eq!(restored.number, 3);
        assert_ne!(restored.id, first.id);
        assert_eq!(restored.definition, first.definition);
        assert_eq!(
            revisions::Entity::find_by_id(first.id)
                .one(&a.db)
                .await
                .unwrap()
                .unwrap(),
            first
        );
        assert_eq!(
            revisions::Entity::find_by_id(second.id)
                .one(&a.db)
                .await
                .unwrap()
                .unwrap(),
            second
        );
    }

    #[tokio::test]
    async fn competing_saves_allocate_distinct_numbers_and_rollback_has_no_history() {
        let (_dir, a, b) = fixture().await;
        save(&a, r#"{"value":1}"#).await;
        let (left, right) = tokio::join!(save(&a, r#"{"value":2}"#), save(&b, r#"{"value":3}"#));
        assert_ne!(left.id, right.id);
        let mut numbers = [left.number, right.number];
        numbers.sort();
        assert_eq!(numbers, [2, 3]);
        let tx = crate::variables::transaction(&a, 11.into()).await.unwrap();
        let abandoned = record(
            &a,
            &tx,
            11.into(),
            "pipeline",
            7.into(),
            r#"{"value":4}"#,
            3.into(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(abandoned.number, 4);
        tx.rollback().await.unwrap();
        assert!(
            revisions::Entity::find_by_id(abandoned.id)
                .one(&b.db)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(revisions::Entity::find().count(&b.db).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn revision_message_limits_count_unicode_characters_and_never_rewrite_history() {
        let (_dir, a, _) = fixture().await;
        let tx = crate::variables::transaction(&a, 11.into()).await.unwrap();
        let message = "版".repeat(200);
        let first = record(
            &a,
            &tx,
            11.into(),
            "pipeline",
            7.into(),
            "{}",
            3.into(),
            Some(&message),
        )
        .await
        .unwrap();
        assert_eq!(first.message.as_deref(), Some(message.as_str()));
        let error = record(
            &a,
            &tx,
            11.into(),
            "pipeline",
            7.into(),
            "{}",
            3.into(),
            Some(&"版".repeat(201)),
        )
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        let same = record(
            &a,
            &tx,
            11.into(),
            "pipeline",
            7.into(),
            "{}",
            4.into(),
            Some("changed"),
        )
        .await
        .unwrap();
        assert_eq!(same, first);
        tx.commit().await.unwrap();
        assert_eq!(revisions::Entity::find().count(&a.db).await.unwrap(), 1);
    }
}
