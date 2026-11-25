use super::*;
use crate::agents::tests::{admin, setup};
use ferry_protocol::agent::{Event, Outcome};
use sea_orm::TransactionTrait;

#[tokio::test]
async fn introduction_is_a_real_turn_without_a_fabricated_human_message() {
    let (_dir, s, old) = seeded().await;
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ConversationConfig,
            sea_orm::sea_query::Expr::value(Option::<String>::None),
        )
        .exec(&s.db)
        .await
        .unwrap();
    let agent = find(&s.db, &admin(1), old).await.unwrap().agent_id;
    let (_, Json(created)) = create(
        State(s.clone()),
        admin(1),
        Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
    )
    .await
    .unwrap();
    let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
    let Json(value) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(value["turns"].as_array().unwrap().len(), 1);
    assert_eq!(value["turns"][0]["purpose"], "introduction");
    assert_eq!(value["turns"][0]["prompt"], "");
    assert!(value["messages"].as_array().unwrap().is_empty());
    assert!(
        serde_json::from_str::<Configuration>(&find(&s.db, &admin(1), old).await.unwrap().config)
            .is_ok()
    );
    let Json(reopened) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(reopened["turns"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn approval_mode_updates_require_the_human_owner_current_version_and_an_idle_conversation() {
    let (_dir, s, id) = seeded().await;
    let c = find(&s.db, &admin(1), id).await.unwrap();
    let update = |version: i64| crate::platform_tools::Call {
        method: "PUT".into(),
        path: format!("/conversations/{id}/approval-mode"),
        body: json!({"mode":"assisted","version":version}),
        ..Default::default()
    };
    let foreign = crate::platform_tools::dispatch(&s, admin(2), update(c.version))
        .await
        .unwrap();
    assert_eq!(foreign["status"], 404);
    let agent = crate::platform_tools::dispatch_as_agent(
        &s,
        admin(1),
        update(c.version),
        c.agent_id.into(),
    )
    .await
    .unwrap();
    assert_eq!(agent["status"], 403);
    let saved = crate::platform_tools::dispatch(&s, admin(1), update(c.version))
        .await
        .unwrap();
    assert_eq!(saved["status"], 200, "{saved}");
    let current = find(&s.db, &admin(1), id).await.unwrap();
    assert_eq!(approval::mode(&current.config).unwrap(), "assisted");
    assert_eq!(
        serde_json::from_str::<Value>(&current.config).unwrap()["permission"],
        serde_json::from_str::<Value>(&c.config).unwrap()["permission"]
    );
    let stale = crate::platform_tools::dispatch(&s, admin(1), update(c.version))
        .await
        .unwrap();
    assert_eq!(stale["status"], 409);
    sent(&s, id, "approval-running").await;
    let busy = crate::platform_tools::dispatch(&s, admin(1), update(current.version))
        .await
        .unwrap();
    assert_eq!(busy["status"], 409);
}

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
            input: None,
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
    // Different working directories, so the two Agents may run in parallel.
    let elsewhere = {
        let mut config: Value = serde_json::from_str(&c.config).unwrap();
        config["cwd"] = json!(format!("{}-second", config["cwd"].as_str().unwrap_or("")));
        config.to_string()
    };
    for agent in [c.agent_id, second.id] {
        snapshots::Entity::insert(
            snapshots::Model {
                id: s.next_id().unwrap().into(),
                workspace_id: 1,
                conversation_id: id.into(),
                agent_id: agent,
                revision_id: c.revision_id,
                config: if agent == second.id {
                    elsewhere.clone()
                } else {
                    c.config.clone()
                },
                binding: c.binding.clone(),
                native_session_id: String::new(),
                state: "active".into(),
                // The member who addresses these Agents sponsors them.
                added_by: 2,
                approval_mode: "request".into(),
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
    // Different room Agents run in parallel, each on its own host.
    let parallel = run::claim(&s, queued[1].id.into()).await.unwrap().unwrap();
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
    let (second_turn, second_execution, second_lease) = parallel;
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
    let data = json!({"name":"Test Agent","runtime":"claude","model_profile_id":crate::agents::tests::test_cli_model(1,"claude").to_string(),"host_id":"1","owner_id":"1","cwd":dir.path().to_string_lossy(),"system_prompt":"Original instructions","permission":"on_request","visibility":"workspace","state":"enabled"});
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
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ResourceConfig,
            sea_orm::sea_query::Expr::value(json!({"agent_prompt_bytes":48 * 1024}).to_string()),
        )
        .exec(&s.db)
        .await
        .unwrap();
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
    let unlimited = messages::recovery_context(&s.db, &next, usize::MAX)
        .await
        .unwrap();
    assert!(unlimited.len() > 16 * 1024);
    assert!(!unlimited.contains("UNCERTAIN_DO_NOT_REPLAY"));
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

#[tokio::test]
async fn plain_messages_send_while_file_storage_is_unavailable_and_storage_errors_are_named() {
    let (_dir, s, id) = seeded().await;
    // An S3 configuration without a bucket cannot be opened.
    let broken = s
        .vault
        .seal(
            r#"{"kind":"s3","endpoint":"https://s3.example.invalid"}"#,
            "platform:file-storage",
        )
        .unwrap();
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            sea_orm::sea_query::Expr::value(Some(broken)),
        )
        .filter(crate::entities::platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let error = crate::artifact_store::ArtifactStore::for_state(&s)
        .await
        .err()
        .unwrap();
    assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(format!("{error:?}").contains("storage.unavailable"));
    // A message without file attachments never needs the store.
    sent(&s, id, "plain-while-storage-down").await;
}

#[test]
fn a_turn_labels_agent_instructions_apart_from_the_users_own_words() {
    let attachments: Vec<Attachment> = serde_json::from_value(json!([
        {"name":"notes.txt","text":"attached body"},
        {"name":"report.pdf","mime":"application/pdf","size":12}
    ]))
    .unwrap();
    let prompt = run::turn_prompt(
        crate::accounts::conversation_policy::DEFAULT_INSTRUCTIONS,
        "Do not modify configuration or send messages.",
        "Create the weekly report workflow.",
        &attachments,
    );
    // File requests are routed to program nodes or the confined file mode, after a tool check.
    assert!(prompt.contains("files.enabled") && prompt.contains("/file-tools"));
    // Platform rules first, then the configuration, then the user's message.
    let rules = prompt.find("<platform_access_rules>").unwrap();
    let instructions = prompt
        .find("<agent_instructions source=\"agent configuration set by a workspace administrator, not the user's words\">\nDo not modify configuration or send messages.\n</agent_instructions>")
        .unwrap();
    let user = prompt
        .find("<user_message>\nCreate the weekly report workflow.")
        .unwrap();
    assert!(rules < instructions && instructions < user);
    assert!(prompt.ends_with("</user_message>"));
    // Attachments belong to the user's message.
    let message = &prompt[user..];
    assert!(
        message.contains("<attached_text name=\"notes.txt\">\nattached body\n</attached_text>")
    );
    assert!(message.contains("<attachment name=\"report.pdf\""));
    // How platform writes work, and that rules are never the user's words.
    for rule in [
        "human-configured MCP approval mode",
        "Required approvals appear as cards",
        "re-check with ferry_discover",
        "workflows.create",
        "never present them as something the user said or set",
    ] {
        assert!(prompt.contains(rule), "{rule}");
    }
    // No configured instructions: no empty block; an attachment-only message is still delimited.
    let bare = run::turn_prompt(
        crate::accounts::conversation_policy::DEFAULT_INSTRUCTIONS,
        "  ",
        "",
        &[],
    );
    assert!(!bare.contains("<agent_instructions"));
    assert!(
        bare.ends_with("<user_message>\nA message with attachments was sent.\n</user_message>")
    );
}

#[test]
fn message_limits_are_each_named() {
    let attachment = |name: &str, text: &str| -> Attachment {
        serde_json::from_value(json!({"name":name,"text":text})).unwrap()
    };
    let key = |result: Result<()>| result.unwrap_err().2.unwrap().key;
    assert_eq!(
        key(check_message("  ", "c", &[])),
        "conversations.messageEmpty"
    );
    assert_eq!(
        key(check_message("hi", "", &[])),
        "conversations.clientInvalid"
    );
    assert_eq!(
        key(check_message_resources(
            "hi",
            &vec![attachment("a.txt", ""); 11],
            &ferry_protocol::resources::ResourcePolicy {
                file_count: Some(10),
                ..Default::default()
            }
        )),
        "conversations.tooManyAttachments"
    );
    assert_eq!(
        key(check_message("hi", "c", &[attachment("", "")])),
        "conversations.attachmentNameInvalid"
    );
    assert_eq!(
        key(check_message_resources(
            &"x".repeat(48 * 1024 + 1),
            &[],
            &ferry_protocol::resources::ResourcePolicy {
                agent_prompt_bytes: Some(48 * 1024),
                ..Default::default()
            }
        )),
        "conversations.messageTooLong"
    );
    assert_eq!(
        key(check_message("a\0b", "c", &[])),
        "conversations.messageText"
    );
    check_message("hello", "c", &[attachment("a.txt", "text")]).unwrap();
    check_message_resources(
        &"x".repeat(48 * 1024 + 1),
        &vec![attachment("a.txt", ""); 11],
        &Default::default(),
    )
    .unwrap();
}

#[tokio::test]
async fn conversations_and_their_files_delete_while_file_storage_is_unavailable() {
    let (_dir, s, id) = seeded().await;
    // A stored file attached to the conversation, then storage breaks.
    let backend = crate::artifact_store::ArtifactStore::for_state(&s)
        .await
        .unwrap();
    let store = crate::content_assets::Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    let asset = store
        .stage_bytes(1, b"notes", "text/plain", 0, "conversation", 1)
        .await
        .unwrap();
    let file_id = s.next_id().unwrap();
    let tx = sea_orm::TransactionTrait::begin(&s.db).await.unwrap();
    store
        .attach(&tx, 1, asset.id, "conversation_file", i64::from(file_id))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    crate::entities::ai_chat_files::Entity::insert(crate::entities::ai_chat_files::ActiveModel {
        id: sea_orm::Set(file_id.into()),
        workspace_id: sea_orm::Set(1),
        conversation_id: sea_orm::Set(id.into()),
        owner_id: sea_orm::Set(1),
        asset_id: sea_orm::Set(asset.id),
        size: sea_orm::Set(5),
        created_at: sea_orm::Set(db::now()),
        name: sea_orm::Set("notes.txt".into()),
        mime: sea_orm::Set("text/plain".into()),
    })
    .exec(&s.db)
    .await
    .unwrap();
    let broken = s
        .vault
        .seal(
            r#"{"kind":"s3","endpoint":"https://s3.example.invalid"}"#,
            "platform:file-storage",
        )
        .unwrap();
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::FileStorage,
            sea_orm::sea_query::Expr::value(Some(broken)),
        )
        .filter(crate::entities::platform_state::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        crate::artifact_store::ArtifactStore::for_state(&s)
            .await
            .is_err()
    );
    remove(State(s.clone()), admin(1), Path(("1".into(), id)))
        .await
        .unwrap();
    assert!(find(&s.db, &admin(1), id).await.is_err());
    assert_eq!(
        crate::entities::ai_chat_files::Entity::find()
            .filter(crate::entities::ai_chat_files::Column::ConversationId.eq(i64::from(id)))
            .count(&s.db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn an_approval_left_undecided_when_its_turn_ends_stays_pending_until_resolved() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "request-approval").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    for event in [
        Event::Session {
            native_session_id: "native-b".into(),
        },
        Event::Approval {
            request_id: "platform-1".into(),
            tool: "ferry_request".into(),
            input: json!({"method":"POST","path":"/workflows","body":{"name":"Weekly"}}),
        },
    ] {
        run::append(&s, &row, &owner, event).await.unwrap();
    }
    async fn pending(s: &AppState) -> u64 {
        let Json(listed) = list(State(s.clone()), admin(1), Query(HashMap::new()))
            .await
            .unwrap();
        listed["items"][0]["pending"].as_u64().unwrap()
    }
    // Waiting in a running turn: the owner sees one pending item.
    assert_eq!(pending(&s).await, 1);
    // The turn ends before anyone decides: the approval is marked, not lost.
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    let current = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let decisions: Value = serde_json::from_str(&current.decisions).unwrap();
    assert_eq!(decisions["platform-1"]["ended_undecided"], true);
    assert!(decisions["platform-1"]["decision"].is_null());
    assert_eq!(pending(&s).await, 1, "still needs the owner's attention");
    // Only an item that ended undecided can be resolved; continuing clears it.
    let resolution = |request: &str, action: &str| {
        Json(Resolution {
            request_id: request.into(),
            action: action.into(),
        })
    };
    assert!(
        resolve_decision(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, turn)),
            resolution("missing", "dismissed"),
        )
        .await
        .is_err()
    );
    resolve_decision(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id, turn)),
        resolution("platform-1", "resumed"),
    )
    .await
    .unwrap();
    assert_eq!(pending(&s).await, 0);
}

