use super::*;
use crate::{
    agents::tests::admin,
    entities::workspace_roles,
    error::{Json, Path},
    platform_tools::tests::setup,
    snowflake::Id,
};
use axum::extract::State;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};

fn node(params: Value) -> Value {
    json!({"id":"read","name":"Read","type":"ferry.request","type_version":1,
        "params":params,"position":[300,0]})
}
fn params(operation: &str, path_params: Value) -> Value {
    json!({"operation":operation,"path_params":path_params,"query":{},
        "split_items":true,"all_pages":false,"max_pages":50})
}
fn graph(read: Value) -> Value {
    json!({"engine":"ferry-graph","schema":1,"settings":{"timezone":"UTC"},"nodes":[
        {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,
            "params":{"input_schema":{"type":"object"}},"position":[0,0]},
        node(read)
    ],"connections":[{"from":{"node":"start","port":"main","index":0},
        "to":{"node":"read","port":"main","index":0}}]})
}
fn key(error: &ApiError) -> &str {
    &error.2.as_ref().unwrap().key
}
fn saves(read: Value) -> Result<String> {
    super::super::graph::validate(&graph(read).to_string())
}

#[test]
fn only_catalogue_get_operations_with_exact_safe_path_params_can_be_saved() {
    saves(params("/workflows", json!({}))).unwrap();
    saves(params(
        "/code-insights/{id}/reports/{kind}",
        json!({"id":"7","kind":"contributors"}),
    ))
    .unwrap();
    // Expressions are accepted at save time and checked again as literals per item.
    saves(params("/workflows/{id}", json!({"id":"={{ $json.id }}"}))).unwrap();
    for (operation, reason) in [
        (
            "/workflows/{id}/publications/{revision}",
            "unknown template",
        ),
        ("/code-insights/{id}/sync", "POST only"),
        ("/workflows?page=2", "query is not part of the template"),
        ("/../2/workflows", "relative step"),
        ("/api/workflows", "not a template"),
    ] {
        let error = saves(params(operation, json!({}))).unwrap_err();
        assert_eq!(key(&error), "workflow.platformOperation", "{reason}");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        key(&saves(params("/workflows/{id}", json!({"id":""}))).unwrap_err()),
        "workflow.graphSchema"
    );
    for path_params in [
        json!({}),
        json!({"id":"1/2"}),
        json!({"id":".."}),
        json!({"id":"a..b"}),
        json!({"id":"a\\b"}),
        json!({"id":"1","workspace":"2"}),
    ] {
        let error = saves(params("/workflows/{id}", path_params.clone())).unwrap_err();
        assert_eq!(key(&error), "workflow.platformPathParam", "{path_params}");
    }
    // Unknown parameters (such as a workspace selector) are rejected by the contract.
    let mut read = params("/workflows", json!({}));
    read["workspace_id"] = json!("2");
    assert_eq!(key(&saves(read).unwrap_err()), "workflow.graphSchema");
    for (field, value) in [
        ("max_pages", json!(0)),
        ("operation", json!("")),
        ("query", json!({"nested":{"a":1}})),
    ] {
        let mut read = params("/workflows", json!({}));
        read[field] = value;
        assert_eq!(
            key(&saves(read).unwrap_err()),
            "workflow.graphSchema",
            "{field}"
        );
    }
    let mut read = params("/workflows", json!({}));
    read["query"] = Value::Object((0..65).map(|i| (format!("k{i}"), json!(i))).collect());
    assert!(saves(read).is_ok());
    let mut legacy = params("/workflows", json!({}));
    legacy["max_pages"] = json!(51);
    assert!(saves(legacy).is_ok());
}

