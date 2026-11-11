use super::*;

#[test]
fn swarm_managers_do_not_multiply_cluster_replicas_or_hide_conflicts() {
    let sample = |name: &str, expected, healthy, unknown| runtime::Service {
        name: name.into(),
        expected,
        healthy,
        observed: expected,
        unknown,
    };
    let merged = runtime::cluster_services(vec![
        sample("web", 3, 3, 0),
        sample("web", 3, 2, 0),
        sample("db", 1, 1, 0),
        sample("db", 1, 0, 1),
    ]);
    assert_eq!(
        merged.iter().map(|service| service.expected).sum::<u32>(),
        4
    );
    let web = merged.iter().find(|service| service.name == "web").unwrap();
    assert_eq!((web.expected, web.healthy, web.unknown), (3, 2, 0));
    let db = merged.iter().find(|service| service.name == "db").unwrap();
    assert_eq!((db.expected, db.healthy, db.unknown), (1, 0, 1));
}

#[test]
fn independent_swarm_clusters_keep_their_own_replica_demand() {
    let mut first = row();
    first.kind = "runtime".into();
    first.role = "service".into();
    first.host_id = 10;
    let result = CheckResult {
        confirmed: "up".into(),
        observation: Observation {
            outcome: "up".into(),
            runtime_scope: "swarm:first".into(),
            services: vec![runtime::Service {
                name: "web".into(),
                expected: 3,
                healthy: 3,
                observed: 3,
                unknown: 0,
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    first.result = serde_json::to_string(&result).unwrap();
    let mut second = first.clone();
    second.id = 2;
    second.host_id = 20;
    assert_eq!(runtime::aggregate(&[&first, &second], 1001).expected, 3);
    let mut other = result;
    other.observation.runtime_scope = "swarm:second".into();
    second.result = serde_json::to_string(&other).unwrap();
    assert_eq!(runtime::aggregate(&[&first, &second], 1001).expected, 6);
}

async fn fixture(address: String) -> (tempfile::TempDir, AppState, checks::Model) {
    use sea_orm::{Database, IntoActiveModel};
    use sea_orm_migration::MigratorTrait;
    use std::sync::Arc;
    let directory = tempfile::tempdir().unwrap();
    let db = Database::connect(format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("checks.sqlite").display()
    ))
    .await
    .unwrap();
    crate::migration::Migrator::up(&db, None).await.unwrap();
    db.execute_unprepared("INSERT INTO users (id,email,name,password_hash,is_superadmin,active,version,created_at,updated_at) VALUES (1,'checks@example.invalid','Checks','unused',1,1,1,0,0); INSERT INTO workspaces(id,slug,name,owner_id,join_code,pipeline_run_number,workflow_execution_number,created_at,updated_at) VALUES(1,'checks','Checks',1,'',0,0,0,0)").await.unwrap();
    let state = Arc::new(crate::state::State {
        ids: Arc::new(crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap()),
        db,
        data_dir: directory.path().into(),
        instance_id: "checks-test".into(),
        server_mode: true,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(directory.path(), true).unwrap(),
        cluster: None,
        host_monitor: Default::default(),
        monitor: Default::default(),
    });
    let mut check = row();
    check.deployment_id = 0;
    check.checked_at = 0;
    check.settings = serde_json::to_string(&Settings {
        address,
        ..Default::default()
    })
    .unwrap();
    checks::Entity::insert(check.clone().into_active_model())
        .exec(&state.db)
        .await
        .unwrap();
    (directory, state, check)
}

#[tokio::test]
async fn competing_collectors_take_one_sample_and_revoked_actor_cannot_probe() {
    use axum::{Router, routing::get};
    use sea_orm::PaginatorTrait;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "ok"
                    }
                }),
            ),
        )
        .into_future(),
    );
    let (_directory, state, check) = fixture(format!("http://{address}/")).await;
    let client = reqwest::Client::new();
    let results =
        futures::future::join_all((0..4).map(|_| collect_one(&state, &client, check.clone())))
            .await;
    assert!(results.into_iter().all(|result| result.is_ok()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        observations::Entity::find().count(&state.db).await.unwrap(),
        1
    );
    state
        .db
        .execute_unprepared(
            "UPDATE users SET active=0 WHERE id=1; UPDATE health_checks SET due_at=0 WHERE id=1",
        )
        .await
        .unwrap();
    let revoked = checks::Entity::find_by_id(1)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    collect_one(&state, &client, revoked).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let result = checks::Entity::find_by_id(1)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::state(&result, db::now()), "unknown");
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn editing_while_probe_is_in_flight_rejects_its_late_result_and_history() {
    use axum::{Router, routing::get};
    use sea_orm::PaginatorTrait;
    use std::sync::Arc;
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let a = started.clone();
    let b = release.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(move || {
                    let a = a.clone();
                    let b = b.clone();
                    async move {
                        a.notify_one();
                        b.notified().await;
                        "ok"
                    }
                }),
            ),
        )
        .into_future(),
    );
    let (_directory, state, check) = fixture(format!("http://{address}/")).await;
    let copy = state.clone();
    let worker =
        tokio::spawn(async move { collect_one(&copy, &reqwest::Client::new(), check).await });
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .unwrap();
    state.db.execute_unprepared("UPDATE health_checks SET version=2,enabled=0,result='{}',checked_at=0,due_at=0 WHERE id=1").await.unwrap();
    release.notify_one();
    worker.await.unwrap().unwrap();
    let stored = checks::Entity::find_by_id(1)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.version, 2);
    assert_eq!(stored.checked_at, 0);
    assert_eq!(stored.result, "{}");
    assert_eq!(
        observations::Entity::find().count(&state.db).await.unwrap(),
        0
    );
    server.abort();
    let _ = server.await;
}

