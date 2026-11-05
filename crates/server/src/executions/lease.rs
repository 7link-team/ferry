//! Durable execution ownership. A token never authorizes a write on its own:
//! mutations must include `fence()` in the same SQL statement/transaction.
use sea_orm::{
    Condition, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr,
    TransactionTrait,
    sea_query::{Alias, Expr, ExprTrait, Query, SimpleExpr},
};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::lock;

pub(crate) const TTL_MS: i64 = 30_000;
pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
pub(crate) const SELF_FENCE_AFTER: Duration = Duration::from_secs(22);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Pipeline,
    Workflow,
}

impl Kind {
    fn table(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline_runs",
            Self::Workflow => "workflow_executions",
        }
    }

    fn lock_prefix(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline-run",
            Self::Workflow => "workflow-execution",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Execution {
    pub kind: Kind,
    pub workspace: i64,
    pub id: i64,
}

/// The per-boot identity is persisted separately from the lock owner, whose
/// epoch also separates successive attempts within the same server process.
#[derive(Clone, Debug)]
pub(crate) struct Lease {
    backend: DbBackend,
    execution: Execution,
    boot: String,
    epoch: i64,
    lock_name: String,
    lock_owner: String,
    claimed_at: Instant,
    locally_alive: Arc<AtomicBool>,
}

impl Lease {
    /// Lock acquisition and the execution CAS commit together. Returning None
    /// rolls back a lock taken for a missing, terminal or concurrently changed row.
    pub(crate) async fn claim(
        db: &DatabaseConnection,
        execution: Execution,
        expected_epoch: i64,
        instance_boot: &str,
        now: i64,
    ) -> Result<Option<Self>, DbErr> {
        let tx = db.begin().await?;
        let token = Self::claim_in(&tx, execution, expected_epoch, instance_boot, now).await?;
        if token.is_some() {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(token)
    }

    /// Claim under an existing scheduling transaction, so workspace limits and
    /// execution ownership commit together. The caller must roll back on None.
    pub(crate) async fn claim_in(
        tx: &DatabaseTransaction,
        execution: Execution,
        expected_epoch: i64,
        instance_boot: &str,
        now: i64,
    ) -> Result<Option<Self>, DbErr> {
        if execution.id <= 0
            || execution.workspace <= 0
            || expected_epoch < 0
            || instance_boot.is_empty()
            || instance_boot.len() > 128
        {
            return Err(DbErr::Custom("invalid execution lease identity".into()));
        }
        let epoch = expected_epoch
            .checked_add(1)
            .ok_or_else(|| DbErr::Custom("execution epoch exhausted".into()))?;
        let token = Self {
            backend: tx.get_database_backend(),
            execution,
            boot: instance_boot.into(),
            epoch,
            lock_name: format!("{}:{}", execution.kind.lock_prefix(), execution.id),
            // locks.owner is VARCHAR(64) on PostgreSQL/MySQL. Preserve the full
            // boot/execution/epoch identity without truncation or schema drift.
            lock_owner: hex::encode(Sha256::digest(
                format!("{instance_boot}:{}:{epoch}", execution.id).as_bytes(),
            )),
            claimed_at: Instant::now(),
            locally_alive: Arc::new(AtomicBool::new(true)),
        };
        if !lock::acquire(tx, &token.lock_name, &token.lock_owner, now, TTL_MS).await? {
            return Ok(None);
        }
        let update = Query::update()
            .table(Alias::new(execution.kind.table()))
            .value(Alias::new("lease_owner"), instance_boot)
            .value(Alias::new("lease_epoch"), epoch)
            .value(
                Alias::new("state"),
                Expr::case(
                    Expr::col(Alias::new("state")).eq("cancelling"),
                    "cancelling",
                )
                .finally("running"),
            )
            .value(
                Alias::new("started_at"),
                Expr::case(Expr::col(Alias::new("started_at")).eq(0), now)
                    .finally(Expr::col(Alias::new("started_at"))),
            )
            .and_where(Expr::col(Alias::new("id")).eq(execution.id))
            .and_where(Expr::col(Alias::new("workspace_id")).eq(execution.workspace))
            .and_where(Expr::col(Alias::new("lease_epoch")).eq(expected_epoch))
            .and_where(Expr::col(Alias::new("state")).is_in([
                "queued",
                "waiting",
                "running",
                "cancelling",
            ]))
            // Unowned legacy running rows are never swept by a new scheduler.
            .and_where(
                Expr::col(Alias::new("state"))
                    .is_in(["queued", "waiting"])
                    .or(Expr::col(Alias::new("lease_owner")).ne("")),
            )
            .and_where(
                Expr::col(Alias::new("state"))
                    .ne("waiting")
                    .or(Expr::col(Alias::new("not_before")).is_not_null()),
            )
            .and_where(
                Expr::col(Alias::new("state"))
                    .is_in(["running", "cancelling"])
                    .or(Expr::col(Alias::new("not_before")).is_null())
                    .or(Expr::col(Alias::new("not_before")).lte(now)),
            )
            .to_owned();
        if tx
            .execute_raw(tx.get_database_backend().build(&update))
            .await?
            .rows_affected()
            != 1
        {
            return Ok(None);
        }
        Ok(Some(token))
    }

    pub(crate) fn epoch(&self) -> i64 {
        self.epoch
    }

    pub(crate) fn execution(&self) -> Execution {
        self.execution
    }

    pub(crate) fn claimed_at(&self) -> Instant {
        self.claimed_at
    }

    pub(crate) fn invalidate(&self) {
        self.locally_alive.store(false, Ordering::Release);
    }

    /// Use in every run/job/step/log mutation, not as a separate preflight read.
    /// Checking the execution epoch AND the live lock fences paused old owners
    /// even before another server has reclaimed the execution.
    fn live_lock(&self, now: SimpleExpr) -> SimpleExpr {
        let lock = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new("locks"))
            .and_where(Expr::col(Alias::new("name")).eq(&self.lock_name))
            .and_where(Expr::col(Alias::new("owner")).eq(&self.lock_owner))
            .and_where(Expr::col(Alias::new("expires_at")).gt(now))
            .to_owned();
        Expr::exists(lock)
    }

    fn identity(&self) -> Condition {
        Condition::all()
            .add(Expr::val(self.locally_alive.load(Ordering::Acquire)).eq(true))
            .add(Expr::col(Alias::new("id")).eq(self.execution.id))
            .add(Expr::col(Alias::new("workspace_id")).eq(self.execution.workspace))
            .add(Expr::col(Alias::new("lease_owner")).eq(&self.boot))
            .add(Expr::col(Alias::new("lease_epoch")).eq(self.epoch))
            .add(Expr::col(Alias::new("state")).is_in(["running", "cancelling"]))
    }

    /// Guard a mutation of the execution row directly. Keeping its identity in
    /// the outer WHERE avoids MySQL's prohibition on selecting the update target.
    pub(super) fn execution_condition_at(&self, now: SimpleExpr) -> Condition {
        self.identity().add(self.live_lock(now))
    }

    /// Guard mutations of child tables (jobs, steps and logs).
    pub(super) fn fence_at(&self, now: SimpleExpr) -> SimpleExpr {
        let run = Query::select()
            .expr(Expr::val(1))
            .from(Alias::new(self.execution.kind.table()))
            .cond_where(self.execution_condition_at(now))
            .to_owned();
        Expr::exists(run)
    }

    /// Evaluate expiry when SQL executes, including time spent waiting for a
    /// pooled connection. PostgreSQL CURRENT_TIMESTAMP is transaction-start time.
    pub(super) fn database_now(&self) -> SimpleExpr {
        Expr::cust(match self.backend {
            DbBackend::Sqlite => {
                "(CAST(strftime('%s','now') AS INTEGER) * 1000 + CAST(substr(strftime('%f','now'),4,3) AS INTEGER))"
            }
            DbBackend::Postgres => {
                "CAST(FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000) AS BIGINT)"
            }
            DbBackend::MySql => "CAST(UNIX_TIMESTAMP(SYSDATE(3)) * 1000 AS SIGNED)",
            _ => "NULL", // Unknown backends cannot authorize an execution write.
        })
    }

    #[cfg(test)]
    pub(crate) fn execution_condition(&self, now: i64) -> Condition {
        self.execution_condition_at(Expr::val(now).into())
    }

    #[cfg(test)]
    pub(crate) fn fence(&self, now: i64) -> SimpleExpr {
        self.fence_at(Expr::val(now).into())
    }

    /// Unlike the general reentrant lock, an expired execution lease cannot be
    /// revived. Ownership loss is final for this token, even without a successor.
    pub(crate) async fn renew_live(
        &self,
        db: &impl ConnectionTrait,
        now: i64,
    ) -> Result<bool, DbErr> {
        self.renew_at(db, now, self.database_now()).await
    }

    #[cfg(test)]
    pub(crate) async fn renew(&self, db: &impl ConnectionTrait, now: i64) -> Result<bool, DbErr> {
        self.renew_at(db, now, Expr::val(now).into()).await
    }

    async fn renew_at(
        &self,
        db: &impl ConnectionTrait,
        now: i64,
        clock: SimpleExpr,
    ) -> Result<bool, DbErr> {
        let expiry = now
            .checked_add(TTL_MS)
            .ok_or_else(|| DbErr::Custom("execution lease expiry overflow".into()))?;
        let update = Query::update()
            .table(Alias::new("locks"))
            .value(Alias::new("expires_at"), expiry)
            .and_where(Expr::col(Alias::new("name")).eq(&self.lock_name))
            .and_where(Expr::col(Alias::new("owner")).eq(&self.lock_owner))
            .and_where(Expr::col(Alias::new("expires_at")).gt(now))
            .and_where(Expr::col(Alias::new("expires_at")).gt(clock))
            .and_where(Expr::exists(
                Query::select()
                    .expr(Expr::val(1))
                    .from(Alias::new(self.execution.kind.table()))
                    .cond_where(self.identity())
                    .to_owned(),
            ))
            .to_owned();
        Ok(db
            .execute_raw(db.get_database_backend().build(&update))
            .await?
            .rows_affected()
            == 1)
    }

    pub(crate) async fn release(&self, db: &impl ConnectionTrait) -> Result<bool, DbErr> {
        Ok(lock::release(db, &self.lock_name, &self.lock_owner).await? == 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Renewal {
    Renewed,
    Rejected,
    Unavailable,
}

/// The scheduler polls this clock independently of blocked database I/O. A
/// successful response received after the self-fence deadline cannot resurrect
/// an orchestrator whose remote streams must already have been cancelled.
pub(crate) struct Watchdog {
    last_success: Instant,
    lost: bool,
}

impl Watchdog {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            last_success: now,
            lost: false,
        }
    }

    pub(crate) fn alive(&mut self, now: Instant) -> bool {
        self.lost |= now.saturating_duration_since(self.last_success) >= SELF_FENCE_AFTER;
        !self.lost
    }

    pub(crate) fn observe(&mut self, result: Renewal, now: Instant) -> bool {
        if !self.alive(now) {
            return false;
        }
        match result {
            Renewal::Renewed => self.last_success = now,
            Renewal::Rejected => self.lost = true,
            Renewal::Unavailable => {}
        }
        !self.lost
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::entities::locks;
    use sea_orm::{ConnectOptions, Database, EntityTrait, Statement};

    // Independent pools over the same on-disk DB exercise actual cross-owner
    // transactions; no mocked ownership or SQL success responses.
    pub(crate) async fn database() -> (tempfile::TempDir, DatabaseConnection, DatabaseConnection) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("lease.sqlite").display()
        );
        let mut options = ConnectOptions::new(url);
        options.max_connections(1).sqlx_logging(false);
        let a = Database::connect(options.clone()).await.unwrap();
        let b = Database::connect(options).await.unwrap();
        a.execute_unprepared("CREATE TABLE locks (name TEXT PRIMARY KEY, owner TEXT NOT NULL CHECK(length(owner) <= 64), acquired_at BIGINT NOT NULL, expires_at BIGINT NOT NULL)").await.unwrap();
        for table in ["pipeline_runs", "workflow_executions"] {
            a.execute_unprepared(&format!("CREATE TABLE {table} (id BIGINT PRIMARY KEY, workspace_id BIGINT NOT NULL, state TEXT NOT NULL, lease_owner TEXT NOT NULL DEFAULT '', lease_epoch BIGINT NOT NULL DEFAULT 0, started_at BIGINT NOT NULL DEFAULT 0, not_before BIGINT NULL, value BIGINT NOT NULL DEFAULT 0)")).await.unwrap();
            a.execute_unprepared(&format!(
                "INSERT INTO {table} (id,workspace_id,state) VALUES (7,11,'queued')"
            ))
            .await
            .unwrap();
        }
        (dir, a, b)
    }

    pub(crate) fn execution(kind: Kind) -> Execution {
        Execution {
            kind,
            workspace: 11,
            id: 7,
        }
    }

    pub(crate) async fn write(db: &DatabaseConnection, lease: &Lease, now: i64) -> u64 {
        let update = Query::update()
            .table(Alias::new(lease.execution.kind.table()))
            .value(Alias::new("value"), Expr::col(Alias::new("value")).add(1))
            .cond_where(lease.execution_condition(now))
            .to_owned();
        db.execute_raw(db.get_database_backend().build(&update))
            .await
            .unwrap()
            .rows_affected()
    }

    #[tokio::test]
    async fn full_boot_identity_fits_lock_owner_and_takeover_changes_its_fence() {
        let (_dir, a, b) = database().await;
        let boot = "b".repeat(128);
        let old = Lease::claim(&a, execution(Kind::Workflow), 0, &boot, 100)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old.lock_owner.len(), 64);
        assert!(old.renew(&a, 101).await.unwrap());
        let new = Lease::claim(&b, execution(Kind::Workflow), 1, &boot, 30_102)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(new.lock_owner, old.lock_owner);
        assert!(!old.release(&a).await.unwrap());
        assert_eq!(write(&b, &new, 30_103).await, 1);
    }

    #[tokio::test]
    async fn competing_pools_have_one_owner_for_both_execution_kinds() {
        let (_dir, a, b) = database().await;
        for kind in [Kind::Pipeline, Kind::Workflow] {
            let (first, second) = tokio::join!(
                Lease::claim(&a, execution(kind), 0, "instance-a:boot-1", 100),
                Lease::claim(&b, execution(kind), 0, "instance-b:boot-1", 100)
            );
            assert_ne!(first.unwrap().is_some(), second.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn takeover_fences_old_writes_renewals_and_release() {
        let (_dir, a, b) = database().await;
        for kind in [Kind::Pipeline, Kind::Workflow] {
            let old = Lease::claim(&a, execution(kind), 0, "a:boot", 100)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(write(&a, &old, 101).await, 1);
            let new = Lease::claim(&b, execution(kind), 1, "b:boot", 30_101)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(new.epoch(), 2);
            assert_eq!(write(&a, &old, 30_102).await, 0);
            assert!(!old.renew(&a, 30_102).await.unwrap());
            assert!(!old.release(&a).await.unwrap());
            assert_eq!(write(&b, &new, 30_102).await, 1);
            assert!(new.renew(&b, 30_102).await.unwrap());
        }
    }

    #[tokio::test]
    async fn expired_lease_cannot_revive_without_a_successor() {
        let (_dir, a, _b) = database().await;
        let lease = Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 100)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(write(&a, &lease, 30_100).await, 0);
        assert!(!lease.renew(&a, 30_100).await.unwrap());
        assert!(!lease.renew(&a, 30_101).await.unwrap());
    }

    #[tokio::test]
    async fn stale_epoch_rolls_back_acquired_lock() {
        let (_dir, a, _b) = database().await;
        assert!(
            Lease::claim(&a, execution(Kind::Pipeline), 9, "a:boot", 100)
                .await
                .unwrap()
                .is_none()
        );
        assert!(locks::Entity::find().all(&a).await.unwrap().is_empty());
        assert!(
            Lease::claim(&a, execution(Kind::Pipeline), 0, "b:boot", 100)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn workspace_mismatch_cannot_claim_or_leave_a_lock() {
        let (_dir, a, _b) = database().await;
        let mut other = execution(Kind::Workflow);
        other.workspace = 12;
        assert!(
            Lease::claim(&a, other, 0, "a:boot", 100)
                .await
                .unwrap()
                .is_none()
        );
        assert!(locks::Entity::find().all(&a).await.unwrap().is_empty());
        assert!(
            Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 100)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn execution_kind_namespaces_are_independent() {
        let (_dir, a, _b) = database().await;
        for kind in [Kind::Pipeline, Kind::Workflow] {
            assert!(
                Lease::claim(&a, execution(kind), 0, "same:boot", 100)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(locks::Entity::find().all(&a).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn terminal_and_legacy_unowned_running_are_not_recovered() {
        let (_dir, a, _b) = database().await;
        for state in ["succeeded", "failed", "cancelled", "running", "cancelling"] {
            a.execute_unprepared(&format!("UPDATE pipeline_runs SET state='{state}'"))
                .await
                .unwrap();
            assert!(
                Lease::claim(&a, execution(Kind::Pipeline), 0, "a:boot", 100)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(locks::Entity::find().all(&a).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancellation_survives_takeover_and_start_time_is_immutable() {
        let (_dir, a, b) = database().await;
        let old = Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 100)
            .await
            .unwrap()
            .unwrap();
        a.execute_unprepared("UPDATE workflow_executions SET state='cancelling'")
            .await
            .unwrap();
        let new = Lease::claim(&b, execution(Kind::Workflow), old.epoch(), "b:boot", 30_101)
            .await
            .unwrap()
            .unwrap();
        let row = b
            .query_one_raw(Statement::from_string(
                b.get_database_backend(),
                "SELECT state,started_at FROM workflow_executions",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "state").unwrap(), "cancelling");
        assert_eq!(row.try_get::<i64>("", "started_at").unwrap(), 100);
        assert_eq!(write(&b, &new, 30_102).await, 1);
    }

    #[tokio::test]
    async fn same_boot_competitors_cannot_both_claim_the_same_epoch() {
        let (_dir, a, b) = database().await;
        let (first, second) = tokio::join!(
            Lease::claim(&a, execution(Kind::Pipeline), 0, "same:boot", 100),
            Lease::claim(&b, execution(Kind::Pipeline), 0, "same:boot", 100)
        );
        assert_ne!(first.unwrap().is_some(), second.unwrap().is_some());
    }

    #[tokio::test]
    async fn invalid_identity_and_epoch_overflow_never_take_locks() {
        let (_dir, a, _b) = database().await;
        for (epoch, boot) in [(-1, "a:boot"), (i64::MAX, "a:boot"), (0, "")] {
            assert!(
                Lease::claim(&a, execution(Kind::Pipeline), epoch, boot, 100)
                    .await
                    .is_err()
            );
        }
        assert!(locks::Entity::find().all(&a).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn manual_wait_is_not_woken_and_delayed_work_is_not_claimed_early() {
        let (_dir, a, _b) = database().await;
        a.execute_unprepared("UPDATE workflow_executions SET state='waiting'")
            .await
            .unwrap();
        assert!(
            Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 100)
                .await
                .unwrap()
                .is_none()
        );
        a.execute_unprepared("UPDATE workflow_executions SET not_before=500")
            .await
            .unwrap();
        assert!(
            Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 499)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 500)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn log_inserts_use_the_fence_in_the_same_statement() {
        let (_dir, a, b) = database().await;
        a.execute_unprepared("CREATE TABLE test_logs (run_id BIGINT NOT NULL, seq BIGINT NOT NULL, PRIMARY KEY(run_id,seq))").await.unwrap();
        let old = Lease::claim(&a, execution(Kind::Workflow), 0, "a:boot", 100)
            .await
            .unwrap()
            .unwrap();
        let insert = |lease: &Lease, now| {
            let select = Query::select()
                .expr(Expr::val(7_i64))
                .expr(Expr::val(1_i64))
                .and_where(lease.fence(now))
                .to_owned();
            let mut query = Query::insert();
            query
                .into_table(Alias::new("test_logs"))
                .columns([Alias::new("run_id"), Alias::new("seq")])
                .select_from(select)
                .unwrap();
            a.get_database_backend().build(&query)
        };
        assert_eq!(
            a.execute_raw(insert(&old, 101))
                .await
                .unwrap()
                .rows_affected(),
            1
        );
        a.execute_unprepared("DELETE FROM test_logs").await.unwrap();
        let new = Lease::claim(&b, execution(Kind::Workflow), 1, "b:boot", 30_101)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            a.execute_raw(insert(&old, 30_102))
                .await
                .unwrap()
                .rows_affected(),
            0
        );
        assert_eq!(
            b.execute_raw(insert(&new, 30_102))
                .await
                .unwrap()
                .rows_affected(),
            1
        );
    }

    #[test]
    fn database_errors_self_fence_at_22_seconds_and_never_resurrect() {
        let start = Instant::now();
        let mut watch = Watchdog::new(start);
        assert!(watch.observe(Renewal::Unavailable, start + Duration::from_secs(10)));
        assert!(watch.alive(start + Duration::from_millis(21_999)));
        assert!(!watch.alive(start + SELF_FENCE_AFTER));
        assert!(!watch.observe(Renewal::Renewed, start + Duration::from_secs(23)));
    }

    #[test]
    fn successful_renewal_moves_deadline_but_rejection_is_immediate() {
        let start = Instant::now();
        let mut watch = Watchdog::new(start);
        assert!(watch.observe(Renewal::Renewed, start + HEARTBEAT_INTERVAL));
        assert!(watch.alive(start + Duration::from_secs(31)));
        assert!(!watch.observe(Renewal::Rejected, start + Duration::from_secs(31)));
        assert!(!watch.observe(Renewal::Renewed, start + Duration::from_secs(31)));
    }

    #[test]
    fn slow_renewal_is_credited_from_request_time_not_response_time() {
        let start = Instant::now();
        let mut watch = Watchdog::new(start);
        let requested = start + Duration::from_secs(10);
        let received = start + Duration::from_secs(21);
        assert!(watch.alive(received));
        assert!(watch.observe(Renewal::Renewed, requested));
        assert!(watch.alive(start + Duration::from_secs(31)));
        assert!(!watch.alive(start + Duration::from_secs(32)));
        assert!(!watch.observe(Renewal::Renewed, received));
    }
}
