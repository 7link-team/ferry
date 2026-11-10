use super::*;
use crate::{
    agents::tests::admin,
    conversations::tests::{seeded, sent},
    entities::{definition_revisions, host_groups, workspace_roles},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};

async fn enabled() -> (tempfile::TempDir, AppState, turns::Model, String) {
    let (dir, s, id) = seeded().await;
    let c = conversations::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut revision = definition_revisions::Entity::find_by_id(c.revision_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut definition: Value = serde_json::from_str(&revision.definition).unwrap();
    definition["platform_role_id"] = json!("1");
    revision.definition = definition.to_string();
    revision
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let turn = sent(&s, id, "platform-test").await;
    let (row, _, owner) = claim(&s, turn).await.unwrap().unwrap();
    (dir, s, row, owner)
}

async fn decision(s: &AppState, row: &turns::Model, owner: &str, id: &str, allow: bool) -> Value {
    super::super::decide(
        State(s.clone()),
        admin(1),
        Path(("1".into(), row.conversation_id.into(), row.id.into())),
        Json(serde_json::from_value(json!({"request_id":id,"allow":allow})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(heartbeat(s, row, owner).await.unwrap().1.len(), 1);
    let arguments = approved_platform_call(s, row, owner, id)
        .await
        .unwrap()
        .unwrap();
    platform_call(
        s.clone(),
        row.clone(),
        owner.into(),
        id.into(),
        "ferry_request".into(),
        arguments,
        Some(allow),
    )
    .await
    .1
}

#[tokio::test]
async fn platform_writes_wait_for_confirmation_hide_secrets_and_decline_closes_receipt() {
    let (_dir, s, row, owner) = enabled().await;
    let arguments = json!({"method":"POST","path":"/node-groups","body":{"name":"Confirmed group","parent_id":"0","position":0,"password":"never-public"}});
    let result = platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "write-1".into(),
        "ferry_request".into(),
        arguments.clone(),
        None,
    )
    .await
    .1;
    assert!(result.is_null());
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
    let current = turns::Entity::find_by_id(row.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let public = super::super::view_turn(&current).to_string();
    assert!(!public.contains("never-public"));
    assert!(!public.contains("_sealed"));
    let events = super::super::history(
        State(s.clone()),
        admin(1),
        Path(("1".into(), row.conversation_id.into())),
        Query(super::super::Cursor { after: 0 }),
    )
    .await
    .unwrap()
    .0
    .to_string();
    assert!(!events.contains("never-public"));
    assert!(!events.contains("_sealed"));
    let result = decision(&s, &row, &owner, "write-1", false).await;
    assert_eq!(result["ok"], false);
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 0);
    assert!(
        approved_platform_call(&s, &row, &owner, "write-1")
            .await
            .is_err()
    );
    let duplicate = platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "write-1".into(),
        "ferry_request".into(),
        arguments,
        None,
    )
    .await
    .1;
    assert_eq!(duplicate["ok"], false);
}

#[tokio::test]
async fn platform_confirmation_rechecks_revoked_permissions_and_success_is_once() {
    let (_dir, s, row, owner) = enabled().await;
    let args = json!({"method":"POST","path":"/node-groups","body":{"name":"Agent group","parent_id":"0","position":0}});
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "allowed".into(),
        "ferry_request".into(),
        args.clone(),
        None,
    )
    .await;
    assert_eq!(
        decision(&s, &row, &owner, "allowed", true).await["status"],
        201
    );
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 1);
    assert!(
        approved_platform_call(&s, &row, &owner, "allowed")
            .await
            .is_err()
    );
    platform_call(
        s.clone(),
        row.clone(),
        owner.clone(),
        "revoked".into(),
        "ferry_request".into(),
        args,
        None,
    )
    .await;
    let mut role = workspace_roles::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut grants: Vec<String> = serde_json::from_str(&role.permissions).unwrap();
    grants.retain(|p| p != "host-groups.create");
    role.permissions = json!(grants).to_string();
    role.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let result = decision(&s, &row, &owner, "revoked", true).await;
    assert_eq!(result["status"], 403);
    assert_eq!(host_groups::Entity::find().count(&s.db).await.unwrap(), 1);
    assert!(
        approved_platform_call(&s, &row, &owner, "revoked")
            .await
            .is_err()
    );
}
