use super::*;
use crate::agents::tests::{admin, setup};

fn body() -> Body {
    serde_json::from_value(json!({"name":"Local CLI","kind":"cli","runtime":"claude","models":[],"default_model":"","tool_version":"","state":"enabled"})).unwrap()
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
