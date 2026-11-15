use super::*;
use crate::agents::tests::{admin, setup};
use sea_orm::PaginatorTrait;

fn body() -> Body {
    serde_json::from_value(json!({"name":"Local CLI","kind":"cli","runtime":"claude","models":[],"default_model":"","tool_version":"","state":"enabled"})).unwrap()
}

#[tokio::test]
async fn inline_keys_rotate_without_leaking_or_changing_ledger_identity_and_rollback_atomically() {
    let (_dir, s) = setup().await;
    crate::entities::workspace_roles::Entity::insert(
        crate::entities::workspace_roles::Model {
            id: 1,
            workspace_id: 1,
            name: "Manager".into(),
            permissions: serde_json::to_string(&admin(1).permissions).unwrap(),
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
    let input = json!({"name":"Direct key","kind":"api","state":"enabled","default_model":"m","connection":{
        "name":"Direct key","api_key":"first-private-token","protocol":"openai","base_url":"https://api.example.invalid/v1","currency":"USD","timezone":"UTC","monthly_limit_micros":0,"qps":1,"concurrency":1,
        "prices":[{"model":"m","operation":"ai.llm.chat","input_micros":0,"output_micros":0,"unit_size":1}]}});
    let created = save(
        &s,
        &admin(1),
        None,
        serde_json::from_value(input.clone()).unwrap(),
    )
    .await
    .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(first) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    let credential: Id =
        serde_json::from_value(first["connection"]["credential_id"].clone()).unwrap();
    assert!(!first.to_string().contains("private-token"));
    assert!(first["connection"].get("api_key").is_none());
    let stored = credentials::Entity::find_by_id(credential)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.secret.contains("first-private-token"));
    assert_eq!(
        crate::credentials::unseal(&s, &1.into(), &credential)
            .await
            .unwrap()
            .2,
        "first-private-token"
    );
    let mut rotated = first.clone();
    rotated["connection"]["api_key"] = json!("second-private-token");
    save(
        &s,
        &admin(1),
        Some(id),
        serde_json::from_value(rotated).unwrap(),
    )
    .await
    .unwrap();
    let Json(second) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(
        second["connection"]["credential_id"],
        first["connection"]["credential_id"]
    );
    assert_eq!(
        crate::credentials::unseal(&s, &1.into(), &credential)
            .await
            .unwrap()
            .2,
        "second-private-token"
    );
    let mut blank = second.clone();
    blank["connection"]["api_key"] = json!("");
    save(
        &s,
        &admin(1),
        Some(id),
        serde_json::from_value(blank).unwrap(),
    )
    .await
    .unwrap();
    let Json(third) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    let mut bad = third.clone();
    bad["connection"]["api_key"] = json!("rollback-private-token");
    bad["default_model"] = json!("--invalid");
    assert!(
        save(
            &s,
            &admin(1),
            Some(id),
            serde_json::from_value(bad).unwrap()
        )
        .await
        .is_err()
    );
    assert_eq!(
        crate::credentials::unseal(&s, &1.into(), &credential)
            .await
            .unwrap()
            .2,
        "second-private-token"
    );
    let count = credentials::Entity::find().count(&s.db).await.unwrap();
    let mut bad = input;
    bad["default_model"] = json!("--invalid");
    assert!(
        save(&s, &admin(1), None, serde_json::from_value(bad).unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        credentials::Entity::find().count(&s.db).await.unwrap(),
        count
    );
    let revisions = crate::entities::definition_revisions::Entity::find()
        .all(&s.db)
        .await
        .unwrap();
    assert!(!format!("{revisions:?}").contains("private-token"));
}

#[tokio::test]
async fn profiles_need_no_runner_and_preserve_revisions_and_workspace_scope() {
    let (_dir, s) = setup().await;
    for id in [1, 2] {
        crate::entities::workspace_roles::Entity::insert(
            crate::entities::workspace_roles::Model {
                id,
                workspace_id: id,
                name: "Model manager".into(),
                permissions: serde_json::to_string(&admin(id).permissions).unwrap(),
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
    let first = save(&s, &admin(1), None, body()).await.unwrap();
    let id: Id = serde_json::from_value(first["id"].clone()).unwrap();
    assert_eq!(first["provider_id"], "0");
    assert!(find(&s.db, &admin(2), id).await.is_err());
    let old_revision = first["revision_id"].clone();
    let mut next: Body = serde_json::from_value(first).unwrap();
    next.name = "Changed".into();
    let second = save(&s, &admin(1), Some(id), next.clone()).await.unwrap();
    assert_eq!(second["version"], 2);
    assert_ne!(second["revision_id"], old_revision);
    assert_eq!(
        save(&s, &admin(1), Some(id), next).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    let revisions = crate::entities::definition_revisions::Entity::find()
        .filter(crate::entities::definition_revisions::Column::SubjectKind.eq("model"))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 2);
    let mut denied = admin(1);
    denied.permissions.remove("ai-models.create");
    assert_eq!(
        save(&s, &denied, None, body()).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn catalog_accepts_manual_models_but_rejects_bad_runtime_and_cross_workspace_provider() {
    let (_dir, s) = setup().await;
    let mut b = body();
    b.default_model = " manual-model ".into();
    b.models = vec!["manual-model".into(), "manual-model".into()];
    b.validate(&s.db, &admin(1)).await.unwrap();
    assert_eq!(b.models, vec!["manual-model"]);
    b.runtime = "unregistered-shell".into();
    assert!(b.validate(&s.db, &admin(1)).await.is_err());
    let mut b = body();
    b.kind = "api".into();
    b.runtime.clear();
    b.default_model = "m".into();
    assert!(b.validate(&s.db, &admin(1)).await.is_err());
    let mut b = body();
    b.provider_id = "999".into();
    assert!(b.validate(&s.db, &admin(1)).await.is_err());
    let mut b = body();
    b.models = vec!["--execute".into()];
    assert!(b.validate(&s.db, &admin(1)).await.is_err());
}

#[tokio::test]
async fn api_connection_saves_without_metering_fields() {
    let (_dir, s) = setup().await;
    crate::entities::workspace_roles::Entity::insert(
        crate::entities::workspace_roles::Model {
            id: 1,
            workspace_id: 1,
            name: "Manager".into(),
            permissions: serde_json::to_string(&admin(1).permissions).unwrap(),
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
    let input = json!({"name":"Unmetered","kind":"api","state":"enabled","default_model":"m","models":["m"],"connection":{
        "name":"Unmetered","api_key":"unmetered-token","protocol":"openai","base_url":"https://api.example.invalid/v1","qps":1,"concurrency":1}});
    // Without a default model the API model is refused with a message naming the
    // field, and its connection is not left behind.
    let mut missing = input.clone();
    missing["default_model"] = json!("");
    missing["models"] = json!([]);
    let error = save(
        &s,
        &admin(1),
        None,
        serde_json::from_value(missing).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:?}").contains("defaultModelRequired"));
    assert_eq!(providers::Entity::find().count(&s.db).await.unwrap(), 0);
    let created = save(&s, &admin(1), None, serde_json::from_value(input).unwrap())
        .await
        .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(row) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(row["connection"]["prices"], json!([]));
    assert_eq!(row["connection"]["currency"], "USD");
    assert_eq!(row["connection"]["timezone"], "UTC");
    assert_eq!(row["connection"]["monthly_limit_micros"], 0);
}

async fn manager(s: &AppState) {
    crate::entities::workspace_roles::Entity::insert(
        crate::entities::workspace_roles::Model {
            id: 1,
            workspace_id: 1,
            name: "Manager".into(),
            permissions: serde_json::to_string(&admin(1).permissions).unwrap(),
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

fn cli_body(runtime: &str, options: Value, connection: Value) -> Body {
    serde_json::from_value(
        json!({"name":"CLI","kind":"cli","runtime":runtime,"state":"enabled",
        "models":["m"],"default_model":"m","cli_options":options,"connection":connection}),
    )
    .unwrap()
}

#[tokio::test]
async fn cli_custom_api_is_its_own_connection_and_reaches_the_runner_request() {
    let (_dir, s) = setup().await;
    manager(&s).await;
    let connection = json!({"name":"x","api_key":"cli-private-key","protocol":"openai",
        "base_url":"https://gw.example.invalid/anthropic","qps":1,"concurrency":1});
    // A format the CLI cannot call directly is refused; so is any custom API for login-only CLIs.
    for (runtime, options) in [
        ("claude", json!({"format":"openai_chat"})),
        ("copilot", json!({"format":"openai_chat"})),
        ("claude", json!({"format":"anthropic","x":{"nested":true}})),
    ] {
        assert!(
            save(
                &s,
                &admin(1),
                None,
                cli_body(runtime, options, connection.clone())
            )
            .await
            .is_err(),
            "{runtime}"
        );
    }
    let created = save(
        &s,
        &admin(1),
        None,
        cli_body(
            "claude",
            json!({"format":"anthropic","opus_model":"big"}),
            connection,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        created["cli_options"],
        json!({"format":"anthropic","opus_model":"big"})
    );
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(detail) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    // Anthropic format stores an Anthropic connection; the key never comes back.
    assert_eq!(detail["connection"]["protocol"], "anthropic");
    assert!(!detail.to_string().contains("cli-private-key"));
    crate::entities::ai_agents::Entity::insert(
        crate::entities::ai_agents::Model {
            id: 777,
            model_profile_id: i64::from(id),
            workspace_id: 1,
            name: "a".into(),
            avatar: None,
            role: String::new(),
            description: String::new(),
            runtime: "claude".into(),
            model: "m".into(),
            host_id: 0,
            owner_id: 1,
            cwd: String::new(),
            system_prompt: String::new(),
            permission: "on_request".into(),
            visibility: "workspace".into(),
            state: "enabled".into(),
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
    let provider = cli_provider(&s, 1.into(), 777, ferry_protocol::agent::Runtime::Claude)
        .await
        .unwrap()
        .expect("custom provider");
    assert_eq!(provider.api_key, "cli-private-key");
    assert_eq!(provider.format, "anthropic");
    assert_eq!(provider.base_url, "https://gw.example.invalid/anthropic");
    assert_eq!(provider.options["opus_model"], "big");
    assert!(!format!("{provider:?}").contains("cli-private-key"));
    // A conversation whose CLI no longer matches the Agent's model is refused,
    // never run on the host's own configuration.
    let error = cli_provider(&s, 1.into(), 777, ferry_protocol::agent::Runtime::Codex)
        .await
        .unwrap_err();
    assert_eq!(error.2.unwrap().key, "agents.modelRequired");
}

#[tokio::test]
async fn editing_a_cli_that_shares_an_api_connection_splits_it_instead_of_changing_the_api_model() {
    let (_dir, s) = setup().await;
    manager(&s).await;
    let api = save(
        &s,
        &admin(1),
        None,
        serde_json::from_value(json!({"name":"API","kind":"api","state":"enabled","default_model":"m","connection":{
            "name":"API","api_key":"api-key","protocol":"openai","base_url":"https://api.example.invalid/v1","qps":1,"concurrency":1}}))
        .unwrap(),
    )
    .await
    .unwrap();
    let shared = api["provider_id"].as_str().unwrap().to_owned();
    // A profile saved by the previous release: CLI referencing the API model's connection.
    let legacy = save(
        &s,
        &admin(1),
        None,
        serde_json::from_value(
            json!({"name":"Old CLI","kind":"cli","runtime":"codex","state":"enabled",
            "provider_id":shared,"models":["m"],"default_model":"m"}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(legacy["cli_options"], json!({"format":"openai_responses"}));
    let id: Id = serde_json::from_value(legacy["id"].clone()).unwrap();
    let mut edit = cli_body(
        "codex",
        json!({"format":"openai_responses"}),
        json!({"name":"Old CLI","protocol":"openai","base_url":"https://changed.example.invalid/v1","qps":1,"concurrency":1,"credential_id":"0","version":1}),
    );
    edit.provider_id = shared.clone();
    edit.version = 1;
    let error = save(&s, &admin(1), Some(id), edit).await.err().unwrap();
    assert!(format!("{error:?}").contains("sharedConnectionNeedsKey"));
    // Referencing the shared OpenAI connection with an Anthropic format would need conversion.
    let mut wrong = cli_body("claude", json!({"format":"anthropic"}), Value::Null);
    wrong.connection = None;
    wrong.provider_id = shared.clone();
    assert!(save(&s, &admin(1), None, wrong).await.is_err());
    // A CLI may reference the API model's connection as is.
    let mut linked = cli_body("codex", json!({"format":"openai_responses"}), Value::Null);
    linked.connection = None;
    linked.provider_id = shared.clone();
    let linked = save(&s, &admin(1), None, linked).await.unwrap();
    assert_eq!(linked["provider_id"], json!(shared));
    let mut edit = cli_body(
        "codex",
        json!({"format":"openai_responses"}),
        json!({"name":"Old CLI","api_key":"cli-key","protocol":"openai","base_url":"https://changed.example.invalid/v1","qps":1,"concurrency":1,"credential_id":"0","version":1}),
    );
    edit.provider_id = shared.clone();
    edit.version = 1;
    let saved = save(&s, &admin(1), Some(id), edit).await.unwrap();
    assert_ne!(saved["provider_id"], json!(shared));
    let original = providers::Entity::find_by_id(shared.parse::<i64>().unwrap())
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.base_url, "https://api.example.invalid/v1");
}

#[tokio::test]
async fn a_cli_model_time_zone_is_validated_and_api_models_cannot_set_one() {
    let (_dir, s) = setup().await;
    let mut b = body();
    b.timezone = " Asia/Shanghai ".into();
    b.validate(&s.db, &admin(1)).await.unwrap();
    assert_eq!(b.timezone, "Asia/Shanghai");
    assert_eq!(
        serde_json::to_value(&b).unwrap()["timezone"],
        "Asia/Shanghai"
    );
    // An unset zone stays out of the recorded definition.
    assert!(
        serde_json::to_value(body())
            .unwrap()
            .get("timezone")
            .is_none()
    );
    let mut b = body();
    b.timezone = "Mars/Olympus".into();
    let error = b.validate(&s.db, &admin(1)).await.unwrap_err();
    assert!(format!("{error:?}").contains("runner.timezoneInvalid"));
    let mut b = body();
    b.kind = "api".into();
    b.runtime.clear();
    b.default_model = "m".into();
    b.timezone = "UTC".into();
    let error = b.validate(&s.db, &admin(1)).await.unwrap_err();
    assert!(format!("{error:?}").contains("models.timezoneCliOnly"));
}

fn host(os: &str, detected: &str, supported: bool) -> crate::entities::hosts::Model {
    crate::entities::hosts::Model {
        id: 1,
        workspace_id: 1,
        name: "h".into(),
        address: String::new(),
        port: 0,
        username: String::new(),
        connection: "runner".into(),
        state: "online".into(),
        group_id: 0,
        description: String::new(),
        host_key: String::new(),
        runner_key_hash: String::new(),
        enroll_token: String::new(),
        enroll_token_hash: String::new(),
        jump_host_id: 0,
        seen_at: 0,
        created_at: 0,
        updated_at: 0,
        version: 1,
        os: os.into(),
        arch: "x86_64".into(),
        agent_version: String::new(),
        agent_update: None,
        runner_auto_update: "inherit".into(),
        metrics: String::new(),
        credential_id: 0,
        timezone: String::new(),
        detected_timezone: detected.into(),
        timezone_supported: i64::from(supported),
    }
}

#[test]
fn a_required_zone_refuses_runners_that_cannot_apply_it() {
    assert_eq!(
        timezone_for(String::new(), &host("linux", "", false)).unwrap(),
        None
    );
    let error = timezone_for("Asia/Tokyo".into(), &host("linux", "", false)).unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert!(format!("{error:?}").contains("runner.timezoneUpgrade"));
    assert_eq!(
        timezone_for("Asia/Tokyo".into(), &host("linux", "UTC", true))
            .unwrap()
            .as_deref(),
        Some("Asia/Tokyo")
    );
    assert!(windows_timezone_caveat(
        "Asia/Tokyo",
        &host("windows", "Asia/Shanghai", true)
    ));
    assert!(!windows_timezone_caveat(
        "Asia/Tokyo",
        &host("windows", "Asia/Tokyo", true)
    ));
    assert!(!windows_timezone_caveat(
        "Asia/Tokyo",
        &host("linux", "UTC", true)
    ));
}
