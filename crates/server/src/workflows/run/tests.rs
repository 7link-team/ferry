use super::*;
use sea_orm::ConnectionTrait;

#[tokio::test]
async fn running_cancellation_waits_for_owner_cleanup_and_preserves_successful_nodes() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    let journal = fixture.journal();
    let source = journal
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let completed = journal
        .complete(
            source.execution_version,
            31,
            source.node.version,
            Output {
                version: 1,
                ports: vec![Port {
                    name: "main".into(),
                    index: 0,
                    skipped: false,
                    items: vec![],
                }],
            },
        )
        .await
        .unwrap();
    let active = journal
        .activate(completed.execution_version, activation(32, "/nodes/1"))
        .await
        .unwrap();
    let before = fixture.run().await;
    let tx = fixture.db.begin().await.unwrap();
    request_cancel_in(&tx, &before, 3).await.unwrap();
    tx.commit().await.unwrap();
    let pending = fixture.run().await;
    assert_eq!(pending.state, "cancelling");
    assert_eq!(pending.finished_at, 0);
    assert_eq!(
        nodes::Entity::find_by_id(32)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        active.node
    );
    // A late success after the request must not win over cancellation.
    finish(&fixture.db, &fixture.lease, "succeeded", "")
        .await
        .unwrap();
    let ended = fixture.run().await;
    assert_eq!(ended.state, "cancelled");
    assert_eq!(ended.end_reason, "user_cancelled");
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        completed.node
    );
    let cancelled = nodes::Entity::find_by_id(32)
        .one(&fixture.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.finished_at, ended.finished_at);
    assert_eq!(cancelled.resolved_params, active.node.resolved_params);
    let tx = fixture.db.begin().await.unwrap();
    request_cancel_in(&tx, &ended, 3).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(fixture.run().await, ended);
}

#[tokio::test]
async fn queued_and_parked_cancellation_ends_without_dispatch_and_stale_owner_cannot_finish() {
    use super::super::journal::tests::{Fixture, activation};
    for state in ["queued", "waiting"] {
        let fixture = Fixture::new().await;
        let active = fixture
            .journal()
            .activate(fixture.run().await.version, activation(31, "/nodes/0"))
            .await
            .unwrap();
        fixture
            .db
            .execute_unprepared(&format!("UPDATE workflow_executions SET state='{state}'"))
            .await
            .unwrap();
        fixture
            .db
            .execute_unprepared("UPDATE workflow_node_runs SET state='waiting'")
            .await
            .unwrap();
        let before = fixture.run().await;
        let tx = fixture.db.begin().await.unwrap();
        request_cancel_in(&tx, &before, 3).await.unwrap();
        tx.commit().await.unwrap();
        let ended = fixture.run().await;
        assert_eq!(ended.state, "cancelled");
        assert_eq!(ended.finished_by, 3);
        let node = nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.state, "cancelled");
        assert_eq!(node.version, active.node.version + 1);
        assert!(
            finish(&fixture.db, &fixture.lease, "succeeded", "")
                .await
                .is_err()
        );
        assert_eq!(fixture.run().await, ended);
    }
}

#[tokio::test]
async fn cancellation_cannot_let_an_expired_owner_change_active_nodes() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    let active = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let tx = fixture.db.begin().await.unwrap();
    request_cancel_in(&tx, &fixture.run().await, 3)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    fixture
        .db
        .execute_unprepared("UPDATE locks SET expires_at=0")
        .await
        .unwrap();
    let before = fixture.run().await;
    assert!(
        finish(&fixture.db, &fixture.lease, "cancelled", "user_cancelled")
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

#[test]
fn publication_checks_all_nodes_without_using_manual_pins_or_running_expressions() {
    for locale in crate::i18n::Locale::ALL {
        for key in [
            "workflow.publicationName",
            "workflow.publicationRequest",
            "workflow.publicationConflict",
            "workflow.publicationIdentity",
        ] {
            let message = crate::i18n::render(locale, &Message::new(key));
            assert!(!message.contains("workflow.") && !message.is_empty());
        }
    }
    let mut graph = json!({"engine":"ferry-graph","schema":1,"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,
            "position":[0,0],"params":{"input_schema":{"type":"object"}}},
        {"id":"stop","name":"Stop","type":"core.stop","type_version":1,"position":[1,0],
            "params":{"message":"={{ (()=>{throw new Error('MUST_NOT_RUN')})() }}"}}],
        "connections":[{"from":{"node":"start","port":"main","index":0},
            "to":{"node":"stop","port":"main","index":0}}],"pin_data":{"stop":[]}});
    let original = graph.clone();
    publication_ready(&graph).unwrap();
    assert_eq!(graph, original);
    graph["nodes"].as_array_mut().unwrap().push(json!({"id":"unavailable","name":"Unavailable",
        "type":"core.aggregate","type_version":1,"position":[2,0],"params":{"mode":"collect","group_by":[]}}));
    assert!(publication_ready(&graph).is_err());
    graph = original;
    graph["nodes"][1]["settings"] = json!({"mode":"bypass"});
    assert!(publication_ready(&graph).is_err());
    graph["nodes"] = json!([]);
    assert!(publication_ready(&graph).is_err());
}