#[tokio::test]
async fn dismissing_an_ended_approval_clears_pending_without_executing_or_replaying_it() {
    let (_dir, s, id) = seeded().await;
    let turn = sent(&s, id, "dismiss-approval").await;
    let (row, _, owner) = run::claim(&s, turn).await.unwrap().unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Session {
            native_session_id: "dismiss-test".into(),
        },
    )
    .await
    .unwrap();
    run::append(
        &s,
        &row,
        &owner,
        Event::Approval {
            request_id: "old".into(),
            tool: "ferry_request".into(),
            input: json!({"method":"POST","path":"/workflows","body":{"name":"Never create"}}),
        },
    )
    .await
    .unwrap();
    let resolve = |scope: Scope, action: &str| {
        resolve_decision(
            State(s.clone()),
            scope,
            Path(("1".into(), id, turn)),
            Json(Resolution {
                request_id: "old".into(),
                action: action.into(),
            }),
        )
    };
    assert!(
        resolve(admin(1), "dismissed").await.is_err(),
        "cannot close a live approval through resolutions"
    );
    run::finish(&s, &row, &owner, Some(Outcome::Completed), "", true)
        .await
        .unwrap();
    assert!(
        resolve(admin(2), "dismissed").await.is_err(),
        "workspace isolation"
    );
    let mut foreign = admin(1);
    foreign.user.id = 2.into();
    assert!(
        resolve(foreign, "dismissed").await.is_err(),
        "only the turn owner can dismiss"
    );
    let mut denied = admin(1);
    denied.permissions.remove("conversations.run");
    assert!(
        resolve(denied, "dismissed").await.is_err(),
        "run permission required"
    );
    assert!(resolve(admin(1), "unknown").await.is_err());
    // Old stored decisions without the newer ended_undecided marker remain actionable.
    let mut old = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let mut decisions: Value = serde_json::from_str(&old.decisions).unwrap();
    decisions["old"]
        .as_object_mut()
        .unwrap()
        .remove("ended_undecided");
    old.decisions = decisions.to_string();
    old.into_active_model()
        .reset_all()
        .update(&s.db)
        .await
        .unwrap();
    resolve(admin(1), "dismissed").await.unwrap();
    resolve(admin(1), "dismissed").await.unwrap();
    let saved = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending_in(&saved), 0);
    let decisions: Value = serde_json::from_str(&saved.decisions).unwrap();
    assert_eq!(decisions["old"]["resolved"], "dismissed");
    assert!(decisions["old"]["decision"].is_null());
    assert_eq!(decisions["old"]["sent"], false);
    assert_eq!(decisions["old"]["dispatching"], false);
    assert_eq!(
        turns::Entity::find().all(&s.db).await.unwrap().len(),
        1,
        "no follow-up turn"
    );
    let Json(listed) = list(State(s.clone()), admin(1), Query(HashMap::new()))
        .await
        .unwrap();
    assert_eq!(listed["items"][0]["pending"], 0);
}
#[tokio::test]
async fn api_context_schema_is_private_and_downgrade_guards_history() {
    use sea_orm_migration::{MigrationTrait, SchemaManager};
    let (_dir, s) = setup().await;
    let m = SchemaManager::new(&s.db);
    assert!(m.has_table("ai_chat_api_sessions").await.unwrap());
    assert!(m.has_column("ai_calls", "conversation_id").await.unwrap());
    s.db.execute_unprepared("INSERT INTO ai_chat_api_sessions (id,workspace_id,conversation_id,agent_id,model_profile_id,provider_id,model,encrypted_history) VALUES (1,1,2,3,4,5,'model','sealed')").await.unwrap();
    let err = crate::migration::chat_api::Migration
        .down(&m)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("history"));
    assert!(m.has_table("ai_chat_api_sessions").await.unwrap());
}

