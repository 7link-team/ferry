use super::*;
use axum::{body::Bytes, extract::State, http::HeaderMap, response::IntoResponse, routing::any};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, Set};
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use std::sync::{Arc, Mutex};

type Requests = Arc<Mutex<Vec<(String, String, String)>>>;
#[tokio::test]
async fn codeup_uses_yunxiao_header_organization_and_provider_pagination() {
    let (origin, requests, task) = server().await;
    let catalog = catalog();
    for declaration in catalog["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["ui"]["application"] == "codeup")
    {
        let mut http = declaration["http"].clone();
        http["origin"] = json!(origin);
        let params = json!({"credential_id":"123","organization_id":"org-1","repository_id":"group/repo","page":2,"per_page":25,"ref":"feature/a"});
        let reply = request(&client().unwrap(), &http, &params, "PRIVATE_TOKEN")
            .unwrap()
            .send()
            .await
            .unwrap();
        response(&http, reply, "PRIVATE_TOKEN").await.unwrap();
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    for (url, auth, _) in requests.iter() {
        assert!(url.starts_with("/oapi/v1/codeup/organizations/org%2D1/repositories/group%2Frepo"));
        assert_eq!(auth, "PRIVATE_TOKEN");
    }
    assert!(requests[2].0.contains("perPage=25"));
    assert!(requests[2].0.contains("refName=feature%2Fa"));
    task.abort();
}
async fn fixture(
    State(requests): State<Requests>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    requests.lock().unwrap().push((
        uri.to_string(),
        headers
            .get("authorization")
            .or_else(|| headers.get("private-token"))
            .or_else(|| headers.get("x-yunxiao-token"))
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
        String::from_utf8(body.to_vec()).unwrap(),
    ));
    match uri.path() {
        "/redirect" => (StatusCode::FOUND, [("location", "/target")], "").into_response(),
        "/rate" => (StatusCode::TOO_MANY_REQUESTS, "secret upstream detail").into_response(),
        "/rejected" => axum::Json(json!({"ok":false,"error":"missing_scope"})).into_response(),
        "/invalid" => "not JSON".into_response(),
        "/echo" => axum::Json(json!({"token":"PRIVATE_TOKEN"})).into_response(),
        "/large" => "x".repeat(1024 * 1024 + 1).into_response(),
        "/empty" => StatusCode::NO_CONTENT.into_response(),
        _ => axum::Json(json!({"ok":true,"channels":[{"id":"C1","name":"general"}],"response_metadata":{"next_cursor":"next"}})).into_response(),
    }
}
async fn server() -> (String, Requests, tokio::task::JoinHandle<()>) {
    let requests = Requests::default();
    let router = Router::new()
        .fallback(any(fixture))
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (origin, requests, task)
}
fn http(origin: &str, path: &str) -> Value {
    json!({"origin":origin,"path":path,"method":"GET","auth":{"header":"Authorization","prefix":"Bearer "},"query":{},"body":{}})
}

#[tokio::test]
async fn declarative_http_encodes_paths_queries_and_json_and_preserves_pagination() {
    let (origin, requests, task) = server().await;
    let mut declaration = http(&origin, "/api/v4/projects/{project}/issues");
    declaration["method"] = "POST".into();
    declaration["query"] = json!({"state":"state","page":"page","cursor":"cursor"});
    declaration["body"] = json!({"title":"title","description":"description"});
    let params = json!({"project":"group/name","state":"open & closed","page":2,"title":"A title","cursor":""});
    let reply = request(&client().unwrap(), &declaration, &params, "PRIVATE_TOKEN")
        .unwrap()
        .send()
        .await
        .unwrap();
    let output = response(&declaration, reply, "PRIVATE_TOKEN")
        .await
        .unwrap();
    assert_eq!(output["response_metadata"]["next_cursor"], "next");
    let requests = requests.lock().unwrap();
    assert!(
        requests[0]
            .0
            .starts_with("/api/v4/projects/group%2Fname/issues?")
    );
    assert!(requests[0].0.contains("state=open+%26+closed"));
    assert!(!requests[0].0.contains("cursor="));
    assert_eq!(requests[0].1, "Bearer PRIVATE_TOKEN");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].2).unwrap(),
        json!({"title":"A title"})
    );
    task.abort();
}

#[tokio::test]
async fn http_failures_do_not_leak_provider_details_redirect_credentials_or_claim_unknown_success()
{
    let (origin, requests, task) = server().await;
    for (path, key) in [
        ("/redirect", "workflow.integrationRemote"),
        ("/rate", "workflow.integrationRemote"),
        ("/rejected", "workflow.integrationRemote"),
        ("/invalid", "workflow.integrationUnknown"),
        ("/echo", "workflow.integrationUnknown"),
        ("/large", "workflow.integrationUnknown"),
    ] {
        let mut declaration = http(&origin, path);
        if path == "/rejected" {
            declaration["success"] = json!({"pointer":"/ok","equals":true});
        }
        let reply = request(
            &client().unwrap(),
            &declaration,
            &json!({}),
            "PRIVATE_TOKEN",
        )
        .unwrap()
        .send()
        .await
        .unwrap();
        let error = response(&declaration, reply, "PRIVATE_TOKEN")
            .await
            .unwrap_err();
        assert_eq!(error.2.as_ref().unwrap().key, key);
        assert!(!format!("{error:?}").contains("PRIVATE_TOKEN"));
    }
    assert_eq!(requests.lock().unwrap().len(), 6);
    let declaration = http(&origin, "/empty");
    let reply = request(
        &client().unwrap(),
        &declaration,
        &json!({}),
        "PRIVATE_TOKEN",
    )
    .unwrap()
    .send()
    .await
    .unwrap();
    assert_eq!(
        response(&declaration, reply, "PRIVATE_TOKEN")
            .await
            .unwrap()["status_code"],
        204
    );
    task.abort();
}

