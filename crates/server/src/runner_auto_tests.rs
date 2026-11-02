use super::*;
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn persisted_update_http_status_uses_each_readers_language() {
    use crate::i18n::{Locale, Message};
    let f = Fixture::new().await;
    let message = Message::new("runner.verification")
        .with_message("detail", Message::new("runner.release.signatureInvalid"));
    let stored = crate::i18n::store_message(&message);
    let job = UpdateJob {
        auto: None,
        owner: None,
        request: "localized".into(),
        version: "9.9.9".into(),
        state: "failed".into(),
        message: stored.clone(),
    };
    persist_update(&f.s, f.id, &job).await.unwrap();
    let id = f.id;
    let app = axum::Router::new()
        .route(
            "/status",
            axum::routing::get(move |State(s): State<AppState>| async move {
                let scope = Scope {
                    workspace_id: 1.into(),
                    user: crate::auth::User {
                        id: 1.into(),
                        name: "reader".into(),
                        email: String::new(),
                        is_superadmin: false,
                    },
                    permissions: ["hosts.read".into()].into_iter().collect(),
                };
                update_status(State(s), scope, Path(("1".into(), id))).await
            }),
        )
        .with_state(f.s.clone())
        .layer(axum::middleware::from_fn(crate::i18n::middleware));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/status", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    for locale in Locale::ALL {
        let response = client
            .get(&url)
            .header("Accept-Language", locale.tag())
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["Content-Language"], locale.tag());
        let value: Value = response.json().await.unwrap();
        assert_eq!(
            value["update"]["message"],
            crate::i18n::render(locale, &message)
        );
        assert!(
            !value["update"]["message"]
                .as_str()
                .unwrap()
                .contains("$system_message")
        );
        assert_eq!(
            f.job().await.message,
            stored,
            "a reader must not rewrite stored identity"
        );
    }
    let legacy = "provider/user output {runner.verification}";
    let mut job = job;
    job.message = legacy.into();
    persist_update(&f.s, f.id, &job).await.unwrap();
    let value: Value = client
        .get(&url)
        .header("Accept-Language", "ja-JP")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["update"]["message"], legacy);
    server.abort();
}

#[test]
fn runner_report_identity_is_optional_and_raw_details_stay_literal() {
    use crate::i18n::{Locale, Message};
    let old: wire::Report = serde_json::from_value(
        json!({"type":"update_failed","request":"old","error":"raw diagnostic"}),
    )
    .unwrap();
    assert!(matches!(
        old,
        wire::Report::UpdateFailed { message: None, .. }
    ));
    let stored = stored_update_failure(
        "legacy diagnostic",
        Some(wire::SystemMessage::new("runner.updateFailed")),
    );
    for locale in Locale::ALL {
        assert_eq!(
            crate::i18n::render_stored(locale, &stored),
            crate::i18n::render(locale, &Message::new("runner.updateFailed"))
        );
    }
    let raw = crate::i18n::store_message(&Message::new("runner.confirmed"));
    let stored = stored_update_failure(&raw, None);
    assert!(crate::i18n::render_stored(Locale::JaJp, &stored).ends_with(&raw));
}

