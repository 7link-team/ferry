//! Runner update state and the policy, admission, persistence, and recovery boundaries.
use super::*;
mod jobs;
mod persistence;
mod policy;
mod recovery;
use jobs::*;
pub use jobs::{UpdateBody, start_update, update_status};
use persistence::*;
pub(crate) use policy::automatic_updates;
pub(crate) use policy::older_version;
use policy::*;
pub(crate) use recovery::recover_updates;
#[cfg(test)]
mod auto_tests;

pub(super) fn stored_update_failure(error: &str, identity: Option<wire::SystemMessage>) -> String {
    let message = identity
        .filter(|message| {
            message.params.is_empty()
                && matches!(
                    message.key.as_str(),
                    "runner.updateUnavailable" | "runner.failedTarget" | "runner.updateFailed"
                )
        })
        .map(|message| crate::i18n::Message::new(message.key))
        .unwrap_or_else(|| {
            crate::i18n::Message::new("runner.updateRejected")
                .with("detail", error.chars().take(512).collect::<String>())
        });
    crate::i18n::store_message(&message)
}

// Upgrade tasks outlive their HTTP request so closing the page cannot release
// the exclusive host reservation while a remote version switch is in flight.
#[derive(Clone, Serialize, Deserialize)]
struct AutoRetry {
    attempts: u32,
    next_retry_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
struct UpdateJob {
    #[serde(default)]
    auto: Option<AutoRetry>,
    request: String,
    version: String,
    state: String,
    message: String,
    #[serde(default)]
    owner: Option<String>,
}

static UPDATES: LazyLock<Mutex<HashMap<String, UpdateJob>>> = LazyLock::new(Default::default);

#[cfg(test)]
mod tests {
    #[test]
    fn empty_update_request_selects_server_catalog_without_breaking_explicit_requests() {
        let automatic: super::UpdateBody = serde_json::from_str("{}").unwrap();
        assert!(
            automatic.version.is_empty()
                && automatic.manifest.is_empty()
                && automatic.signature.is_empty()
        );
        let explicit: super::UpdateBody = serde_json::from_value(serde_json::json!({"version":"1.2.3","manifest":"signed catalog","signature":"signature"})).unwrap();
        assert_eq!(explicit.version, "1.2.3");
        assert_eq!(explicit.manifest, "signed catalog");
        assert_eq!(explicit.signature, "signature");
    }
    use super::*;
    use crate::hosts::runner::test_support::scratch_db;