fn row() -> checks::Model {
    checks::Model {
        id: 1,
        workspace_id: 1,
        name: "test".into(),
        description: String::new(),
        kind: "http".into(),
        role: "entry".into(),
        deployment_id: 1,
        host_id: 0,
        service_name: String::new(),
        settings: serde_json::to_string(&Settings::default()).unwrap(),
        enabled: 1,
        run_as: 1,
        result: "{}".into(),
        version: 1,
        created_at: 0,
        created_by: 1,
        updated_at: 0,
        updated_by: 1,
        checked_at: 1000,
        checked_by: 0,
        due_at: 0,
        due_by: 0,
    }
}
fn observed(outcome: &str) -> Observation {
    Observation {
        outcome: outcome.into(),
        ..Default::default()
    }
}

#[test]
fn confirmed_failure_requires_full_recovery_and_unknown_is_not_recovery() {
    let settings = Settings::default();
    let mut result = CheckResult::default();
    result = transition(&result, observed("up"), &settings);
    assert_eq!(result.confirmed, "up");
    for index in 1..=3 {
        result = transition(&result, observed("down"), &settings);
        assert_eq!(result.confirmed, if index == 3 { "down" } else { "up" });
    }
    result = transition(&result, observed("up"), &settings);
    assert_eq!(result.confirmed, "down");
    result = transition(&result, observed("unknown"), &settings);
    assert_eq!(result.confirmed, "down");
    assert_eq!(result.successes, 0);
    // Unknown breaks continuity and cannot itself produce an alert recovery.
    let mut check = row();
    check.result = serde_json::to_string(&result).unwrap();
    assert_eq!(state(&check, 1001), "unknown");
    result = transition(&result, observed("up"), &settings);
    assert_eq!(result.confirmed, "down");
    result = transition(&result, observed("up"), &settings);
    assert_eq!(result.confirmed, "up");
}
#[test]
fn paused_stale_and_unobserved_never_claim_health() {
    let mut check = row();
    check.result = serde_json::to_string(&transition(
        &CheckResult::default(),
        observed("up"),
        &Settings::default(),
    ))
    .unwrap();
    assert_eq!(state(&check, 1001), "up");
    assert_eq!(state(&check, 91001), "stale");
    check.enabled = 0;
    assert_eq!(state(&check, 1001), "paused");
    check.enabled = 1;
    check.checked_at = 0;
    assert_eq!(state(&check, 1001), "unknown");
}
#[test]
fn validates_numeric_bounds_and_addresses_without_command_or_url_credentials() {
    let mut settings = Settings::default();
    settings.address = "https://example.invalid/health".into();
    assert!(settings.validate("http").is_ok());
    for address in [
        "file:///etc/passwd",
        "https://user:secret@example.invalid/",
        "https://example.invalid/#x",
    ] {
        settings.address = address.into();
        assert!(settings.validate("http").is_err());
    }
    for address in ["localhost:1234", "[::1]:1234"] {
        settings.address = address.into();
        assert!(settings.validate("tcp").is_ok());
    }
    for address in [
        "localhost:0",
        "localhost",
        "host:1234/path",
        "host:1234?x=1",
        "user:pass@host:1",
    ] {
        settings.address = address.into();
        assert!(settings.validate("tcp").is_err());
    }
    settings.address = "localhost:1234".into();
    settings.interval_seconds = 0;
    assert!(settings.validate("tcp").is_err());
    settings.interval_seconds = 30;
    settings.stale_seconds = 34;
    assert!(settings.validate("tcp").is_err());
}
#[tokio::test]
async fn real_http_and_tcp_success_failure_body_and_timeout() {
    use axum::{Router, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/ok", get(|| async { "healthy" }))
                .route(
                    "/slow",
                    get(|| async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        "late"
                    }),
                )
                .route(
                    "/redirect",
                    get(|| async { axum::response::Redirect::temporary("/ok") }),
                ),
        )
        .into_future(),
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut settings = Settings {
        address: format!("http://{address}/ok"),
        body_contains: "healthy".into(),
        ..Default::default()
    };
    assert_eq!(
        network_probe(&client, "http", &settings).await.outcome,
        "up"
    );
    settings.body_contains = "missing".into();
    assert_eq!(
        network_probe(&client, "http", &settings).await.reason,
        "monitoring.bodyMismatch"
    );
    settings.body_contains.clear();
    settings.address = format!("http://{address}/redirect");
    assert_eq!(
        network_probe(&client, "http", &settings).await.reason,
        "monitoring.statusMismatch"
    );
    settings.address = format!("http://{address}/slow");
    settings.timeout_seconds = 1;
    assert_eq!(
        network_probe(&client, "http", &settings).await.reason,
        "monitoring.timeout"
    );
    settings.address = address.to_string();
    assert_eq!(network_probe(&client, "tcp", &settings).await.outcome, "up");
    server.abort();
    let _ = server.await;
    assert_eq!(
        network_probe(&client, "tcp", &settings).await.outcome,
        "down"
    );
}
#[test]
fn runtime_parsing_distinguishes_process_container_health_and_unavailable_evidence() {
    let native = runtime::parse(
        "native",
        "web",
        "node1",
        "LoadState=loaded\nActiveState=active\nSubState=running\nMainPID=42\n",
    )
    .unwrap();
    assert_eq!(native[0].state, "up");
    assert_eq!(native[0].evidence, "process");
    assert!(runtime::parse("native", "web", "node1", "").is_err());
    let containers = runtime::parse(
        "docker",
        "web",
        "node1",
        r#"{"ID":"abc","State":"running","Status":"Up 1 minute (unhealthy)"}"#,
    )
    .unwrap();
    assert_eq!(containers[0].state, "down");
    let tasks = runtime::parse(
        "stack",
        "web",
        "manager",
        r#"{"ID":"task1","Node":"worker2","CurrentState":"Running 2 minutes ago"}"#,
    )
    .unwrap();
    assert_eq!(tasks[0].node, "worker2");
    assert_eq!(tasks[0].evidence, "task");
}
#[test]
fn application_entry_success_cannot_hide_a_service_failure_or_stale_observer() {
    let mut entry = row();
    entry.result = serde_json::to_string(&transition(
        &CheckResult::default(),
        observed("up"),
        &Settings::default(),
    ))
    .unwrap();
    let mut runtime = row();
    runtime.id = 2;
    runtime.kind = "runtime".into();
    runtime.role = "service".into();
    runtime.host_id = 3;
    let mut observation = observed("down");
    observation.services = vec![
        runtime::Service {
            name: "web".into(),
            expected: 3,
            healthy: 2,
            observed: 3,
            unknown: 0,
        },
        runtime::Service {
            name: "db".into(),
            expected: 1,
            healthy: 1,
            observed: 1,
            unknown: 0,
        },
    ];
    runtime.result = serde_json::to_string(&CheckResult {
        confirmed: "down".into(),
        failures: 3,
        observation,
        ..Default::default()
    })
    .unwrap();
    let summary = runtime::aggregate(&[&entry, &runtime], 1001);
    assert_eq!(summary.state, "degraded");
    assert_eq!(summary.healthy, 3);
    assert_eq!(summary.expected, 4);
    runtime.checked_at = 1;
    assert_eq!(
        runtime::aggregate(&[&entry, &runtime], 91000).state,
        "unknown"
    );
    assert_eq!(runtime::aggregate(&[&entry], 1001).state, "unknown");
}
