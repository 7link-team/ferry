//! Scheduling primitives shared by Actions pipelines and graph workflows.
pub(crate) mod lease;

use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use futures::{StreamExt, stream::FuturesUnordered};
use lease::{HEARTBEAT_INTERVAL, Lease, Renewal, Watchdog};
use sea_orm::{
    Condition, DatabaseConnection,
    sea_query::{Expr, ExprTrait, SimpleExpr},
};
use std::{
    future::Future,
    time::{Duration, Instant},
};
use tokio::sync::watch;

tokio::task_local! {
    static CURRENT: Lease;
    static STOP: watch::Receiver<bool>;
}

pub(crate) fn stop_requested() -> bool {
    STOP.try_with(|stop| *stop.borrow()).unwrap_or(false)
}

pub(crate) fn stop_signal() -> Option<watch::Receiver<bool>> {
    STOP.try_with(Clone::clone).ok()
}

pub(crate) fn current() -> Option<Lease> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Unscoped callers are control-plane operations (cancel/approve) or historical
/// execution adapters. Every scheduler-owned execution runs inside supervise.
pub(crate) fn pipeline_fence(run: i64) -> SimpleExpr {
    match current() {
        Some(lease)
            if lease.execution().kind == lease::Kind::Pipeline && lease.execution().id == run =>
        {
            lease.fence_at(lease.database_now())
        }
        Some(_) => Expr::val(false).eq(true),
        None => Expr::val(true).eq(true),
    }
}

/// Direct execution-row updates must not SELECT their own target on MySQL.
pub(crate) fn pipeline_condition(run: i64) -> Condition {
    match current() {
        Some(lease)
            if lease.execution().kind == lease::Kind::Pipeline && lease.execution().id == run =>
        {
            lease.execution_condition_at(lease.database_now())
        }
        Some(_) => Condition::all().add(Expr::val(false).eq(true)),
        None => Condition::all(),
    }
}

/// Graph writes always carry an explicit lease, including recovery writes.
/// There is no unscoped fallback for the new executor.
pub(crate) fn workflow_condition(lease: &Lease, version: i64) -> Condition {
    workflow_state_condition(lease, version, "running")
}

pub(crate) fn workflow_state_condition(lease: &Lease, version: i64, state: &str) -> Condition {
    if lease.execution().kind != lease::Kind::Workflow {
        return Condition::all().add(Expr::val(false).eq(true));
    }
    lease
        .execution_condition_at(lease.database_now())
        .add(Expr::col(sea_orm::sea_query::Alias::new("version")).eq(version))
        .add(Expr::col(sea_orm::sea_query::Alias::new("state")).eq(state))
}

pub(crate) fn ownership_lost() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::CONFLICT,
        Message::new("execution.leaseLost"),
    )
}

/// Workflow numbers are workspace-wide. Keep allocation in the transaction
/// that inserts the execution and its delivery; never derive a number from MAX.
pub(crate) async fn next_workflow_number(
    tx: &sea_orm::DatabaseTransaction,
    workspace: crate::snowflake::Id,
) -> Result<i64> {
    use crate::entities::workspaces;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let row = crate::accounts::workspace_admin::lock_workspace(tx, workspace).await?;
    let number = row
        .workflow_execution_number
        .checked_add(1)
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                Message::new("execution.versionConflict"),
            )
        })?;
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::WorkflowExecutionNumber,
            Expr::value(number),
        )
        .filter(workspaces::Column::Id.eq(workspace))
        .exec(tx)
        .await?;
    Ok(number)
}

