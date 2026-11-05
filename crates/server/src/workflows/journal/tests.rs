use super::*;
use crate::{
    entities::workflows,
    executions::lease::{Execution, Kind},
    migration::{Migrator, workflow_tests},
};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, PaginatorTrait};
use sea_orm_migration::MigratorTrait;

pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    url: String,
    pub(crate) db: DatabaseConnection,
    vault: Vault,
    pub(crate) lease: Lease,
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("journal.sqlite").display()
        );
        if let Ok(root_url) = std::env::var("FERRY_TEST_JOURNAL_MYSQL_ROOT_URL") {
            assert!(root_url.starts_with("mysql://root@127.0.0.1:"));
            let admin = Database::connect(&root_url).await.unwrap();
            let name = format!("workflow_journal_{:016x}", rand::random::<u64>());
            admin
                .execute_unprepared(&format!(
                    "CREATE DATABASE `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin"
                ))
                .await
                .unwrap();
            admin.close().await.unwrap();
            url = format!("{}/{name}", root_url.trim_end_matches('/'));
            eprintln!("isolated journal fixture: {name}");
        }
        let mut options = ConnectOptions::new(&url);
        options.max_connections(4).sqlx_logging(false);
        let db = Database::connect(options).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        let graph = json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"Asia/Kuala_Lumpur"},
            "nodes":[{"id":"source","name":"Source","type":"data.set","type_version":1,"params":params("draft"),"position":[0,0]},
                {"id":"target","name":"Target","type":"data.set","type_version":1,"params":params("draft"),"position":[1,0]}],"connections":[]});
        let mut definition = workflow_tests::definition(7);
        definition.definition = graph.to_string();
        workflows::Entity::insert(definition.into_active_model())
            .exec(&db)
            .await
            .unwrap();
        definition_revisions::Entity::insert(
            definition_revisions::Model {
                id: 15,
                workspace_id: 11,
                subject_kind: "workflow".into(),
                subject_id: 7,
                number: 1,
                definition: graph.to_string(),
                message: None,
                created_by: 3,
                created_at: 100,
                version: 1,
            }
            .into_active_model(),
        )
        .exec(&db)
        .await
        .unwrap();
        let mut execution = workflow_tests::execution(8);
        execution.context = None;
        runs::Entity::insert(execution.into_active_model())
            .exec(&db)
            .await
            .unwrap();
        let vault = Vault::open(dir.path(), false).unwrap();
        let lease = Lease::claim(
            &db,
            Execution {
                kind: Kind::Workflow,
                workspace: 11,
                id: 8,
            },
            0,
            "first:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
        Self {
            _dir: dir,
            url,
            db,
            vault,
            lease,
        }
    }
    pub(crate) fn journal(&self) -> Journal<'_> {
        Journal {
            authority: None,
            assets: None,
            db: &self.db,
            vault: &self.vault,
            lease: &self.lease,
        }
    }
    pub(crate) async fn run(&self) -> runs::Model {
        runs::Entity::find_by_id(8)
            .one(&self.db)
            .await
            .unwrap()
            .unwrap()
    }
    async fn takeover(&mut self) {
        assert!(self.lease.release(&self.db).await.unwrap());
        self.lease = Lease::claim(
            &self.db,
            Execution {
                kind: Kind::Workflow,
                workspace: 11,
                id: 8,
            },
            self.lease.epoch(),
            "next:boot",
            crate::db::now(),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
fn params(value: &str) -> Value {
    json!({"fields":[{"path":"value","value":value}],"keep_other_fields":true})
}
pub(crate) fn activation(id: i64, path: &str) -> Activation {
    Activation {
        id,
        node_path: path.into(),
        iteration: json!([]),
        attempt: 1,
        redo_round: 0,
        params: vec![params("PRIVATE_FROZEN_VALUE")],
        frozen_at_ms: 1793550612345,
        sources: vec![],
        seed: Some(42),
    }
}

#[test]
fn output_packets_require_the_declared_outputless_or_port_shape() {
    let empty = Output {
        version: 1,
        ports: vec![],
    };
    assert!(empty.validate(true).is_ok());
    assert!(empty.validate(false).is_err());
    let main = Output {
        version: 1,
        ports: vec![Port {
            name: "main".into(),
            index: 0,
            skipped: false,
            items: vec![],
        }],
    };
    assert!(main.validate(false).is_ok());
    assert!(main.validate(true).is_err());
    assert!(
        Output {
            version: 2,
            ports: vec![]
        }
        .validate(true)
        .is_err()
    );
}

#[tokio::test]
async fn ordinary_and_legacy_snapshots_cannot_commit_an_empty_port_packet() {
    let fixture = Fixture::new().await;
    let active = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    assert!(!active.snapshot.outputless);
    let mut old = serde_json::to_value(&active.snapshot).unwrap();
    old.as_object_mut().unwrap().remove("outputless");
    assert!(!serde_json::from_value::<Snapshot>(old).unwrap().outputless);
    let before = fixture.run().await;
    assert!(
        fixture
            .journal()
            .complete(
                before.version,
                31,
                active.node.version,
                Output {
                    version: 1,
                    ports: vec![]
                }
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await, before);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        active.node
    );
}

#[tokio::test]
async fn outputless_stop_snapshot_rejects_fake_ports_and_preserves_skipped_history_after_takeover()
{
    let mut fixture = Fixture::new().await;
    let revision = definition_revisions::Entity::find_by_id(15)
        .one(&fixture.db)
        .await
        .unwrap()
        .unwrap();
    let mut graph: Value = serde_json::from_str(&revision.definition).unwrap();
    graph["nodes"][0]["type"] = json!("core.stop");
    graph["nodes"][0]["params"] = json!({"message":"PRIVATE_STOP_MESSAGE"});
    graph["nodes"][0]["settings"] = json!({"mode":"mute"});
    definition_revisions::Entity::update_many()
        .col_expr(
            definition_revisions::Column::Definition,
            Expr::value(graph.to_string()),
        )
        .filter(definition_revisions::Column::Id.eq(15))
        .exec(&fixture.db)
        .await
        .unwrap();
    let mut request = activation(31, "/nodes/0");
    request.params.clear();
    let active = fixture
        .journal()
        .activate(fixture.run().await.version, request)
        .await
        .unwrap();
    assert!(active.snapshot.outputless);
    let before = fixture.run().await;
    assert!(
        fixture
            .journal()
            .complete(
                before.version,
                31,
                active.node.version,
                Output {
                    version: 1,
                    ports: vec![Port {
                        name: "main".into(),
                        index: 0,
                        skipped: true,
                        items: vec![]
                    }]
                }
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await, before);
    let complete = fixture
        .journal()
        .complete(
            before.version,
            31,
            active.node.version,
            Output {
                version: 1,
                ports: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(complete.node.state, "skipped");
    assert_eq!(complete.node.items_out, 0);
    assert!(
        fixture
            .journal()
            .output(&fixture.db, &fixture.run().await, &complete.node)
            .await
            .unwrap()
            .ports
            .is_empty()
    );
    let version = fixture.run().await.version;
    let replay = fixture
        .journal()
        .complete(
            version,
            31,
            complete.node.version,
            Output {
                version: 1,
                ports: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(replay.node, complete.node);
    assert_eq!(fixture.run().await.version, version);
    fixture.takeover().await;
    let replay = fixture
        .journal()
        .activate(fixture.run().await.version, activation(32, "/nodes/0"))
        .await
        .unwrap();
    assert_eq!(replay.node, complete.node);
    assert!(replay.snapshot.outputless);
}

#[tokio::test]
async fn node_budget_serializes_attempts_and_allows_replay_at_the_limit() {
    let fixture = Fixture::new().await;
    let first = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    // Populate historical attempts, then race the last available budget slot.
    let mut historical = Vec::new();
    for id in 1000..5998 {
        let mut node = first.node.clone();
        node.id = id;
        node.node_id = format!("historical-{id}");
        node.state = "succeeded".into();
        historical.push(node.into_active_model());
    }
    for batch in historical.chunks(200) {
        nodes::Entity::insert_many(batch.to_vec())
            .exec(&fixture.db)
            .await
            .unwrap();
    }
    let mut other = activation(33, "/nodes/1");
    other.attempt = 2;
    let a = fixture.journal();
    let b = fixture.journal();
    let (a, b) = tokio::join!(
        a.activate(first.execution_version, activation(32, "/nodes/1")),
        b.activate(first.execution_version, other),
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let last = a.or(b).unwrap();
    assert_eq!(
        nodes::Entity::find().count(&fixture.db).await.unwrap(),
        MAX_NODE_EXECUTIONS
    );
    let before = fixture.run().await;
    let mut replay = activation(last.node.id, "/nodes/1");
    replay.attempt = last.node.attempt;
    let repeated = fixture
        .journal()
        .activate(before.version, replay)
        .await
        .unwrap();
    assert_eq!(repeated.node.id, last.node.id);
    assert_eq!(fixture.run().await.version, before.version);
    let mut excess = activation(34, "/nodes/1");
    excess.attempt = 3;
    let error = match fixture.journal().activate(before.version, excess).await {
        Ok(_) => panic!("execution budget must reject attempt 5001"),
        Err(error) => error,
    };
    assert_eq!(error.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(super::super::run::failure_reason(&error), "budget_exceeded");
    let after = fixture.run().await;
    assert_eq!(
        (after.version, after.data_bytes),
        (before.version, before.data_bytes)
    );
    assert_eq!(
        nodes::Entity::find().count(&fixture.db).await.unwrap(),
        MAX_NODE_EXECUTIONS
    );
}
fn output(value: &str) -> Output {
    Output {
        version: 1,
        ports: vec![Port {
            name: "main".into(),
            index: 0,
            skipped: false,
            items: vec![serde_json::from_value(json!({"json":{"value":value}})).unwrap()],
        }],
    }
}
fn source(node: &nodes::Model, output: &Output) -> Source {
    Source {
        node_run: node.id,
        attempt: node.attempt,
        iteration_hash: node.iteration_hash.clone(),
        port: "main".into(),
        port_index: 0,
        output_sha256: digest(&output.ports[0]).unwrap(),
    }
}

#[tokio::test]
async fn muted_and_bypassed_checkpoints_keep_terminal_identity_after_takeover() {
    for mode in ["mute", "bypass", "pinned"] {
        let mut fixture = Fixture::new().await;
        let revision = definition_revisions::Entity::find_by_id(15)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap();
        let mut graph: Value = serde_json::from_str(&revision.definition).unwrap();
        graph["nodes"][0]["settings"] =
            json!({"mode":if mode == "pinned" { "active" } else { mode }});
        if mode == "pinned" {
            graph["pin_data"] = json!({"source":[{"json":{"value":"unchanged"}}]});
        }
        definition_revisions::Entity::update_many()
            .col_expr(
                definition_revisions::Column::Definition,
                Expr::value(graph.to_string()),
            )
            .filter(definition_revisions::Column::Id.eq(15))
            .exec(&fixture.db)
            .await
            .unwrap();
        let mut input = activation(31, "/nodes/0");
        input.params.clear();
        let active = fixture
            .journal()
            .activate(fixture.run().await.version, input)
            .await
            .unwrap();
        assert_eq!(active.snapshot.pinned, mode == "pinned");
        let mut result = output("unchanged");
        if mode == "mute" {
            result.ports[0].skipped = true;
            result.ports[0].items.clear();
        }
        let done = fixture
            .journal()
            .complete(
                active.execution_version,
                31,
                active.node.version,
                serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            done.node.state,
            if mode == "mute" {
                "skipped"
            } else if mode == "pinned" {
                "pinned"
            } else {
                "bypassed"
            }
        );
        let original = done.node.clone();
        fixture.takeover().await;
        let before = fixture.run().await;
        let replay = fixture
            .journal()
            .activate(before.version, activation(999, "/nodes/0"))
            .await
            .unwrap();
        assert_eq!(replay.node, original);
        assert!(replay.snapshot.params.is_empty());
        let repeated = fixture
            .journal()
            .complete(
                before.version,
                31,
                active.node.version,
                serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(repeated.node, original);
        assert_eq!(fixture.run().await.version, before.version);
        assert_eq!(fixture.run().await.data_bytes, before.data_bytes);
        assert!(
            fixture
                .journal()
                .complete(before.version, 31, done.node.version, output("conflict"))
                .await
                .is_err()
        );
        let mut next = activation(32, "/nodes/1");
        next.sources = vec![source(&done.node, &result)];
        fixture
            .journal()
            .activate(before.version, next)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn large_packets_are_encrypted_referenced_and_resume_with_original_parameters() {
    use crate::{
        artifact_store::ArtifactStore,
        content_assets::{DEFAULT_QUOTA, Store},
        entities::{asset_refs, content_assets},
    };
    let mut fixture = Fixture::new().await;
    let backend = if std::env::var_os("FERRY_TEST_ASSET_S3").is_some() {
        ArtifactStore::configured(&fixture._dir.path().join("objects")).unwrap()
    } else {
        ArtifactStore::local(&fixture._dir.path().join("objects")).unwrap()
    };
    let ids = crate::snowflake::Generator::new(2, std::time::Instant::now()).unwrap();
    let mut input = activation(31, "/nodes/0");
    input.params = vec![params(&"PRIVATE_LARGE".repeat(7000))];
    let store = Store {
        db: &fixture.db,
        backend: &backend,
        vault: &fixture.vault,
        ids: &ids,
        quota: DEFAULT_QUOTA,
    };
    let journal = Journal {
        authority: None,
        db: &fixture.db,
        vault: &fixture.vault,
        lease: &fixture.lease,
        assets: Some(&store),
    };
    let active = journal
        .activate(fixture.run().await.version, input)
        .await
        .unwrap();
    assert!(active.node.resolved_params.as_ref().unwrap().len() < 2048);
    let large = output(&"SECRET_OUTPUT".repeat(12000));
    let source_ref = source(&active.node, &large);
    let done = journal
        .complete(active.execution_version, 31, active.node.version, large)
        .await
        .unwrap();
    assert!(done.node.output_asset.is_some());
    assert!(done.node.output.len() < 2048);
    assert_eq!(
        asset_refs::Entity::find().count(&fixture.db).await.unwrap(),
        2
    );
    assert_eq!(
        content_assets::Entity::find()
            .count(&fixture.db)
            .await
            .unwrap(),
        2
    );
    let bytes = fixture.run().await.data_bytes;
    assert!(bytes > 200000);
    let duplicate = journal
        .complete(
            done.execution_version,
            31,
            done.node.version,
            output(&"SECRET_OUTPUT".repeat(12000)),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.execution_version, done.execution_version);
    assert_eq!(fixture.run().await.data_bytes, bytes);
    drop(journal);
    drop(store);
    fixture.takeover().await;
    let store = Store {
        db: &fixture.db,
        backend: &backend,
        vault: &fixture.vault,
        ids: &ids,
        quota: DEFAULT_QUOTA,
    };
    let journal = Journal {
        authority: None,
        db: &fixture.db,
        vault: &fixture.vault,
        lease: &fixture.lease,
        assets: Some(&store),
    };
    let mut replay = activation(999, "/nodes/0");
    replay.params = vec![json!({"invalid":"new params ignored"})];
    let replay = journal
        .activate(fixture.run().await.version, replay)
        .await
        .unwrap();
    assert_eq!(replay.snapshot.params, active.snapshot.params);
    let mut target = activation(32, "/nodes/1");
    target.sources = vec![source_ref];
    let target = journal
        .activate(replay.execution_version, target)
        .await
        .unwrap();
    assert_eq!(target.node.items_in, 1);
    let restored = journal
        .output(&fixture.db, &fixture.run().await, &done.node)
        .await
        .unwrap();
    assert_eq!(
        restored.ports[0].items[0].json["value"],
        "SECRET_OUTPUT".repeat(12000)
    );
    let mut moved = done.node.clone();
    moved.id = 32;
    assert!(
        journal
            .output(&fixture.db, &fixture.run().await, &moved)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn packet_boundary_and_failed_fence_preserve_limits_versions_and_track_orphans() {
    use crate::{
        artifact_store::ArtifactStore,
        content_assets::{DEFAULT_QUOTA, Store},
        entities::{asset_gc_jobs, asset_refs, content_assets},
    };
    let fixture = Fixture::new().await;
    let backend = ArtifactStore::local(&fixture._dir.path().join("objects")).unwrap();
    let ids = crate::snowflake::Generator::new(3, std::time::Instant::now()).unwrap();
    let store = Store {
        db: &fixture.db,
        backend: &backend,
        vault: &fixture.vault,
        ids: &ids,
        quota: DEFAULT_QUOTA,
    };
    let journal = Journal {
        authority: None,
        db: &fixture.db,
        vault: &fixture.vault,
        lease: &fixture.lease,
        assets: Some(&store),
    };
    let run = fixture.run().await;
    let exact = "x".repeat(INLINE_BYTES - 2); // JSON string adds two quotes.
    let inline = journal.packet(&run, 31, "params", &exact).await.unwrap();
    assert!(inline.asset.is_none());
    let external = journal
        .packet(&run, 31, "params", &(exact + "x"))
        .await
        .unwrap();
    assert!(external.asset.is_some());
    assert!(output(&"x".repeat(1024 * 1024)).validate(false).is_err());
    let active = journal
        .activate(run.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let before = fixture.run().await;
    assert!(
        journal
            .complete(
                run.version,
                31,
                active.node.version,
                output(&"STALE_LARGE".repeat(8000))
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, before.version);
    assert_eq!(fixture.run().await.data_bytes, before.data_bytes);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
    assert_eq!(
        asset_refs::Entity::find().count(&fixture.db).await.unwrap(),
        0
    );
    assert_eq!(
        content_assets::Entity::find()
            .count(&fixture.db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        asset_gc_jobs::Entity::find()
            .count(&fixture.db)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn immutable_attempt_clock_and_output_survive_real_database_reopen_and_takeover() {
    let mut fixture = Fixture::new().await;
    let checkpoint = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    assert_eq!(
        checkpoint.snapshot.params,
        vec![params("PRIVATE_FROZEN_VALUE")]
    );
    assert_eq!(checkpoint.snapshot.now["zone"], "Asia/Kuala_Lumpur");
    let stored = nodes::Entity::find_by_id(31)
        .one(&fixture.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !stored
            .resolved_params
            .as_ref()
            .unwrap()
            .contains("PRIVATE_FROZEN_VALUE")
    );
    let complete = fixture
        .journal()
        .complete(
            checkpoint.execution_version,
            31,
            checkpoint.node.version,
            output("PRIVATE_OUTPUT"),
        )
        .await
        .unwrap();
    assert_eq!(complete.node.state, "succeeded");
    assert!(!complete.node.output.contains("PRIVATE_OUTPUT"));
    let old_lease = fixture.lease.clone();
    fixture.takeover().await;
    let new_db = Database::connect(&fixture.url).await.unwrap();
    let journal = Journal {
        authority: None,
        assets: None,
        db: &new_db,
        vault: &fixture.vault,
        lease: &fixture.lease,
    };
    let mut replay = activation(999, "/nodes/0");
    replay.params = vec![params("NEW_VARIABLE")];
    replay.frozen_at_ms = 0;
    let replayed = journal
        .activate(complete.execution_version, replay)
        .await
        .unwrap();
    assert_eq!(replayed.node.id, 31);
    assert_eq!(replayed.snapshot.params, complete.snapshot.params);
    assert_eq!(
        replayed.snapshot.frozen_at_ms,
        complete.snapshot.frozen_at_ms
    );
    assert_eq!(replayed.snapshot.now, complete.snapshot.now);
    assert_eq!(fixture.run().await.version, complete.execution_version);
    assert_eq!(
        journal
            .output(&fixture.db, &fixture.run().await, &replayed.node)
            .await
            .unwrap()
            .ports[0]
            .items[0]
            .json["value"],
        "PRIVATE_OUTPUT"
    );
    assert!(
        Journal {
            authority: None,
            assets: None,
            db: &new_db,
            vault: &fixture.vault,
            lease: &old_lease
        }
        .activate(complete.execution_version, activation(32, "/nodes/1"))
        .await
        .is_err()
    );
    assert_eq!(nodes::Entity::find().count(&fixture.db).await.unwrap(), 1);
}

#[tokio::test]
async fn duplicate_completion_does_not_change_versions_and_conflicting_results_roll_back() {
    let fixture = Fixture::new().await;
    let activated = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let completed = fixture
        .journal()
        .complete(
            activated.execution_version,
            31,
            activated.node.version,
            output("same"),
        )
        .await
        .unwrap();
    let repeated = fixture
        .journal()
        .complete(
            completed.execution_version,
            31,
            activated.node.version,
            output("same"),
        )
        .await
        .unwrap();
    assert_eq!(repeated.execution_version, completed.execution_version);
    assert_eq!(repeated.node.version, completed.node.version);
    assert_eq!(repeated.node.finished_at, completed.node.finished_at);
    assert!(
        fixture
            .journal()
            .complete(
                completed.execution_version,
                31,
                completed.node.version,
                output("changed")
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, completed.execution_version);
}

#[tokio::test]
async fn exact_source_references_are_frozen_and_missing_corrupt_or_cross_execution_data_is_rejected()
 {
    let fixture = Fixture::new().await;
    let activated = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let original = output("source");
    let reference = source(&activated.node, &original);
    let completed = fixture
        .journal()
        .complete(
            activated.execution_version,
            31,
            activated.node.version,
            original,
        )
        .await
        .unwrap();
    let mut next = activation(32, "/nodes/1");
    next.sources = vec![reference.clone()];
    let target = fixture
        .journal()
        .activate(completed.execution_version, next)
        .await
        .unwrap();
    assert_eq!(target.node.items_in, 1);
    let mut bad = activation(33, "/nodes/1");
    bad.attempt = 2;
    bad.sources = vec![reference.clone()];
    bad.sources[0].output_sha256 = "0".repeat(64);
    assert!(
        fixture
            .journal()
            .activate(target.execution_version, bad)
            .await
            .is_err()
    );
    let mut bad = activation(33, "/nodes/1");
    bad.attempt = 2;
    bad.sources = vec![reference];
    bad.sources[0].node_run = 999;
    assert!(
        fixture
            .journal()
            .activate(target.execution_version, bad)
            .await
            .is_err()
    );
    nodes::Entity::update_many()
        .col_expr(nodes::Column::Output, Expr::value(completed.node.output))
        .filter(nodes::Column::Id.eq(32))
        .exec(&fixture.db)
        .await
        .unwrap();
    assert!(
        fixture
            .journal()
            .output(
                &fixture.db,
                &fixture.run().await,
                &nodes::Entity::find_by_id(32)
                    .one(&fixture.db)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .await
            .is_err()
    );
    nodes::Entity::delete_by_id(31)
        .exec(&fixture.db)
        .await
        .unwrap();
    assert!(
        fixture
            .journal()
            .complete(
                target.execution_version,
                32,
                target.node.version,
                output("must-not-commit")
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, target.execution_version);
    assert_eq!(
        nodes::Entity::find_by_id(32)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
}

#[tokio::test]
async fn cancellation_and_oversized_packets_cannot_partially_commit_a_node_or_parent() {
    let fixture = Fixture::new().await;
    let activated = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    assert!(
        fixture
            .journal()
            .complete(
                activated.execution_version,
                31,
                activated.node.version,
                output(&"x".repeat(INLINE_BYTES))
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, activated.execution_version);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
    runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("cancelling"))
        .filter(runs::Column::Id.eq(8))
        .exec(&fixture.db)
        .await
        .unwrap();
    assert!(
        fixture
            .journal()
            .complete(
                activated.execution_version,
                31,
                activated.node.version,
                output("late")
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, activated.execution_version);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
}

#[tokio::test]
async fn timed_wait_releases_ownership_and_resumes_original_attempt_without_replaying_completed_work()
 {
    let mut fixture = Fixture::new().await;
    let first = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let second = fixture
        .journal()
        .activate(first.execution_version, activation(32, "/nodes/1"))
        .await
        .unwrap();
    let wake_at = crate::db::now() - 1;
    let waiting = fixture
        .journal()
        .wait(second.execution_version, 32, second.node.version, wake_at)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .journal()
            .wait(waiting, 32, second.node.version, wake_at)
            .await
            .unwrap(),
        waiting
    );
    assert!(fixture.journal().park(waiting).await.is_err());
    let finished = fixture
        .journal()
        .complete(waiting, 31, first.node.version, output("keep"))
        .await
        .unwrap();
    let parked = fixture
        .journal()
        .park(finished.execution_version)
        .await
        .unwrap();
    assert_eq!(fixture.run().await.state, "waiting");
    assert_eq!(fixture.run().await.not_before, Some(wake_at));
    assert!(!fixture.run().await.context.unwrap().contains("waits"));
    fixture.takeover().await;
    let resumed = fixture.journal().wake(parked).await.unwrap();
    let mut replay = activation(99, "/nodes/1");
    replay.frozen_at_ms = 0;
    let replayed = fixture.journal().activate(resumed, replay).await.unwrap();
    assert_eq!(replayed.node.id, 32);
    assert_eq!(replayed.node.state, "running");
    assert_eq!(replayed.node.version, 3);
    assert_eq!(replayed.snapshot.frozen_at_ms, second.snapshot.frozen_at_ms);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        finished.node
    );
    assert_eq!(fixture.run().await.not_before, None);
    assert!(fixture.run().await.waiting_ms >= 0);
}

#[tokio::test]
async fn two_database_connections_cannot_activate_the_same_expected_version_twice() {
    let fixture = Fixture::new().await;
    let other = Database::connect(&fixture.url).await.unwrap();
    let first = fixture.journal();
    let second = Journal {
        authority: None,
        assets: None,
        db: &other,
        vault: &fixture.vault,
        lease: &fixture.lease,
    };
    let version = fixture.run().await.version;
    let (a, b) = tokio::join!(
        first.activate(version, activation(31, "/nodes/0")),
        second.activate(version, activation(32, "/nodes/0"))
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(nodes::Entity::find().count(&fixture.db).await.unwrap(), 1);
}

#[tokio::test]
async fn expired_journal_write_is_rejected_without_a_successor() {
    let fixture = Fixture::new().await;
    let expiry = crate::db::now() - 1;
    fixture
        .db
        .execute_unprepared(&format!("UPDATE locks SET expires_at={expiry}"))
        .await
        .unwrap();
    let version = fixture.run().await.version;
    assert!(
        fixture
            .journal()
            .activate(version, activation(31, "/nodes/0"))
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, version);
    assert_eq!(nodes::Entity::find().count(&fixture.db).await.unwrap(), 0);
}

#[tokio::test]
async fn expiry_between_attempt_insert_and_final_parent_write_rolls_back_the_entire_transaction() {
    let fixture = Fixture::new().await;
    fixture.db.execute_unprepared("CREATE TRIGGER expire_during_attempt AFTER INSERT ON workflow_node_runs BEGIN UPDATE locks SET expires_at=0; END")
        .await.unwrap();
    let version = fixture.run().await.version;
    assert!(
        fixture
            .journal()
            .activate(version, activation(31, "/nodes/0"))
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await.version, version);
    assert_eq!(nodes::Entity::find().count(&fixture.db).await.unwrap(), 0);
    // The trigger's lease mutation is rolled back too.
    assert!(
        fixture
            .lease
            .renew_live(&fixture.db, crate::db::now())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn pool_wait_crossing_expiry_cannot_authorize_a_prepared_activation() {
    let fixture = Fixture::new().await;
    let mut options = ConnectOptions::new(&fixture.url);
    options.max_connections(1).sqlx_logging(false);
    let db = Database::connect(options).await.unwrap();
    let expiry = crate::db::now() + 200;
    db.execute_unprepared(&format!("UPDATE locks SET expires_at={expiry}"))
        .await
        .unwrap();
    let version = fixture.run().await.version;
    let tx = db.begin().await.unwrap();
    let journal = Journal {
        authority: None,
        assets: None,
        db: &db,
        vault: &fixture.vault,
        lease: &fixture.lease,
    };
    let prepared = journal.activate(version, activation(31, "/nodes/0"));
    let release = async {
        while crate::db::now() <= expiry {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        tx.rollback().await.unwrap();
    };
    let (result, _) = tokio::join!(prepared, release);
    assert!(result.is_err());
    assert_eq!(fixture.run().await.version, version);
    assert_eq!(nodes::Entity::find().count(&fixture.db).await.unwrap(), 0);
}
