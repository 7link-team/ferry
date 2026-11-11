//! Workspace market merges platform and own entries; installation copies packages with config files.
use super::*;
use super::catalog::Install;
use crate::entities::{app_templates, config_templates};
use crate::i18n::{Locale, with_locale};
use sea_orm::{PaginatorTrait, TransactionTrait};

fn app(file_ref: Option<&str>) -> Value {
    let mut service = json!({"name":"web","image":"busybox:1.36"});
    if let Some(name) = file_ref {
        service["files"] = json!([{"path":"/etc/web.conf","fromTemplate":name}]);
    }
    json!({"apiVersion":"ferry/v2","kind":"Application","metadata":{"name":"web"},"services":[service]})
}
fn bundle(key: &str, with_file: bool, kind: &str) -> String {
    let mut version = json!({"version":"1.0.0","blueprint":app(with_file.then_some("redis-start"))});
    if with_file {
        version["files"] = json!([{"name":"redis-start","content":"echo start"}]);
    }
    json!({"schemaVersion":2,"packages":[{"key":key,"kind":kind,"meta":{"name":{"en-US":"Website","zh-CN":"网站"},"category":"web","topology":"single","drivers":["docker"]},"versions":[version]}]}).to_string()
}
async fn source(s: &AppState, workspace: i64, trust: &str, enabled: bool, text: &str) -> i64 {
    let id: i64 = s.next_id().unwrap().into();
    sources::Entity::insert(sources::ActiveModel {
        id: Set(id),
        workspace_id: Set(workspace),
        name: Set(format!("{trust} source")),
        repository_id: Set(0),
        config: Set("{}".into()),
        state: Set("idle".into()),
        last_error: Set(String::new()),
        synced_at: Set(0),
        run_as: Set(1),
        version: Set(1),
        created_at: Set(1),
        updated_at: Set(1),
        trust: Set(trust.into()),
        enabled: Set(i64::from(enabled)),
        ..Default::default()
    })
    .exec(&s.db)
    .await
    .unwrap();
    let rows = index::parse_bundle(text, &semver::Version::new(0, 3, 0), "en-US").unwrap();
    let tx = s.db.begin().await.unwrap();
    store::replace_entries(s, &tx, workspace, id, 1, rows, std::time::Instant::now() + Duration::from_secs(20)).await.unwrap();
    tx.commit().await.unwrap();
    id
}
async fn items(s: &AppState, workspace: i64) -> Vec<Value> {
    let Json(v) = list(State(s.clone()), crate::agents::tests::admin(workspace), Query(HashMap::new())).await.unwrap();
    v["items"].as_array().unwrap().clone()
}
async fn entry_of(s: &AppState, source: i64) -> Id {
    entries::Entity::find().filter(entries::Column::SourceId.eq(source)).one(&s.db).await.unwrap().unwrap().id.into()
}
async fn install_entry(s: &AppState, scope: Scope, id: Id) -> Result<Value> {
    install(State(s.clone()), scope, Path(("w".into(), id)), Json(Install { version: 1 })).await.map(|(_, Json(v))| v)
}

#[tokio::test]
async fn workspace_market_lists_platform_and_own_entries() {
    let (_dir, s) = crate::agents::tests::setup().await;
    source(&s, 0, "official", true, &bundle("web", false, "app")).await;
    source(&s, 1, "workspace", true, &bundle("own", false, "app")).await;
    let mine = items(&s, 1).await;
    assert_eq!(mine.len(), 2);
    let mut trusts: Vec<_> = mine.iter().map(|i| i["trust"].as_str().unwrap().to_owned()).collect();
    trusts.sort();
    assert_eq!(trusts, ["official", "workspace"]);
    assert!(mine.iter().all(|i| i["package_version"] == "1.0.0" && i["compatible"] == true));
    let other = items(&s, 2).await;
    assert_eq!(other.len(), 1);
    assert_eq!(other[0]["trust"], "official");
}

