use super::*;
use crate::{
    entities::{locks, pipeline_run_jobs as jobs, settings},
    executions,
};
use sea_orm::{ConnectOptions, Database, DatabaseConnection, Schema};

pub(crate) fn run(id: i64) -> runs::Model {
    runs::Model {
        id,
        workspace_id: 11,
        pipeline_id: 1,
        revision_id: 0,
        environment_id: 0,
        environment_name: String::new(),
        host_id: 1,
        pipeline_name: String::new(),
        host_name: String::new(),
        triggered_by_name: String::new(),
        inputs: None,
        concurrency_group: String::new(),
        lease_owner: String::new(),
        lease_epoch: 0,
        not_before: None,
        number: id,
        state: "queued".into(),
        trigger_kind: "manual".into(),
        triggered_by: 1,
        definition: String::new(),
        variable_snapshot: None,
        repo_url: String::new(),
        branch: String::new(),
        commit: String::new(),
        credential_id: 0,
        created_at: id,
        started_at: 0,
        finished_at: 0,
    }
}

fn state(db: DatabaseConnection, dir: &std::path::Path, node: u16) -> AppState {
    std::sync::Arc::new(crate::state::State {
        host_monitor: Default::default(),
        monitor: Default::default(),
        cluster: None,
        ids: std::sync::Arc::new(
            crate::snowflake::Generator::new(node, std::time::Instant::now()).unwrap(),
        ),
        db,
        data_dir: dir.into(),
        instance_id: format!("test-{node}"),
        server_mode: true,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(dir, true).unwrap(),
    })
}

