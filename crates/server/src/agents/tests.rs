use super::*;
use sea_orm::{QueryOrder, Set};
use std::sync::Arc;

const TEST_RUNTIMES: [&str; 11] = [
    "codex",
    "claude",
    "gemini",
    "qwen",
    "opencode",
    "pi",
    "dsh",
    "kimi",
    "grok",
    "copilot",
    "codebuddy",
];

/// The seeded native CLI model for `runtime` in `workspace` (see `setup`).
pub(crate) fn test_cli_model(workspace: i64, runtime: &str) -> i64 {
    let index = TEST_RUNTIMES
        .iter()
        .position(|r| *r == runtime)
        .expect("test runtime") as i64;
    7_000_000 + workspace * 100 + index
}

pub(crate) async fn setup() -> (tempfile::TempDir, AppState) {
    let (dir, db) = crate::lock::tests::database().await;
    let s = Arc::new(crate::state::State {
        ids: Arc::new(
            crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std()).unwrap(),
        ),
        db,
        data_dir: dir.path().into(),
        instance_id: "agents-test".into(),
        server_mode: false,
        secure_cookie: false,
        bind: "127.0.0.1:0".parse().unwrap(),
        vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
        host_monitor: Default::default(),
        monitor: Default::default(),
        cluster: None,
    });
    crate::entities::platform_state::Entity::update_many()
        .col_expr(crate::entities::platform_state::Column::AccessConfig,
            sea_orm::sea_query::Expr::value(json!({"public_url":"http://console.localhost", "preview_domain":"preview.localhost"}).to_string()))
        .exec(&s.db).await.unwrap();
    for id in [1, 2] {
        crate::entities::workspaces::Entity::insert(crate::entities::workspaces::ActiveModel {
            id: Set(id),
            slug: Set(format!("w{id}")),
            name: Set(format!("Workspace {id}")),
            owner_id: Set(id),
            join_code: Set(format!("join{id}")),
            pipeline_run_number: Set(0),
            workflow_execution_number: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        users::Entity::insert(
            users::Model {
                id,
                email: format!("user{id}@example.invalid"),
                name: format!("Owner {id}"),
                password_hash: "test".into(),
                last_login_at: None,
                last_login_ip: None,
                is_superadmin: 0,
                version: 1,
                active: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        workspace_members::Entity::insert(workspace_members::ActiveModel {
            id: Set(id),
            workspace_id: Set(id),
            user_id: Set(id),
            role_id: Set(id),
            state: Set("active".into()),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
        })
        .exec(&s.db)
        .await
        .unwrap();
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(id),
            workspace_id: Set(id),
            name: Set(format!("Host {id}")),
            address: Set("127.0.0.1".into()),
            port: Set(22),
            username: Set("test".into()),
            connection: Set("runner".into()),
            state: Set("offline".into()),
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
            os: Set(String::new()),
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
    // Agents run only through a configured model: every workspace gets one CLI
    // model per CLI that keeps that CLI's own configuration (no custom API).
    for workspace in [1, 2] {
        for runtime in TEST_RUNTIMES {
            crate::entities::ai_model_profiles::Entity::insert(
                crate::entities::ai_model_profiles::Model {
                    id: test_cli_model(workspace, runtime),
                    workspace_id: workspace,
                    name: format!("{runtime} (native)"),
                    kind: "cli".into(),
                    runtime: runtime.into(),
                    provider_id: 0,
                    models: "[]".into(),
                    default_model: String::new(),
                    tool_version: String::new(),
                    state: "enabled".into(),
                    cli_options: None,
                    hosts: "[]".into(),
                    timezone: String::new(),
                    revision_id: 0,
                    version: 1,
                    created_at: 0,
                    updated_at: 0,
                }
                .into_active_model(),
            )
            .exec(&s.db)
            .await
            .unwrap();
        }
    }
    (dir, s)
}
pub(crate) fn admin(workspace: i64) -> Scope {
    Scope {
        workspace_id: workspace.into(),
        user: crate::accounts::auth::User {
            id: workspace.into(),
            email: "test@example.invalid".into(),
            name: "Owner".into(),
            is_superadmin: false,
        },
        permissions: crate::accounts::workspace::PERMISSIONS
            .iter()
            .map(|p| p.0.into())
            .collect(),
    }
}
fn body() -> Value {
    json!({"name":"Agent A","role":"Code review","description":"User text","runtime":"codex","model_profile_id":test_cli_model(1,"codex").to_string(),"model":"","host_id":"1","owner_id":"1","cwd":if cfg!(windows){"C:/agent-tests"}else{"/tmp/agent-tests"},"system_prompt":"Original instructions","permission":"read_only","visibility":"owner","state":"enabled"})
}
fn parse(v: Value) -> Body {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn a_legacy_host_pin_is_shown_and_removed_with_host_administration_only() {
    let (_dir, s) = setup().await;
    let mut input = body();
    input["permission"] = json!("on_request");
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(parse(input)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(before) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(before["host_id"], "1");
    assert_eq!(before["host_name"], "Host 1");
    let version = before["version"].as_i64().unwrap();
    let unpinning = |scope: Scope, version: i64| {
        unpin(
            State(s.clone()),
            scope,
            Path(("1".into(), id)),
            Query(Unpin { version }),
        )
    };
    let mut no_hosts = admin(1);
    no_hosts.permissions.remove("hosts.update");
    assert_eq!(
        unpinning(no_hosts, version).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    assert!(unpinning(admin(1), version - 1).await.is_err());
    unpinning(admin(1), version).await.unwrap();
    let Json(after) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(after["host_id"], "0");
    assert_eq!(after["cwd"], "");
    // Everything else is kept in a new revision; the permission belonged to the
    // pinned directory and returns to the default, as for any unpinned Agent.
    for field in [
        "name",
        "role",
        "system_prompt",
        "visibility",
        "model_profile_id",
    ] {
        assert_eq!(after[field], before[field], "{field}");
    }
    assert_eq!(after["permission"], "read_only");
    assert_ne!(after["revision_id"], before["revision_id"]);
    // Removing it again changes nothing.
    let version = after["version"].as_i64().unwrap();
    unpinning(admin(1), version).await.unwrap();
    let Json(again) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(again["revision_id"], after["revision_id"]);
}

#[tokio::test]
async fn saving_without_a_host_keeps_a_legacy_pin_until_it_is_removed() {
    let (_dir, s) = setup().await;
    let mut input = body();
    input["permission"] = json!("on_request");
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(parse(input)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let current = || async {
        detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap()
            .0
    };
    let pinned = current().await;
    // What the Agent form sends: no host, directory or permission.
    let form = |row: &Value, name: &str| {
        let mut sent = json!({});
        for field in [
            "name",
            "role",
            "description",
            "platform_role_id",
            "owner_id",
            "model_profile_id",
            "model",
            "system_prompt",
            "visibility",
            "state",
            "version",
        ] {
            sent[field] = row[field].clone();
        }
        sent["name"] = json!(name);
        parse(sent)
    };
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(form(&pinned, "Renamed")),
    )
    .await
    .unwrap();
    let saved = current().await;
    assert_eq!(saved["name"], "Renamed");
    for field in ["host_id", "cwd", "permission"] {
        assert_eq!(saved[field], pinned[field], "{field}");
    }
    // Only the explicit removal clears it.
    unpin(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Query(Unpin {
            version: saved["version"].as_i64().unwrap(),
        }),
    )
    .await
    .unwrap();
    let unpinned = current().await;
    assert_eq!(unpinned["host_id"], "0");
    // An unpinned Agent saved from the form stays unpinned, with the default permission.
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(form(&unpinned, "Again")),
    )
    .await
    .unwrap();
    let again = current().await;
    assert_eq!(again["host_id"], "0");
    assert_eq!(again["cwd"], "");
    assert_eq!(again["permission"], "read_only");
}

#[tokio::test]
async fn avatar_update_preserves_omitted_value_and_respects_workspace() {
    let (_dir, s) = setup().await;
    let mut input = body();
    input["avatar"] = json!({"style":"pixel","seed":"chosen"});
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(parse(input)))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(mut current) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(current["avatar"]["seed"], "chosen");
    current.as_object_mut().unwrap().remove("avatar");
    current["name"] = json!("New name");
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(current)),
    )
    .await
    .unwrap();
    let Json(current) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(current["avatar"]["seed"], "chosen");
    assert_eq!(
        detail(State(s), admin(2), Path(("2".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn model_profile_agents_need_no_host_and_resolve_only_the_selected_catalog() {
    use crate::entities::ai_model_profiles as profiles;
    let (_dir, s) = setup().await;
    hosts::Entity::delete_many().exec(&s.db).await.unwrap();
    for (id, workspace, kind, runtime, state) in [
        (901, 1, "cli", "claude", "enabled"),
        (902, 2, "cli", "codex", "enabled"),
        (903, 1, "cli", "codex", "disabled"),
        (904, 1, "api", "", "enabled"),
    ] {
        profiles::Entity::insert(
            profiles::Model {
                id,
                workspace_id: workspace,
                name: format!("Profile {id}"),
                kind: kind.into(),
                runtime: runtime.into(),
                provider_id: 0,
                models: json!(["model-a", "model-b"]).to_string(),
                default_model: "model-a".into(),
                tool_version: String::new(),
                state: state.into(),
                cli_options: None,
                hosts: "[]".into(),
                timezone: String::new(),
                revision_id: 0,
                version: 1,
                created_at: 0,
                updated_at: 0,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    let mut manager = admin(1);
    manager.permissions.retain(|p| !p.starts_with("hosts."));
    let request = json!({"name":"Independent Agent","owner_id":"1","model_profile_id":"901","model":"model-b","system_prompt":"Stable identity","visibility":"workspace","state":"enabled"});
    let (_, Json(created)) = create(
        State(s.clone()),
        manager.clone(),
        Json(parse(request.clone())),
    )
    .await
    .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let row = find(&s.db, &manager, id).await.unwrap();
    assert_eq!(row.host_id, 0);
    assert!(row.cwd.is_empty());
    assert_eq!(row.runtime, "claude");
    assert_eq!(row.model_profile_id, 901);
    assert_eq!(resolve_model(&s.db, &row).await.unwrap().model, "model-b");
    let Json(mut edited) = detail(State(s.clone()), manager.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(edited["model_profile_name"], "Profile 901");
    edited["model"] = json!("");
    update(
        State(s.clone()),
        manager.clone(),
        Path(("1".into(), id)),
        Json(parse(edited)),
    )
    .await
    .unwrap();
    let row = find(&s.db, &manager, id).await.unwrap();
    assert_eq!(resolve_model(&s.db, &row).await.unwrap().model, "model-a");
    for (profile, model) in [("902", ""), ("903", ""), ("901", "invented-model")] {
        let mut invalid = request.clone();
        invalid["model_profile_id"] = json!(profile);
        invalid["model"] = json!(model);
        assert!(
            create(State(s.clone()), manager.clone(), Json(parse(invalid)))
                .await
                .is_err()
        );
    }
    let mut api = request;
    api["model_profile_id"] = json!("904");
    api["model"] = json!("");
    let (_, Json(created)) = create(State(s.clone()), manager, Json(parse(api)))
        .await
        .unwrap();
    let row = agents::Entity::find_by_id(created["id"].as_str().unwrap().parse::<i64>().unwrap())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.host_id, 0);
    assert_eq!(row.runtime, "api");
}

#[tokio::test]
async fn configuration_revision_is_immutable_and_stale_updates_are_refused() {
    let (_dir, s) = setup().await;
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(parse(body())))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(first) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(first["host_name"], "Host 1");
    assert_eq!(first["owner_name"], "Owner 1");
    let mut edited = first.clone();
    edited["system_prompt"] = json!("Changed instructions");
    edited["state"] = json!("disabled");
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(edited.clone())),
    )
    .await
    .unwrap();
    assert_eq!(
        update(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(parse(edited))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let Json(current) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(current["version"], 2);
    assert_ne!(current["revision_id"], first["revision_id"]);
    let revisions = crate::entities::definition_revisions::Entity::find()
        .filter(crate::entities::definition_revisions::Column::SubjectKind.eq("agent"))
        .order_by_asc(crate::entities::definition_revisions::Column::Number)
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&revisions[0].definition).unwrap()["system_prompt"],
        "Original instructions"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&revisions[1].definition).unwrap()["state"],
        "disabled"
    );
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(current)),
    )
    .await
    .unwrap();
    let Json(history) = super::revisions(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Query(HashMap::new()),
    )
    .await
    .unwrap();
    assert_eq!(
        history["total"], 2,
        "no-op save retains the executable revision"
    );
}

#[tokio::test]
async fn workspace_visibility_and_permissions_cannot_be_bypassed() {
    let (_dir, s) = setup().await;
    let (_, Json(created)) = create(State(s.clone()), admin(1), Json(parse(body())))
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    assert_eq!(
        detail(State(s.clone()), admin(2), Path(("2".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let mut reader = admin(1);
    reader.user.id = 2.into();
    reader.permissions = ["ai-agents.read", "ai-agents.use"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let Json(hidden) = list(State(s.clone()), reader.clone(), Query(HashMap::new()))
        .await
        .unwrap();
    assert_eq!(hidden["total"], 0);
    assert_eq!(
        detail(State(s.clone()), reader.clone(), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        super::probe(State(s.clone()), reader.clone(), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create(State(s.clone()), reader.clone(), Json(parse(body())))
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    let Json(mut shared) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    shared["visibility"] = json!("workspace");
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(parse(shared)),
    )
    .await
    .unwrap();
    let Json(visible) = list(State(s.clone()), reader.clone(), Query(HashMap::new()))
        .await
        .unwrap();
    assert_eq!(visible["total"], 1);
    assert!(!reader.permissions.contains("hosts.terminal"));
    for change in [
        json!({"host_id":"2"}),
        json!({"owner_id":"2"}),
        // The CLI comes from the model: none, or another workspace's, is refused.
        json!({"model_profile_id":"0"}),
        json!({"model_profile_id":test_cli_model(2, "codex").to_string()}),
        json!({"cwd":"relative"}),
        json!({"reasoning_effort":"high"}),
        json!({"workspace_id":"2"}),
    ] {
        let mut invalid = body();
        invalid
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(
            create(State(s.clone()), admin(1), Json(parse(invalid)))
                .await
                .is_err()
        );
    }
    let mut manager = admin(1);
    manager.permissions.remove("hosts.update");
    assert_eq!(
        create(State(s), manager, Json(parse(body())))
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
}