#[tokio::test]
async fn message_admission_has_no_hidden_63kib_ceiling_and_honors_resource_policy() {
    let (_dir, s, id) = seeded().await;
    let request = |prompt: String, client_id: &str| Send {
        reply_to: None,
        prompt,
        client_id: client_id.into(),
        attachments: vec![],
    };
    let accepted = send(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(request("x".repeat(128 * 1024), "large-default")),
    )
    .await;
    assert!(
        accepted.is_ok(),
        "default-unlimited message was rejected: {:?}",
        accepted.err()
    );
    crate::entities::platform_state::Entity::update_many()
        .col_expr(
            crate::entities::platform_state::Column::ResourceConfig,
            sea_orm::sea_query::Expr::value(Some(json!({"agent_prompt_bytes":1024}).to_string())),
        )
        .exec(&s.db)
        .await
        .unwrap();
    let rejected = send(
        State(s.clone()),
        admin(1),
        Path(("1".into(), id)),
        Json(request("x".repeat(2048), "configured-small")),
    )
    .await
    .unwrap_err();
    assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn both_engines_business_clock_context_is_workspace_scoped_and_legacy_stays_empty() {
    use crate::entities::workspaces;
    use sea_orm::sea_query::Expr;
    let (_dir, s) = setup().await;
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-01T16:30:12.345Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(
        run::workspace_clock_context(&s.db, 1.into(), at)
            .await
            .unwrap(),
        ""
    );
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::Timezone,
            Expr::value(Some("Asia/Kuala_Lumpur")),
        )
        .filter(workspaces::Column::Id.eq(1))
        .exec(&s.db)
        .await
        .unwrap();
    let context = run::workspace_clock_context(&s.db, 1.into(), at)
        .await
        .unwrap();
    assert!(context.contains("2026-11-02T00:30:12.345+08:00"));
    assert!(context.contains("2026-11-02T00:00:00.000+08:00"));
    assert!(context.contains("Asia/Kuala_Lumpur"));
    assert_eq!(
        run::workspace_clock_context(&s.db, 2.into(), at)
            .await
            .unwrap(),
        ""
    );
}