#[tokio::test]
async fn disabled_platform_source_hidden_and_install_rejected() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "official", true, &bundle("web", false, "app")).await;
    let entry = entry_of(&s, platform).await;
    let installed = install_entry(&s, crate::agents::tests::admin(1), entry).await.unwrap();
    sources::Entity::update_many().col_expr(sources::Column::Enabled, Expr::value(0)).filter(sources::Column::Id.eq(platform)).exec(&s.db).await.unwrap();
    assert!(items(&s, 1).await.is_empty());
    assert_eq!(get(State(s.clone()), crate::agents::tests::admin(1), Path(("w".into(), entry))).await.err().unwrap().0, StatusCode::NOT_FOUND);
    assert_eq!(install_entry(&s, crate::agents::tests::admin(1), entry).await.err().unwrap().0, StatusCode::NOT_FOUND);
    let kept: Id = serde_json::from_value(installed["id"].clone()).unwrap();
    assert!(app_templates::Entity::find_by_id(i64::from(kept)).one(&s.db).await.unwrap().is_some());
}

#[tokio::test]
async fn same_key_in_platform_and_workspace_sources_are_distinct() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "official", true, &bundle("web", false, "app")).await;
    let own = source(&s, 1, "workspace", true, &bundle("web", false, "app")).await;
    assert_eq!(items(&s, 1).await.len(), 2);
    let a = install_entry(&s, crate::agents::tests::admin(1), entry_of(&s, platform).await).await.unwrap();
    let b = install_entry(&s, crate::agents::tests::admin(1), entry_of(&s, own).await).await.unwrap();
    assert_ne!(a["id"], b["id"]);
    let origins: Vec<i64> = app_templates::Entity::find().filter(app_templates::Column::OriginKey.eq("web")).all(&s.db).await.unwrap().iter().map(|r| r.origin_source_id).collect();
    assert!(origins.contains(&platform) && origins.contains(&own));
}

#[tokio::test]
async fn install_renames_conflicting_config_templates() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "official", true, &bundle("redis", true, "app")).await;
    let existing = serde_json::from_value(json!({"name":"redis-start","content":"echo mine","parameters":"[]"})).unwrap();
    crate::templates::configs::create(State(s.clone()), crate::agents::tests::admin(1), Json(existing)).await.unwrap();
    let created = install_entry(&s, crate::agents::tests::admin(1), entry_of(&s, platform).await).await.unwrap();
    let config = config_templates::Entity::find().filter(config_templates::Column::WorkspaceId.eq(1)).filter(config_templates::Column::Name.eq("redis-start-2")).one(&s.db).await.unwrap().unwrap();
    assert_eq!(config.content, "echo start");
    assert_eq!(config.origin_key, "redis");
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let template = app_templates::Entity::find_by_id(i64::from(id)).one(&s.db).await.unwrap().unwrap();
    let content: Value = serde_json::from_str(&template.content).unwrap();
    assert_eq!(content["services"][0]["files"][0]["fromTemplate"], Id::from(config.id).to_string());
    assert_eq!(template.origin_source_id, platform);
    assert_eq!(template.origin_key, "redis");
    assert_eq!(template.origin_version, "1.0.0");
    assert_eq!(template.origin_digest.len(), 64);
}

#[tokio::test]
async fn install_rewrites_blueprint_role_file_references() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let blueprint = json!({"apiVersion":"ferry/blueprint-v1","kind":"Blueprint","metadata":{"name":"mq"},
        "roles":[{"name":"nats","instances":{"min":1,"max":3,"default":1},"identity":"stable",
            "service":{"image":"nats:2","files":[{"path":"/etc/nats.conf","fromTemplate":"nats.conf"}]}}]});
    let text = json!({"schemaVersion":2,"packages":[{"key":"mq","kind":"app",
        "meta":{"name":{"en-US":"MQ"},"category":"messaging","topology":"cluster","drivers":["docker"]},
        "versions":[{"version":"1.0.0","blueprint":blueprint,
            "files":[{"name":"nats.conf","content":"server_name: {{ .Instance.Name }}"}]}]}]}).to_string();
    let platform = source(&s, 0, "official", true, &text).await;
    let existing = serde_json::from_value(json!({"name":"nats.conf","content":"mine","parameters":"[]"})).unwrap();
    crate::templates::configs::create(State(s.clone()), crate::agents::tests::admin(1), Json(existing)).await.unwrap();
    let created = install_entry(&s, crate::agents::tests::admin(1), entry_of(&s, platform).await).await.unwrap();
    let config = config_templates::Entity::find().filter(config_templates::Column::WorkspaceId.eq(1)).filter(config_templates::Column::Name.eq("nats.conf-2")).one(&s.db).await.unwrap().unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let template = app_templates::Entity::find_by_id(i64::from(id)).one(&s.db).await.unwrap().unwrap();
    let content: Value = serde_json::from_str(&template.content).unwrap();
    assert_eq!(content["roles"][0]["service"]["files"][0]["fromTemplate"], Id::from(config.id).to_string());
}

