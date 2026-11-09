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
        .filter(|n| n["ui"]["application"] == "codeup" && n["http"].is_object())
    {
        let mut http = declaration["http"].clone();
        http["origin"] = json!(origin);
        let mut params = declaration["example_params"].clone();
        params["credential_id"] = json!("123");
        params["organization_id"] = json!("org-1");
        params["repository_id"] = json!("group/repo");
        params["page"] = json!(2);
        params["per_page"] = json!(25);
        params["ref"] = json!("feature/a");
        let reply = request(&client().unwrap(), &http, &params, "PRIVATE_TOKEN")
            .unwrap()
            .send()
            .await
            .unwrap();
        response(&http, reply, "PRIVATE_TOKEN").await.unwrap();
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 27);
    for (url, auth, _) in requests.iter() {
        assert!(url.starts_with("/oapi/v1/codeup/organizations/org%2D1/repositories"));
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
    let nodes = catalog["nodes"].as_array().unwrap();
    let types = nodes.iter().map(|n| n["type"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(types.len(), nodes.len());
    for (provider, count) in [("github",29),("gitlab",29),("codeup",28),("git",26)] {
        assert_eq!(nodes.iter().filter(|n| n["ui"]["application"] == provider).count(), count);
    }
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

#[test]
fn repository_connections_use_their_own_origin_prefix_org_and_oauth_header() {
    use crate::repositories::auth::RepositoryApiAccess;
    let access = |api: &str, organization: &str, personal| RepositoryApiAccess {
        credential_id: 7.into(), api: api.parse().unwrap(), token: "PRIVATE_TOKEN".into(),
        organization: organization.into(), personal,
    };
    let find = |kind: &str| catalog()["nodes"].as_array().unwrap().iter().find(|n|n["type"]==kind).unwrap().clone();
    let gitlab = find("integration.gitlab.file.get");
    let (http, params) = connection_http(&gitlab["http"], &json!({"project":"group/repo","file_path":"src/a b.txt","ref":"main"}), &access("https://git.example/custom/api/v4", "", false)).unwrap();
    let built = request(&client().unwrap(), &http, &params, "PRIVATE_TOKEN").unwrap().build().unwrap();
    assert_eq!(built.url().host_str(), Some("git.example"));
    assert!(built.url().path().starts_with("/custom/api/v4/projects/group%2Frepo/repository/files/src%2Fa%20b%2Etxt"));
    assert_eq!(built.headers()["authorization"], "Bearer PRIVATE_TOKEN");
    assert!(!built.headers().contains_key("private-token"));
    let github = find("integration.github.branch.list");
    let (http, params) = connection_http(&github["http"], &json!({"owner":"team","repo":"app"}), &access("https://github.example/api/v3", "", true)).unwrap();
    assert!(request(&client().unwrap(), &http, &params, "PRIVATE_TOKEN").unwrap().build().unwrap().url().path().starts_with("/api/v3/repos/team/app/branches"));
    let codeup = find("integration.codeup.branch.create");
    for org in ["org-1", ""] {
        let (http, params) = connection_http(&codeup["http"], &json!({"organization_id":"untrusted-override","repository_id":"group/repo","branch":"feature/a","ref":"main"}), &access("https://codeup.example", org, true)).unwrap();
        let built = request(&client().unwrap(), &http, &params, "PRIVATE_TOKEN").unwrap().build().unwrap();
        assert!(!built.url().as_str().contains("untrusted"));
        assert_eq!(built.url().as_str().contains("/organizations/"), !org.is_empty());
        assert_eq!(built.headers()["x-yunxiao-token"], "PRIVATE_TOKEN");
        assert_eq!(built.method(), reqwest::Method::POST);
        assert!(built.url().query().unwrap().contains("branch=feature%2Fa"));
    }
}

#[test]
fn git_provider_mutation_requests_keep_required_provider_specific_fields() {
    let find = |kind: &str| catalog()["nodes"].as_array().unwrap().iter().find(|n|n["type"]==kind).unwrap().clone();
    for (kind, method, expected) in [
        ("integration.codeup.file.create", "POST", json!({"branch":"main","commitMessage":"write","content":"body","encoding":"text","filePath":"src/a.txt"})),
        ("integration.gitlab.file.update", "PUT", json!({"branch":"main","commit_message":"write","content":"body","encoding":"text"})),
        ("integration.github.file.update", "PUT", json!({"branch":"main","message":"write","content":"body","sha":"abc"})),
    ] {
        let n = find(kind);
        let mut params = n["example_params"].clone();
        params["branch"] = json!("main"); params["message"] = json!("write");
        params["content"] = json!("body"); params["encoding"] = json!("text");
        params["file_path"] = json!("src/a.txt"); params["sha"] = json!("abc");
        let built = request(&client().unwrap(), &n["http"], &params, "PRIVATE_TOKEN").unwrap().build().unwrap();
        assert_eq!(built.method().as_str(),method);
        assert_eq!(serde_json::from_slice::<Value>(built.body().unwrap().as_bytes().unwrap()).unwrap(),expected);
    }
    let codeup = find("integration.codeup.merge_request.merge");
    let built = request(&client().unwrap(), &codeup["http"], &json!({"organization_id":"org","repository_id":"repo","local_id":12,"merge_type":"squash","remove_source_branch":false}), "PRIVATE_TOKEN").unwrap().build().unwrap();
    assert!(built.url().path().ends_with("/changeRequests/12/merge"));
    assert_eq!(serde_json::from_slice::<Value>(built.body().unwrap().as_bytes().unwrap()).unwrap(),json!({"mergeType":"squash","removeSourceBranch":false}));
    let node = json!({"type":"integration.codeup.branch.create","type_version":1});
    assert!(graph::validate_resolved(&node,"/test",&json!({"connection_id":"7","repository_id":"repo","branch":"feature/a","ref":"main"})).is_ok());
    assert!(graph::validate_resolved(&node,"/test",&json!({"repository_id":"repo","branch":"feature/a","ref":"main"})).is_err());
    assert!(graph::validate_resolved(&node,"/test",&json!({"credential_id":"7","organization_id":"org","repository_id":"repo","branch":"feature/a","ref":"main"})).is_ok());
}

#[tokio::test]
async fn connection_reuse_rechecks_tenant_provider_state_and_permissions_and_never_falls_back() {
    use crate::entities::repository_connections as connections;
    let (_dir,s,_replica)=crate::pipelines::runs::lease_tests::fixture(1).await;
    let schema=sea_orm::Schema::new(s.db.get_database_backend());
    s.db.execute(&schema.create_table_from_entity(connections::Entity)).await.unwrap();
    s.db.execute(&schema.create_table_from_entity(credentials::Entity)).await.unwrap();
    let scope=Scope {workspace_id:11.into(),user:crate::accounts::auth::User{id:1.into(),name:"Actor".into(),email:"actor@example.invalid".into(),is_superadmin:false},permissions:["repositories.read","credentials.use"].into_iter().map(str::to_owned).collect()};
    assert!(git_credential(&s,&scope,&json!({"auth_mode":"anonymous","url":"https://example.invalid/public.git"}),None).await.unwrap().1.is_none());
    for params in [json!({"auth_mode":"anonymous","url":"ssh://git@example.invalid/repo"}),json!({"auth_mode":"anonymous","connection_id":"4","url":"https://example.invalid/repo"})] {
        assert!(git_credential(&s,&scope,&params,None).await.is_err());
    }
    credentials::Entity::insert(credentials::Model{id:5,workspace_id:11,name:"Existing repository token".into(),scope:"ops".into(),kind:"token".into(),username:"git".into(),description:String::new(),secret:s.vault.seal("PRIVATE_TOKEN",&crate::vault::aad("11","credentials","5")).unwrap(),created_at:0,updated_at:0,version:1}.into_active_model()).exec(&s.db).await.unwrap();
    let mut row=connections::Model{id:4,workspace_id:11,user_id:99,provider:"gitlab".into(),name:"Shared GitLab".into(),description:String::new(),credential_id:5,account:String::new(),refresh_secret:String::new(),expires_at:0,state:"connected".into(),last_error:String::new(),client_id:String::new(),config_fingerprint:String::new(),token_config:Some(json!({"base_url":"https://git.example","api_url":"https://git.example/api/v4"}).to_string()),version:1,created_at:0,updated_at:0};
    row.config_fingerprint=hex::encode(Sha256::digest(json!(["","https://git.example/","https://git.example/api/v4"]).to_string().as_bytes()));
    connections::Entity::insert(row.clone().into_active_model()).exec(&s.db).await.unwrap();
    let clone_definition=json!({"nodes":[{"id":"clone","type":"integration.codeup.repository.clone","type_version":1,"params":{"connection_id":"4","url":"https://git.example/team/app.git","directory":"/srv/app"}}]}).to_string();
    assert!(validate_connections(&s.db,&scope,&clone_definition).await.is_err());
    assert!(git_credential(&s,&scope,&json!({"connection_id":"4","url":"https://git.example/team/app.git"}),Some("codeup")).await.is_err());
    assert!(git_credential(&s,&scope,&json!({"auth_mode":"anonymous","url":"https://git.example/team/app.git"}),Some("codeup")).await.is_err());
    let mut codeup = row.clone().into_active_model();codeup.provider=Set("codeup".into());codeup.update(&s.db).await.unwrap();
    assert!(validate_connections(&s.db,&scope,&clone_definition).await.is_ok());
    let mut restore = row.clone().into_active_model();restore.provider=Set("gitlab".into());restore.update(&s.db).await.unwrap();
    let declaration=catalog()["nodes"].as_array().unwrap().iter().find(|n|n["type"]=="integration.gitlab.project.get").unwrap();
    let params=json!({"connection_id":"4","credential_id":"999","project":"team/repo"});
    let definition=json!({"nodes":[{"id":"repo","type":declaration["type"],"type_version":1,"params":params}]}).to_string();
    assert!(validate_connections(&s.db,&scope,&definition).await.is_ok());
    let (http,_params,id,secret)=resolve_http(&s,&scope,&declaration["http"],&params).await.unwrap();
    assert_eq!(id,5.into());assert_eq!(secret,"PRIVATE_TOKEN");assert_eq!(http["origin"],"https://git.example");
    let mut other=scope.clone();other.workspace_id=12.into();
    assert!(validate_connections(&s.db,&other,&definition).await.is_err());
    assert!(resolve_http(&s,&other,&declaration["http"],&params).await.is_err());
    for permission in ["credentials.use","repositories.read"] {
        let mut denied=scope.clone();denied.permissions.remove(permission);
        assert_eq!(validate_connections(&s.db,&denied,&definition).await.unwrap_err().0,StatusCode::FORBIDDEN);
        assert_eq!(resolve_http(&s,&denied,&declaration["http"],&params).await.unwrap_err().0,StatusCode::FORBIDDEN);
    }
    let mut different=declaration["http"].clone();different["auth"]["profile"]=json!("github");
    assert!(resolve_http(&s,&scope,&different,&params).await.is_err());
    let mut changed = row.clone().into_active_model(); changed.state=Set("reauthorize".into());changed.update(&s.db).await.unwrap();
    assert!(validate_connections(&s.db,&scope,&definition).await.is_err());
    assert!(resolve_http(&s,&scope,&declaration["http"],&params).await.is_err());
    let mut changed = row.into_active_model(); changed.state=Set("connected".into());changed.name=Set(String::new());changed.update(&s.db).await.unwrap();
    assert!(resolve_http(&s,&scope,&declaration["http"],&params).await.is_err());
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