struct Fixture {
    s: AppState,
    id: Id,
    peer: Arc<Live>,
    receiver: mpsc::Receiver<wire::Control>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut options = sea_orm::ConnectOptions::new("sqlite://runner-auto");
        let path = dir.path().join("db.sqlite");
        options
            .sqlx_logging(false)
            .map_sqlx_sqlite_opts(move |opts| opts.filename(&path).create_if_missing(true));
        let db = Database::connect(options).await.unwrap();
        crate::migration::Migrator::up(&db, None).await.unwrap();
        let s = Arc::new(crate::state::State {
            host_monitor: Default::default(),
            cluster: None,
            ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
            db,
            data_dir: dir.path().into(),
            instance_id: "auto-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let id = Id::from(i64::from(rand::random::<u32>()) + 100);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id.into()),
            workspace_id: Set(1),
            name: Set("auto fixture".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            os: Set("linux".into()),
            arch: Set("x86_64".into()),
            agent_version: Set("0.0.1".into()),
            metrics: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&s.db)
        .await
        .unwrap();
        let (control, receiver) = mpsc::channel(4);
        let (disconnect, _) = watch::channel(false);
        let peer = Arc::new(Live {
            owner: None,
            owner_revision: std::sync::atomic::AtomicU64::new(0),
            key_hash: String::new(),
            control,
            disconnect,
            managed: true,
            supports_monitor: true,
            version: "0.0.1".into(),
            ready: std::sync::atomic::AtomicBool::new(true),
            failed_version: Mutex::new(None),
            update_error: Mutex::new(None),
        });
        live().insert(id.to_string(), peer.clone());
        Self {
            s,
            id,
            peer,
            receiver,
            _dir: dir,
        }
    }

    async fn host(&self) -> hosts::Model {
        hosts::Entity::find_by_id(i64::from(self.id))
            .one(&self.s.db)
            .await
            .unwrap()
            .unwrap()
    }

    async fn job(&self) -> UpdateJob {
        serde_json::from_str(self.host().await.agent_update.as_deref().unwrap()).unwrap()
    }

    async fn wait_state(&self, expected: &str) -> UpdateJob {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let job = self.job().await;
                if job.state == expected {
                    return job;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn start(&self) -> Result<(StatusCode, Json<Value>)> {
        begin_update(
            self.s.clone(),
            self.host().await,
            UpdateBody {
                version: env!("CARGO_PKG_VERSION").into(),
                manifest: "{}".into(),
                signature: "fixture".into(),
            },
            (Id::from(0), "system".into()),
            true,
        )
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        live().remove(&self.id.to_string());
        UPDATES.lock().unwrap().remove(&self.id.to_string());
    }
}

#[test]
fn automatic_policy_resolution_semver_and_retry_bounds() {
    for global in [false, true] {
        assert_eq!(policy_enabled(global, "inherit"), global);
        assert!(policy_enabled(global, "on"));
        assert!(!policy_enabled(global, "off"));
        assert!(!policy_enabled(global, "invalid"));
    }
    assert!(older_version("1.0.0-alpha.9", "1.0.0-alpha.10"));
    assert!(older_version("1.0.0-rc.1", "1.0.0"));
    for current in ["1.0.0", "1.0.1", "1.0.0+local", "invalid"] {
        assert!(!older_version(current, "1.0.0"));
    }
    assert_eq!(retry_delay(1), 30_000);
    assert_eq!(retry_delay(2), 60_000);
    assert_eq!(retry_delay(u32::MAX), 3_600_000);
    let old: UpdateJob = serde_json::from_str(
        r#"{"request":"r","version":"1.0.0","state":"failed","message":"legacy"}"#,
    )
    .unwrap();
    assert!(old.auto.is_none());
}

#[tokio::test]
async fn automatic_defaults_and_eligibility_preserve_failed_target_and_backoff() {
    let f = Fixture::new().await;
    assert!(global_auto_update(&f.s).await.unwrap());
    let mut host = f.host().await;
    assert_eq!(host.runner_auto_update, "inherit");
    assert_eq!(runner_target(&host).unwrap(), "x86_64-unknown-linux-gnu");
    let target = env!("CARGO_PKG_VERSION");
    assert!(auto_due(&host, &f.peer, true, target, 100));
    assert!(!auto_due(&host, &f.peer, false, target, 100));
    host.runner_auto_update = "on".into();
    assert!(auto_due(&host, &f.peer, false, target, 100));
    f.peer
        .ready
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(!auto_due(&host, &f.peer, true, target, 100));
    f.peer
        .ready
        .store(true, std::sync::atomic::Ordering::Release);
    *f.peer.failed_version.lock().unwrap() = Some(target.into());
    assert!(!auto_due(&host, &f.peer, true, target, 100));
    *f.peer.failed_version.lock().unwrap() = None;
    let job = UpdateJob {
        auto: Some(AutoRetry {
            attempts: 2,
            next_retry_at: 200,
        }),
        request: "r".into(),
        version: target.into(),
        state: "retrying".into(),
        message: String::new(),
        owner: None,
    };
    persist_update(&f.s, f.id, &job).await.unwrap();
    recover_updates(&f.s).await.unwrap();
    host = f.host().await;
    assert!(!auto_due(&host, &f.peer, true, target, 199));
    assert!(auto_due(&host, &f.peer, true, target, 200));
    host.agent_update = Some(
        serde_json::to_string(&UpdateJob {
            state: "rolled_back".into(),
            ..job
        })
        .unwrap(),
    );
    assert!(!auto_due(&host, &f.peer, true, target, 999));
}

#[tokio::test]
async fn automatic_drain_can_be_disabled_without_interrupting_whole_task() {
    let mut f = Fixture::new().await;
    let existing_task = crate::runner_updates::lease(&f.id.to_string()).await;
    let (_, Json(response)) = f.start().await.unwrap();
    assert_eq!(response["state"], "draining");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), f.receiver.recv())
            .await
            .is_err()
    );
    f.s.db
        .execute_unprepared("UPDATE platform_state SET runner_auto_update=0 WHERE id=1")
        .await
        .unwrap();
    f.wait_state("cancelled").await;
    // Cancellation releases the queued writer while the original task remains alive.
    let new_task = tokio::time::timeout(
        Duration::from_secs(1),
        crate::runner_updates::lease(&f.id.to_string()),
    )
    .await
    .unwrap();
    assert!(f.receiver.try_recv().is_err());
    drop(new_task);
    drop(existing_task);
}

