//! Object grants are intersected with live workspace permissions. A resource
//! group is an ACL subject, never a new user-membership or role system.
use crate::{
    accounts::{workspace::Scope, workspace_admin},
    db,
    entities::{definition_acl as grants, pipelines, users, workflows, workspace_members},
    error::{ApiError, Json, Path, Query, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    IntoActiveModel, IsolationLevel, PaginatorTrait, QueryFilter, QuerySelect, QueryTrait,
    TransactionTrait,
    sea_query::{Alias, Condition, Expr, ExprTrait, Query as SqlQuery},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    View,
    Run,
    Edit,
    Delete,
}
impl Operation {
    fn permission(self, pipeline: bool) -> &'static str {
        match (pipeline, self) {
            (false, Self::View) => "workflows.read",
            (false, Self::Run) => "workflows.run",
            (false, Self::Edit) => "workflows.update",
            (false, Self::Delete) => "workflows.delete",
            (true, Self::View) => "pipelines.read",
            (true, Self::Run) => "pipelines.run",
            (true, Self::Edit) => "pipelines.update",
            (true, Self::Delete) => "pipelines.delete",
        }
    }
    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::View => &["view", "run", "all"],
            Self::Run => &["run", "all"],
            _ => &["all"],
        }
    }
}

/// Repeatable reads keep the authorized page and count on one database snapshot.
pub(crate) async fn snapshot(db: &DatabaseConnection) -> Result<DatabaseTransaction> {
    Ok(if db.get_database_backend() == sea_orm::DbBackend::Sqlite {
        db.begin().await?
    } else {
        db.begin_with_config(Some(IsolationLevel::RepeatableRead), None)
            .await?
    })
}

pub(crate) fn visible(scope: &Scope, operation: Operation) -> Result<Condition> {
    visible_for(scope, operation, false)
}
pub(crate) fn pipeline_visible(scope: &Scope, operation: Operation) -> Result<Condition> {
    visible_for(scope, operation, true)
}
pub(crate) fn pipeline_ids(
    scope: &Scope,
    operation: Operation,
) -> Result<sea_orm::sea_query::SelectStatement> {
    Ok(pipelines::Entity::find()
        .select_only()
        .column(pipelines::Column::Id)
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipeline_visible(scope, operation)?)
        .into_query())
}
pub(crate) async fn require_pipeline(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
    operation: Operation,
) -> Result<()> {
    if pipelines::Entity::find_by_id(i64::from(id))
        .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(pipeline_visible(scope, operation)?)
        .count(conn)
        .await?
        == 0
    {
        if pipelines::Entity::find_by_id(i64::from(id))
            .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
            .count(conn)
            .await?
            == 0
        {
            return Err(ApiError::missing());
        }
        return Err(ApiError::forbidden());
    }
    Ok(())
}
fn visible_for(scope: &Scope, operation: Operation, pipeline: bool) -> Result<Condition> {
    scope.require(operation.permission(pipeline))?;
    // The existing platform superadmin is the only global object override.
    // Ordinary workspace read/update (including a custom role) grants none.
    if scope.user.is_superadmin {
        return Ok(Condition::all());
    }
    let table = Alias::new(if pipeline { "pipelines" } else { "workflows" });
    let kind = if pipeline { "pipeline" } else { "workflow" };
    let group = if pipeline {
        "pipeline_group"
    } else {
        "workflow_group"
    };
    let inherited = Condition::any()
        .add(
            Condition::all()
                .add(
                    Expr::col((Alias::new("definition_acl"), grants::Column::SubjectKind)).eq(kind),
                )
                .add(
                    Expr::col((Alias::new("definition_acl"), grants::Column::SubjectId))
                        .equals((table.clone(), Alias::new("id"))),
                ),
        )
        .add(
            Condition::all()
                .add(
                    Expr::col((Alias::new("definition_acl"), grants::Column::SubjectKind))
                        .eq(group),
                )
                .add(
                    Expr::col((Alias::new("definition_acl"), grants::Column::SubjectId))
                        .equals((table.clone(), Alias::new("group_id"))),
                ),
        );
    let subquery = SqlQuery::select()
        .expr(Expr::val(1_i64))
        .from(grants::Entity)
        .and_where(
            Expr::col((grants::Entity, grants::Column::WorkspaceId))
                .equals((table.clone(), Alias::new("workspace_id"))),
        )
        .and_where(Expr::col((grants::Entity, grants::Column::UserId)).eq(scope.user.id))
        .and_where(
            Expr::col((grants::Entity, grants::Column::Role))
                .is_in(operation.roles().iter().copied()),
        )
        .cond_where(inherited)
        .to_owned();
    Ok(Condition::any()
        .add(Expr::col((table, Alias::new("owner_id"))).eq(scope.user.id))
        .add(Expr::exists(subquery)))
}