#[test]
fn all_bundled_operations_use_generic_registration_validation_and_request_mapping() {
    let catalog: Value = serde_json::from_str(include_str!(
        "../../../../../src/shared/workflow-contracts/workflow-integration-nodes.json"
    ))
    .unwrap();
    assert_eq!(catalog["nodes"].as_array().unwrap().len(), 37);
    for declaration in catalog["nodes"].as_array().unwrap() {
        let node = json!({"type":declaration["type"],"type_version":1});
        assert!(supports(&node));
        let mut params = declaration["example_params"].clone();
        for key in declaration["params_schema"]["required"].as_array().unwrap() {
            let key = key.as_str().unwrap();
            if params[key] == "" || key == "credential_id" {
                params[key] = "sample".into();
            }
        }
        if declaration["git"].is_object() {
            graph::validate_resolved(&node, "/test", &params).unwrap();
            super::super::host::git_script(
                declaration["git"]["operation"].as_str().unwrap(),
                &params,
            )
            .unwrap();
            continue;
        }
        params["credential_id"] = "123".into();
        graph::validate_resolved(&node, "/test", &params).unwrap();
        let built = request(
            &client().unwrap(),
            &declaration["http"],
            &params,
            "PRIVATE_TOKEN",
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            built.url().origin().ascii_serialization(),
            declaration["http"]["origin"].as_str().unwrap()
        );
        assert!(!built.url().as_str().contains("PRIVATE_TOKEN"));
    }
    for locale in crate::i18n::Locale::ALL {
        for key in [
            "workflow.integrationInvalid",
            "workflow.integrationUnknown",
            "workflow.integrationRemote",
        ] {
            assert!(!crate::i18n::render(locale, &Message::new(key)).contains("workflow."));
        }
    }
    assert!(
        request(
            &client().unwrap(),
            &http("https://example.invalid", "/{id}"),
            &json!({"id":".."}),
            "token"
        )
        .is_err()
    );
}

#[test]
fn public_read_requests_omit_auth_and_required_auth_cannot_be_empty() {
    let mut public = http("https://example.invalid", "/public");
    public["auth"]["optional"] = true.into();
    assert!(
        !request(&client().unwrap(), &public, &json!({}), "")
            .unwrap()
            .build()
            .unwrap()
            .headers()
            .contains_key("authorization")
    );
    public["auth"]["optional"] = false.into();
    assert!(request(&client().unwrap(), &public, &json!({}), "").is_err());
}

#[tokio::test]
async fn durable_send_gate_blocks_concurrent_duplicates_and_crash_replay_but_reuses_committed_results()
 {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = sea_orm_migration::SchemaManager::new(&db);
    crate::migration::Migrator::up(&db, None).await.unwrap();
    crate::migration::IntegrationCalls
        .up(&manager)
        .await
        .unwrap();
    let call = calls::Model {
        id: 1,
        workspace_id: 11,
        execution_id: 12,
        node_run_id: 13,
        item_index: 0,
        credential_id: 14,
        request_hash: "hash".into(),
        state: "started".into(),
        result: None,
        created_at: 1,
        created_by: 2,
        updated_at: 1,
        updated_by: 2,
        version: 1,
    };
    let mut concurrent = call.clone();
    concurrent.id = 2;
    let (a, b) = tokio::join!(
        reserve_call(&db, call.clone()),
        reserve_call(&db, concurrent)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(reserve_call(&db, call.clone()).await.is_err());
    let saved = calls::Entity::find().one(&db).await.unwrap().unwrap();
    let mut saved = saved.into_active_model();
    saved.state = Set("succeeded".into());
    saved.result = Set(Some("encrypted result".into()));
    saved.update(&db).await.unwrap();
    assert_eq!(
        reserve_call(&db, call.clone())
            .await
            .unwrap()
            .unwrap()
            .result
            .as_deref(),
        Some("encrypted result")
    );
    let mut changed = call;
    changed.request_hash = "changed".into();
    assert!(reserve_call(&db, changed).await.is_err());
    db.execute_unprepared("UPDATE workflow_integration_calls SET state='unknown'")
        .await
        .unwrap();
    let saved = calls::Entity::find().one(&db).await.unwrap().unwrap();
    assert!(reserve_call(&db, saved).await.is_err());
    assert!(
        crate::migration::IntegrationCalls
            .down(&manager)
            .await
            .is_err()
    );
}