#[tokio::test]
async fn room_agent_sponsors_backfill_owner_and_mode_and_allow_one_copy_per_sponsor() {
    use crate::entities::ai_chat_agents as room_agents;
    use crate::migration::room_agent_sponsors::Migration;
    use sea_orm_migration::{MigrationTrait, SchemaManager};
    let (_dir, s, id) = seeded().await;
    let c = find(&s.db, &admin(1), id).await.unwrap();
    let mut room = c.clone().into_active_model();
    room.config = sea_orm::Set(r#"{"approval_mode":"assisted"}"#.into());
    room.update(&s.db).await.unwrap();
    let row = |row_id: i64, added_by: i64| room_agents::Model {
        id: row_id,
        workspace_id: 1,
        conversation_id: id.into(),
        agent_id: c.agent_id,
        revision_id: c.revision_id,
        config: c.config.clone(),
        binding: c.binding.clone(),
        native_session_id: String::new(),
        state: "active".into(),
        added_by,
        approval_mode: "request".into(),
    };
    // A row written before the migration has no sponsor.
    room_agents::Entity::insert(row(9001, 0).into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    let manager = SchemaManager::new(&s.db);
    Migration.up(&manager).await.unwrap();
    Migration.up(&manager).await.unwrap();
    let migrated = room_agents::Entity::find_by_id(9001_i64)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(migrated.added_by, c.owner_id);
    assert_eq!(migrated.approval_mode, "assisted");
    // Another developer may add the same Agent definition; the same sponsor may not twice.
    room_agents::Entity::insert(row(9002, c.owner_id + 1).into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        room_agents::Entity::insert(row(9003, c.owner_id).into_active_model())
            .exec(&s.db)
            .await
            .is_err()
    );
    assert!(manager.has_column("ai_chat_turns", "hop").await.unwrap());
    assert!(Migration.down(&manager).await.is_err());
}

/// A room with human members 1 (owner) and 2, and the seeded Agent sponsored by 1.
async fn sponsored_room() -> (tempfile::TempDir, AppState, Id, Scope, Scope) {
    use crate::entities::{
        ai_chat_agents as room_rows, ai_chat_members as members, workspace_members,
    };
    let (dir, s, id) = seeded().await;
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
    room_rows::Entity::insert(
        room_rows::Model {
            id: s.next_id().unwrap().into(),
            workspace_id: 1,
            conversation_id: id.into(),
            agent_id: c.agent_id,
            revision_id: c.revision_id,
            config: c.config.clone(),
            binding: c.binding.clone(),
            native_session_id: String::new(),
            state: "active".into(),
            added_by: 1,
            approval_mode: "request".into(),
        }
        .into_active_model(),
    )
    .exec(&s.db)
    .await
    .unwrap();
    let owner = admin(1);
    let mut member = admin(1);
    member.user.id = 2.into();
    (dir, s, id, owner, member)
}

fn message(json: Value) -> Json<groups::MessageBody> {
    Json(serde_json::from_value(json).unwrap())
}

#[tokio::test]
async fn members_bring_their_own_copy_of_an_agent_and_address_rows_exactly() {
    use super::room_agents;
    use crate::entities::{ai_chat_agents as room_rows, ai_chat_turns as links};
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let agent = find(&s.db, &owner, id).await.unwrap().agent_id;
    let (status, Json(added)) = room_agents::add(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(added["added_by"], json!(Id::from(2)));
    // The same sponsor cannot add the same Agent twice.
    assert!(
        room_agents::add(
            State(s.clone()),
            member.clone(),
            Path(("1".into(), id)),
            Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
        )
        .await
        .is_err()
    );
    let rows = room_rows::Entity::find()
        .filter(room_rows::Column::ConversationId.eq(id))
        .filter(room_rows::Column::State.ne("removed"))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    // A legacy definition target is ambiguous now.
    let error = groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"hi","client_id":"legacy","targets":[Id::from(agent)]})),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(error.2.unwrap().key, "conversations.ambiguousTarget");
    // Addressing the member's row reaches exactly that row.
    let row: Id = serde_json::from_value(added["id"].clone()).unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"hi","client_id":"exact","agents":[row]})),
    )
    .await
    .unwrap();
    let link = links::Entity::find()
        .filter(links::Column::ConversationId.eq(id))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(link.len(), 1);
    assert_eq!(link[0].chat_agent_id, i64::from(row));
}

