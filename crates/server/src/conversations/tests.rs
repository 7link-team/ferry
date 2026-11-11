use super::*;
use crate::agents::tests::{admin, setup};
use ferry_protocol::agent::{Event, Outcome};
use sea_orm::TransactionTrait;

#[tokio::test]
async fn audit_conversation_update_records_committed_before_after_and_rejects_stale_write() {
    let (_dir, s, id) = seeded().await;
    let old = find(&s.db, &admin(1), id).await.unwrap();
    let body = || Update {
        title: "Renamed audit room".into(),
        state: "archived".into(),
        version: old.version,
    };
    update(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(body()),
    )
    .await
    .unwrap();
    let entries = crate::entities::activity::Entity::find()
        .filter(crate::entities::activity::Column::ResourceId.eq(id))
        .filter(crate::entities::activity::Column::Action.eq("update"))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    let details: Value = serde_json::from_str(&entries[0].details).unwrap();
    assert_eq!(details["changes"]["title"]["before"], old.title);
    assert_eq!(details["changes"]["title"]["after"], "Renamed audit room");
    assert_eq!(details["changes"]["state"]["before"], old.state);
    assert_eq!(details["changes"]["state"]["after"], "archived");
    assert!(
        update(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(body())
        )
        .await
        .is_err()
    );
    let count = crate::entities::activity::Entity::find()
        .filter(crate::entities::activity::Column::ResourceId.eq(id))
        .filter(crate::entities::activity::Column::Action.eq("update"))
        .all(&s.db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn audit_current_titles_preserve_workspace_permissions_and_room_membership() {
    let (_dir, s, id) = seeded().await;
    let owner = admin(1);
    let titles = audit_titles(&s.db, &owner, vec![id.into()]).await.unwrap();
    assert_eq!(titles.len(), 1);
    let mut denied = owner.clone();
    denied.permissions.remove("conversations.read");
    assert!(
        audit_titles(&s.db, &denied, vec![id.into()])
            .await
            .unwrap()
            .is_empty()
    );
    let mut stranger = admin(1);
    stranger.user.id = 2.into();
    assert!(
        audit_titles(&s.db, &stranger, vec![id.into()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        audit_titles(&s.db, &admin(2), vec![id.into()])
            .await
            .unwrap()
            .is_empty()
    );
    use crate::entities::ai_chat_members as members;
    members::Entity::insert(members::ActiveModel {
        id: sea_orm::Set(s.next_id().unwrap().into()),
        workspace_id: sea_orm::Set(1),
        conversation_id: sea_orm::Set(id.into()),
        user_id: sea_orm::Set(2),
    })
    .exec(&s.db)
    .await
    .unwrap();
    assert_eq!(
        audit_titles(&s.db, &stranger, vec![id.into()])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        audit_titles(&s.db, &admin(2), vec![id.into()])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn conversation_member_avatars_are_scoped_and_do_not_expose_agent_config() {
    let (_dir, s, id) = seeded().await;
    let avatar = serde_json::from_value(json!({"style":"emoji","emoji":"🙂"})).unwrap();
    crate::accounts::preferences::save_avatar(&s.db, 1, &avatar)
        .await
        .unwrap();
    let c = find(&s.db, &admin(1), id).await.unwrap();
    ai_agents::Entity::update_many()
        .col_expr(
            ai_agents::Column::Avatar,
            sea_orm::sea_query::Expr::value(Some(
                json!({"style":"pixel","seed":"agent"}).to_string(),
            )),
        )
        .filter(ai_agents::Column::Id.eq(c.agent_id))
        .exec(&s.db)
        .await
        .unwrap();
    let Json(value) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(value["members"][0]["avatar"]["emoji"], "🙂");
    assert_eq!(value["agent_avatar"]["seed"], "agent");
    assert!(value["members"][0].get("email").is_none());
    assert!(value["members"][0].get("password_hash").is_none());
    assert_eq!(
        detail(State(s), admin(2), Path(("2".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn superadmin_group_owner_does_not_require_workspace_membership_or_join_it() {
    use crate::entities::{ai_chat_members as people, workspace_members};
    let (_dir, s, id) = seeded().await;
    let mut scope = admin(1);
    scope.user.id = 2.into();
    scope.user.is_superadmin = true;
    let body: Create = serde_json::from_value(json!({"kind":"group","member_ids":["1"]})).unwrap();
    let tx = s.db.begin().await.unwrap();
    groups::insert(&s, &tx, &scope, id, &body, vec![])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        people::Entity::find()
            .filter(people::Column::ConversationId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        workspace_members::Entity::find()
            .filter(workspace_members::Column::WorkspaceId.eq(1))
            .filter(workspace_members::Column::UserId.eq(2))
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
    let mut c = conversations::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    c.owner_id = 2;
    c.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let version = people::Entity::find()
        .filter(people::Column::ConversationId.eq(id))
        .order_by_desc(people::Column::Id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .id;
    interactions::members(
        State(s.clone()),
        scope.clone(),
        Path(("1".into(), id)),
        Json(
            serde_json::from_value(
                json!({"version":Id::from(version),"member_ids":["1"],"agent_ids":[]}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let mut outsider = scope.clone();
    outsider.user.id = 999.into();
    assert!(find(&s.db, &outsider, id).await.is_err());
    people::Entity::delete_many()
        .filter(people::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let bad: Create = serde_json::from_value(json!({"kind":"group","member_ids":["999"]})).unwrap();
    let tx = s.db.begin().await.unwrap();
    assert_eq!(
        groups::insert(&s, &tx, &scope, id, &bad, vec![])
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    tx.rollback().await.unwrap();
    scope.user.is_superadmin = false;
    let tx = s.db.begin().await.unwrap();
    assert_eq!(
        groups::insert(&s, &tx, &scope, id, &body, vec![])
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn persisted_chat_is_independent_of_execution_logs_and_final_text_replaces_deltas() {
    use crate::entities::ai_chat_messages as chat;
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "durable-message").await;
    assert_eq!(sent(&s, id, "durable-message").await, turn);
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    for event in [
        Event::Session {
            native_session_id: "native".into(),
        },
        Event::Text {
            item_id: "part-a".into(),
            text: "partial".into(),
            replace: false,
        },
        Event::Tool {
            item_id: "tool".into(),
            name: "LOG_ONLY_SECRET_MARKER".into(),
            state: "finished".into(),
        },
        Event::Text {
            item_id: "part-a".into(),
            text: "Final answer".into(),
            replace: true,
        },
        Event::Text {
            item_id: "part-b".into(),
            text: "Second paragraph".into(),
            replace: false,
        },
    ] {
        run::append(&s, &row, &owner, event).await.unwrap();
    }
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let before = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap()
        .0["messages"]
        .clone();
    assert_eq!(before.as_array().unwrap().len(), 2);
    let reply = before
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "assistant")
        .unwrap();
    assert_eq!(reply["prompt"], "Final answer\n\nSecond paragraph");
    assert_eq!(reply["turn_id"], json!(turn));
    assert!(!before.to_string().contains("LOG_ONLY_SECRET_MARKER"));
    events::Entity::delete_many()
        .filter(events::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    turns::Entity::delete_many()
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let after = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap()
        .0["messages"]
        .clone();
    assert_eq!(after, before);
    assert_eq!(
        chat::Entity::find()
            .filter(chat::Column::ConversationId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        2
    );
    let mut outsider = admin(1);
    outsider.user.id = 2.into();
    assert_eq!(
        detail(State(s), outsider, Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn legacy_private_chat_backfill_is_lossless_and_idempotent() {
    use crate::entities::ai_chat_messages as chat;
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "legacy").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    for event in [
        Event::Session {
            native_session_id: "legacy-native".into(),
        },
        Event::Text {
            item_id: "answer".into(),
            text: "old streamed text".into(),
            replace: false,
        },
        Event::Text {
            item_id: "answer".into(),
            text: "历史回复 ✅".into(),
            replace: true,
        },
    ] {
        run::append(&s, &row, &owner, event).await.unwrap();
    }
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    chat::Entity::delete_many()
        .filter(chat::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    messages::backfill(&s.db).await.unwrap();
    let before = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap()
        .0["messages"]
        .clone();
    assert_eq!(before.as_array().unwrap().len(), 2);
    assert!(
        before
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "assistant" && m["prompt"] == "历史回复 ✅")
    );
    messages::backfill(&s.db).await.unwrap();
    assert_eq!(
        detail(State(s), admin(1), Path(("1".into(), id)))
            .await
            .unwrap()
            .0["messages"],
        before
    );
}

#[tokio::test]
async fn deletion_requires_owner_and_idle_execution_and_never_removes_source_directory() {
    let (dir, s, id) = seeded().await;
    let mut other = admin(1);
    other.user.id = 2.into();
    other.user.is_superadmin = true;
    assert_eq!(
        remove(State(s.clone()), other, Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    let turn = sent(&s, id, "delete-busy").await;
    assert_eq!(
        remove(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    cancel(State(s.clone()), admin(1), Path(("1".into(), id, turn)))
        .await
        .unwrap();
    remove(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert!(
        conversations::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
    assert!(dir.path().exists());
}

#[tokio::test]
async fn groups_require_membership_only_mentioned_agents_reply_with_separate_native_state() {
    use crate::entities::{
        ai_chat_agents as snapshots, ai_chat_members as members, ai_chat_messages as messages,
        workspace_members,
    };
    let (_dir, s, id) = seeded().await;
    let c = find(&s.db, &admin(1), id).await.unwrap();
    workspace_members::Entity::insert(
        workspace_members::Model {
            id: 3,
            workspace_id: 1,
            user_id: 2,
            role_id: 1,
            state: "active".into(),
            created_at: 0,
            updated_at: 0,
            version: 1,
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    for user in [1, 2] {
        members::Entity::insert(
            members::Model {
                id: s.next_id().unwrap().into(),
                workspace_id: 1,
                conversation_id: id.into(),
                user_id: user,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    let mut second = ai_agents::Entity::find_by_id(c.agent_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    second.id = s.next_id().unwrap().into();
    second.name = "Second Agent".into();
    ai_agents::Entity::insert(second.clone().into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    for agent in [c.agent_id, second.id] {
        snapshots::Entity::insert(
            snapshots::Model {
                id: s.next_id().unwrap().into(),
                workspace_id: 1,
                conversation_id: id.into(),
                agent_id: agent,
                revision_id: c.revision_id,
                config: c.config.clone(),
                binding: c.binding.clone(),
                native_session_id: String::new(),
                state: "active".into(),
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
    }
    let mut member = admin(1);
    member.user.id = 2.into();
    assert!(find(&s.db, &member, id).await.is_ok());
    let Json(list) = list(State(s.clone()), member.clone(), Query(HashMap::new()))
        .await
        .unwrap();
    assert_eq!(list["total"], 1);
    let mut outsider = member.clone();
    outsider.user.id = 999.into();
    outsider.user.is_superadmin = true;
    assert!(find(&s.db, &outsider, id).await.is_err());
    let body = |targets: Vec<Id>, client: &str| {
        Json(
            serde_json::from_value::<groups::MessageBody>(
                json!({"prompt":"room message","client_id":client,"targets":targets}),
            )
            .unwrap(),
        )
    };
    groups::send_message(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        body(vec![], "human-only"),
    )
    .await
    .unwrap();
    assert_eq!(
        turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
    let (status, Json(message)) = groups::send_message(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        body(vec![c.agent_id.into(), second.id.into()], "targets"),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    let (again, _) = groups::send_message(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        body(vec![second.id.into(), c.agent_id.into()], "targets"),
    )
    .await
    .unwrap();
    assert_eq!(again, StatusCode::OK);
    let queued = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .order_by_asc(turns::Column::Id)
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(queued.len(), 2);
    let (first, execution, lease) = run::claim(&s, queued[0].id.into()).await.unwrap().unwrap();
    assert!(run::claim(&s, queued[1].id.into()).await.unwrap().is_none());
    run::append(
        &s,
        &first,
        &lease,
        Event::Session {
            native_session_id: "agent-one-history".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        decide(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, first.id.into())),
            Json(Decision {
                request_id: "not-owner".into(),
                allow: true
            })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::NOT_FOUND
    );
    run::finish(&s, &first, &lease, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let (second_turn, second_execution, second_lease) =
        run::claim(&s, queued[1].id.into()).await.unwrap().unwrap();
    assert_ne!(execution.agent_id, second_execution.agent_id);
    assert!(second_execution.native_session_id.is_empty());
    assert!(
        find(&s.db, &admin(1), id)
            .await
            .unwrap()
            .native_session_id
            .is_empty()
    );
    let saved = snapshots::Entity::find()
        .filter(snapshots::Column::ConversationId.eq(id))
        .filter(snapshots::Column::AgentId.eq(execution.agent_id))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.native_session_id, "agent-one-history");
    run::append(
        &s,
        &second_turn,
        &second_lease,
        Event::Session {
            native_session_id: "agent-two-history".into(),
        },
    )
    .await
    .unwrap();
    run::append(
        &s,
        &second_turn,
        &second_lease,
        Event::Text {
            item_id: "second-answer".into(),
            text: "Second Agent reply".into(),
            replace: true,
        },
    )
    .await
    .unwrap();
    run::finish(
        &s,
        &second_turn,
        &second_lease,
        Some(Outcome::Completed),
        "",
        true,
    )
    .await
    .unwrap();
    let reply = messages::Entity::find()
        .filter(messages::Column::TurnId.eq(second_turn.id))
        .filter(messages::Column::Role.eq("assistant"))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.agent_id, second_execution.agent_id);
    assert_eq!(
        reply.reply_to,
        Some(message["id"].as_str().unwrap().parse().unwrap())
    );
    assert_eq!(reply.owner_id, 2);
    // Existing group user messages must not be duplicated during upgrade.
    messages::Entity::delete_by_id(reply.id)
        .exec(&s.db)
        .await
        .unwrap();
    super::messages::backfill(&s.db).await.unwrap();
    super::messages::backfill(&s.db).await.unwrap();
    let restored = messages::Entity::find_by_id(reply.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.id, reply.id);
    assert_eq!(restored.prompt, reply.prompt);
    assert_eq!(restored.agent_id, reply.agent_id);
    assert_eq!(restored.reply_to, reply.reply_to);
    assert_eq!(
        messages::Entity::find()
            .filter(messages::Column::ConversationId.eq(id))
            .count(&s.db)
            .await
            .unwrap(),
        3
    );
    assert!(message["id"].is_string());
    assert_eq!(
        remove(State(s.clone()), member, Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn files_are_conversation_scoped_canonical_and_released_by_delete() {
    use crate::{
        artifact_store::ArtifactStore,
        content_assets::{DEFAULT_QUOTA, Store},
        entities::{ai_chat_files as files, asset_refs},
    };
    let (_dir, s, id) = seeded().await;
    let backend = ArtifactStore::for_state(&s).await.unwrap();
    let storage = Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    };
    let original = "exact UTF-8 attachment";
    let asset = storage
        .stage_bytes(1, original.as_bytes(), "text/plain", 0, "test", 1)
        .await
        .unwrap();
    let file = files::Model {
        id: s.next_id().unwrap().into(),
        workspace_id: 1,
        conversation_id: id.into(),
        owner_id: 1,
        asset_id: asset.id,
        size: original.len() as i64,
        name: "notes.txt".into(),
        mime: "text/plain".into(),
        created_at: 0,
    };
    let tx = crate::variables::transaction(&s, 1.into()).await.unwrap();
    storage
        .attach(&tx, 1, asset.id, "conversation_file", file.id)
        .await
        .unwrap();
    files::Entity::insert(file.clone().into_active_model())
        .exec(&tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut attachments = vec![Attachment {
        name: "forged.html".into(),
        text: "forged content".into(),
        file_id: Some(file.id.into()),
        mime: "text/html".into(),
        size: 900,
    }];
    super::files::canonicalize(&s, &admin(1), id, &mut attachments)
        .await
        .unwrap();
    assert_eq!(attachments[0].name, "notes.txt");
    assert_eq!(attachments[0].text, original);
    assert_eq!(attachments[0].mime, "text/plain");
    assert!(
        super::files::resolve(&s.db, 1.into(), (i64::from(id) + 1).into(), file.id.into())
            .await
            .is_err()
    );
    assert!(
        super::files::resolve(&s.db, 2.into(), id, file.id.into())
            .await
            .is_err()
    );
    remove(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(
        asset_refs::Entity::find()
            .filter(asset_refs::Column::OwnerKind.eq("conversation_file"))
            .filter(asset_refs::Column::OwnerId.eq(file.id))
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
    assert!(
        files::Entity::find_by_id(file.id)
            .one(&s.db)
            .await
            .unwrap()
            .is_none()
    );
    // Shared content is handled by the existing candidate GC, not an unsafe unlink.
    assert_eq!(
        storage.read_bytes(&s.db, 1, asset.id).await.unwrap(),
        original.as_bytes()
    );
}

#[test]
fn media_ranges_and_active_documents_use_shared_safe_response() {
    use axum::http::{HeaderMap, header};
    let mut headers = HeaderMap::new();
    headers.insert(header::RANGE, "bytes=2-4".parse().unwrap());
    let response =
        crate::workflows::media::media_response(b"abcdef".to_vec(), "video/mp4", &headers, None)
            .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 2-4/6");
    let response = crate::workflows::media::media_response(
        b"<script>bad</script>".to_vec(),
        "text/html",
        &HeaderMap::new(),
        Some("report.html"),
    )
    .unwrap();
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    headers.insert(header::RANGE, "bytes=99-".parse().unwrap());
    assert_eq!(
        crate::workflows::media::media_response(b"abcdef".to_vec(), "video/mp4", &headers, None)
            .unwrap()
            .status(),
        StatusCode::RANGE_NOT_SATISFIABLE
    );
}
pub(super) async fn seeded() -> (tempfile::TempDir, AppState, Id) {
    let (dir, s) = setup().await;
    crate::entities::workspace_roles::Entity::insert(
        crate::entities::workspace_roles::Model {
            id: 1,
            workspace_id: 1,
            name: "Conversation test role".into(),
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
    let data = json!({"name":"Test Agent","runtime":"claude","host_id":"1","owner_id":"1","cwd":dir.path().to_string_lossy(),"system_prompt":"Original instructions","permission":"on_request","visibility":"workspace","state":"enabled"});
    let (_, Json(created)) = crate::agents::create(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(data).unwrap()),
    )
    .await
    .unwrap();
    let agent =
        ai_agents::Entity::find_by_id(created["id"].as_str().unwrap().parse::<i64>().unwrap())
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
    let id = s.next_id().unwrap();
    conversations::Entity::insert(conversations::Model{id:id.into(),workspace_id:1,owner_id:1,agent_id:agent.id,revision_id:agent.revision_id,title:"Private conversation".into(),state:"active".into(),
        config:serde_json::to_string(&Configuration::from_agent(&agent).unwrap()).unwrap(),binding:json!({"execution_user":"test","canonical_directory":dir.path().to_string_lossy(),"executable":"test-cli"}).to_string(),
        native_session_id:String::new(),next_sequence:0,version:1,created_at:db::now(),updated_at:db::now()}.into_active_model()).exec(&s.db).await.unwrap();
    (dir, s, id)
}
pub(super) async fn sent(s: &AppState, id: Id, client: &str) -> Id {
    let (_, Json(value)) = send(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(Send {
            reply_to: None,
            prompt: "hello".into(),
            client_id: client.into(),
            attachments: vec![],
        }),
    )
    .await
    .unwrap();
    serde_json::from_value(value["id"].clone()).unwrap()
}
#[tokio::test]
async fn directory_and_tool_permissions_are_frozen_per_conversation_not_agent() {
    let (_dir, s, id) = seeded().await;
    let c = find(&s.db, &admin(1), id).await.unwrap();
    let agent = ai_agents::Entity::find_by_id(c.agent_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let body: Create = serde_json::from_value(json!({"agent_id":Id::from(agent.id),"cwd":if cfg!(windows){"D:/session-work"}else{"/tmp/session-work"},"permission":"read_only"})).unwrap();
    let saved = Configuration::for_conversation(&agent, &body).unwrap();
    assert_eq!(saved.cwd, body.cwd.unwrap());
    assert_eq!(
        saved.permission,
        ferry_protocol::agent::Permission::ReadOnly
    );
    let original: Configuration = serde_json::from_str(&c.config).unwrap();
    assert_eq!(original.cwd, agent.cwd);
    assert_eq!(
        original.permission,
        ferry_protocol::agent::Permission::OnRequest
    );
    let invalid: Create =
        serde_json::from_value(json!({"agent_id":Id::from(agent.id),"cwd":"relative"})).unwrap();
    assert!(Configuration::for_conversation(&agent, &invalid).is_err());
}
#[tokio::test]
async fn disabled_agent_blocks_new_admission_but_preserves_running_approval_and_cancel() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "running").await;
    let (row, c, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Approval {
            request_id: "pending".into(),
            tool: "Write".into(),
            input: json!({}),
        },
    )
    .await
    .unwrap();
    let mut agent = ai_agents::Entity::find_by_id(c.agent_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    agent.state = "disabled".into();
    agent
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    assert!(
        available(&s.db, &admin(1), c.agent_id.into())
            .await
            .is_err()
    );
    assert!(!run::heartbeat(&s, &row, &owner).await.unwrap().0);
    decide(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id, turn)),
        Json(Decision {
            request_id: "pending".into(),
            allow: false,
        }),
    )
    .await
    .unwrap();
    let (stop, decisions) = run::heartbeat(&s, &row, &owner).await.unwrap();
    assert!(!stop);
    assert_eq!(decisions.len(), 1);
    cancel(State(s.clone()), admin(1), Path(("1".into(), id, turn)))
        .await
        .unwrap();
    assert!(run::heartbeat(&s, &row, &owner).await.unwrap().0);
    run::finish(&s, &row, &owner, Some(Outcome::Cancelled), "", true)
        .await
        .unwrap();
    assert_eq!(
        send(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(Send {
                reply_to: None,
                prompt: "blocked".into(),
                client_id: "new".into(),
                attachments: vec![]
            })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    assert!(view(&c)["config"].get("runner_key_hash").is_none());
}
#[tokio::test]
async fn ownership_is_required_even_for_managers_and_superadmins() {
    let (_dir, s, id) = seeded().await;
    let mut other = admin(1);
    other.user.id = 2.into();
    other.user.is_superadmin = true;
    assert_eq!(
        detail(State(s.clone()), other.clone(), Path(("1".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        list(State(s.clone()), other.clone(), Query(HashMap::new()))
            .await
            .unwrap()
            .0["total"],
        0
    );
    assert_eq!(
        history(
            State(s.clone()),
            other,
            Path(("1".into(), id)),
            Query(Cursor { after: 0 })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        detail(State(s), admin(2), Path(("2".into(), id)))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn messages_are_idempotent_frozen_and_serialized_without_duplicate_runs() {
    let (_dir, s, id) = seeded().await;
    let first = sent(&s, id, "request-1").await;
    assert_eq!(sent(&s, id, "request-1").await, first);
    assert_eq!(
        send(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(Send {
                reply_to: None,
                prompt: "next".into(),
                client_id: "request-2".into(),
                attachments: vec![]
            })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let c = find(&s.db, &admin(1), id).await.unwrap();
    let mut agent = ai_agents::Entity::find_by_id(c.agent_id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    agent.system_prompt = "New mutable Agent instructions".into();
    agent
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&find(&s.db, &admin(1), id).await.unwrap().config).unwrap()["system_prompt"],
        "Original instructions"
    );
    let mut bad = Send {
        reply_to: None,
        prompt: "hello".into(),
        client_id: "request-3".into(),
        attachments: vec![Attachment {
            name: "binary.txt".into(),
            text: "\0".into(),
            ..Default::default()
        }],
    };
    assert_eq!(
        send(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(bad)
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::BAD_REQUEST
    );
    bad = Send {
        reply_to: None,
        prompt: "x".repeat(49 * 1024),
        client_id: "request-4".into(),
        attachments: vec![],
    };
    assert_eq!(
        send(State(s), admin(1), Path(("1".into(), id)), Json(bad))
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn decisions_persist_once_and_only_native_receipt_marks_them_sent() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "request-1").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "native-a".into(),
        },
    )
    .await
    .unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Approval {
            request_id: "native-a:permission-1".into(),
            tool: "Write".into(),
            input: json!({"file":"test.txt"}),
        },
    )
    .await
    .unwrap();
    for _ in 0..2 {
        decide(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, turn)),
            Json(Decision {
                request_id: "native-a:permission-1".into(),
                allow: true,
            }),
        )
        .await
        .unwrap();
    }
    let (stop, inputs) = run::heartbeat(&s, &row, &owner).await.unwrap();
    assert!(!stop);
    assert_eq!(inputs.len(), 1);
    assert!(run::heartbeat(&s, &row, &owner).await.unwrap().1.is_empty());
    let current = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&current.decisions).unwrap()["native-a:permission-1"]["sent"],
        false
    );
    run::append(
        &s,
        &row,
        &owner,
        Event::DecisionSent {
            request_id: "native-a:permission-1".into(),
            allowed: true,
        },
    )
    .await
    .unwrap();
    let first = history(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Query(Cursor { after: 0 }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(first["items"].as_array().unwrap().len(), 3);
    assert_eq!(first["next"], 3);
    assert_eq!(
        history(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Query(Cursor { after: 2 })
        )
        .await
        .unwrap()
        .0["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    assert_eq!(
        find(&s.db, &admin(1), id).await.unwrap().native_session_id,
        "native-a"
    );
    assert_eq!(
        decide(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, turn)),
            Json(Decision {
                request_id: "native-a:permission-1".into(),
                allow: true
            })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let next = sent(&s, id, "request-2").await;
    assert_eq!(
        turns::Entity::find_by_id(next)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .native_session_id,
        "native-a"
    );
}
#[tokio::test]
async fn expired_worker_cannot_write_and_recovery_never_replays_unknown_side_effects() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "request-1").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    let mut expired = row.clone();
    expired.expires_at = db::now() - 1;
    expired
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    assert!(
        run::append(
            &s,
            &row,
            &owner,
            Event::Text {
                item_id: "a".into(),
                text: "stale".into(),
                replace: false
            }
        )
        .await
        .is_err()
    );
    assert!(
        run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
            .await
            .is_err()
    );
    assert!(run::claim(&s, turn).await.unwrap().is_none());
    assert_eq!(
        turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "interrupted"
    );
    assert_eq!(
        find(&s.db, &admin(1), id).await.unwrap().state,
        "interrupted"
    );
    assert_eq!(events::Entity::find().count(&s.db).await.unwrap(), 0);
    assert!(
        send(
            State(s),
            admin(1),
            Path(("1".into(), id)),
            Json(Send {
                reply_to: None,
                prompt: "do not replay".into(),
                client_id: "retry".into(),
                attachments: vec![]
            })
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn explicit_recovery_preserves_attempts_and_starts_a_fresh_native_session() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "first").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    run::finish(&s, &row, &owner, Some(Outcome::Failed), "", true)
        .await
        .unwrap();
    let c = find(&s.db, &admin(1), id).await.unwrap();
    let body = || {
        Json(Update {
            title: c.title.clone(),
            state: "active".into(),
            version: c.version,
        })
    };
    let mut other = admin(1);
    other.user.id = 999.into();
    assert!(
        update(State(s.clone()), other, Path(("1".into(), id)), body())
            .await
            .is_err()
    );
    assert!(
        update(State(s.clone()), admin(2), Path(("2".into(), id)), body())
            .await
            .is_err()
    );
    let mut forbidden = admin(1);
    forbidden.permissions.remove("conversations.run");
    assert_eq!(
        update(State(s.clone()), forbidden, Path(("1".into(), id)), body())
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    update(State(s.clone()), admin(1), Path(("1".into(), id)), body())
        .await
        .unwrap();
    assert_eq!(
        update(State(s.clone()), admin(1), Path(("1".into(), id)), body())
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    let next = sent(&s, id, "explicit-retry").await;
    let retry = turns::Entity::find_by_id(next)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(next, turn);
    assert!(retry.native_session_id.is_empty());
    assert_eq!(
        turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "failed"
    );
    assert_eq!(find(&s.db, &admin(1), id).await.unwrap().state, "active");
    let active = find(&s.db, &admin(1), id).await.unwrap();
    assert_eq!(
        update(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(Update {
                title: active.title,
                state: "active".into(),
                version: active.version
            })
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn recovered_context_contains_only_completed_same_room_messages_and_is_bounded() {
    let (_dir, s, id) = seeded().await;
    let first = sent(&s, id, "completed").await;
    let (row, _, owner) = run::claim(&s, first).await.unwrap().unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "original-native".into(),
        },
    )
    .await
    .unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Text {
            item_id: "reply".into(),
            text: "COMPLETED_CONTEXT中文".repeat(2000),
            replace: false,
        },
    )
    .await
    .unwrap();
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let failed = sent(&s, id, "failed").await;
    let mut failure = turns::Entity::find_by_id(failed)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    failure.prompt = "UNCERTAIN_DO_NOT_REPLAY".into();
    failure
        .into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    let (row, _, owner) = run::claim(&s, failed).await.unwrap().unwrap();
    run::finish(&s, &row, &owner, Some(Outcome::Failed), "", true)
        .await
        .unwrap();
    let mut next = row.clone();
    next.id += 1;
    let context = messages::recovery_context(&s.db, &next, 1024)
        .await
        .unwrap();
    assert!(context.len() <= 1024);
    assert!(context.contains("COMPLETED_CONTEXT"));
    assert!(!context.contains("UNCERTAIN_DO_NOT_REPLAY"));
    assert!(
        messages::recovery_context(&s.db, &next, 10)
            .await
            .unwrap()
            .is_empty()
    );
    next.conversation_id += 1000;
    assert!(
        messages::recovery_context(&s.db, &next, 1024)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn stopping_requires_receipt_and_permission_revocation_stops_instead_of_dispatching() {
    let (_dir, s, id) = seeded().await;
    let first = sent(&s, id, "request-1").await;
    cancel(State(s.clone()), admin(1), Path(("1".into(), id, first)))
        .await
        .unwrap();
    assert!(run::claim(&s, first).await.unwrap().is_none());
    let turn = sent(&s, id, "request-2").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    cancel(State(s.clone()), admin(1), Path(("1".into(), id, turn)))
        .await
        .unwrap();
    assert_eq!(
        turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .state,
        "stopping"
    );
    assert!(run::heartbeat(&s, &row, &owner).await.unwrap().0);
    run::finish(&s, &row, &owner, Some(Outcome::Cancelled), "", true)
        .await
        .unwrap();
    let next = sent(&s, id, "request-3").await;
    let (row, _, owner) = run::claim(&s, next).await.unwrap().unwrap();
    let mut role = crate::entities::workspace_roles::Entity::find_by_id(1)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    role.permissions = "[]".into();
    role.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    assert!(run::heartbeat(&s, &row, &owner).await.unwrap().0);
}