pub(crate) async fn require(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    row: &workflows::Model,
    operation: Operation,
) -> Result<()> {
    let visible = visible(scope, operation)?;
    if workflows::Entity::find_by_id(row.id)
        .filter(workflows::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(visible)
        .count(conn)
        .await?
        == 0
    {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Subject {
    Workflow,
    Pipeline,
    PipelineGroup,
    Group,
}
impl Subject {
    fn kind(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline",
            Self::PipelineGroup => "pipeline_group",
            Self::Workflow => "workflow",
            Self::Group => "workflow_group",
        }
    }
    fn resource(self) -> &'static str {
        match self {
            Self::Pipeline => "pipelines",
            Self::PipelineGroup => "pipeline-groups",
            Self::Workflow => "workflows",
            Self::Group => "workflow-groups",
        }
    }
    fn permission(self) -> &'static str {
        match self {
            Self::Pipeline => "pipelines.update",
            Self::PipelineGroup => "pipeline-groups.update",
            Self::Workflow => "workflows.update",
            Self::Group => "workflow-groups.update",
        }
    }
    async fn manage(self, conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<()> {
        scope.require(self.permission())?;
        match self {
            Self::Workflow => {
                let row = crate::workflows::scoped(conn, scope.workspace_id, id).await?;
                require(conn, scope, &row, Operation::Edit).await
            }
            Self::Pipeline => require_pipeline(conn, scope, id, Operation::Edit).await,
            Self::PipelineGroup => {
                crate::pipelines::contexts::scoped(conn, scope.workspace_id, "group", id).await?;
                scope.require("roles.update")
            }
            Self::Group => {
                crate::pipelines::contexts::scoped(conn, scope.workspace_id, "workflow_group", id)
                    .await?;
                // Granting group-wide access uses the existing workspace role-
                // administration authority, in addition to group edit rights.
                scope.require("roles.update")
            }
        }
    }
}

const SPEC: Spec = Spec {
    search: &[],
    filters: &["user_id", "role"],
    sortable: &["user_id", "role", "created_at"],
    default_sort: "-created_at",
};
fn json(row: grants::Model) -> Value {
    json!({"id":Id::from(row.id), "subject_kind":row.subject_kind,
        "subject_id":Id::from(row.subject_id), "user_id":Id::from(row.user_id),
        "role":row.role, "created_at":row.created_at, "created_by":Id::from(row.created_by), "version":row.version})
}
async fn list(
    s: AppState,
    scope: Scope,
    id: Id,
    raw: HashMap<String, String>,
    subject: Subject,
) -> Result<Json<Value>> {
    let q = ListQuery::parse(&SPEC, &raw)?;
    let tx = snapshot(&s.db).await?;
    let scope = workspace_admin::authorize(&tx, &scope, subject.permission()).await?;
    subject.manage(&tx, &scope, id).await?;
    let (items, total) = resource::list(
        &tx,
        &SPEC,
        grants::Entity::find()
            .filter(grants::Column::SubjectKind.eq(subject.kind()))
            .filter(grants::Column::SubjectId.eq(id)),
        &scope.workspace_id,
        &q,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"items":items.into_iter().map(json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    user_id: Id,
    role: String,
}
async fn create(
    s: AppState,
    scope: Scope,
    id: Id,
    body: Grant,
    subject: Subject,
) -> Result<(StatusCode, Json<Value>)> {
    if !matches!(body.role.as_str(), "view" | "run" | "all") {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workflow.aclRole"),
        ));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, subject.permission()).await?;
    subject.manage(&tx, &scope, id).await?;
    let member = workspace_members::Entity::find()
        .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workspace_members::Column::UserId.eq(body.user_id))
        .filter(workspace_members::Column::State.eq("active"))
        .count(&tx)
        .await?;
    let active = users::Entity::find_by_id(i64::from(body.user_id))
        .filter(users::Column::Active.ne(0_i64))
        .count(&tx)
        .await?;
    if member == 0 || active == 0 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workflow.aclMember"),
        ));
    }
    let previous = grants::Entity::find()
        .filter(grants::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(grants::Column::SubjectKind.eq(subject.kind()))
        .filter(grants::Column::SubjectId.eq(id))
        .filter(grants::Column::UserId.eq(body.user_id))
        .filter(grants::Column::Role.eq(&body.role))
        .one(&tx)
        .await?;
    let row = if let Some(row) = previous {
        row
    } else {
        let row = grants::Model {
            id: s.next_id()?.into(),
            workspace_id: scope.workspace_id.into(),
            subject_kind: subject.kind().into(),
            subject_id: id.into(),
            user_id: body.user_id.into(),
            role: body.role,
            created_at: db::now(),
            created_by: scope.user.id.into(),
            version: 1,
        };
        grants::Entity::insert(row.clone().into_active_model())
            .exec(&tx)
            .await?;
        crate::hosts::activity_in(
            &s,
            &tx,
            &scope,
            "grant",
            subject.resource(),
            &id,
            json!({"grant_id":Id::from(row.id),"user_id":body.user_id,"role":row.role}),
        )
        .await?;
        row
    };
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json(row))))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    version: i64,
}
async fn remove(
    s: AppState,
    scope: Scope,
    id: Id,
    grant: Id,
    body: Version,
    subject: Subject,
) -> Result<StatusCode> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope = workspace_admin::authorize(&tx, &scope, subject.permission()).await?;
    subject.manage(&tx, &scope, id).await?;
    let row = grants::Entity::find_by_id(i64::from(grant))
        .filter(grants::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(grants::Column::SubjectKind.eq(subject.kind()))
        .filter(grants::Column::SubjectId.eq(id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.version != body.version {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("execution.versionConflict"),
        ));
    }
    grants::Entity::delete_by_id(row.id).exec(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "revoke",
        subject.resource(),
        &id,
        json!({"grant_id":grant,"user_id":Id::from(row.user_id),"role":row.role}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

macro_rules! api {
    ($module:ident,$subject:expr) => {
        pub(crate) mod $module {
            use super::*;
            pub async fn list(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id)): Path<(String, Id)>,
                Query(raw): Query<HashMap<String, String>>,
            ) -> Result<Json<Value>> {
                super::list(s, scope, id, raw, $subject).await
            }
            pub async fn create(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id)): Path<(String, Id)>,
                Json(body): Json<Grant>,
            ) -> Result<(StatusCode, Json<Value>)> {
                super::create(s, scope, id, body, $subject).await
            }
            pub async fn remove(
                State(s): State<AppState>,
                scope: Scope,
                Path((_, id, grant)): Path<(String, Id, Id)>,
                Query(body): Query<Version>,
            ) -> Result<StatusCode> {
                super::remove(s, scope, id, grant, body, $subject).await
            }
        }
    };
}
api!(workflow, Subject::Workflow);
api!(pipeline, Subject::Pipeline);
api!(pipeline_groups, Subject::PipelineGroup);
api!(groups, Subject::Group);