pub(crate) async fn fixture(limit: i64) -> (tempfile::TempDir, AppState, AppState) {
    let dir = tempfile::tempdir().unwrap();
    let mut options = ConnectOptions::new(format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("pipeline.sqlite").display()
    ));
    options.max_connections(1).sqlx_logging(false);
    let a = Database::connect(options.clone()).await.unwrap();
    let b = Database::connect(options).await.unwrap();
    let schema = Schema::new(a.get_database_backend());
    for statement in [
        schema.create_table_from_entity(runs::Entity),
        schema.create_table_from_entity(logs::Entity),
        schema.create_table_from_entity(steps::Entity),
        schema.create_table_from_entity(jobs::Entity),
        schema.create_table_from_entity(locks::Entity),
        schema.create_table_from_entity(workspaces::Entity),
        schema.create_table_from_entity(settings::Entity),
        schema.create_table_from_entity(crate::entities::pipeline_artifacts::Entity),
        schema.create_table_from_entity(crate::entities::platform_state::Entity),
        schema.create_table_from_entity(crate::entities::holes::Entity),
    ] {
        a.execute(&statement).await.unwrap();
    }
    a.execute_unprepared("INSERT INTO platform_state (id,initialized,registration_enabled,runner_auto_update,version) VALUES (1,1,1,0,1)")
        .await
        .unwrap();
    a.execute_unprepared("CREATE UNIQUE INDEX log_sequence ON pipeline_run_logs(run_id,seq)")
        .await
        .unwrap();
    workspaces::Entity::insert(
        workspaces::Model {
            id: 11,
            slug: "tests".into(),
            name: "Tests".into(),
            owner_id: 1,
            join_code: String::new(),
            pipeline_run_number: 0,
            workflow_execution_number: 0,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&a)
    .await
    .unwrap();
    settings::Entity::insert(
        settings::Model {
            id: 1,
            workspace_id: 11,
            key: "pipeline_concurrent_runs".into(),
            category: "pipelines".into(),
            value_type: "integer".into(),
            value_json: None,
            schema_json: None,
            value: limit,
            name: String::new(),
            description: String::new(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
        .into_active_model(),
    )
    .exec(&a)
    .await
    .unwrap();
    for id in [7, 8] {
        runs::Entity::insert(run(id).into_active_model())
            .exec(&a)
            .await
            .unwrap();
        steps::Entity::insert(steps::ActiveModel {
            id: Set(id * 10),
            run_id: Set(id),
            step_index: Set(0),
            name: Set("command".into()),
            state: Set("running".into()),
            exit_code: Set(None),
            started_at: Set(1),
            finished_at: Set(0),
        })
        .exec(&a)
        .await
        .unwrap();
        jobs::Entity::insert(jobs::ActiveModel {
            id: Set(id * 10 + 1),
            run_id: Set(id),
            job_key: Set("build".into()),
            host_id: Set(1),
            state: Set("running".into()),
            outputs: Set(None),
            concurrency_group: Set(String::new()),
            started_at: Set(1),
            finished_at: Set(0),
        })
        .exec(&a)
        .await
        .unwrap();
    }
    let first = state(a, dir.path(), 1);
    let second = state(b, dir.path(), 2);
    (dir, first, second)
}

/// A transport fixture that holds the only SQL connection until remote cancel
/// is acknowledged. It reproduces the log/status-vs-process polling deadlock.
pub(crate) struct BlockedCommand {
    pub(crate) db: DatabaseConnection,
    pub(crate) started: tokio::sync::Notify,
    pub(crate) acknowledged: std::sync::atomic::AtomicBool,
}

impl BlockedCommand {
    pub(crate) fn new(s: &AppState) -> Self {
        Self {
            db: s.db.clone(),
            started: Default::default(),
            acknowledged: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl HostChannel for BlockedCommand {
    async fn exec_stream(
        &self,
        _: &str,
        _: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        let tx = self.db.begin().await?;
        output
            .send((
                "stdout".into(),
                b"waiting for cancellation\ntrailing line\n".to_vec(),
            ))
            .await
            .unwrap();
        self.started.notify_one();
        while !*cancel.borrow() {
            cancel.changed().await.unwrap();
        }
        self.acknowledged
            .store(true, std::sync::atomic::Ordering::Release);
        tx.rollback().await?;
        Ok(-130)
    }
    async fn read_file(
        &self,
        path: &str,
    ) -> Result<(
        crate::hosts::channel::FileMeta,
        crate::hosts::channel::ByteStream,
    )> {
        assert!(path.ends_with("artifact.bin"));
        Ok((
            crate::hosts::channel::FileMeta {
                size: 8,
                mode: 0o644,
                modified_at: 0,
            },
            Box::pin(futures::stream::once(async {
                Ok(Bytes::from_static(b"artifact"))
            })),
        ))
    }
    async fn monitor(&self) -> Result<ferry_protocol::HostMonitor> {
        panic!("unexpected monitor")
    }
    async fn exec(&self, _: &str, _: Duration) -> Result<crate::hosts::channel::Output> {
        panic!("unexpected exec")
    }
    async fn open_pty(
        &self,
        _: crate::hosts::channel::PtySize,
    ) -> Result<Box<dyn crate::hosts::channel::Pty>> {
        panic!("unexpected pty")
    }
    async fn list_dir(&self, _: &str) -> Result<Vec<crate::hosts::channel::DirEntry>> {
        panic!("unexpected list")
    }
    async fn create_file(&self, _: &str) -> Result<()> {
        panic!("unexpected create file")
    }
    async fn create_dir(&self, _: &str) -> Result<()> {
        panic!("unexpected create directory")
    }
    async fn write_file(
        &self,
        _: &str,
        _: u32,
        _: crate::hosts::channel::ByteStream,
    ) -> Result<()> {
        panic!("unexpected write")
    }
    async fn delete_file(&self, _: &str) -> Result<()> {
        panic!("unexpected delete")
    }
}

#[tokio::test]
async fn independent_schedulers_share_the_last_workspace_slot() {
    let (_dir, a, b) = fixture(1).await;
    let (first, second) = tokio::join!(claim(&a, "a:boot"), claim(&b, "b:boot"));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(first.is_some(), second.is_some());
    let (run, lease, interrupted) = first.or(second).unwrap();
    assert!(!interrupted);
    assert_eq!(run.lease_epoch, 1);
    assert_eq!(
        runs::Entity::find()
            .filter(runs::Column::State.eq("running"))
            .count(&a.db)
            .await
            .unwrap(),
        1
    );
    assert!(claim(&b, "c:boot").await.unwrap().is_none());
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    executions::supervise(lease, &a.db, stopped, |_| async {
        finish(&a, run.id, "succeeded").await
    })
    .await
    .unwrap();
    let (next, _, _) = claim(&b, "b:next").await.unwrap().unwrap();
    assert_ne!(next.id, run.id);
}

#[tokio::test]
async fn concurrency_group_is_exclusive_across_independent_pools() {
    let (_dir, a, b) = fixture(0).await;
    a.db.execute_unprepared("UPDATE pipeline_runs SET concurrency_group='group'")
        .await
        .unwrap();
    let (first, second) = tokio::join!(claim(&a, "a:boot"), claim(&b, "b:boot"));
    assert_ne!(first.unwrap().is_some(), second.unwrap().is_some());
    assert_eq!(
        runs::Entity::find()
            .filter(runs::Column::State.eq("running"))
            .count(&a.db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn takeover_rejects_actual_old_log_and_finish_writes() {
    let (_dir, a, b) = fixture(1).await;
    let (old_run, old, _) = claim(&a, "a:boot").await.unwrap().unwrap();
    let started = old_run.started_at;
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let result = executions::supervise(old, &a.db, stopped, |_| async {
        let mut seq = 0;
        append(&a, 7, 0, &mut seq, "stdout", "before\n")
            .await
            .unwrap();
        a.db.execute_unprepared("UPDATE locks SET expires_at=0")
            .await
            .unwrap();
        let (successor, lease, interrupted) = claim(&b, "b:boot").await.unwrap().unwrap();
        assert!(interrupted);
        assert_eq!(successor.lease_epoch, 2);
        assert_eq!(successor.started_at, started);
        assert!(
            append(&a, 7, 0, &mut seq, "stdout", "stale\n")
                .await
                .is_err()
        );
        assert_eq!(seq, 1);
        assert!(finish(&a, 7, "failed").await.is_err());
        let step = steps::Entity::find_by_id(70)
            .one(&b.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(step.state, "running");
        let (_stop, stopped) = tokio::sync::watch::channel(false);
        executions::supervise(lease, &b.db, stopped, |_| async {
            append(&b, 7, 0, &mut seq, "stdout", "successor\n")
                .await
                .unwrap();
            finish(&b, 7, "succeeded").await
        })
        .await
        .unwrap();
        Err::<(), _>(executions::ownership_lost())
    })
    .await;
    assert!(result.is_err());
    let row = runs::Entity::find_by_id(7)
        .one(&b.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "succeeded");
    assert_eq!(row.lease_owner, "b:boot");
    let records = logs::Entity::find()
        .order_by_asc(logs::Column::Seq)
        .all(&b.db)
        .await
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|log| !log.chunk.contains("stale")));
}

#[tokio::test]
async fn a_lease_never_authorizes_another_run() {
    let (_dir, a, _) = fixture(0).await;
    let (_, lease, _) = claim(&a, "a:boot").await.unwrap().unwrap();
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    executions::supervise(lease, &a.db, stopped, |_| async {
        assert!(
            append(&a, 8, 0, &mut 0, "stdout", "foreign\n")
                .await
                .is_err()
        );
        assert!(finish(&a, 8, "succeeded").await.is_err());
        assert!(!active(&a, 8).await.unwrap());
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        runs::Entity::find_by_id(8)
            .one(&a.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "queued"
    );
}

#[tokio::test]
async fn future_queue_time_and_cancellation_survive_real_claim() {
    let (_dir, a, b) = fixture(0).await;
    runs::Entity::update_many()
        .col_expr(runs::Column::NotBefore, Expr::value(db::now() + 60_000))
        .exec(&a.db)
        .await
        .unwrap();
    assert!(claim(&a, "a:boot").await.unwrap().is_none());
    runs::Entity::update_many()
        .col_expr(runs::Column::NotBefore, Expr::value(0_i64))
        .exec(&a.db)
        .await
        .unwrap();
    let (first, _, _) = claim(&a, "a:boot").await.unwrap().unwrap();
    a.db.execute_unprepared("UPDATE pipeline_runs SET state='cancelling' WHERE id=7")
        .await
        .unwrap();
    a.db.execute_unprepared("UPDATE locks SET expires_at=0")
        .await
        .unwrap();
    let (recovered, _, interrupted) = claim(&b, "b:boot").await.unwrap().unwrap();
    assert!(interrupted);
    assert_eq!(recovered.id, first.id);
    assert_eq!(recovered.state, "cancelling");
    assert_eq!(recovered.started_at, first.started_at);
}

#[tokio::test]
async fn a_stale_artifact_upload_cannot_publish_or_remove_a_successor_file() {
    use crate::{entities::pipeline_artifacts, pipelines::artifacts};
    let (_dir, a, b) = fixture(1).await;
    let (run, old, _) = claim(&a, "a:boot").await.unwrap().unwrap();
    let transport = BlockedCommand::new(&a);
    let paths = vec!["artifact.bin".into()];
    let retention = BTreeMap::new();
    let modes = BTreeMap::new();
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let result = executions::supervise(old, &a.db, stopped, |_| async {
        a.db.execute_unprepared("UPDATE locks SET expires_at=0")
            .await
            .unwrap();
        let (_, next, _) = claim(&b, "b:boot").await.unwrap().unwrap();
        let (_stop, mut stopped) = tokio::sync::watch::channel(false);
        assert!(
            artifacts::collect_with_modes(
                &a,
                &run,
                &transport,
                "/job",
                &paths,
                &retention,
                &mut stopped,
                &modes
            )
            .await
            .is_err()
        );
        assert_eq!(
            pipeline_artifacts::Entity::find()
                .count(&a.db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            std::fs::read_dir(artifacts::directory(&a, run.id))
                .unwrap()
                .count(),
            0
        );
        let (_stop, stopped) = tokio::sync::watch::channel(false);
        executions::supervise(next, &b.db, stopped, |_| async {
            let (_stop, mut stopped) = tokio::sync::watch::channel(false);
            artifacts::collect_with_modes(
                &b,
                &run,
                &transport,
                "/job",
                &paths,
                &retention,
                &mut stopped,
                &modes,
            )
            .await
        })
        .await
        .unwrap();
        Err::<(), _>(executions::ownership_lost())
    })
    .await;
    assert!(result.is_err());
    let rows = pipeline_artifacts::Entity::find().all(&b.db).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        tokio::fs::read(artifacts::file_path(&b, &rows[0]))
            .await
            .unwrap(),
        b"artifact"
    );
}