#[tokio::test]
async fn install_incompatible_entry_rejected() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "community", true, &bundle("build", false, "pipeline")).await;
    let listed = items(&s, 1).await;
    assert_eq!(listed[0]["compatible"], false);
    assert!(!listed[0]["incompatible_reason"].as_str().unwrap().is_empty());
    assert_eq!(install_entry(&s, crate::agents::tests::admin(1), entry_of(&s, platform).await).await.err().unwrap().0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn install_requires_config_create_permission_when_files_present() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "official", true, &bundle("redis", true, "app")).await;
    let mut scope = crate::agents::tests::admin(1);
    scope.permissions = ["marketplace.read", "marketplace.install", "app-templates.read", "app-templates.create"].into_iter().map(Into::into).collect();
    assert_eq!(install_entry(&s, scope, entry_of(&s, platform).await).await.err().unwrap().0, StatusCode::FORBIDDEN);
    assert_eq!(app_templates::Entity::find().filter(app_templates::Column::OriginKey.eq("redis")).count(&s.db).await.unwrap(), 0);
    assert_eq!(config_templates::Entity::find().filter(config_templates::Column::OriginKey.eq("redis")).count(&s.db).await.unwrap(), 0);
}

#[tokio::test]
async fn search_matches_localized_names() {
    let (_dir, s) = crate::agents::tests::setup().await;
    source(&s, 0, "official", true, &bundle("web", false, "app")).await;
    let query = HashMap::from([("q".to_owned(), "网站".to_owned())]);
    let Json(found) = with_locale(Locale::ZhCn, list(State(s.clone()), crate::agents::tests::admin(1), Query(query))).await.unwrap();
    assert_eq!(found["total"], 1);
}

#[tokio::test]
async fn list_rows_omit_heavy_fields_detail_keeps_them() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let platform = source(&s, 0, "official", true, &bundle("redis", true, "app")).await;
    let row = &items(&s, 1).await[0];
    assert!(row.get("content").is_none() && row.get("readme").is_none() && row.get("parameters").is_none());
    assert!(row["meta"].get("icon").is_none());
    assert_eq!(row["files"], json!(["redis-start"]));
    let Json(detail) = get(State(s.clone()), crate::agents::tests::admin(1), Path(("w".into(), entry_of(&s, platform).await))).await.unwrap();
    assert!(detail["content"].is_string() && detail.get("readme").is_some() && detail["meta"].get("icon").is_some());
}

#[tokio::test]
async fn other_workspace_cannot_read_or_install_own_entries_by_id() {
    let (_dir, s) = crate::agents::tests::setup().await;
    let own = source(&s, 1, "workspace", true, &bundle("own", false, "app")).await;
    let entry = entry_of(&s, own).await;
    assert_eq!(get(State(s.clone()), crate::agents::tests::admin(2), Path(("w".into(), entry))).await.err().unwrap().0, StatusCode::NOT_FOUND);
    assert_eq!(install_entry(&s, crate::agents::tests::admin(2), entry).await.err().unwrap().0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_name_follows_request_locale() {
    let (_dir, s) = crate::agents::tests::setup().await;
    source(&s, 0, "official", true, &bundle("web", false, "app")).await;
    let ms = with_locale(Locale::MsMy, items(&s, 1)).await;
    assert_eq!(ms[0]["name"], "Website");
    let zh = with_locale(Locale::ZhCn, items(&s, 1)).await;
    assert_eq!(zh[0]["name"], "网站");
}