#[tokio::test]
async fn outsiders_cannot_add_and_only_sponsor_or_owner_removes_room_agents() {
    use super::room_agents;
    use crate::entities::ai_chat_agents as room_rows;
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let agent = find(&s.db, &owner, id).await.unwrap().agent_id;
    let mut outsider = admin(1);
    outsider.user.id = 99.into();
    assert!(
        room_agents::add(
            State(s.clone()),
            outsider.clone(),
            Path(("1".into(), id)),
            Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
        )
        .await
        .is_err()
    );
    let owners_row = room_rows::Entity::find()
        .filter(room_rows::Column::ConversationId.eq(id))
        .filter(room_rows::Column::AddedBy.eq(1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    // A member cannot remove the owner's Agent...
    let error = room_agents::remove(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id, owners_row.id.into())),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    // ...but the owner can remove a member's Agent.
    let (_, Json(added)) = room_agents::add(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
    )
    .await
    .unwrap();
    let row: Id = serde_json::from_value(added["id"].clone()).unwrap();
    assert_eq!(
        room_agents::remove(State(s.clone()), owner.clone(), Path(("1".into(), id, row)))
            .await
            .unwrap(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn room_agents_run_as_their_sponsor_with_their_own_approval_mode() {
    use super::room_agents;
    use crate::entities::ai_chat_turns as links;
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let agent = find(&s.db, &owner, id).await.unwrap().agent_id;
    let (_, Json(added)) = room_agents::add(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
    )
    .await
    .unwrap();
    let row: Id = serde_json::from_value(added["id"].clone()).unwrap();
    // The owner loosens the room, then addresses the member's Agent.
    let mut room = find(&s.db, &owner, id).await.unwrap().into_active_model();
    room.config = sea_orm::Set(r#"{"approval_mode":"yolo"}"#.into());
    room.update(&s.db).await.unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"please","client_id":"cross","agents":[row]})),
    )
    .await
    .unwrap();
    let link = links::Entity::find()
        .filter(links::Column::ChatAgentId.eq(i64::from(row)))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let turn = turns::Entity::find_by_id(link.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        turn.owner_id, 2,
        "runs as the Agent's sponsor, not the sender"
    );
    let c = find(&s.db, &owner, id).await.unwrap();
    let execution = groups::execution(&s.db, &turn, c).await.unwrap();
    assert_eq!(
        approval::mode(&execution.config).unwrap(),
        "request",
        "a room-wide yolo never loosens another sponsor's Agent"
    );
    // Only the sponsor changes that Agent's mode.
    let patch = |mode: &str| Json(serde_json::from_value(json!({"approval_mode":mode})).unwrap());
    let error = room_agents::update(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id, row)),
        patch("yolo"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    let Json(updated) = room_agents::update(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id, row)),
        patch("assisted"),
    )
    .await
    .unwrap();
    assert_eq!(updated["approval_mode"], "assisted");
}