#[test]
fn stop_uses_generic_localized_errors_and_rejects_undefined_pin_or_bypass_routing() {
    for locale in crate::i18n::Locale::ALL {
        let message = crate::i18n::render(locale, &Message::new("workflow.nodeStopped"));
        assert!(
            !message.contains("workflow.") && !message.contains("PRIVATE") && !message.is_empty()
        );
    }
    let mut graph =
        json!({"nodes":[{"id":"stop","type":"core.stop","params":{"message":"PRIVATE"}}]});
    let plan = plan::Plan {
        version: 1,
        entry_node_id: "start".into(),
        nodes: vec![plan::Node {
            id: "stop".into(),
            path: "/nodes/0".into(),
            inputs: vec![],
        }],
    };
    assert!(supported(&graph, &plan).is_ok());
    graph["nodes"][0]["settings"] = json!({"mode":"mute","always_output":true});
    assert!(supported(&graph, &plan).is_ok());
    graph["nodes"][0]["settings"] = json!({"mode":"bypass"});
    assert!(supported(&graph, &plan).is_err());
    graph["nodes"][0]["settings"] = json!({"mode":"active"});
    graph["pin_data"] = json!({"stop":[]});
    assert!(supported(&graph, &plan).is_err());
}

#[tokio::test]
async fn failed_execution_finishes_active_attempt_and_preserves_completed_history() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    let journal = fixture.journal();
    let source = journal
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let completed = journal
        .complete(
            source.execution_version,
            31,
            source.node.version,
            Output {
                version: 1,
                ports: vec![Port {
                    name: "main".into(),
                    index: 0,
                    skipped: false,
                    items: vec![],
                }],
            },
        )
        .await
        .unwrap();
    let active = journal
        .activate(completed.execution_version, activation(32, "/nodes/1"))
        .await
        .unwrap();
    let bytes = fixture.run().await.data_bytes;
    finish(&fixture.db, &fixture.lease, "failed", "node_failed")
        .await
        .unwrap();
    let run = fixture.run().await;
    let failed = nodes::Entity::find_by_id(32)
        .one(&fixture.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.state, "failed");
    assert_eq!(run.end_reason, "node_failed");
    assert_eq!(run.data_bytes, bytes);
    assert_eq!(failed.state, "failed");
    assert_eq!(failed.error, "node_failed");
    assert_eq!(failed.finished_at, run.finished_at);
    assert!(failed.finished_at >= failed.started_at && failed.finished_at > 0);
    assert_eq!(failed.finished_by, 0);
    assert_eq!(failed.version, active.node.version + 1);
    assert_eq!(failed.resolved_params, active.node.resolved_params);
    assert_eq!(failed.output, active.node.output);
    assert_eq!(
        nodes::Entity::find_by_id(31)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        completed.node
    );
    assert!(
        finish(&fixture.db, &fixture.lease, "failed", "node_failed")
            .await
            .is_err()
    );
    assert_eq!(fixture.run().await, run);
    assert_eq!(
        nodes::Entity::find_by_id(32)
            .one(&fixture.db)
            .await
            .unwrap()
            .unwrap(),
        failed
    );
}

