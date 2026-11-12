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

// ---------------------------------------------------------------- blueprint deployments

fn application(services: &[(&str, u32)]) -> String {
    let services: Vec<Value> = services
        .iter()
        .map(|(name, replicas)| json!({"name": name, "image": "nats:2", "replicas": replicas}))
        .collect();
    json!({"apiVersion": "ferry/v2", "kind": "Application",
           "metadata": {"name": "nats"}, "services": services})
    .to_string()
}

/// Workspace 1 hosts `base`..`base+4` (`base+4` is in no deployment) and, in workspace 2,
/// host `base+9`. Blueprint deployment `base` (docker) runs `nats-0` on `base` and `nats-1`
/// on `base+1`; deployment `base+1` (stack, managed by `base+2`) pins `nats-0`/`nats-1` to
/// `base`/`base+1` and replicates the stateless `gateway`; deployment `base+3` has never
/// been deployed.
async fn blueprint_fixture(base: i64) -> (tempfile::TempDir, AppState) {
    use crate::entities::{deployment_revisions as revisions, deployment_target_states, hosts};
    use sea_orm::IntoActiveModel;
    let (dir, s) = crate::platform_tools::tests::setup().await;
    for (id, workspace) in (0..5).map(|i| (base + i, 1)).chain([(base + 9, 2)]) {
        crate::hosts::fake::register(id);
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id),
            workspace_id: Set(workspace),
            name: Set(format!("Host {id}")),
            address: Set("192.0.2.1".into()),
            port: Set(22),
            username: Set("test".into()),
            connection: Set("fake".into()),
            state: Set("online".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            enroll_token: Set(String::new()),
            enroll_token_hash: Set(String::new()),
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            os: Set("linux".into()),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            metrics: Set(String::new()),
            credential_id: Set(0),
            ..Default::default()
        })
        .exec(&s.db)
        .await
        .unwrap();
    }
    let bound = |hosts: &[i64]| -> Vec<Value> {
        hosts
            .iter()
            .map(|h| json!({"hostId": h.to_string(), "address": "10.0.0.1"}))
            .collect()
    };
    let docker = json!({"driver": "docker", "roles": {"nats": bound(&[base, base + 1])}});
    let stack = json!({"driver": "stack", "manager": (base + 2).to_string(),
                       "roles": {"nats": bound(&[base, base + 1]),
                                 "gateway": bound(&[base, base + 1])}});
    let rendered = |targets: Vec<(i64, String)>| {
        json!({"targets": targets.into_iter().map(|(host, manifest)|
            json!({"host_id": host.to_string(), "manifest": manifest, "files": []}))
            .collect::<Vec<_>>()})
        .to_string()
    };
    let deployments = [
        (
            base,
            "docker",
            docker,
            rendered(vec![
                (base, application(&[("nats-0", 1)])),
                (base + 1, application(&[("nats-1", 1)])),
            ]),
            vec![base, base + 1],
        ),
        (
            base + 1,
            "stack",
            stack,
            rendered(vec![(
                base + 2,
                application(&[("nats-0", 1), ("nats-1", 1), ("gateway", 2)]),
            )]),
            vec![base + 2],
        ),
    ];
    for (id, mode, binding, rendered, targets) in deployments {
        let revision = id + 100;
        deployments::Entity::insert(
            deployments::Model {
                id,
                workspace_id: 1,
                name: format!("bp-{id}"),
                category: String::new(),
                manifest: "{\"kind\":\"Blueprint\"}".into(),
                preview_artifact: String::new(),
                mode: mode.into(),
                target_kind: "blueprint".into(),
                target_ref: 0,
                workdir: "/srv".into(),
                description: String::new(),
                state: "succeeded".into(),
                revision_number: 1,
                current_revision: revision,
                created_at: 0,
                updated_at: 0,
                version: 1,
                blueprint_binding: Some(binding.to_string()),
                deploy_spec: None,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        revisions::Entity::insert(
            revisions::Model {
                id: revision,
                workspace_id: 1,
                deployment_id: id,
                number: 1,
                action: "deploy".into(),
                source_revision: 0,
                manifest: "{\"kind\":\"Blueprint\"}".into(),
                rendered,
                artifact: String::new(),
                variable_snapshot: None,
                expected_hash: String::new(),
                state: "succeeded".into(),
                output: String::new(),
                triggered_by: 1,
                started_at: 0,
                finished_at: 0,
                created_at: 0,
                updated_at: 0,
                version: 1,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        for (index, host) in (0..).zip(targets) {
            deployment_target_states::Entity::insert(deployment_target_states::ActiveModel {
                id: Set(revision * 10 + index),
                workspace_id: Set(1),
                deployment_id: Set(id),
                host_id: Set(host),
                current_revision: Set(revision),
            })
            .exec(&s.db)
            .await
            .unwrap();
        }
    }
    let mut never = deployments::Entity::find_by_id(base)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    never.id = base + 3;
    never.current_revision = 0;
    never.state = "draft".into();
    deployments::Entity::insert(never.into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    (dir, s)
}

fn admin() -> Scope {
    crate::agents::tests::admin(1)
}

async fn create_check(s: &AppState, body: Value) -> Result<(StatusCode, Json<Value>)> {
    create(
        State(s.clone()),
        admin(),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await
}

fn runtime_body(deployment: i64, host: i64) -> Value {
    json!({"name": "runtime", "kind": "runtime", "role": "service",
           "deployment_id": deployment.to_string(), "host_id": host.to_string()})
}

fn service_body(deployment: i64, service: &str) -> Value {
    json!({"name": "svc", "kind": "http", "role": "service", "address": "http://example.invalid/",
           "deployment_id": deployment.to_string(), "service_name": service})
}

fn error_key(error: ApiError) -> String {
    error.2.map(|m| m.key).unwrap_or(error.1)
}

async fn health_of(s: &AppState, deployment: i64) -> Value {
    let Json(value) = applications(
        State(s.clone()),
        admin(),
        Query(HashMap::from([(
            "id".into(),
            Id::from(deployment).to_string(),
        )])),
    )
    .await
    .unwrap();
    value["items"][0].clone()
}

fn placed(item: &Value) -> Vec<(String, String, String)> {
    item["placement"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap().to_owned(),
                entry["host_name"].as_str().unwrap().to_owned(),
                entry["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn blueprint_application_lists_instances_per_host() {
    let base = 7100;
    let (_dir, s) = blueprint_fixture(base).await;
    // One collector on the first docker host, observing its own instance as healthy.
    let mut check = row();
    check.id = 9;
    check.kind = "runtime".into();
    check.role = "service".into();
    check.deployment_id = base;
    check.host_id = base;
    check.checked_at = db::now();
    let mut observation = observed("up");
    observation.revision_id = Id::from(base + 100).to_string();
    observation.services = vec![runtime::Service {
        name: "nats-0".into(),
        expected: 1,
        healthy: 1,
        observed: 1,
        unknown: 0,
    }];
    check.result = serde_json::to_string(&CheckResult {
        confirmed: "up".into(),
        observation,
        ..Default::default()
    })
    .unwrap();
    {
        use sea_orm::IntoActiveModel;
        checks::Entity::insert(check.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
    }
    let docker = health_of(&s, base).await;
    assert_eq!(
        placed(&docker),
        [
            ("nats-0".into(), format!("Host {base}"), "up".into()),
            (
                "nats-1".into(),
                format!("Host {}", base + 1),
                "unknown".into()
            ),
        ]
    );
    assert_eq!(docker["expected_instances"], 2);
    assert_eq!(docker["healthy_instances"], 1);
    // The second host has no collector: a coverage gap, never a claim of health.
    assert_eq!(docker["unmonitored_hosts"], 1);
    assert_eq!(docker["health"], "unknown");

    // A stack runs on its manager; stable instances report their pinned hosts, the
    // stateless role one entry on the manager.
    let stack = health_of(&s, base + 1).await;
    assert_eq!(
        placed(&stack),
        [
            ("nats-0".into(), format!("Host {base}"), "unknown".into()),
            (
                "nats-1".into(),
                format!("Host {}", base + 1),
                "unknown".into()
            ),
            (
                "gateway".into(),
                format!("Host {}", base + 2),
                "unknown".into()
            ),
        ]
    );
    assert_eq!(stack["expected_instances"], 4);
    assert_eq!(stack["unmonitored_hosts"], 1);

    // Never deployed: nothing to list, and not an error.
    let never = health_of(&s, base + 3).await;
    assert_eq!(never["placement"], json!([]));
    assert_eq!(never["health"], "unknown");
}

#[tokio::test]
async fn runtime_check_allowed_for_blueprint_target_host() {
    let base = 7200;
    let (_dir, s) = blueprint_fixture(base).await;
    for host in [base, base + 1] {
        create_check(&s, runtime_body(base, host)).await.unwrap();
    }
    // A workspace host the current revision does not deploy to.
    let error = create_check(&s, runtime_body(base, base + 4))
        .await
        .unwrap_err();
    assert_eq!(error_key(error), "monitoring.bindingInvalid");
    // Another workspace's host stays invisible.
    assert!(
        create_check(&s, runtime_body(base, base + 9))
            .await
            .is_err()
    );
    // A stack: its manager target and the hosts its binding pins instances to.
    for host in [base + 2, base, base + 1] {
        create_check(&s, runtime_body(base + 1, host))
            .await
            .unwrap();
    }
    let error = create_check(&s, runtime_body(base + 1, base + 4))
        .await
        .unwrap_err();
    assert_eq!(error_key(error), "monitoring.bindingInvalid");
    // Never deployed: no targets yet.
    let error = create_check(&s, runtime_body(base + 3, base))
        .await
        .unwrap_err();
    assert_eq!(error_key(error), "monitoring.bindingInvalid");
}

#[tokio::test]
async fn service_check_validates_against_compiled_manifests() {
    let base = 7300;
    let (_dir, s) = blueprint_fixture(base).await;
    for (deployment, service) in [(base, "nats-0"), (base, "nats-1"), (base + 1, "gateway")] {
        create_check(&s, service_body(deployment, service))
            .await
            .unwrap();
    }
    // The role name and an instance the revision does not deploy are not services.
    for (deployment, service) in [(base, "nats"), (base, "nats-2"), (base + 3, "nats-0")] {
        let error = create_check(&s, service_body(deployment, service))
            .await
            .unwrap_err();
        assert_eq!(error_key(error), "monitoring.bindingInvalid", "{service}");
    }
}

#[tokio::test]
async fn runtime_probe_reads_blueprint_target_manifest() {
    let base = 7400;
    let (_dir, s) = blueprint_fixture(base).await;
    crate::hosts::fake::get(base + 1).respond(
        "docker container ls",
        "{\"ID\":\"c1\",\"State\":\"running\",\"Status\":\"Up 1 minute\"}\n",
    );
    let mut check = row();
    check.kind = "runtime".into();
    check.role = "service".into();
    check.deployment_id = base;
    check.host_id = base + 1;
    let observation = runtime::probe(&s, &admin(), &check, &Settings::default())
        .await
        .unwrap();
    assert_eq!(observation.outcome, "up", "{observation:?}");
    assert_eq!(observation.revision_id, Id::from(base + 100).to_string());
    let names: Vec<_> = observation
        .services
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["nats-1"]);
}