/// The member's own copy of the room Agent, with one queued turn addressed by the owner.
async fn member_agent_with_queued_turn(
    s: &AppState,
    id: Id,
    owner: &Scope,
    member: &Scope,
) -> (Id, i64) {
    use super::room_agents;
    use crate::entities::ai_chat_turns as links;
    let agent = find(&s.db, owner, id).await.unwrap().agent_id;
    let (_, Json(added)) = room_agents::add(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        Json(serde_json::from_value(json!({"agent_id":Id::from(agent)})).unwrap()),
    )
    .await
    .unwrap();
    let row: Id = serde_json::from_value(added["id"].clone()).unwrap();
    // Introductions may queue too; only the addressed message matters here.
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"work","client_id":"queued","agents":[row]})),
    )
    .await
    .unwrap();
    let turn = links::Entity::find()
        .filter(links::Column::ChatAgentId.eq(i64::from(row)))
        .filter(links::Column::MessageId.ne(0))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .id;
    (row, turn)
}

#[tokio::test]
async fn removing_a_member_removes_the_agents_they_sponsor() {
    use crate::entities::{ai_chat_agents as room_rows, ai_chat_members as people};
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let (row, _) = member_agent_with_queued_turn(&s, id, &owner, &member).await;
    // Membership edits wait for an idle room; queued work is handled at claim.
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    let version = people::Entity::find()
        .filter(people::Column::ConversationId.eq(id))
        .all(&s.db)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap();
    let agent = find(&s.db, &owner, id).await.unwrap().agent_id;
    interactions::members(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        Json(
            serde_json::from_value(
                json!({"version":Id::from(version),"member_ids":[],"agent_ids":[Id::from(agent)]}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let a = room_rows::Entity::find_by_id(i64::from(row))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.state, "removed");
}

#[tokio::test]
async fn claim_pauses_an_agent_whose_sponsor_left_the_workspace() {
    use crate::entities::{ai_chat_agents as room_rows, workspace_members};
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let (row, turn) = member_agent_with_queued_turn(&s, id, &owner, &member).await;
    // Earlier queued turns (introductions) would otherwise go first.
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::Id.ne(turn))
        .exec(&s.db)
        .await
        .unwrap();
    workspace_members::Entity::update_many()
        .col_expr(
            workspace_members::Column::State,
            sea_orm::sea_query::Expr::value("disabled"),
        )
        .filter(workspace_members::Column::UserId.eq(2))
        .exec(&s.db)
        .await
        .unwrap();
    assert!(run::claim(&s, turn.into()).await.unwrap().is_none());
    let a = room_rows::Entity::find_by_id(i64::from(row))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.state, "paused");
    let t = turns::Entity::find_by_id(turn)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.state, "cancelled");
    assert!(t.error.contains("conversations.sponsorUnavailable"));
}

#[tokio::test]
async fn the_same_room_agent_still_runs_one_turn_at_a_time() {
    let (_dir, s, id, owner, member) = sponsored_room().await;
    let (row, first) = member_agent_with_queued_turn(&s, id, &owner, &member).await;
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::Id.ne(first))
        .exec(&s.db)
        .await
        .unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"next","client_id":"second","agents":[row]})),
    )
    .await
    .unwrap();
    let second = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::State.eq("queued"))
        .filter(turns::Column::Id.ne(first))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert!(run::claim(&s, first.into()).await.unwrap().is_some());
    assert!(run::claim(&s, second.id.into()).await.unwrap().is_none());
}

/// The owner's Agent running a turn, plus the member's Agent row as a relay target.
async fn running_caller_and_target() -> (
    tempfile::TempDir,
    AppState,
    Id,
    Scope,
    Scope,
    turns::Model,
    Id,
) {
    use crate::entities::ai_chat_agents as room_rows;
    let (dir, s, id, owner, member) = sponsored_room().await;
    let (target, _) = member_agent_with_queued_turn(&s, id, &owner, &member).await;
    let caller_row = room_rows::Entity::find()
        .filter(room_rows::Column::ConversationId.eq(id))
        .filter(room_rows::Column::AddedBy.eq(1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    // Only the caller's turn should run; clear everything else first.
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"build the API","client_id":"caller","agents":[Id::from(caller_row.id)]})),
    )
    .await
    .unwrap();
    let queued = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::State.eq("queued"))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    let (running, _, _) = run::claim(&s, queued.id.into()).await.unwrap().unwrap();
    (dir, s, id, owner, member, running, target)
}

fn relay_call(conversation: Id, agents: Vec<Id>, text: &str) -> crate::platform_tools::Call {
    crate::platform_tools::Call {
        method: "POST".into(),
        path: format!("/conversations/{conversation}/relay"),
        body: json!({"agents":agents,"text":text}),
        transfer: None,
    }
}

