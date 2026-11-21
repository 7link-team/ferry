use super::*;
use crate::{
    agents,
    agents::tests::{admin, setup as base_setup},
    error::Json,
};
use axum::extract::State;
use sea_orm::{ActiveModelTrait, IntoActiveModel};

#[tokio::test]
async fn storage_failure_is_classified_without_matching_localized_text_or_retrying_the_write() {
    use sea_orm::{PaginatorTrait, sea_query::Expr};
    let (_dir, s) = setup().await;
    let definition = json!({"engine":"ferry-graph","schema":1,"nodes":[{"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,"params":{"input_schema":{"type":"object"}},"position":[0,0]}],"connections":[]});
    let created = dispatch(
        &s,
        admin(1),
        Call {
            method: "POST".into(),
            path: "/workflows".into(),
            body: json!({"name":"Storage failure fixture","definition":definition.to_string()}),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(created["status"], 201, "{created}");
    let id = created["data"]["id"].as_str().unwrap();
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            Expr::value("unreadable-storage-fixture"),
        )
        .exec(&s.db)
        .await
        .unwrap();
    for locale in [
        crate::i18n::Locale::EnUs,
        crate::i18n::Locale::ZhCn,
        crate::i18n::Locale::MsMy,
    ] {
        let result = crate::i18n::with_locale(locale, dispatch(&s, admin(1), Call {method:"POST".into(),path:format!("/workflows/{id}/executions"),body:json!({"mode":"manual","revision":"draft","version":1,"entry_node_id":"start"}),..Default::default()})).await.unwrap();
        assert_eq!(result["status"], 503, "{result}");
        assert_eq!(result["guidance"], STORAGE_GUIDANCE);
        assert!(
            !result["guidance"]
                .as_str()
                .unwrap()
                .contains("wait briefly")
        );
        assert_eq!(
            crate::entities::workflow_executions::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(
        denial(&crate::artifact_store::storage_unavailable())["guidance"],
        STORAGE_GUIDANCE
    );
}

#[tokio::test]
async fn configured_tool_limit_accepts_large_results_and_distinguishes_read_failures() {
    use crate::entities::{platform_state, workflows};
    use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};
    let (_dir, s) = setup().await;
    let created=dispatch(&s,admin(1),Call {method:"POST".into(),path:"/workflows".into(),
        body:json!({"name":"Transport fixture","definition":"{\"engine\":\"ferry-graph\",\"schema\":1,\"nodes\":[],\"connections\":[]}"}),..Default::default()}).await.unwrap();
    assert_eq!(created["status"], 201, "{created}");
    let id: Id = serde_json::from_value(created["data"]["id"].clone()).unwrap();
    let fetch = || Call {
        method: "GET".into(),
        path: format!("/workflows/{id}"),
        ..Default::default()
    };
    let text = "x".repeat(600 * 1024);
    workflows::Entity::update_many()
        .col_expr(workflows::Column::Definition, Expr::value(text.clone()))
        .filter(workflows::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let accepted = dispatch(&s, admin(1), fetch()).await.unwrap();
    assert_eq!(accepted["status"], 200, "{accepted}");
    assert_eq!(
        accepted["data"]["definition"].as_str().unwrap().len(),
        text.len()
    );
    let text = "x".repeat(2 * 1024 * 1024);
    workflows::Entity::update_many()
        .col_expr(workflows::Column::Definition, Expr::value(text.clone()))
        .filter(workflows::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let rejected = dispatch(&s, admin(1), fetch()).await.unwrap();
    assert_eq!(rejected["status"], 413, "{rejected}");
    assert!(rejected["error"].as_str().unwrap().contains("read-only"));
    assert!(!rejected["error"].as_str().unwrap().contains("write may"));
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::ConversationConfig,
            Expr::value(json!({"tool_response_limit_kib":4096}).to_string()),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let accepted = dispatch(&s, admin(1), fetch()).await.unwrap();
    assert_eq!(accepted["status"], 200);
    assert_eq!(accepted["data"]["definition"].as_str().unwrap(), text);
    // Exercise the configured maximum with escaping overhead, rather than only
    // counting characters in a plain ASCII result.
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::ConversationConfig,
            Expr::value(json!({"tool_response_limit_kib":8192}).to_string()),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let text = "\\".repeat(3 * 1024 * 1024);
    workflows::Entity::update_many()
        .col_expr(workflows::Column::Definition, Expr::value(text.clone()))
        .filter(workflows::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let accepted = dispatch(&s, admin(1), fetch()).await.unwrap();
    assert_eq!(accepted["status"], 200, "{}", accepted["error"]);
    assert_eq!(accepted["data"]["definition"].as_str().unwrap(), text);
    let text = "x".repeat(8 * 1024 * 1024);
    workflows::Entity::update_many()
        .col_expr(workflows::Column::Definition, Expr::value(text))
        .filter(workflows::Column::Id.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let rejected = dispatch(&s, admin(1), fetch()).await.unwrap();
    assert_eq!(rejected["status"], 413);
    assert!(rejected["error"].as_str().unwrap().contains("read-only"));
}

#[test]
fn workflow_discovery_explains_parameters_and_finds_authoring_contracts() {
    let mut operations = Vec::new();
    let mut offset = 0;
    loop {
        let page = catalogue(&admin(1), "workflows", offset);
        assert!(page["usage"]["response"].is_object());
        assert!(serde_json::to_vec(&page).unwrap().len() < 256 * 1024);
        operations.extend(page["operations"].as_array().unwrap().clone());
        let Some(next) = page["next_offset"].as_u64() else {
            break;
        };
        offset = next as usize;
    }
    for path in ["/workflow-definition-format", "/workflow-node-catalog"] {
        assert!(
            operations.iter().any(|operation| operation["path"] == path),
            "missing {path}"
        );
    }
    let create = operations
        .iter()
        .find(|op| op["path"] == "/workflows" && op["method"] == "POST")
        .unwrap();
    assert!(
        create["documentation"]
            .as_str()
            .unwrap()
            .contains("/workflow-definition-format")
    );
    let list = operations
        .iter()
        .find(|op| op["path"] == "/workflows" && op["method"] == "GET")
        .unwrap();
    assert!(
        list["query"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["name"] == "page_size")
    );
}

#[tokio::test]
async fn reported_invalid_workflow_returns_recovery_instructions_without_creating_it() {
    let (_dir, s) = setup().await;
    let rejected = dispatch(
        &s,
        admin(1),
        Call {
            method: "POST".into(),
            path: "/workflows".into(),
            body: json!({"name":"Weekly insights","definition":"{\"nodes\":[],\"edges\":[]}"}),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rejected["status"], 400, "{rejected}");
    let guidance = rejected["guidance"].as_str().unwrap();
    assert!(
        guidance.contains("/workflow-definition-format"),
        "{guidance}"
    );
    assert!(guidance.contains("connections"), "{guidance}");
    let list = dispatch(
        &s,
        admin(1),
        Call {
            method: "GET".into(),
            path: "/workflows".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(list["data"]["total"], 0);
    let format = dispatch(
        &s,
        admin(1),
        Call {
            method: "GET".into(),
            path: "/workflow-definition-format".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let schedule: crate::cron_jobs::Body =
        serde_json::from_value(format["data"]["requests"]["schedule"]["body"].clone()).unwrap();
    assert!(!schedule.name.is_empty());
    assert_eq!(schedule.action, "workflow");
    let rules = format["data"]["rules"].to_string();
    assert!(
        rules.contains("downstream_id") && rules.contains("downstream_kind"),
        "{rules}"
    );
    assert!(
        rules.contains("assisted") && rules.contains("yolo"),
        "{rules}"
    );
    let call: Call =
        serde_json::from_value(format["data"]["requests"]["create_minimal"].clone()).unwrap();
    let created = dispatch(&s, admin(1), call).await.unwrap();
    assert_eq!(created["status"], 201, "{created}");
}

pub(crate) async fn setup() -> (tempfile::TempDir, AppState) {
    let (dir, s) = base_setup().await;
    for id in [1, 2] {
        workspace_roles::Entity::insert(
            workspace_roles::Model {
                id,
                workspace_id: id,
                name: format!("Owner {id}"),
                permissions: json!(admin(id).permissions).to_string(),
                builtin: 0,
                created_at: 0,
                updated_at: 0,
                version: 1,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    (dir, s)
}

pub(crate) async fn role(s: &AppState, workspace: i64, permissions: Vec<&str>) -> Id {
    let Json(value) = crate::accounts::workspace_admin::create_role(
        State(s.clone()),
        admin(workspace),
        Json(
            serde_json::from_value(json!({"name":"Agent access","permissions":permissions}))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    serde_json::from_value(value["id"].clone()).unwrap()
}
pub(crate) async fn agent(s: &AppState, role: Id) -> ai_agents::Model {
    let body=serde_json::from_value(json!({"name":"Platform assistant","role":"Test","platform_role_id":role.to_string(),"owner_id":"1","model_profile_id":crate::agents::tests::test_cli_model(1,"codex").to_string(),"host_id":"1","runtime":"codex","permission":"read_only","visibility":"workspace","state":"enabled"})).unwrap();
    let (_, Json(value)) = agents::create(State(s.clone()), admin(1), Json(body))
        .await
        .unwrap();
    ai_agents::Entity::find_by_id(value["id"].as_str().unwrap().parse::<i64>().unwrap())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn notifications_and_resumed_workflows_keep_both_permission_ceilings() {
    let (_dir, s) = setup().await;
    let grants = vec!["notifications.read", "notifications.send"];
    let role_id = role(&s, 1, grants).await;
    let agent = agent(&s, role_id).await;
    let body=serde_json::from_value(json!({"name":"Local fixture","provider":"Webhook","webhook":"http://127.0.0.1:9/fixture","role_ids":["1"]})).unwrap();
    let (_, Json(channel)) =
        crate::notifications::center::create(State(s.clone()), admin(1), Json(body))
            .await
            .unwrap();
    let channel_id: Id = serde_json::from_value(channel["id"].clone()).unwrap();
    let row = crate::notifications::center::channel(&s, 1.into(), channel_id)
        .await
        .unwrap();
    for path in [
        "/notification-channels",
        "/notification-templates",
        "/notification-deliveries",
    ] {
        let scope = authorized_scope(&s, admin(1), &agent).await.unwrap();
        let result = dispatch_as_agent(
            &s,
            scope,
            Call {
                method: "GET".into(),
                path: path.into(),
                ..Default::default()
            },
            agent.id.into(),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200, "{path}: {result}");
    }
    // Human role 1 is allowed, but the Agent's independent role is not.
    assert!(
        crate::notifications::center::authorize(&s, &admin(1), &row)
            .await
            .is_ok()
    );
    let denied = delegated_future(&s, admin(1), agent.id.into(), async {
        let scope = Scope::for_actor(&s, 1.into(), 1.into()).await?;
        crate::notifications::center::authorize(&s, &scope, &row).await
    })
    .await
    .unwrap_err();
    assert_eq!(denied.0, axum::http::StatusCode::FORBIDDEN);
    let mut scope = admin(1);
    scope.permissions.remove("notifications.send");
    let result = dispatch_as_agent(
        &s,
        authorized_scope(&s, scope, &agent).await.unwrap(),
        Call {
            method: "POST".into(),
            path: format!("/notification-channels/{channel_id}/deliveries"),
            body: json!({"client_id":"denied","body":"not sent"}),
            ..Default::default()
        },
        agent.id.into(),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], 403);
    let result=dispatch_as_agent(&s,authorized_scope(&s,admin(1),&agent).await.unwrap(),Call {
        method:"POST".into(),path:"/notification-templates".into(),body:json!({"name":"not allowed","language":"en-US","subject":"Subject","body":"Body","format":"text"}),..Default::default()
    },agent.id.into()).await.unwrap();
    assert_eq!(result["status"], 403);
    let mut revoked = workspace_roles::Entity::find_by_id(role_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    revoked.permissions = json!(["notifications.read"]).to_string();
    revoked
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let denied = delegated_future(&s, admin(1), agent.id.into(), async {
        let scope = Scope::for_actor(&s, 1.into(), 1.into()).await?;
        scope.require("notifications.send")
    })
    .await
    .unwrap_err();
    assert_eq!(denied.0, axum::http::StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn role_intersection_each_read_resource_and_workspace_data_isolation() {
    let (_dir, s) = setup().await;
    let grants = vec![
        "hosts.read",
        "repositories.read",
        "code-insights.read",
        "pipelines.read",
        "workflows.read",
        "app-templates.read",
        "deployments.read",
        "credentials.read",
        "settings.read",
        "dictionaries.read",
    ];
    let id = role(&s, 1, grants.clone()).await;
    let agent = agent(&s, id).await;
    for (permission, path) in [
        ("hosts.read", "/nodes"),
        ("repositories.read", "/code-connections"),
        ("code-insights.read", "/code-insights"),
        ("pipelines.read", "/pipelines"),
        ("workflows.read", "/workflows"),
        ("app-templates.read", "/app-templates"),
        ("deployments.read", "/deployments"),
        ("credentials.read", "/credentials"),
        ("settings.read", "/settings"),
        ("dictionaries.read", "/dictionaries"),
    ] {
        let scope = authorized_scope(&s, admin(1), &agent).await.unwrap();
        let result = dispatch(
            &s,
            scope,
            Call {
                method: "GET".into(),
                path: format!("{path}?page_size=1"),
                body: Value::Null,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200, "{permission} {path}: {result}");
        let mut user = admin(1);
        user.permissions.remove(permission);
        let scope = authorized_scope(&s, user, &agent).await.unwrap();
        let result = dispatch(
            &s,
            scope,
            Call {
                method: "GET".into(),
                path: path.into(),
                body: Value::Null,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result["status"], 403,
            "removing user {permission}: {result}"
        );
        let mut role = workspace_roles::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        role.permissions = json!(
            grants
                .iter()
                .filter(|v| **v != permission)
                .collect::<Vec<_>>()
        )
        .to_string();
        role.clone()
            .into_active_model()
            .reset_all()
            .update(&s.db)
            .await
            .unwrap();
        let scope = authorized_scope(&s, admin(1), &agent).await.unwrap();
        let result = dispatch(
            &s,
            scope,
            Call {
                method: "GET".into(),
                path: path.into(),
                body: Value::Null,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result["status"], 403,
            "removing Agent {permission}: {result}"
        );
        role.permissions = json!(grants).to_string();
        role.into_active_model()
            .reset_all()
            .update(&s.db)
            .await
            .unwrap();
    }
    let scope = authorized_scope(&s, admin(1), &agent).await.unwrap();
    let result = dispatch(
        &s,
        scope.clone(),
        Call {
            method: "GET".into(),
            path: "/nodes/2".into(),
            body: Value::Null,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result["status"], 404, "must not expose workspace 2 host");
    assert!(authorized_scope(&s, admin(2), &agent).await.is_err());
    let mut elevated = admin(1);
    elevated.user.is_superadmin = true;
    assert!(
        !authorized_scope(&s, elevated, &agent)
            .await
            .unwrap()
            .user
            .is_superadmin
    );
    workspace_roles::Entity::delete_by_id(id)
        .exec(&s.db)
        .await
        .unwrap();
    assert!(authorized_scope(&s, admin(1), &agent).await.is_err());
}

#[tokio::test]
async fn no_implicit_access_and_normal_write_validation_and_audit() {
    let (_dir, s) = setup().await;
    let row = agent(&s, Id::default()).await;
    assert!(authorized_scope(&s, admin(1), &row).await.is_err());
    let mut scope = admin(1);
    scope.permissions.clear();
    let request = || Call {
        method: "POST".into(),
        path: "/node-groups".into(),
        body: json!({"name":"Agent created group","parent_id":"0","position":0}),
        ..Default::default()
    };
    let denied = dispatch(&s, scope, request()).await.unwrap();
    assert_eq!(denied["status"], 403);
    let allowed = dispatch(&s, admin(1), request()).await.unwrap();
    assert_eq!(allowed["status"], 201, "{allowed}");
    let id = allowed["data"]["id"].as_str().unwrap();
    let wrong = dispatch(
        &s,
        admin(2),
        Call {
            method: "GET".into(),
            path: format!("/node-groups/{id}"),
            body: Value::Null,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(wrong["status"], 404);
}

#[test]
fn transport_cannot_choose_identity_headers_or_an_external_target() {
    for path in [
        "https://example.invalid/",
        "//example.invalid/",
        "/../2/nodes",
        "/api/workspaces/2/nodes",
        "/nodes%2f..%2f..%2f2",
        "/nodes\\..",
        "/nodes#fragment",
    ] {
        assert!(
            Call {
                method: "GET".into(),
                path: path.into(),
                body: Value::Null,
                ..Default::default()
            }
            .validate()
            .is_err(),
            "{path}"
        );
    }
    assert!(
        serde_json::from_value::<Call>(json!({"method":"GET","path":"/nodes","workspace_id":"2"}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Call>(
            json!({"method":"GET","path":"/nodes","headers":{"Cookie":"x"}})
        )
        .is_err()
    );
    let mut value = json!({"password":"not-for-model","nested":{"api_key":"hidden"},"total":3});
    redact(&mut value);
    assert_eq!(value["password"], "[redacted]");
    assert_eq!(value["nested"]["api_key"], "[redacted]");
    assert_eq!(value["total"], 3);
    let mut enrollment = json!({"token":"private-enrollment-token","command":"runner --token private-enrollment-token","nested":["private-enrollment-token"]});
    redact(&mut enrollment);
    assert!(!enrollment.to_string().contains("private-enrollment-token"));
    assert_eq!(enrollment["command"], "runner --token [redacted]");
    assert!(
        Call {
            method: "GET".into(),
            path: "/code-connections/1/branches?url=https%3A%2F%2Fexample.invalid%2Fsource.git"
                .into(),
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[tokio::test]
async fn registered_read_operations_never_regain_invokers_full_role() {
    let (_dir, s) = setup().await;
    let catalogue: Vec<Value> =
        serde_json::from_str(include_str!("../platform_tools_catalog.json")).unwrap();
    let mut scope = admin(1);
    scope.permissions.clear();
    let mut checked = 0;
    for operation in catalogue
        .iter()
        .filter(|op| op["method"] == "GET" && op["permission"].is_string())
    {
        let path = operation["path"].as_str().unwrap();
        let mut path = path
            .split('/')
            .map(|segment| {
                if segment.starts_with('{') {
                    "1"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        if path.ends_with("/branches") {
            path.push_str("?url=https%3A%2F%2Fexample.invalid%2Fsource.git");
        }
        let response = dispatch(
            &s,
            scope.clone(),
            Call {
                method: "GET".into(),
                path,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(response["status"], 403, "{}: {response}", operation["path"]);
        checked += 1;
    }
    assert!(
        checked >= 50,
        "catalogue coverage unexpectedly shrank: {checked}"
    );
    // These handlers reauthorize inside transactions rather than calling require first.
    for path in ["/pipelines/1", "/workflows/1?version=1"] {
        let response = dispatch(
            &s,
            scope.clone(),
            Call {
                method: "DELETE".into(),
                path: path.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(response["status"], 403, "{path}: {response}");
    }
}

#[tokio::test]
async fn same_account_has_independent_space_memberships_and_agent_grants() {
    use crate::entities::workspace_members;
    let (_dir, s) = setup().await;
    let mut limited = workspace_roles::Entity::find_by_id(2)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    limited.permissions = json!(["hosts.read"]).to_string();
    limited
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    workspace_members::Entity::insert(workspace_members::ActiveModel {
        id: sea_orm::Set(3),
        workspace_id: sea_orm::Set(2),
        user_id: sea_orm::Set(1),
        role_id: sea_orm::Set(2),
        state: sea_orm::Set("active".into()),
        created_at: sea_orm::Set(0),
        updated_at: sea_orm::Set(0),
        version: sea_orm::Set(1),
    })
    .exec(&s.db)
    .await
    .unwrap();
    let first = Scope::for_actor(&s, 1.into(), 1.into()).await.unwrap();
    let second = Scope::for_actor(&s, 2.into(), 1.into()).await.unwrap();
    assert!(first.permissions.contains("pipelines.read"));
    assert!(!second.permissions.contains("pipelines.read"));
    let role = role(&s, 1, vec!["hosts.read", "pipelines.read"]).await;
    let row = agent(&s, role).await;
    assert!(authorized_scope(&s, second.clone(), &row).await.is_err());
    let response = dispatch(
        &s,
        second,
        Call {
            method: "GET".into(),
            path: "/pipelines".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(response["status"], 403);
    assert!(Scope::for_actor(&s, 1.into(), 2.into()).await.is_err());
}

#[tokio::test]
async fn transactional_refresh_rechecks_agent_role_after_admission() {
    let (_dir, s) = setup().await;
    let id = role(&s, 1, vec!["pipelines.read", "pipelines.delete"]).await;
    let agent = agent(&s, id).await;
    let admitted = authorized_scope(&s, admin(1), &agent).await.unwrap();
    assert!(admitted.permissions.contains("pipelines.delete"));
    let mut role = workspace_roles::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    role.permissions = json!(["pipelines.read"]).to_string();
    role.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let response = dispatch_as_agent(
        &s,
        admitted,
        Call {
            method: "DELETE".into(),
            path: "/pipelines/1".into(),
            ..Default::default()
        },
        agent.id.into(),
    )
    .await
    .unwrap();
    assert_eq!(
        response["status"], 403,
        "transaction must not retain the revoked Agent grant: {response}"
    );
}
