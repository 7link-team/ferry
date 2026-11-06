//! Exercise production builders and entities, rather than a reduced lease table.
use super::*;
use crate::{
    entities::{workflow_executions as executions, workflow_node_runs as attempts, workflows},
    executions::lease::{Execution, Kind, Lease, TTL_MS},
};
use sea_orm::{
    ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, Set, TransactionTrait, sea_query::Expr,
};

pub(crate) fn definition(id: i64) -> workflows::Model {
    workflows::Model {
        id,
        workspace_id: 11,
        name: "Workflow".into(),
        group_id: 0,
        description: String::new(),
        definition: r#"{"engine":"ferry-graph","schema":1,"nodes":[],"connections":[]}"#.into(),
        draft_revision_id: 15,
        published_revision_id: None,
        settings: "{}".into(),
        owner_id: Some(3),
        trigger_run_as: None,
        lifecycle: "active".into(),
        archived_at: None,
        archived_by: 0,
        created_by: 3,
        created_at: 100,
        updated_at: 100,
        updated_by: 3,
        version: 1,
    }
}

pub(crate) fn execution(id: i64) -> executions::Model {
    executions::Model {
        id,
        workspace_id: 11,
        workflow_id: 7,
        revision_id: 15,
        number: 1,
        mode: "manual".into(),
        state: "queued".into(),
        trigger_kind: "manual".into(),
        trigger_ref: "start".into(),
        triggered_by: 3,
        parent_execution_id: 0,
        retry_of_id: 0,
        root_execution_id: id,
        cost_micros: 0,
        data_bytes: 0,
        created_at: 100,
        created_by: 3,
        started_at: 0,
        started_by: 0,
        finished_at: 0,
        finished_by: 0,
        lease_owner: String::new(),
        lease_epoch: 0,
        priority: 0,
        not_before: None,
        end_reason: String::new(),
        concurrency_key: String::new(),
        concurrency_limit: 0,
        queue_reason: String::new(),
        context: Some(r#"{"entry":"start","ready":["work"],"outputs":{}}"#.into()),
        waiting_ms: 0,
        response_state: "not_applicable".into(),
        response_deadline_at: None,
        response_deadline_by: 0,
        response_delivered_at: None,
        response_delivered_by: 0,
        version: 1,
    }
}

fn attempt(id: i64) -> attempts::Model {
    attempts::Model {
        id,
        execution_id: 8,
        node_id: "work".into(),
        iteration_path: "[]".into(),
        iteration_hash: "0".repeat(64),
        attempt: 1,
        state: "pending".into(),
        items_in: 0,
        items_out: 0,
        output: "{}".into(),
        output_asset: None,
        logs: None,
        logs_truncated: false,
        logs_updated_at: 0,
        error: String::new(),
        cache_key: String::new(),
        cache_hit_from: None,
        cost_micros: 0,
        started_at: 0,
        started_by: 0,
        finished_at: 0,
        finished_by: 0,
        resolved_params: Some(r#"{"value":"frozen"}"#.into()),
        seed: Some(i64::MAX),
        redo_round: 0,
        error_policy: "stop".into(),
        version: 1,
    }
}

async fn database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

#[tokio::test]
async fn workflow_storage_preserves_snapshots_and_null_identity_across_restart_and_repeat_upgrade()
{
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("workflow.sqlite").display()
    );
    let mut options = ConnectOptions::new(url);
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options.clone()).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let mut draft = definition(9_007_199_254_740_993);
    draft.owner_id = None;
    // Exercise the text fields beyond MySQL TEXT's 64 KiB capacity. The shared
    // schema tests additionally require LONGTEXT builders on MySQL.
    draft.description = "文字".repeat(40_000);
    workflows::Entity::insert(draft.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let run = execution(8);
    executions::Entity::insert(run.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let node = attempt(9);
    attempts::Entity::insert(node.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    WorkflowStorage.up(&SchemaManager::new(&db)).await.unwrap();
    assert!(
        WorkflowStorage
            .down(&SchemaManager::new(&db))
            .await
            .is_err()
    );
    db.close().await.unwrap();
    let db = Database::connect(options).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    WorkflowStorage.up(&SchemaManager::new(&db)).await.unwrap();
    assert_eq!(
        workflows::Entity::find_by_id(draft.id)
            .one(&db)
            .await
            .unwrap(),
        Some(draft)
    );
    assert_eq!(
        executions::Entity::find_by_id(run.id)
            .one(&db)
            .await
            .unwrap(),
        Some(run)
    );
    assert_eq!(
        attempts::Entity::find_by_id(node.id)
            .one(&db)
            .await
            .unwrap(),
        Some(node)
    );
    db.close().await.unwrap();
}

#[tokio::test]
async fn execution_numbers_and_node_attempts_have_the_required_identity_boundaries() {
    let db = database().await;
    executions::Entity::insert(execution(8).into_active_model())
        .exec(&db)
        .await
        .unwrap();
    assert!(
        executions::Entity::insert(execution(9).into_active_model())
            .exec(&db)
            .await
            .is_err()
    );
    let mut other = execution(10);
    other.workspace_id = 12;
    executions::Entity::insert(other.into_active_model())
        .exec(&db)
        .await
        .unwrap();
    attempts::Entity::insert(attempt(20).into_active_model())
        .exec(&db)
        .await
        .unwrap();
    assert!(
        attempts::Entity::insert(attempt(21).into_active_model())
            .exec(&db)
            .await
            .is_err()
    );
    let mut next = attempt(22);
    next.attempt = 2;
    attempts::Entity::insert(next.into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let mut iteration = attempt(23);
    iteration.iteration_path = "[1]".into();
    iteration.iteration_hash = "1".repeat(64);
    attempts::Entity::insert(iteration.into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let mut invalid = attempt(24);
    invalid.redo_round = -1;
    assert!(
        attempts::Entity::insert(invalid.into_active_model())
            .exec(&db)
            .await
            .is_err()
    );
    assert_eq!(attempts::Entity::find().count(&db).await.unwrap(), 3);
    let mut invalid = execution(11);
    invalid.number = 2;
    invalid.waiting_ms = -1;
    assert!(
        executions::Entity::insert(invalid.into_active_model())
            .exec(&db)
            .await
            .is_err()
    );
    let mut invalid = definition(7);
    invalid.lifecycle = "deleted".into();
    assert!(
        workflows::Entity::insert(invalid.into_active_model())
            .exec(&db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn production_workflow_table_supports_lease_takeover_without_rewriting_frozen_context() {
    let db = database().await;
    let snapshot = execution(8);
    executions::Entity::insert(snapshot.clone().into_active_model())
        .exec(&db)
        .await
        .unwrap();
    let identity = Execution {
        kind: Kind::Workflow,
        workspace: 11,
        id: 8,
    };
    let first = Lease::claim(&db, identity, 0, "first-boot", 100)
        .await
        .unwrap()
        .unwrap();
    assert!(
        Lease::claim(&db, identity, 1, "second-boot", 101)
            .await
            .unwrap()
            .is_none()
    );
    let expires = 100 + TTL_MS;
    // General lock acquisition retains its strict expiry comparison. At the
    // exact boundary the old owner is fenced, and a successor waits one tick.
    assert!(!first.renew(&db, expires).await.unwrap());
    assert!(
        Lease::claim(&db, identity, 1, "second-boot", expires)
            .await
            .unwrap()
            .is_none()
    );
    let now = expires + 1;
    let second = Lease::claim(&db, identity, 1, "second-boot", now)
        .await
        .unwrap()
        .unwrap();
    let write = |lease: &Lease| {
        executions::Entity::update_many()
            .col_expr(executions::Column::QueueReason, Expr::value("owned"))
            .filter(lease.execution_condition(now))
            .exec(&db)
    };
    assert_eq!(write(&first).await.unwrap().rows_affected, 0);
    assert_eq!(write(&second).await.unwrap().rows_affected, 1);
    let row = executions::Entity::find_by_id(8)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.context, snapshot.context);
    assert_eq!(row.revision_id, snapshot.revision_id);
    assert_eq!(row.triggered_by, snapshot.triggered_by);
    assert_eq!(row.started_at, 100);
    assert_eq!(row.lease_epoch, 2);
    assert_eq!(row.lease_owner, "second-boot");
    assert!(!first.renew(&db, now).await.unwrap());
}

#[tokio::test]
async fn workspace_workflow_number_allocation_serializes_and_rolls_back_with_its_execution() {
    use crate::entities::workspaces;
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("numbers.sqlite").display()
    );
    let mut options = ConnectOptions::new(url);
    options.max_connections(1).sqlx_logging(false);
    let a = Database::connect(options.clone()).await.unwrap();
    Migrator::up(&a, None).await.unwrap();
    let b = Database::connect(options).await.unwrap();
    workspaces::Entity::insert(workspaces::ActiveModel {
        id: Set(11),
        slug: Set("workflow".into()),
        name: Set("Workflow".into()),
        owner_id: Set(3),
        join_code: Set(String::new()),
        pipeline_run_number: Set(71),
        // Leave the new column unset to verify its upgrade/fresh-schema default.
        created_at: Set(100),
        updated_at: Set(100),
        ..Default::default()
    })
    .exec(&a)
    .await
    .unwrap();
    let allocate = |db: DatabaseConnection, id| async move {
        let tx = db.begin().await.unwrap();
        let number = crate::executions::next_workflow_number(&tx, 11.into())
            .await
            .unwrap();
        let mut row = execution(id);
        row.number = number;
        executions::Entity::insert(row.into_active_model())
            .exec(&tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        number
    };
    let (one, two) = tokio::join!(allocate(a.clone(), 8), allocate(b.clone(), 9));
    let mut numbers = [one, two];
    numbers.sort();
    assert_eq!(numbers, [1, 2]);
    let tx = a.begin().await.unwrap();
    assert_eq!(
        crate::executions::next_workflow_number(&tx, 11.into())
            .await
            .unwrap(),
        3
    );
    let mut row = execution(10);
    row.number = 3;
    executions::Entity::insert(row.into_active_model())
        .exec(&tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(executions::Entity::find().count(&b).await.unwrap(), 2);
    let row = workspaces::Entity::find_by_id(11)
        .one(&b)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.workflow_execution_number, 2);
    assert_eq!(row.pipeline_run_number, 71);
    WorkflowStorage.up(&SchemaManager::new(&b)).await.unwrap();
    assert_eq!(allocate(b.clone(), 10).await, 3);
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::WorkflowExecutionNumber,
            Expr::value(i64::MAX),
        )
        .filter(workspaces::Column::Id.eq(11_i64))
        .exec(&b)
        .await
        .unwrap();
    let tx = b.begin().await.unwrap();
    assert!(
        crate::executions::next_workflow_number(&tx, 11.into())
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        workspaces::Entity::find_by_id(11)
            .one(&a)
            .await
            .unwrap()
            .unwrap()
            .workflow_execution_number,
        i64::MAX
    );
}