#[test]
fn editor_operation_list_is_exactly_the_catalogue_get_templates() {
    let catalogue: Vec<Value> =
        serde_json::from_str(include_str!("../../platform_tools_catalog.json")).unwrap();
    let editor: Vec<Value> = serde_json::from_str(include_str!(
        "../../../../../src/shared/workflow-contracts/platform-read-operations.json"
    ))
    .unwrap();
    let reads: Vec<_> = catalogue
        .iter()
        .filter(|row| row["method"] == "GET")
        .map(|row| (row["path"].clone(), row["permission"].clone()))
        .collect();
    let listed: Vec<_> = editor
        .iter()
        .map(|row| (row["path"].clone(), row["permission"].clone()))
        .collect();
    assert_eq!(listed, reads);
    assert!(reads.len() > 100);
    assert!(
        listed
            .iter()
            .all(|(path, _)| platform_tools::read_operation(path.as_str().unwrap()))
    );
}

#[test]
fn path_values_are_single_encoded_segments_and_queries_are_bounded_scalars() {
    assert_eq!(
        path(
            "/workflows/{id}/revisions/{revision}",
            &json!({"id":"a b%","revision":"=1"})
        )
        .unwrap(),
        "/workflows/a%20b%25/revisions/%3D1"
    );
    for value in ["", "1/2", "..", "x/../y", "a\\b", "\u{0}"] {
        let error = path("/workflows/{id}", &json!({ "id": value })).unwrap_err();
        assert_eq!(key(&error), "workflow.platformPathParam", "{value:?}");
    }
    assert_eq!(
        key(&path("/workflows/{id}", &json!({})).unwrap_err()),
        "workflow.platformPathParam"
    );
    let query = json!({"since":"2026-11-01","limit":5,"open":true,"q":"a&b=c"});
    assert_eq!(
        url("/workflows", query.as_object().unwrap()).unwrap(),
        "/workflows?limit=5&open=true&q=a%26b%3Dc&since=2026%2D11%2D01"
    );
    assert_eq!(url("/workflows", &Map::new()).unwrap(), "/workflows");
    for query in [
        json!({"a":null}),
        json!({"a":[1]}),
        json!({"a":{"b":1}}),
        json!({"":"x"}),
        json!({"a":"x".repeat(4096)}),
    ] {
        let error = url("/workflows", query.as_object().unwrap()).unwrap_err();
        assert_eq!(key(&error), "workflow.platformQuery");
    }
}

#[test]
fn only_successful_responses_become_data_and_failures_keep_status_and_message() {
    assert_eq!(
        interpret(json!({"status":200,"ok":true,"data":{"items":[]}})).unwrap(),
        json!({"items":[]})
    );
    let error = interpret(json!({"status":413,"ok":false,"error":"limit"})).unwrap_err();
    assert_eq!(key(&error), "workflow.platformTooLarge");
    for (status, code) in [
        (403, StatusCode::FORBIDDEN),
        (404, StatusCode::NOT_FOUND),
        (400, StatusCode::BAD_REQUEST),
        (500, StatusCode::INTERNAL_SERVER_ERROR),
        (302, StatusCode::BAD_GATEWAY),
    ] {
        let error =
            interpret(json!({"status":status,"ok":false,"data":{"error":"Handler said no"}}))
                .unwrap_err();
        assert_eq!(error.0, code);
        let message = error.2.as_ref().unwrap();
        assert_eq!(message.key, "workflow.platformRequest");
        assert_eq!(message.params["status"], status.to_string());
        assert_eq!(message.params["message"], "Handler said no");
    }
    // Never report success for a body that claims ok with a failing status.
    assert!(interpret(json!({"status":500,"ok":true,"data":{}})).is_err());
    let long =
        interpret(json!({"status":400,"ok":false,"data":{"error":"x".repeat(2000)}})).unwrap_err();
    assert_eq!(long.2.unwrap().params["message"].chars().count(), 500);
    assert_eq!(
        super::super::run::failure_reason(&error_for(403)),
        "permission_revoked"
    );
    assert_eq!(
        super::super::run::failure_reason(&error_for(404)),
        "platform_request"
    );
    assert_eq!(
        super::super::run::failure_reason(&too_large()),
        "platform_too_large"
    );
}
fn error_for(status: u16) -> ApiError {
    interpret(json!({"status":status,"ok":false,"data":{"error":"denied"}})).unwrap_err()
}