/// Drive ownership I/O independently from orchestration and its DB transactions.
/// On loss, stop every remote stream through the same cancellation channel and
/// continue polling until the executor acknowledges termination and cleanup.
pub(crate) async fn supervise<T, F, Fut>(
    lease: Lease,
    db: &DatabaseConnection,
    mut shutdown: watch::Receiver<bool>,
    execute: F,
) -> Result<T>
where
    F: FnOnce(watch::Receiver<bool>) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut watchdog = Watchdog::new(lease.claimed_at());
    if !watchdog.alive(Instant::now()) {
        lease.invalidate();
        return Err(ownership_lost());
    }
    let (cancel, receiver) = watch::channel(*shutdown.borrow());
    let execution = STOP.scope(
        receiver.clone(),
        CURRENT.scope(lease.clone(), execute(receiver)),
    );
    tokio::pin!(execution);
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut clock = tokio::time::interval(Duration::from_millis(100));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut renewals = FuturesUnordered::new();
    let mut lost = false;
    let mut shutdown_open = true;
    let result = loop {
        tokio::select! {
            result = &mut execution => break result,
            changed = shutdown.changed(), if shutdown_open => {
                if changed.is_err() { shutdown_open = false; }
                if !shutdown_open || *shutdown.borrow() { cancel.send_replace(true); }
            },
            _ = heartbeat.tick(), if !lost && renewals.is_empty() => {
                let owned = lease.clone();
                let database = db.clone();
                renewals.push(async move {
                    let started = Instant::now();
                    (started, owned.renew_live(&database, crate::db::now()).await)
                });
            },
            result = renewals.next(), if !renewals.is_empty() => {
                let (started, result) = result.expect("nonempty renewals");
                let renewal = match result {
                    Ok(true) => Renewal::Renewed,
                    Ok(false) => Renewal::Rejected,
                    Err(_) => Renewal::Unavailable,
                };
                // SQL expiry is based on request time. Crediting a slow response
                // with a fresh deadline could outlive the persisted lease.
                if !watchdog.alive(Instant::now()) || !watchdog.observe(renewal, started) {
                    lost = true;
                    lease.invalidate();
                    cancel.send_replace(true);
                    renewals.clear();
                }
            },
            _ = clock.tick(), if !lost => {
                if !watchdog.alive(Instant::now()) {
                    lost = true;
                    lease.invalidate();
                    cancel.send_replace(true);
                    renewals.clear();
                }
            },
        }
    };
    // A pending renewal may own the pool's only connection. Drop it before
    // awaiting release; otherwise its unpolled query can deadlock cleanup.
    drop(renewals);
    // Owner-conditional release cannot delete a successor's lock. A transient
    // failure leaves a TTL-bounded lock; it never changes the execution result.
    lost |= !watchdog.alive(Instant::now());
    if lost {
        lease.invalidate();
    }
    match tokio::time::timeout(Duration::from_secs(2), lease.release(db)).await {
        Ok(Ok(_)) => {}
        _ => tracing::warn!("execution lease release deferred to expiry"),
    }
    if lost { Err(ownership_lost()) } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lease::{
        Kind,
        tests::{database, execution, write},
    };
    use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test]
    async fn prepared_pipeline_guards_reject_writes_after_pool_wait_crosses_expiry() {
        use sea_orm::sea_query::{Alias, Query};
        let (_dir, db, _) = database().await;
        db.execute_unprepared("CREATE TABLE test_logs (seq BIGINT PRIMARY KEY)")
            .await
            .unwrap();
        let lease = Lease::claim(
            &db,
            execution(Kind::Pipeline),
            0,
            "a:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        CURRENT
            .scope(lease.clone(), async {
                let expiry = crate::db::now() + 500;
                db.execute_unprepared(&format!("UPDATE locks SET expires_at={expiry}"))
                    .await
                    .unwrap();
                let parent = Query::update()
                    .table(Alias::new("pipeline_runs"))
                    .value(Alias::new("value"), 1_i64)
                    .cond_where(pipeline_condition(7))
                    .to_owned();
                let select = Query::select()
                    .expr(Expr::val(1_i64))
                    .and_where(pipeline_fence(7))
                    .to_owned();
                let mut child = Query::insert();
                child
                    .into_table(Alias::new("test_logs"))
                    .columns([Alias::new("seq")])
                    .select_from(select)
                    .unwrap();
                let backend = db.get_database_backend();
                let parent = backend.build(&parent);
                let child = backend.build(&child);
                let renewal = lease.renew_live(&db, crate::db::now());
                let tx = db.begin().await.unwrap();
                let release = async {
                    while crate::db::now() <= expiry {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    tx.rollback().await.unwrap();
                };
                let writes = async {
                    let renewed = renewal.await.unwrap();
                    let parent = db.execute_raw(parent).await.unwrap().rows_affected();
                    let child = db.execute_raw(child).await.unwrap().rows_affected();
                    (renewed, parent, child)
                };
                let ((renewed, parent, child), ()) = tokio::join!(writes, release);
                assert_eq!(
                    (renewed, parent, child),
                    (false, 0, 0),
                    "prepared guards must use execution time"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn supervisor_provides_context_and_releases_its_lock() {
        let (_dir, db, _other) = database().await;
        let lease = Lease::claim(
            &db,
            execution(Kind::Workflow),
            0,
            "a:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        let (_stop, shutdown) = watch::channel(false);
        let result = supervise(lease, &db, shutdown, |_| async {
            let owned = current().expect("owned execution context");
            assert_eq!(owned.execution(), execution(Kind::Workflow));
            assert_eq!(write(&db, &owned, crate::db::now()).await, 1);
            Ok(42)
        })
        .await
        .unwrap();
        assert_eq!(result, 42);
        assert!(current().is_none());
        assert!(
            crate::entities::locks::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn rejection_cancels_executor_and_waits_for_cleanup_acknowledgement() {
        let (_dir, db, _other) = database().await;
        let lease = Lease::claim(
            &db,
            execution(Kind::Pipeline),
            0,
            "a:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        db.execute_unprepared("UPDATE locks SET owner='successor'")
            .await
            .unwrap();
        let acknowledged = Arc::new(AtomicBool::new(false));
        let ack = acknowledged.clone();
        let (_stop, shutdown) = watch::channel(false);
        let connection = &db;
        let result = supervise(lease, &db, shutdown, |mut cancel| async move {
            while !*cancel.borrow() {
                cancel.changed().await.unwrap();
            }
            let owned = current().unwrap();
            assert_eq!(write(connection, &owned, crate::db::now()).await, 0);
            tokio::task::yield_now().await;
            ack.store(true, Ordering::Release);
            Ok(())
        })
        .await;
        assert_eq!(result.unwrap_err().0, axum::http::StatusCode::CONFLICT);
        assert!(acknowledged.load(Ordering::Acquire));
        let row = crate::entities::locks::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.owner, "successor");
    }

    #[tokio::test]
    async fn a_blocked_pool_does_not_block_the_monotonic_self_fence() {
        let (_dir, db, _other) = database().await;
        let lease = Lease::claim(
            &db,
            execution(Kind::Workflow),
            0,
            "a:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        // The only connection is owned by an unpolled executor transaction.
        // A heartbeat queued on this pool cannot report either success or error.
        let tx = db.begin().await.unwrap();
        let started = Instant::now();
        let (_stop, shutdown) = watch::channel(false);
        let result = tokio::time::timeout(
            Duration::from_secs(28),
            supervise(lease, &db, shutdown, |mut cancel| async move {
                while !*cancel.borrow() {
                    cancel.changed().await.unwrap();
                }
                tx.rollback().await.unwrap();
                Ok(())
            }),
        )
        .await
        .expect("self-fence must not wait for a database connection");
        assert_eq!(result.unwrap_err().0, axum::http::StatusCode::CONFLICT);
        assert!(started.elapsed() >= Duration::from_secs(21));
        assert!(started.elapsed() < Duration::from_secs(26));
    }

    #[tokio::test]
    async fn shutdown_is_forwarded_without_becoming_ownership_loss() {
        let (_dir, db, _other) = database().await;
        let lease = Lease::claim(
            &db,
            execution(Kind::Workflow),
            0,
            "a:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        let (_stop, shutdown) = watch::channel(true);
        assert_eq!(
            supervise(lease, &db, shutdown, |cancel| async move {
                assert!(*cancel.borrow());
                Ok(7)
            })
            .await
            .unwrap(),
            7
        );
    }
}