#[tokio::test]
async fn expired_owner_cannot_finish_execution_or_mutate_active_attempt() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    let active = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    fixture
        .db
        .execute_unprepared("UPDATE locks SET expires_at=0")
        .await
        .unwrap();
    let before = fixture.run().await;
    assert!(
        finish(&fixture.db, &fixture.lease, "failed", "node_failed")
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
async fn failure_attempt_update_crossing_expiry_rolls_back_parent_and_child() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    let active = fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    let before = fixture.run().await;
    fixture.db.execute_unprepared("CREATE TRIGGER expire_during_failure AFTER UPDATE OF state ON workflow_node_runs WHEN NEW.state='failed' BEGIN UPDATE locks SET expires_at=0; END").await.unwrap();
    assert!(
        finish(&fixture.db, &fixture.lease, "failed", "node_failed")
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
    assert!(
        fixture
            .lease
            .renew_live(&fixture.db, db::now())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn failure_node_version_overflow_rolls_back_terminal_bookkeeping() {
    use super::super::journal::tests::{Fixture, activation};
    let fixture = Fixture::new().await;
    fixture
        .journal()
        .activate(fixture.run().await.version, activation(31, "/nodes/0"))
        .await
        .unwrap();
    nodes::Entity::update_many()
        .col_expr(nodes::Column::Version, Expr::value(i64::MAX))
        .filter(nodes::Column::Id.eq(31))
        .exec(&fixture.db)
        .await
        .unwrap();
    let before = fixture.run().await;
    let node = nodes::Entity::find_by_id(31)
        .one(&fixture.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        finish(&fixture.db, &fixture.lease, "failed", "node_failed")
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
        node
    );
}

#[test]
fn execution_end_reason_is_a_bounded_code_instead_of_serialized_error_text() {
    let error = error("workflow.runInputInvalid", &"x".repeat(255));
    assert_eq!(failure_reason(&error), "node_failed");
    assert_eq!(failure_reason(&ApiError::forbidden()), "permission_revoked");
}

#[test]
fn pin_items_preserve_literals_and_accept_json_integer_numbers_without_fabricating_data() {
    let definition = json!({"pin_data":{"source":[]}});
    let mut node = json!({"id":"source"});
    assert!(graph::pin_data(&definition, &node, "manual").is_some());
    assert!(graph::pin_data(&definition, &node, "test").is_some());
    assert!(graph::pin_data(&definition, &node, "production").is_none());
    for mode in ["mute", "bypass"] {
        node["settings"] = json!({"mode":mode});
        assert!(graph::pin_data(&definition, &node, "manual").is_none());
    }
    let value = json!([{"json":{"literal":"={{ must_not_evaluate() }}"},
        "binary":{"file":{"asset":format!("sha256:{}", "a".repeat(64)),"size":1.0,"mime":"text/plain","name":"sample"}},
        "paired":[{"node":"source","iteration_hash":"b".repeat(64),"index":0.0}]}]);
    let items = graph::pin_items(&value).unwrap();
    assert_eq!(items[0].json["literal"], "={{ must_not_evaluate() }}");
    assert_eq!(items[0].binary["file"].size, 1);
    assert_eq!(items[0].paired[0].index, 0);
    assert!(graph::pin_items(&json!([])).unwrap().is_empty());
    let mut invalid = value;
    invalid[0]["binary"]["file"]["size"] = json!(1.5);
    assert!(graph::pin_items(&invalid).is_err());
    assert!(graph::pin_items(&json!([{"json":[]}])).is_err());
}

#[test]
fn unsupported_execution_policies_are_rejected_instead_of_silently_ignored() {
    let base = json!({"nodes":[{"id":"start","type":"core.trigger.manual","params":{}}]});
    let plan = plan::Plan {
        version: 1,
        entry_node_id: "start".into(),
        nodes: vec![plan::Node {
            id: "start".into(),
            path: "/nodes/0".into(),
            inputs: vec![],
        }],
    };
    assert!(supported(&base, &plan).is_ok());
    for mode in ["active", "mute", "bypass"] {
        let mut graph = base.clone();
        graph["nodes"][0]["settings"] = json!({"mode":mode});
        assert!(supported(&graph, &plan).is_ok());
    }
    for (pointer, value) in [
        ("/nodes/0/settings", json!({"mode":"unknown"})),
        (
            "/nodes/0/settings",
            json!({"retry":{"max_tries":2,"wait_ms":0}}),
        ),
        ("/settings", json!({"timeout_seconds":1})),
        ("/settings", json!({"timeout_seconds":1.0})),
        (
            "/nodes/0/settings",
            json!({"retry":{"max_tries":2.0,"wait_ms":0}}),
        ),
        ("/settings", json!({"save_data":{"manual":false}})),
    ] {
        let mut graph = base.clone();
        if pointer == "/settings" {
            graph["settings"] = value;
        } else {
            graph["nodes"][0]["settings"] = value;
        }
        assert!(supported(&graph, &plan).is_err(), "{graph}");
    }
    let mut pinned = base;
    pinned["pin_data"] = json!({"start":[{"json":{"answer":42}}]});
    assert!(supported(&pinned, &plan).is_ok());
}

#[test]
fn always_output_creates_only_an_empty_item_for_successfully_executed_empty_results() {
    let enabled = json!({"settings":{"always_output":true}});
    let result = successful_output(&enabled, false, vec![]);
    assert_eq!(result.len(), 1);
    assert!(
        result[0].json.is_empty() && result[0].binary.is_empty() && result[0].paired.is_empty()
    );
    assert!(successful_output(&enabled, true, vec![]).is_empty());
    assert!(successful_output(&json!({}), false, vec![]).is_empty());
    for mode in ["mute", "bypass"] {
        assert!(
            successful_output(
                &json!({"settings":{"always_output":true,"mode":mode}}),
                false,
                vec![]
            )
            .is_empty()
        );
    }
    let item: Item = serde_json::from_value(json!({"json":{"value":[false,null]},"paired":[{"node":"source","iteration_hash":"a".repeat(64),"index":2}]})).unwrap();
    let before = serde_json::to_value(&item).unwrap();
    let result = successful_output(&enabled, false, vec![item]);
    assert_eq!(result.len(), 1);
    assert_eq!(serde_json::to_value(&result[0]).unwrap(), before);
}

#[test]
fn manual_schema_does_not_fetch_external_resources_and_validates_original_types() {
    let spec = json!({"params":{"input_schema":{"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"],"additionalProperties":false}}});
    assert!(input(&spec, json!({"answer":42}).as_object().unwrap()).is_ok());
    assert!(input(&spec, json!({"answer":"42"}).as_object().unwrap()).is_err());
    assert!(
        input(
            &spec,
            json!({"answer":42,"extra":true}).as_object().unwrap()
        )
        .is_err()
    );
    let external = json!({"params":{"input_schema":{"$ref":"file:///private-resource.json"}}});
    assert!(input(&external, &Map::new()).is_err());
}
#[test]
fn set_writes_nested_objects_preserves_types_and_paired_lineage() {
    let item = Item {
        json: json!({"original":true}).as_object().unwrap().clone(),
        binary: Default::default(),
        paired: vec![Paired {
            node: "source".into(),
            iteration_hash: "a".repeat(64),
            index: 2,
        }],
    };
    let params = json!({"fields":[{"path":"nested.answer","value":42},{"path":"other","value":[false,null]}],"keep_other_fields":false});
    let result = set(&[item], &[params]).unwrap();
    assert_eq!(
        result[0].json,
        json!({"nested":{"answer":42},"other":[false,null]})
            .as_object()
            .unwrap()
            .clone()
    );
    assert_eq!(result[0].paired[0].index, 2);
}
#[test]
fn set_rejects_overlapping_paths_pollution_and_scalar_traversal_without_partial_output() {
    let item = Item {
        json: json!({"scalar":1}).as_object().unwrap().clone(),
        binary: Default::default(),
        paired: vec![],
    };
    for fields in [
        json!([{"path":"x","value":1},{"path":"x.y","value":2}]),
        json!([{"path":"__proto__.pwn","value":true}]),
        json!([{"path":"scalar.y","value":2}]),
        json!([{"path":"x..y","value":true}]),
    ] {
        assert!(
            set(
                std::slice::from_ref(&item),
                &[json!({"fields":fields,"keep_other_fields":true})]
            )
            .is_err()
        );
        assert_eq!(item.json["scalar"], 1);
    }
}