async fn relayed_turns(
    s: &AppState,
    target: Id,
) -> Vec<(turns::Model, crate::entities::ai_chat_turns::Model)> {
    use crate::entities::ai_chat_turns as links;
    let mut out = vec![];
    for link in links::Entity::find()
        .filter(links::Column::ChatAgentId.eq(i64::from(target)))
        .filter(links::Column::CallerChatAgentId.ne(0))
        .all(&s.db)
        .await
        .unwrap()
    {
        let turn = turns::Entity::find_by_id(link.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        out.push((turn, link));
    }
    out
}

#[tokio::test]
async fn agents_relay_to_each_other_without_approval_as_the_target_sponsor() {
    let (_dir, s, id, _owner, _member, running, target) = running_caller_and_target().await;
    let result = relay::intercept(
        &s,
        &running,
        &relay_call(id, vec![target], "GET /orders returns 500"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["data"]["held"], json!([]));
    let delivered = relayed_turns(&s, target).await;
    assert_eq!(delivered.len(), 1);
    let (turn, link) = &delivered[0];
    assert_eq!(turn.state, "queued");
    assert_eq!(turn.owner_id, 2, "runs as the target's sponsor");
    assert_eq!(link.hop, 1);
    // No approval was requested for the relay itself.
    let caller = turns::Entity::find_by_id(running.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(caller.decisions, "{}");
    // The target is told who wrote, and later turns see the relay in context.
    assert!(turn.prompt.contains("GET /orders returns 500"));
    assert!(turn.prompt.contains("Agent"), "{}", turn.prompt);
    relay::intercept(&s, &running, &relay_call(id, vec![target], "fixed?"))
        .await
        .unwrap()
        .unwrap();
    let later = relayed_turns(&s, target).await.pop().unwrap().0;
    let context = groups::context(&s.db, &later, 64 * 1024).await.unwrap();
    assert!(context.contains("GET /orders returns 500"), "{context}");
    // The target also learns who else is in the room and how to relay back.
    assert!(context.contains("<room_agents>"), "{context}");
    assert!(
        context.contains(&format!("POST /conversations/{id}/relay")),
        "{context}"
    );
    // Self, unknown and other-conversation relays are refused.
    let caller_row = crate::entities::ai_chat_turns::Entity::find_by_id(running.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap()
        .chat_agent_id;
    for call in [
        relay_call(id, vec![caller_row.into()], "self"),
        relay_call(id, vec![Id::from(424242)], "unknown"),
        relay_call(Id::from(1), vec![target], "elsewhere"),
    ] {
        let result = relay::intercept(&s, &running, &call)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["ok"], false, "{result}");
    }
}

#[tokio::test]
async fn relay_holds_past_the_hop_limit_or_when_off_until_a_human_continues() {
    use crate::entities::ai_chat_turns as links;
    let (_dir, s, id, owner, _member, running, target) = running_caller_and_target().await;
    links::Entity::update_many()
        .col_expr(links::Column::Hop, sea_orm::sea_query::Expr::value(10_i64))
        .filter(links::Column::Id.eq(running.id))
        .exec(&s.db)
        .await
        .unwrap();
    let result = relay::intercept(&s, &running, &relay_call(id, vec![target], "hop eleven"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["data"]["queued"], json!([]));
    let delivered = relayed_turns(&s, target).await;
    assert_eq!(delivered[0].0.state, "held");
    // Relay switched off holds even a first hop.
    links::Entity::update_many()
        .col_expr(links::Column::Hop, sea_orm::sea_query::Expr::value(0_i64))
        .filter(links::Column::Id.eq(running.id))
        .exec(&s.db)
        .await
        .unwrap();
    relay::switch(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        Json(serde_json::from_value(json!({"enabled":false})).unwrap()),
    )
    .await
    .unwrap();
    let off = relay::intercept(&s, &running, &relay_call(id, vec![target], "while off"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(off["ok"], true, "{off}");
    assert!(
        relayed_turns(&s, target)
            .await
            .iter()
            .all(|(t, _)| t.state == "held")
    );
    // Continue delivers everything held as a new chain.
    let Json(resumed) = relay::resume(State(s.clone()), owner.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(resumed["queued"], 2);
    for (turn, link) in relayed_turns(&s, target).await {
        assert_eq!(turn.state, "queued");
        assert_eq!(link.hop, 0);
    }
}

#[tokio::test]
async fn room_threads_migration_is_idempotent_and_unique_per_resource() {
    use crate::entities::ai_chat_threads as threads;
    use crate::migration::room_threads::Migration;
    use sea_orm_migration::{MigrationTrait, SchemaManager};
    let (_dir, s, id) = seeded().await;
    let manager = SchemaManager::new(&s.db);
    Migration.up(&manager).await.unwrap();
    let thread = |row: i64| threads::Model {
        id: row,
        workspace_id: 1,
        conversation_id: id.into(),
        kind: "pipeline".into(),
        resource_id: 55,
        linked: 1,
        created_by: 1,
        created_at: 0,
    };
    threads::Entity::insert(thread(1).into_active_model())
        .exec(&s.db)
        .await
        .unwrap();
    assert!(
        threads::Entity::insert(thread(2).into_active_model())
            .exec(&s.db)
            .await
            .is_err()
    );
    assert!(Migration.down(&manager).await.is_err());
}

fn ok_created(id: &str) -> Value {
    json!({"status":201,"ok":true,"data":{"id":id}})
}

#[tokio::test]
async fn room_platform_calls_link_threads_and_runs_without_touching_resources() {
    use crate::entities::{ai_chat_run_links as run_links, ai_chat_threads as thread_rows};
    let (_dir, s, id, owner, _member, running, _target) = running_caller_and_target().await;
    super::threads::record(&s, &running, "POST", "/pipelines", &ok_created("777")).await;
    super::threads::record(
        &s,
        &running,
        "POST",
        "/pipelines/999/runs",
        &ok_created("888"),
    )
    .await;
    // Failed calls and reads record nothing.
    super::threads::record(
        &s,
        &running,
        "POST",
        "/workflows",
        &json!({"ok":false,"status":403,"data":{}}),
    )
    .await;
    super::threads::record(&s, &running, "GET", "/pipelines", &ok_created("1")).await;
    let rows = thread_rows::Entity::find()
        .filter(thread_rows::Column::ConversationId.eq(id))
        .order_by_asc(thread_rows::Column::ResourceId)
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|t| (t.resource_id, t.linked))
            .collect::<Vec<_>>(),
        vec![(777, 1), (999, 0)]
    );
    let link = run_links::Entity::find_by_id(888_i64)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link.kind, "pipeline_run");
    assert!(link.notified_at.is_none());
    // Room detail shows both threads; the start notices sit inside them.
    let Json(room) = detail(State(s.clone()), owner.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(room["threads"].as_array().unwrap().len(), 2);
    let threaded = room["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| !m["thread_id"].is_null())
        .count();
    assert_eq!(threaded, 2);
    // A human can write into a thread.
    let thread: Id = serde_json::from_value(room["threads"][0]["id"].clone()).unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(json!({"prompt":"why did it fail?","client_id":"in-thread","thread_id":thread})),
    )
    .await
    .unwrap();
    let Json(room) = detail(State(s.clone()), owner.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    let posted = room["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["prompt"] == "why did it fail?")
        .unwrap()
        .clone();
    assert_eq!(posted["thread_id"], json!(thread));
}

#[tokio::test]
async fn finished_room_runs_are_reported_once_and_wake_the_starting_agent() {
    use crate::entities::{ai_chat_run_links as run_links, ai_chat_turns as links};
    let (_dir, s, _id, _owner, _member, running, _target) = running_caller_and_target().await;
    // A run id with no run row counts as finished (deleted) and must not stay pending.
    super::threads::record(
        &s,
        &running,
        "POST",
        "/pipelines/999/runs",
        &ok_created("888"),
    )
    .await;
    let caller = links::Entity::find_by_id(running.id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::threads::sweep(&s).await.unwrap(), 1);
    assert_eq!(
        super::threads::sweep(&s).await.unwrap(),
        0,
        "reported exactly once"
    );
    assert!(
        run_links::Entity::find_by_id(888_i64)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
            .notified_at
            .is_some()
    );
    let followups = links::Entity::find()
        .filter(links::Column::ChatAgentId.eq(caller.chat_agent_id))
        .filter(links::Column::Id.ne(running.id))
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(followups.len(), 1);
    assert_eq!(followups[0].hop, caller.hop + 1);
    let turn = turns::Entity::find_by_id(followups[0].id)
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.state, "queued");
    assert!(turn.prompt.contains("/pipeline-runs/"), "{}", turn.prompt);
}

#[tokio::test]
async fn deleting_a_room_removes_threads_but_not_the_resources() {
    use crate::entities::{ai_chat_run_links as run_links, ai_chat_threads as thread_rows};
    let (_dir, s, id, owner, _member, running, _target) = running_caller_and_target().await;
    super::threads::record(&s, &running, "POST", "/pipelines", &ok_created("777")).await;
    super::threads::record(
        &s,
        &running,
        "POST",
        "/pipelines/777/runs",
        &ok_created("888"),
    )
    .await;
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("completed"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    remove(State(s.clone()), owner.clone(), Path(("1".into(), id)))
        .await
        .unwrap();
    assert_eq!(thread_rows::Entity::find().count(&s.db).await.unwrap(), 0);
    assert_eq!(run_links::Entity::find().count(&s.db).await.unwrap(), 0);
}

#[tokio::test]
async fn room_agents_sharing_a_directory_take_turns() {
    use super::room_agents;
    let (_dir, s, id, owner, member) = sponsored_room().await;
    // The member's copy uses the same directory as the owner's Agent.
    let (_, Json(added)) = room_agents::add(
        State(s.clone()),
        member.clone(),
        Path(("1".into(), id)),
        Json(
            serde_json::from_value(
                json!({"agent_id":Id::from(find(&s.db, &owner, id).await.unwrap().agent_id)}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    let theirs: Id = serde_json::from_value(added["id"].clone()).unwrap();
    let mine = crate::entities::ai_chat_agents::Entity::find()
        .filter(crate::entities::ai_chat_agents::Column::ConversationId.eq(id))
        .filter(crate::entities::ai_chat_agents::Column::AddedBy.eq(1))
        .one(&s.db)
        .await
        .unwrap()
        .unwrap();
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&s.db)
        .await
        .unwrap();
    groups::send_message(
        State(s.clone()),
        owner.clone(),
        Path(("1".into(), id)),
        message(
            json!({"prompt":"both","client_id":"shared-dir","agents":[Id::from(mine.id), theirs]}),
        ),
    )
    .await
    .unwrap();
    let queued = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::State.eq("queued"))
        .order_by_asc(turns::Column::Id)
        .all(&s.db)
        .await
        .unwrap();
    assert_eq!(queued.len(), 2);
    assert!(run::claim(&s, queued[0].id.into()).await.unwrap().is_some());
    assert!(
        run::claim(&s, queued[1].id.into()).await.unwrap().is_none(),
        "the second Agent waits instead of failing on the shared directory"
    );
}