#[tokio::test]
async fn automatic_lifecycle_audits_system_and_persists_failed_attempt_backoff() {
    let mut f = Fixture::new().await;
    let _ = f.start().await.unwrap();
    let control = tokio::time::timeout(Duration::from_secs(3), f.receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let wire::Control::Update {
        request, version, ..
    } = control
    else {
        panic!("expected signed update frame")
    };
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    let audit = crate::entities::activity::Entity::find()
        .one(&f.s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.actor_id, 0);
    assert_eq!(audit.action, "runner-update");
    assert_eq!(
        serde_json::from_str::<Value>(&audit.details).unwrap()["automatic"],
        true
    );
    *f.peer.update_error.lock().unwrap() = Some((request, "download failed".into()));
    let failed = f.wait_state("failed").await;
    let retry = failed.auto.unwrap();
    assert_eq!(retry.attempts, 1);
    assert!(retry.next_retry_at > db::now());
    assert!(!auto_due(
        &f.host().await,
        &f.peer,
        true,
        &version,
        db::now()
    ));
    recover_updates(&f.s).await.unwrap();
    assert_eq!(
        f.job().await.auto.unwrap().next_retry_at,
        retry.next_retry_at
    );
}

#[tokio::test]
async fn automatic_catalog_failure_never_sends_and_is_persistently_throttled() {
    let mut f = Fixture::new().await;
    let mut catalog = Some(crate::runner_install::SignedCatalog {
        bytes: b"tampered".to_vec(),
        signature: "invalid".into(),
        key: "invalid".into(),
    });
    automatic_update_cycle(&f.s, &mut catalog).await.unwrap();
    let job = f.wait_state("retrying").await;
    assert_eq!(job.auto.as_ref().unwrap().attempts, 1);
    assert!(f.receiver.try_recv().is_err());
    assert!(catalog.is_none());
    automatic_update_cycle(&f.s, &mut catalog).await.unwrap();
    assert_eq!(f.job().await.request, job.request);
}

#[tokio::test]
async fn automatic_metadata_failure_cannot_overwrite_manual_admission() {
    let f = Fixture::new().await;
    let active = UpdateJob {
        auto: None,
        request: "manual".into(),
        version: "2.0.0".into(),
        state: "draining".into(),
        message: String::new(),
        owner: None,
    };
    persist_update(&f.s, f.id, &active).await.unwrap();
    let failed = UpdateJob {
        request: "automatic".into(),
        state: "retrying".into(),
        ..active
    };
    assert_eq!(
        persist_update(&f.s, f.id, &failed).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    assert_eq!(f.job().await.request, "manual");
}

#[tokio::test]
async fn automatic_drain_holds_new_work_until_healthy_target_reconnects() {
    let mut f = Fixture::new().await;
    let task = crate::runner_updates::lease(&f.id.to_string()).await;
    let _ = f.start().await.unwrap();
    assert!(f.start().await.is_err(), "one active update per host");
    tokio::task::yield_now().await;
    let id = f.id.to_string();
    let mut new_task = tokio::spawn(async move { crate::runner_updates::lease(&id).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut new_task)
            .await
            .is_err()
    );
    assert!(
        f.receiver.try_recv().is_err(),
        "busy host must receive no update"
    );
    drop(task);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), f.receiver.recv())
            .await
            .unwrap(),
        Some(wire::Control::Update { .. })
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut new_task)
            .await
            .is_err()
    );
    let healthy = Arc::new(Live {
        owner: None,
        owner_revision: std::sync::atomic::AtomicU64::new(0),
        key_hash: String::new(),
        control: f.peer.control.clone(),
        disconnect: f.peer.disconnect.clone(),
        managed: true,
        supports_monitor: true,
        version: env!("CARGO_PKG_VERSION").into(),
        ready: std::sync::atomic::AtomicBool::new(false),
        failed_version: Mutex::new(None),
        update_error: Mutex::new(None),
    });
    live().insert(f.id.to_string(), healthy.clone());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut new_task)
            .await
            .is_err(),
        "version alone does not confirm health"
    );
    healthy
        .ready
        .store(true, std::sync::atomic::Ordering::Release);
    f.wait_state("succeeded").await;
    let _lease = tokio::time::timeout(Duration::from_secs(1), new_task)
        .await
        .unwrap()
        .unwrap();
}