#[test]
fn split_items_emits_one_item_per_listed_element_otherwise_one_item() {
    let rows = |data: Value, split| {
        Value::from(
            values(data, split)
                .into_iter()
                .map(Value::Object)
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(rows(json!([{"a":1},2]), true), json!([{"a":1},{"value":2}]));
    assert_eq!(
        rows(json!({"items":[{"a":1},{"a":2}],"total":2}), true),
        json!([{"a":1},{"a":2}])
    );
    assert_eq!(rows(json!({"items":[],"total":0}), true), json!([]));
    assert_eq!(rows(json!([]), true), json!([]));
    assert_eq!(rows(json!({"a":1}), true), json!([{"a":1}]));
    assert_eq!(
        rows(json!({"items":"text"}), true),
        json!([{"items":"text"}])
    );
    assert_eq!(
        rows(json!({"text":"plain"}), true),
        json!([{"text":"plain"}])
    );
    assert_eq!(
        rows(json!({"items":[1],"total":1}), false),
        json!([{"items":[1],"total":1}])
    );
    assert_eq!(rows(json!([1]), false), json!([{"value":[1]}]));
    assert_eq!(rows(json!(3), true), json!([{"value":3}]));
}

struct Fixture {
    _dir: tempfile::TempDir,
    s: AppState,
    run: workflow_executions::Model,
}

async fn workflow(s: &AppState, workspace: i64, name: &str, definition: &Value) -> Id {
    let (_, Json(workflow)) = super::super::create(
        State(s.clone()),
        admin(workspace),
        Json(
            serde_json::from_value(json!({"name":name,"definition":definition.to_string()}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    serde_json::from_value(workflow["id"].clone()).unwrap()
}

impl Fixture {
    async fn new() -> Self {
        let (dir, s) = setup().await;
        let id = workflow(
            &s,
            1,
            "Platform data",
            &graph(params("/workflows", json!({}))),
        )
        .await;
        let (_, Json(created)) = super::super::run::create(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(
                serde_json::from_value(
                    json!({"mode":"manual","revision":"draft","version":1,"entry_node_id":"start"}),
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let execution: Id = serde_json::from_value(created["id"].clone()).unwrap();
        workflow_executions::Entity::update_many()
            .col_expr(workflow_executions::Column::State, Expr::value("running"))
            .filter(workflow_executions::Column::Id.eq(i64::from(execution)))
            .exec(&s.db)
            .await
            .unwrap();
        let run = workflow_executions::Entity::find_by_id(i64::from(execution))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        Self { _dir: dir, s, run }
    }
    async fn read(&self, params: Value) -> Result<Vec<Item>> {
        let input = Item {
            json: json!({"from":"trigger"}).as_object().unwrap().clone(),
            binary: Default::default(),
            paired: vec![ferry_protocol::sandbox::Paired {
                node: "start".into(),
                iteration_hash: String::new(),
                index: 0,
            }],
        };
        execute(&self.s, &self.run, &[input], &[params]).await
    }
    async fn grant(&self, permissions: Vec<&str>) {
        workspace_roles::Entity::update_many()
            .col_expr(
                workspace_roles::Column::Permissions,
                Expr::value(json!(permissions).to_string()),
            )
            .filter(workspace_roles::Column::Id.eq(1))
            .exec(&self.s.db)
            .await
            .unwrap();
    }
}

fn names(items: &[Item]) -> Vec<String> {
    let mut names: Vec<_> = items
        .iter()
        .map(|item| item.json["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn reads_only_the_execution_workspace_and_splits_listed_rows() {
    let fixture = Fixture::new().await;
    let other = workflow(
        &fixture.s,
        2,
        "Other workspace",
        &graph(params("/workflows", json!({}))),
    )
    .await;
    let items = fixture.read(params("/workflows", json!({}))).await.unwrap();
    assert_eq!(names(&items), ["Platform data"]);
    // Each row replaces the input JSON and keeps the input item's lineage.
    assert!(items.iter().all(|item| item.json.get("from").is_none()));
    assert!(items.iter().all(|item| item.paired[0].node == "start"));
    let mut whole = params("/workflows", json!({}));
    whole["split_items"] = json!(false);
    let items = fixture.read(whole).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].json["total"], json!(1));
    // Another workspace's resource is not reachable by ID; the handler's 404 passes through.
    let error = fixture
        .read(params("/workflows/{id}", json!({"id":other.to_string()})))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
    assert_eq!(key(&error), "workflow.platformRequest");
    assert_eq!(error.2.as_ref().unwrap().params["status"], "404");
    // Graph parameters cannot select a workspace or escape the workspace prefix.
    for (operation, path_params) in [
        ("/workflows/{id}", json!({"id":"../../2/workflows"})),
        ("/workflows/{id}", json!({"id":"%2e%2e"})),
        ("/../2/workflows", json!({})),
    ] {
        let error = fixture
            .read(params(operation, path_params.clone()))
            .await
            .unwrap_err();
        assert_ne!(error.0, StatusCode::OK, "{operation} {path_params}");
        assert!(
            matches!(
                key(&error),
                "workflow.platformPathParam"
                    | "workflow.platformOperation"
                    | "workflow.platformRequest"
            ),
            "{operation} {path_params}: {}",
            key(&error)
        );
    }
    // Unknown or write operations are rejected at execution too.
    let error = fixture
        .read(params("/code-insights/{id}/sync", json!({"id":"1"})))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.platformOperation");
}

#[tokio::test]
async fn execution_identity_permissions_are_rechecked_for_every_request() {
    let fixture = Fixture::new().await;
    // The handler's own permission check denies a run actor without read access.
    fixture.grant(vec!["workflows.run"]).await;
    let error = fixture
        .read(params("/workflows", json!({})))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert_eq!(key(&error), "workflow.platformRequest");
    assert_eq!(
        super::super::run::failure_reason(&error),
        "permission_revoked"
    );
    // Revoking the run permission stops the node before any request is sent.
    fixture.grant(vec!["workflows.read"]).await;
    let error = fixture
        .read(params("/workflows", json!({})))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(error.2.is_none() || key(&error) != "workflow.platformRequest");
    fixture.grant(vec!["workflows.read", "workflows.run"]).await;
    assert_eq!(
        fixture
            .read(params("/workflows", json!({})))
            .await
            .unwrap()
            .len(),
        1
    );
    // A finished execution can no longer read.
    workflow_executions::Entity::update_many()
        .col_expr(workflow_executions::Column::State, Expr::value("cancelled"))
        .filter(workflow_executions::Column::Id.eq(fixture.run.id))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .read(params("/workflows", json!({})))
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn all_pages_reads_every_page_and_fails_instead_of_truncating() {
    let fixture = Fixture::new().await;
    for index in 0..3 {
        workflow(
            &fixture.s,
            1,
            &format!("Extra {index}"),
            &graph(params("/workflows", json!({}))),
        )
        .await;
    }
    let mut paged = params("/workflows", json!({}));
    paged["query"] = json!({"page_size":1,"page":9});
    paged["all_pages"] = json!(true);
    let items = fixture.read(paged.clone()).await.unwrap();
    assert_eq!(
        names(&items),
        ["Extra 0", "Extra 1", "Extra 2", "Platform data"]
    );
    paged["split_items"] = json!(false);
    let items = fixture.read(paged.clone()).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].json["total"], json!(4));
    assert_eq!(items[0].json["items"].as_array().unwrap().len(), 4);
    paged["max_pages"] = json!(4);
    assert_eq!(fixture.read(paged.clone()).await.unwrap().len(), 1);
    paged["max_pages"] = json!(3);
    assert_eq!(fixture.read(paged.clone()).await.unwrap().len(), 1);
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ResourceConfig,
            Expr::value(Some(json!({"workflow_platform_pages":3}).to_string())),
        )
        .exec(&fixture.s.db)
        .await
        .unwrap();
    let error = fixture.read(paged.clone()).await.unwrap_err();
    assert_eq!(key(&error), "workflow.platformPageLimit");
    assert_eq!(error.2.as_ref().unwrap().params["max_pages"], "3");
    assert_eq!(
        super::super::run::failure_reason(&error),
        "platform_page_limit"
    );
    // Without all_pages only the requested page is returned.
    paged["all_pages"] = json!(false);
    paged["split_items"] = json!(true);
    paged["query"] = json!({"page_size":1});
    assert_eq!(fixture.read(paged).await.unwrap().len(), 1);
    // Responses without the items/total envelope cannot be paged.
    let mut single = params(
        "/workflows/{id}",
        json!({"id":Id::from(fixture.run.workflow_id).to_string()}),
    );
    assert_eq!(fixture.read(single.clone()).await.unwrap().len(), 1);
    single["all_pages"] = json!(true);
    let error = fixture.read(single).await.unwrap_err();
    assert_eq!(key(&error), "workflow.platformPagination");
}

#[tokio::test]
async fn oversized_gateway_responses_fail_with_a_narrow_the_query_error() {
    let fixture = Fixture::new().await;
    let mut large = graph(params("/workflows", json!({})));
    large["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"set","name":"Set","type":"data.set",
        "type_version":1,"params":{"fields":[{"path":"blob","value":"x".repeat(300 * 1024)}],
        "keep_other_fields":true},"position":[600,0]}));
    let id = workflow(&fixture.s, 1, "Large", &large).await;
    // The gateway now has an administrator-configured limit, defaulting to
    // 1 MiB. This former 256 KiB regression fixture must be accepted at the
    // default, and must still fail with the same precise error at a lower cap.
    let accepted = fixture
        .read(params("/workflows/{id}", json!({"id":id.to_string()})))
        .await
        .unwrap();
    assert!(accepted[0].json["definition"].as_str().unwrap().len() > 300 * 1024);
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ResourceConfig,
            Expr::value(Some(json!({"tool_response_bytes":256*1024}).to_string())),
        )
        .filter(crate::entities::platform_state::Column::Id.eq(1))
        .exec(&fixture.s.db)
        .await
        .unwrap();
    let error = fixture
        .read(params("/workflows/{id}", json!({"id":id.to_string()})))
        .await
        .unwrap_err();
    assert_eq!(key(&error), "workflow.platformTooLarge");
    assert_eq!(error.0, StatusCode::PAYLOAD_TOO_LARGE);
}

#[test]
fn node_catalog_lists_every_runnable_node_and_only_those() {
    let nodes = crate::workflows::integration::builtin_nodes();
    let types: std::collections::BTreeSet<&str> = nodes
        .iter()
        .filter_map(|node| node["type"].as_str())
        .collect();
    for required in [
        "core.trigger.manual",
        "data.set",
        "ferry.request",
        "ai.agent.run",
        "core.sort",
        "core.if",
    ] {
        assert!(types.contains(required), "catalog misses {required}");
    }
    assert_eq!(types.len(), nodes.len(), "a node type is listed twice");
    // Everything listed runs, so nothing may still carry the draft label that
    // makes agents refuse to build with it.
    for node in &nodes {
        assert_eq!(
            node["status"], "implemented",
            "{} is labelled {}",
            node["type"], node["status"]
        );
    }
    for node in &nodes {
        let probe = serde_json::json!({"type": node["type"], "type_version": node["type_version"], "params": node["example_params"]});
        assert!(
            crate::workflows::run::runnable(&probe),
            "catalog offers {} which no executor runs",
            node["type"]
        );
    }
}