/// Called inside the parent's delete transaction, after reference protection.
pub(crate) async fn delete_grants(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    id: Id,
) -> Result<()> {
    grants::Entity::delete_many()
        .filter(grants::Column::WorkspaceId.eq(workspace))
        .filter(grants::Column::SubjectKind.eq(kind))
        .filter(grants::Column::SubjectId.eq(id))
        .exec(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, QueryTrait, Set};
    use sea_orm_migration::MigratorTrait;

    fn scope() -> Scope {
        Scope {
            workspace_id: 11.into(),
            user: crate::accounts::auth::User {
                id: 3.into(),
                email: "reader@example.invalid".into(),
                name: "Reader".into(),
                is_superadmin: false,
            },
            permissions: [
                "workflows.read",
                "workflows.run",
                "workflows.update",
                "workflows.delete",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }
    fn row(id: i64) -> workflows::Model {
        workflows::Model {
            id,
            workspace_id: 11,
            name: format!("Workflow {id:02}"),
            group_id: 50,
            description: String::new(),
            definition: "{}".into(),
            draft_revision_id: 1,
            published_revision_id: None,
            settings: "{}".into(),
            owner_id: Some(1),
            trigger_run_as: None,
            lifecycle: "active".into(),
            archived_at: None,
            archived_by: 0,
            created_by: 1,
            created_at: 1,
            updated_at: 1,
            updated_by: 1,
            version: 1,
        }
    }
    async fn database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migration::Migrator::up(&db, None).await.unwrap();
        db
    }
    async fn grant(
        db: &DatabaseConnection,
        id: i64,
        workspace: i64,
        kind: &str,
        subject: i64,
        role: &str,
    ) {
        grants::Entity::insert(
            grants::Model {
                id,
                workspace_id: workspace,
                subject_kind: kind.into(),
                subject_id: subject,
                user_id: 3,
                role: role.into(),
                created_at: 1,
                created_by: 1,
                version: 1,
            }
            .into_active_model(),
        )
        .exec(db)
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn object_and_current_group_grants_union_without_exceeding_workspace_ceiling() {
        let db = database().await;
        let row = row(7);
        workflows::Entity::insert(row.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        let mut scope = scope();
        assert!(require(&db, &scope, &row, Operation::View).await.is_err());
        grant(&db, 1, 11, "workflow", 7, "view").await;
        grant(&db, 2, 11, "workflow_group", 50, "run").await;
        for op in [Operation::View, Operation::Run] {
            require(&db, &scope, &row, op).await.unwrap();
        }
        assert!(require(&db, &scope, &row, Operation::Edit).await.is_err());
        scope.permissions.remove("workflows.run");
        assert!(require(&db, &scope, &row, Operation::Run).await.is_err());
        scope.permissions.insert("workflows.run".into());
        grant(&db, 3, 11, "workflow_group", 50, "all").await;
        require(&db, &scope, &row, Operation::Edit).await.unwrap();
        grants::Entity::delete_by_id(3).exec(&db).await.unwrap();
        assert!(require(&db, &scope, &row, Operation::Edit).await.is_err());
        workflows::Entity::update(workflows::ActiveModel {
            id: Set(7),
            group_id: Set(51),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        assert!(require(&db, &scope, &row, Operation::Run).await.is_err());
        require(&db, &scope, &row, Operation::View).await.unwrap();
        grants::Entity::delete_by_id(1).exec(&db).await.unwrap();
        assert!(require(&db, &scope, &row, Operation::View).await.is_err());
    }
    #[tokio::test]
    async fn owners_and_superadmin_still_need_workspace_permission_and_foreign_grants_do_not_leak()
    {
        let db = database().await;
        let mut scope = scope();
        let mut row = row(7);
        workflows::Entity::insert(row.clone().into_active_model())
            .exec(&db)
            .await
            .unwrap();
        grant(&db, 1, 12, "workflow", 7, "all").await;
        grant(&db, 2, 11, "workflow_group", 7, "all").await;
        grant(&db, 3, 11, "pipeline", 7, "all").await;
        assert!(require(&db, &scope, &row, Operation::View).await.is_err());
        row.owner_id = Some(3);
        workflows::Entity::update(workflows::ActiveModel {
            id: Set(7),
            owner_id: Set(Some(3)),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        require(&db, &scope, &row, Operation::Delete).await.unwrap();
        scope.permissions.remove("workflows.delete");
        assert!(require(&db, &scope, &row, Operation::Delete).await.is_err());
        scope.user.is_superadmin = true;
        assert!(require(&db, &scope, &row, Operation::Delete).await.is_err());
    }
    #[tokio::test]
    async fn database_authorization_precedes_pagination_count_and_ignores_duplicate_grants() {
        let db = database().await;
        let scope = scope();
        for id in 1..=25 {
            workflows::Entity::insert(row(id).into_active_model())
                .exec(&db)
                .await
                .unwrap();
        }
        for id in 23..=25 {
            grant(&db, id, 11, "workflow", id, "view").await;
        }
        grant(&db, 30, 11, "workflow", 23, "run").await;
        let spec = Spec {
            search: &["name"],
            filters: &[],
            sortable: &["name"],
            default_sort: "name",
        };
        let q = ListQuery::parse(&spec, &[("page_size".into(), "2".into())].into()).unwrap();
        let tx = snapshot(&db).await.unwrap();
        let (items, total): (Vec<workflows::Model>, _) = resource::list(
            &tx,
            &spec,
            workflows::Entity::find().filter(visible(&scope, Operation::View).unwrap()),
            &scope.workspace_id,
            &q,
        )
        .await
        .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.iter().map(|r| r.id).collect::<Vec<_>>(), [23, 24]);
        tx.commit().await.unwrap();
        let q = ListQuery::parse(
            &spec,
            &[
                ("page_size".into(), "2".into()),
                ("page".into(), "2".into()),
            ]
            .into(),
        )
        .unwrap();
        let (items, total): (Vec<workflows::Model>, _) = resource::list(
            &db,
            &spec,
            workflows::Entity::find().filter(visible(&scope, Operation::View).unwrap()),
            &scope.workspace_id,
            &q,
        )
        .await
        .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items[0].id, 25);
    }
    #[tokio::test]
    async fn pipeline_grants_are_kind_scoped_live_and_bounded_by_workspace_permissions() {
        let db = database().await;
        let mut scope = scope();
        for permission in [
            "pipelines.read",
            "pipelines.run",
            "pipelines.update",
            "pipelines.delete",
        ] {
            scope.permissions.insert(permission.into());
        }
        pipelines::Entity::insert(pipelines::ActiveModel {
            id: Set(7),
            workspace_id: Set(11),
            owner_id: Set(None),
            name: Set("pipeline".into()),
            group_id: Set(50),
            application_id: Set(0),
            repository_id: Set(0),
            definition: Set("{}".into()),
            target_kind: Set("host".into()),
            target_ref: Set(0),
            description: Set(String::new()),
            created_at: Set(1),
            updated_at: Set(1),
            version: Set(1),
        })
        .exec(&db)
        .await
        .unwrap();
        grant(&db, 1, 11, "workflow", 7, "all").await;
        grant(&db, 2, 12, "pipeline", 7, "all").await;
        assert_eq!(
            require_pipeline(&db, &scope, 7.into(), Operation::View)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        for operation in [
            Operation::View,
            Operation::Run,
            Operation::Edit,
            Operation::Delete,
        ] {
            assert_eq!(
                require_pipeline(&db, &scope, 8.into(), operation)
                    .await
                    .unwrap_err()
                    .0,
                StatusCode::NOT_FOUND
            );
            scope.workspace_id = 12.into();
            assert_eq!(
                require_pipeline(&db, &scope, 7.into(), operation)
                    .await
                    .unwrap_err()
                    .0,
                StatusCode::NOT_FOUND
            );
            scope.workspace_id = 11.into();
        }
        grant(&db, 3, 11, "pipeline", 7, "view").await;
        require_pipeline(&db, &scope, 7.into(), Operation::View)
            .await
            .unwrap();
        assert!(
            require_pipeline(&db, &scope, 7.into(), Operation::Run)
                .await
                .is_err()
        );
        grant(&db, 4, 11, "pipeline_group", 50, "run").await;
        require_pipeline(&db, &scope, 7.into(), Operation::Run)
            .await
            .unwrap();
        pipelines::Entity::update_many()
            .col_expr(pipelines::Column::GroupId, Expr::value(51_i64))
            .filter(pipelines::Column::Id.eq(7_i64))
            .exec(&db)
            .await
            .unwrap();
        assert!(
            require_pipeline(&db, &scope, 7.into(), Operation::Run)
                .await
                .is_err()
        );
        pipelines::Entity::update_many()
            .col_expr(pipelines::Column::OwnerId, Expr::value(3_i64))
            .filter(pipelines::Column::Id.eq(7_i64))
            .exec(&db)
            .await
            .unwrap();
        require_pipeline(&db, &scope, 7.into(), Operation::Edit)
            .await
            .unwrap();
        scope.permissions.remove("pipelines.update");
        assert!(
            require_pipeline(&db, &scope, 7.into(), Operation::Edit)
                .await
                .is_err()
        );
        crate::migration::Migrator::up(&db, None).await.unwrap();
        assert_eq!(
            pipelines::Entity::find_by_id(7_i64)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .owner_id,
            Some(3)
        );
    }

    #[test]
    fn authorization_query_binds_identity_and_preserves_correlation_on_all_three_backends() {
        let scope = scope();
        for backend in [
            sea_orm::DbBackend::Sqlite,
            sea_orm::DbBackend::Postgres,
            sea_orm::DbBackend::MySql,
        ] {
            let sql = workflows::Entity::find()
                .filter(workflows::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(visible(&scope, Operation::View).unwrap())
                .build(backend);
            assert!(sql.sql.contains("EXISTS"));
            assert!(!sql.sql.contains("'workflow'"));
            assert!(
                sql.values
                    .unwrap()
                    .0
                    .contains(&sea_orm::Value::BigInt(Some(11)))
            );
        }
    }
}