    #[tokio::test]
    async fn audit_failure_recovers_after_failed_status_write_becomes_available() {
        use sea_orm::ConnectionTrait;
        let scratch = scratch_db().await;
        let dir = tempfile::tempdir().unwrap();
        let id = Id::from(9107);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("update recovery".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set("1.0.0".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&scratch.db)
        .await
        .unwrap();
        // Real database failures at the two distinct writes, without mocking
        // admission or the persistence code being exercised.
        scratch.db.execute_unprepared("CREATE TRIGGER fail_audit BEFORE INSERT ON activity BEGIN SELECT RAISE(FAIL, 'audit unavailable'); END").await.unwrap();
        scratch.db.execute_unprepared("CREATE TRIGGER fail_update_status BEFORE UPDATE OF agent_update ON hosts WHEN NEW.agent_update LIKE '%failed%' BEGIN SELECT RAISE(FAIL, 'status unavailable'); END").await.unwrap();
        let s = Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db: scratch.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "update-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let (control, mut receiver) = mpsc::channel(1);
        let (disconnect, _) = watch::channel(false);
        live().insert(
            id.to_string(),
            Arc::new(Live {
                owner: None,
                owner_revision: std::sync::atomic::AtomicU64::new(0),
                key_hash: String::new(),
                control,
                disconnect,
                managed: true,
            installation_scope: None,
                supports_monitor: true,
                supports_agent: false,
                supports_agent_probe: false,
                version: "1.0.0".into(),
                ready: std::sync::atomic::AtomicBool::new(true),
                failed_version: Mutex::new(None),
                update_error: Mutex::new(None),
            }),
        );
        let scope = Scope {
            workspace_id: Id::from(1),
            user: crate::accounts::auth::User {
                id: Id::from(1),
                email: "test@example.invalid".into(),
                name: "test".into(),
                is_superadmin: false,
            },
            permissions: ["hosts.update".into()].into_iter().collect(),
        };
        let mut request = tokio::spawn(start_update(
            State(s),
            scope,
            Path(("test".into(), id)),
            Json(UpdateBody {
                version: "2.0.0".into(),
                manifest: "{}".into(),
                signature: "fixture".into(),
            }),
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut request)
                .await
                .is_err(),
            "admission must keep recovering after audit and status writes both fail"
        );
        assert!(
            receiver.try_recv().is_err(),
            "an unaudited update must never reach the runner"
        );
        scratch
            .db
            .execute_unprepared("DROP TRIGGER fail_update_status")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), request)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<UpdateJob>(row.agent_update.as_deref().unwrap())
                .unwrap()
                .state,
            "failed"
        );
        assert!(
            !UPDATES
                .lock()
                .unwrap()
                .get(&id.to_string())
                .is_some_and(|job| job.state == "draining")
        );
        live().remove(&id.to_string());
        UPDATES.lock().unwrap().remove(&id.to_string());
    }

    #[tokio::test]
    async fn recovery_blocks_tasks_until_health_and_durable_terminal_status() {
        use sea_orm::ConnectionTrait;
        let scratch = scratch_db().await;
        let dir = tempfile::tempdir().unwrap();
        let id = Id::from(9108);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("update recovery".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set("1.0.0".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&scratch.db)
        .await
        .unwrap();
        let s = Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db: scratch.db.clone(),
            data_dir: dir.path().into(),
            instance_id: "update-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let job = UpdateJob {
            auto: None,
            owner: None,
            request: "recovery".into(),
            version: "2.0.0".into(),
            state: "updating".into(),
            message: String::new(),
        };
        persist_update(&s, id, &job).await.unwrap();
        recover_updates(&s).await.unwrap();
        let mut task = tokio::spawn(async move {
            crate::hosts::runner::reservations::lease(&id.to_string()).await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut task)
                .await
                .is_err(),
            "offline recovery must exclude new work"
        );
        let (control, _receiver) = mpsc::channel(1);
        let (disconnect, _) = watch::channel(false);
        let peer = Arc::new(Live {
            owner: None,
            owner_revision: std::sync::atomic::AtomicU64::new(0),
            key_hash: String::new(),
            control,
            disconnect,
            managed: true,
            installation_scope: None,
            supports_monitor: true,
            supports_agent: false,
            supports_agent_probe: false,
            version: "2.0.0".into(),
            ready: std::sync::atomic::AtomicBool::new(false),
            failed_version: Mutex::new(None),
            update_error: Mutex::new(None),
        });
        live().insert(id.to_string(), peer.clone());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut task)
                .await
                .is_err(),
            "unconfirmed connection must not release recovery lock"
        );
        scratch.db.execute_unprepared("CREATE TRIGGER fail_terminal_status BEFORE UPDATE OF agent_update ON hosts WHEN NEW.agent_update LIKE '%succeeded%' BEGIN SELECT RAISE(FAIL, 'status unavailable'); END").await.unwrap();
        peer.ready.store(true, std::sync::atomic::Ordering::Release);
        assert!(
            tokio::time::timeout(Duration::from_millis(500), &mut task)
                .await
                .is_err(),
            "health without durable status must not release recovery lock"
        );
        scratch
            .db
            .execute_unprepared("DROP TRIGGER fail_terminal_status")
            .await
            .unwrap();
        drop(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap(),
        );
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<UpdateJob>(row.agent_update.as_deref().unwrap())
                .unwrap()
                .state,
            "succeeded"
        );
        live().remove(&id.to_string());
        UPDATES.lock().unwrap().remove(&id.to_string());
    }

    #[tokio::test]
    #[ignore = "requires NATS_TEST_URL pointing at an isolated JetStream server"]
    async fn cluster_update_recovery_fences_stale_workers_without_blocking_collection() {
        let scratch = scratch_db().await;
        let dir = tempfile::tempdir().unwrap();
        let url = std::env::var("NATS_TEST_URL").unwrap();
        let prefix = format!("updates_{}", uuid::Uuid::new_v4().simple());
        let a = crate::cluster::Cluster::connect(&url, &prefix, 1, scratch.db.clone())
            .await
            .unwrap();
        let b = crate::cluster::Cluster::connect(&url, &prefix, 1, scratch.db.clone())
            .await
            .unwrap();
        let state = |cluster: Arc<crate::cluster::Cluster>| {
            Arc::new(crate::state::State {
                host_monitor: Default::default(),
                monitor: Default::default(),
                cluster: Some(cluster),
                ids: Arc::new(
                    crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
                ),
                db: scratch.db.clone(),
                data_dir: dir.path().into(),
                instance_id: secret(),
                server_mode: false,
                secure_cookie: false,
                bind: "127.0.0.1:0".parse().unwrap(),
                vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            })
        };
        let sa = state(a.clone());
        let sb = state(b.clone());
        let id = Id::from(9119);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("recovery".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("offline".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set("1.0.0".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&scratch.db)
        .await
        .unwrap();
        let mut old = UpdateJob {
            auto: None,
            owner: Some(a.node.clone()),
            request: secret(),
            version: "2.0.0".into(),
            state: "updating".into(),
            message: String::new(),
        };
        persist_update(&sa, id, &old).await.unwrap();
        assert_eq!(
            persist_update(&sb, id, &old).await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        orphan_update(&sa, id, &mut old).await;
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        let orphan: UpdateJob = serde_json::from_str(row.agent_update.as_deref().unwrap()).unwrap();
        assert!(orphan.owner.is_none());
        assert_eq!(orphan.state, "unknown");
        assert!(!transition_update(&sa, id, &mut old, "failed", "stale worker").await);
        let (_, revision) = b
            .acquire_owner(&sb, &id.to_string(), 1.into(), secret())
            .await
            .unwrap();
        let reader = b.reserve(&id.to_string(), true).await.unwrap();
        tokio::time::timeout(Duration::from_millis(500), recover_updates(&sb))
            .await
            .expect("recovery must not await the execution lock on the collection task")
            .unwrap();
        let recovered = hosts::Entity::find_by_id(i64::from(id))
            .one(&scratch.db)
            .await
            .unwrap()
            .unwrap();
        let recovered: UpdateJob =
            serde_json::from_str(recovered.agent_update.as_deref().unwrap()).unwrap();
        assert_eq!(recovered.owner.as_deref(), Some(b.node.as_str()));
        assert_eq!(recovered.state, "unknown");
        assert_eq!(
            persist_update(&sa, id, &old).await.unwrap_err().0,
            StatusCode::CONFLICT
        );
        drop(reader);
        b.release(&id.to_string(), revision).await;
        UPDATES.lock().unwrap().remove(&id.to_string());
    }
}
