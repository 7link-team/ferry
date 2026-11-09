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
