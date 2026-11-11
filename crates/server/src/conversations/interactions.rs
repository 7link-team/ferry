//! Human chat interactions. Execution prompts and native histories stay immutable.
use super::*;
use crate::entities::{ai_chat_messages as chat, ai_chat_reads as reads, ai_chat_saves as saves};
use sea_orm::sea_query::OnConflict;

pub(super) async fn counts(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    rooms: &[conversations::Model],
) -> Result<HashMap<i64, (usize, usize)>> {
    if rooms.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<i64> = rooms.iter().map(|c| c.id).collect();
    let read = reads::Entity::find()
        .filter(reads::Column::ConversationId.is_in(ids.clone()))
        .filter(reads::Column::UserId.eq(scope.user.id))
        .all(conn)
        .await?;
    let rows = chat::Entity::find()
        .select_only()
        .columns([
            chat::Column::ConversationId,
            chat::Column::Id,
            chat::Column::OwnerId,
            chat::Column::Role,
            chat::Column::Mentions,
        ])
        .filter(chat::Column::ConversationId.is_in(ids))
        .filter(chat::Column::RetractedAt.is_null())
        .into_tuple::<(i64, i64, i64, String, String)>()
        .all(conn)
        .await?;
    let mut result = HashMap::new();
    for (room, id, owner, role, mentions) in rows {
        if id
            <= read
                .iter()
                .find(|r| r.conversation_id == room)
                .map_or(0, |r| r.message_id)
            || (role == "user" && owner == i64::from(scope.user.id))
        {
            continue;
        }
        let counts = result.entry(room).or_insert((0, 0));
        counts.0 += 1;
        if serde_json::from_str::<Vec<Id>>(&mentions)
            .unwrap_or_default()
            .contains(&scope.user.id)
        {
            counts.1 += 1;
        }
    }
    Ok(result)
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/conversations/{id}/read", post(mark_read))
        .route(
            "/{workspace}/conversations/{id}/messages/{message}",
            axum::routing::patch(edit).delete(retract),
        )
        .route(
            "/{workspace}/conversations/{id}/messages/{message}/saved",
            axum::routing::put(save).delete(unsave),
        )
        .route(
            "/{workspace}/conversations/{id}/members",
            axum::routing::put(members),
        )
}
pub(super) async fn parent(conn: &impl ConnectionTrait, id: Id, parent: Option<Id>) -> Result<()> {
    if let Some(parent) = parent {
        let m = message(conn, id, parent).await?;
        if m.retracted_at.is_some() {
            return Err(conflict());
        }
    }
    Ok(())
}
async fn message(conn: &impl ConnectionTrait, id: Id, message: Id) -> Result<chat::Model> {
    chat::Entity::find_by_id(message)
        .filter(chat::Column::ConversationId.eq(id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(super) async fn decorate(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    c: &conversations::Model,
    value: &mut Value,
) -> Result<()> {
    let read = reads::Entity::find_by_id((c.id, i64::from(scope.user.id)))
        .one(conn)
        .await?
        .map_or(0, |r| r.message_id);
    let saved = saves::Entity::find()
        .filter(saves::Column::ConversationId.eq(c.id))
        .filter(saves::Column::UserId.eq(scope.user.id))
        .all(conn)
        .await?;
    let messages = chat::Entity::find()
        .filter(chat::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    value["last_read_id"] = json!(Id::from(read));
    if let Some(rows) = value["messages"].as_array_mut() {
        for v in rows {
            if let Some(m) = messages.iter().find(|m| json!(Id::from(m.id)) == v["id"]) {
                v["version"] = json!(m.version);
                v["edited_at"] = json!(m.edited_at);
                v["retracted_at"] = json!(m.retracted_at);
                if m.retracted_at.is_some() {
                    v["attachments"] = json!([]);
                }
                v["mentions"] = serde_json::from_str(&m.mentions).unwrap_or(json!([]));
                v["saved"] = json!(saved.iter().any(|s| s.message_id == m.id));
                v["unread"] = json!(
                    m.id > read
                        && m.retracted_at.is_none()
                        && (m.role == "assistant" || m.owner_id != i64::from(scope.user.id))
                );
            }
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    message_id: Id,
}
async fn mark_read(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Read>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    find(&tx, &scope, id).await?;
    message(&tx, id, body.message_id).await?;
    let old = reads::Entity::find_by_id((i64::from(id), i64::from(scope.user.id)))
        .one(&tx)
        .await?;
    let latest = old.map_or(i64::from(body.message_id), |r| {
        r.message_id.max(body.message_id.into())
    });
    reads::Entity::insert(
        reads::Model {
            conversation_id: id.into(),
            workspace_id: scope.workspace_id.into(),
            user_id: scope.user.id.into(),
            message_id: latest,
        }
        .into_active_model(),
    )
    .on_conflict(
        OnConflict::columns([reads::Column::ConversationId, reads::Column::UserId])
            .update_column(reads::Column::MessageId)
            .to_owned(),
    )
    .exec(&tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    prompt: String,
    version: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    version: i64,
}
async fn editable(
    scope: &Scope,
    conn: &impl ConnectionTrait,
    id: Id,
    msg: Id,
    version: i64,
) -> Result<chat::Model> {
    scope.require("conversations.run")?;
    let c = find(conn, scope, id).await?;
    let row = message(conn, id, msg).await?;
    if row.owner_id != i64::from(scope.user.id) || row.role != "user" {
        return Err(ApiError::missing());
    }
    if row.version != version || row.retracted_at.is_some() || c.state != "active" {
        return Err(conflict());
    }
    if has_active(conn, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    Ok(row)
}
async fn edit(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, msg)): Path<(String, Id, Id)>,
    Json(body): Json<Edit>,
) -> Result<Json<Value>> {
    if body.prompt.trim().is_empty() || body.prompt.len() > 48 * 1024 || body.prompt.contains('\0')
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut row = editable(&scope, &tx, id, msg, body.version).await?;
    row.prompt = body.prompt;
    row.edited_at = Some(db::now());
    row.version += 1;
    row.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "edit-message",
        "conversations",
        &id,
        json!({"message_id":msg}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn retract(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, msg)): Path<(String, Id, Id)>,
    Json(body): Json<Version>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut row = editable(&scope, &tx, id, msg, body.version).await?;
    row.prompt.clear();
    row.targets = "[]".into();
    row.mentions = "[]".into();
    row.retracted_at = Some(db::now());
    row.version += 1;
    row.into_active_model().reset_all().update(&tx).await?;
    saves::Entity::delete_many()
        .filter(saves::Column::MessageId.eq(msg))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "retract-message",
        "conversations",
        &id,
        json!({"message_id":msg}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn save(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, msg)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    find(&tx, &scope, id).await?;
    let row = message(&tx, id, msg).await?;
    if row.retracted_at.is_some() {
        return Err(conflict());
    }
    let key = (i64::from(msg), i64::from(scope.user.id));
    if saves::Entity::find_by_id(key).one(&tx).await?.is_none() {
        saves::Entity::insert(
            saves::Model {
                message_id: msg.into(),
                user_id: scope.user.id.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn unsave(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, msg)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    find(&tx, &scope, id).await?;
    message(&tx, id, msg).await?;
    saves::Entity::delete_by_id((i64::from(msg), i64::from(scope.user.id)))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Members {
    version: Id,
    member_ids: Vec<Id>,
    agent_ids: Vec<Id>,
}
pub(super) async fn members(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<Members>,
) -> Result<Json<Value>> {
    use crate::entities::{
        ai_chat_agents as agents, ai_chat_members as people, users, workspace_members,
    };
    scope.require("conversations.update")?;
    let c = find(&s.db, &scope, id).await?;
    if c.owner_id != i64::from(scope.user.id) || !groups::is_group(&s.db, id).await? {
        return Err(ApiError::missing());
    }
    body.member_ids.push(scope.user.id);
    body.member_ids.sort();
    body.member_ids.dedup();
    body.agent_ids.sort();
    body.agent_ids.dedup();
    if body.member_ids.len() > 100 || body.agent_ids.len() > 8 {
        return Err(invalid());
    }
    let existing = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(id))
        .all(&s.db)
        .await?;
    let mut additions = vec![];
    for agent_id in &body.agent_ids {
        if existing
            .iter()
            .any(|a| a.agent_id == i64::from(*agent_id) && a.state != "removed")
        {
            continue;
        }
        let a = available(&s.db, &scope, *agent_id).await?;
        if a.visibility != "workspace" {
            return Err(error(
                StatusCode::CONFLICT,
                "conversations.groupAgentPrivate",
            ));
        }
        let create: Create =
            serde_json::from_value(json!({"agent_id":agent_id})).map_err(|_| invalid())?;
        let (config, binding) = prepare_execution(&s, &scope, &a, &create, id).await?;
        additions.push((a, config, binding));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    let membership_version = people::Entity::find()
        .filter(people::Column::ConversationId.eq(id))
        .order_by_desc(people::Column::Id)
        .one(&tx)
        .await?
        .map_or(0, |m| m.id);
    if membership_version != i64::from(body.version) {
        return Err(conflict());
    }
    if has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    for user in &body.member_ids {
        if *user == scope.user.id && scope.user.is_superadmin {
            continue;
        }
        if workspace_members::Entity::find()
            .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(workspace_members::Column::UserId.eq(*user))
            .filter(workspace_members::Column::State.eq("active"))
            .count(&tx)
            .await?
            == 0
        {
            return Err(ApiError::missing());
        }
    }
    let previous_people: Vec<i64> = people::Entity::find()
        .select_only()
        .column(people::Column::UserId)
        .filter(people::Column::ConversationId.eq(id))
        .into_tuple()
        .all(&tx)
        .await?;
    let previous_agents: Vec<i64> = agents::Entity::find()
        .select_only()
        .column(agents::Column::AgentId)
        .filter(agents::Column::ConversationId.eq(id))
        .filter(agents::Column::State.ne("removed"))
        .into_tuple()
        .all(&tx)
        .await?;
    let next_people: Vec<i64> = body.member_ids.iter().copied().map(i64::from).collect();
    let next_agents: Vec<i64> = body.agent_ids.iter().copied().map(i64::from).collect();
    let people_names: HashMap<i64, String> = users::Entity::find()
        .select_only()
        .columns([users::Column::Id, users::Column::Name])
        .filter(users::Column::Id.is_in(previous_people.iter().chain(&next_people).copied()))
        .into_tuple::<(i64, String)>()
        .all(&tx)
        .await?
        .into_iter()
        .collect();
    let agent_names: HashMap<i64, String> = ai_agents::Entity::find()
        .select_only()
        .columns([ai_agents::Column::Id, ai_agents::Column::Name])
        .filter(ai_agents::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(ai_agents::Column::Id.is_in(previous_agents.iter().chain(&next_agents).copied()))
        .into_tuple::<(i64, String)>()
        .all(&tx)
        .await?
        .into_iter()
        .collect();
    let changes = |before: &[i64], after: &[i64], names: &HashMap<i64, String>| {
        json!({
            "added": after.iter().copied().filter(|id| !before.contains(id))
                .map(|id| json!({"id":Id::from(id),"name":names.get(&id)})).collect::<Vec<_>>(),
            "removed": before.iter().copied().filter(|id| !after.contains(id))
                .map(|id| json!({"id":Id::from(id),"name":names.get(&id)})).collect::<Vec<_>>(),
        })
    };
    let membership = json!({
        "members": changes(&previous_people, &next_people, &people_names),
        "agents": changes(&previous_agents, &next_agents, &agent_names),
    });
    let title = c.title.clone();
    people::Entity::delete_many()
        .filter(people::Column::ConversationId.eq(id))
        .exec(&tx)
        .await?;
    for user in body.member_ids {
        people::Entity::insert(
            people::Model {
                id: s.next_id()?.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                user_id: user.into(),
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
    }
    for mut a in existing {
        if !body.agent_ids.contains(&Id::from(a.agent_id)) {
            a.state = "removed".into();
            a.into_active_model().reset_all().update(&tx).await?;
        }
    }
    for (a, config, binding) in additions {
        if available(&tx, &scope, a.id.into()).await?.revision_id != a.revision_id {
            return Err(conflict());
        }
        if let Some(mut old) = agents::Entity::find()
            .filter(agents::Column::ConversationId.eq(id))
            .filter(agents::Column::AgentId.eq(a.id))
            .one(&tx)
            .await?
        {
            old.state = "active".into();
            old.config = serde_json::to_string(&config).unwrap();
            old.binding = serde_json::to_string(&binding).unwrap();
            old.revision_id = a.revision_id;
            old.native_session_id.clear();
            old.into_active_model().reset_all().update(&tx).await?;
        } else {
            agents::Entity::insert(
                agents::Model {
                    id: s.next_id()?.into(),
                    workspace_id: scope.workspace_id.into(),
                    conversation_id: id.into(),
                    agent_id: a.id,
                    revision_id: a.revision_id,
                    config: serde_json::to_string(&config).unwrap(),
                    binding: serde_json::to_string(&binding).unwrap(),
                    native_session_id: String::new(),
                    state: "active".into(),
                }
                .into_active_model(),
            )
            .exec(&tx)
            .await?;
        }
    }
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "members",
        "conversations",
        &id,
        json!({"name":title,"membership":membership}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

pub(super) async fn delete(conn: &impl ConnectionTrait, id: Id) -> Result<()> {
    reads::Entity::delete_many()
        .filter(reads::Column::ConversationId.eq(id))
        .exec(conn)
        .await?;
    saves::Entity::delete_many()
        .filter(saves::Column::ConversationId.eq(id))
        .exec(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::admin;
    use crate::conversations::tests::{seeded, sent};
    #[tokio::test]
    async fn membership_audit_records_names_and_actual_deltas_in_the_write_transaction() {
        use crate::entities::{activity, ai_chat_agents as agents, ai_chat_members as people};
        let (_dir, s, id) = seeded().await;
        let c = find(&s.db, &admin(1), id).await.unwrap();
        let version = s.next_id().unwrap();
        people::Entity::insert(
            people::Model {
                id: version.into(),
                workspace_id: 1,
                conversation_id: id.into(),
                user_id: 2,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        agents::Entity::insert(
            agents::Model {
                id: s.next_id().unwrap().into(),
                workspace_id: 1,
                conversation_id: id.into(),
                agent_id: c.agent_id,
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
        let mut owner = admin(1);
        owner.user.is_superadmin = true;
        let request = || {
            Json(Members {
                version,
                member_ids: vec![],
                agent_ids: vec![],
            })
        };
        members(
            State(s.clone()),
            owner.clone(),
            Path(("1".into(), id)),
            request(),
        )
        .await
        .unwrap();
        let row = activity::Entity::find()
            .filter(activity::Column::ResourceId.eq(id))
            .filter(activity::Column::Action.eq("members"))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let details: Value = serde_json::from_str(&row.details).unwrap();
        assert_eq!(details["name"], c.title);
        assert_eq!(details["membership"]["members"]["added"][0]["id"], "1");
        assert!(details["membership"]["members"]["added"][0]["name"].is_string());
        assert_eq!(details["membership"]["members"]["removed"][0]["id"], "2");
        assert_eq!(
            details["membership"]["agents"]["removed"][0]["id"],
            json!(Id::from(c.agent_id))
        );
        assert_eq!(
            details["membership"]["agents"]["removed"][0]["name"],
            "Test Agent"
        );
        assert!(
            members(State(s.clone()), owner, Path(("1".into(), id)), request())
                .await
                .is_err()
        );
        assert_eq!(
            activity::Entity::find()
                .filter(activity::Column::ResourceId.eq(id))
                .filter(activity::Column::Action.eq("members"))
                .count(&s.db)
                .await
                .unwrap(),
            1
        );
    }
    use sea_orm::sea_query::Expr;

    async fn idle(s: &AppState, id: Id) {
        turns::Entity::update_many()
            .col_expr(turns::Column::State, Expr::value("cancelled"))
            .filter(turns::Column::ConversationId.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn chat_edit_retract_versions_and_execution_separation() {
        let (_dir, s, id) = seeded().await;
        let msg = sent(&s, id, "interaction").await;
        assert_eq!(
            edit(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, msg)),
                Json(Edit {
                    prompt: "edited".into(),
                    version: 1
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::CONFLICT
        );
        idle(&s, id).await;
        let mut other = admin(1);
        other.user.id = 2.into();
        assert_eq!(
            edit(
                State(s.clone()),
                other,
                Path(("1".into(), id, msg)),
                Json(Edit {
                    prompt: "intruder".into(),
                    version: 1
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::NOT_FOUND
        );
        edit(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, msg)),
            Json(Edit {
                prompt: "edited".into(),
                version: 1,
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            turns::Entity::find_by_id(msg)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .prompt,
            "hello"
        );
        assert_eq!(
            edit(
                State(s.clone()),
                admin(1),
                Path(("1".into(), id, msg)),
                Json(Edit {
                    prompt: "stale".into(),
                    version: 1
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::CONFLICT
        );
        save(State(s.clone()), admin(1), Path(("1".into(), id, msg)))
            .await
            .unwrap();
        save(State(s.clone()), admin(1), Path(("1".into(), id, msg)))
            .await
            .unwrap();
        let view = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap()
            .0;
        assert_eq!(view["messages"][0]["prompt"], "edited");
        assert_eq!(view["messages"][0]["saved"], true);
        assert_eq!(view["messages"][0]["version"], 2);
        retract(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id, msg)),
            Json(Version { version: 2 }),
        )
        .await
        .unwrap();
        let m = message(&s.db, id, msg).await.unwrap();
        assert!(m.retracted_at.is_some());
        assert!(m.prompt.is_empty());
        assert_eq!(saves::Entity::find().count(&s.db).await.unwrap(), 0);
        assert!(
            save(State(s.clone()), admin(1), Path(("1".into(), id, msg)))
                .await
                .is_err()
        );
        assert_eq!(
            turns::Entity::find_by_id(msg)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .prompt,
            "hello"
        );
    }
    #[tokio::test]
    async fn chat_reads_saves_mentions_are_personal_and_read_position_never_regresses() {
        use crate::entities::ai_chat_members;
        let (_dir, s, id) = seeded().await;
        let first = sent(&s, id, "first").await;
        idle(&s, id).await;
        let second = sent(&s, id, "second").await;
        idle(&s, id).await;
        ai_chat_members::Entity::insert(
            ai_chat_members::Model {
                id: 8,
                workspace_id: 1,
                conversation_id: id.into(),
                user_id: 2,
            }
            .into_active_model(),
        )
        .exec(&s.db)
        .await
        .unwrap();
        chat::Entity::update_many()
            .col_expr(chat::Column::Mentions, Expr::value("[\"2\"]"))
            .filter(chat::Column::Id.eq(second))
            .exec(&s.db)
            .await
            .unwrap();
        let mut peer = admin(1);
        peer.user.id = 2.into();
        let c = find(&s.db, &peer, id).await.unwrap();
        let totals = counts(&s.db, &peer, std::slice::from_ref(&c))
            .await
            .unwrap();
        assert_eq!(totals[&c.id], (2, 1));
        save(
            State(s.clone()),
            peer.clone(),
            Path(("1".into(), id, first)),
        )
        .await
        .unwrap();
        assert_eq!(
            detail(State(s.clone()), admin(1), Path(("1".into(), id)))
                .await
                .unwrap()
                .0["messages"][0]["saved"],
            false
        );
        assert_eq!(
            detail(State(s.clone()), peer.clone(), Path(("1".into(), id)))
                .await
                .unwrap()
                .0["messages"][0]["saved"],
            true
        );
        mark_read(
            State(s.clone()),
            peer.clone(),
            Path(("1".into(), id)),
            Json(Read { message_id: second }),
        )
        .await
        .unwrap();
        mark_read(
            State(s.clone()),
            peer.clone(),
            Path(("1".into(), id)),
            Json(Read { message_id: first }),
        )
        .await
        .unwrap();
        assert_eq!(
            reads::Entity::find_by_id((i64::from(id), 2))
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .message_id,
            i64::from(second)
        );
        assert!(
            !counts(&s.db, &peer, &[c])
                .await
                .unwrap()
                .contains_key(&i64::from(id))
        );
    }
    #[tokio::test]
    async fn chat_reply_rejects_other_room_and_preserves_parent_after_restart_read() {
        let (_dir, s, id) = seeded().await;
        let root = sent(&s, id, "root").await;
        idle(&s, id).await;
        let (_, Json(reply)) = send(
            State(s.clone()),
            admin(1),
            Path(("1".into(), id)),
            Json(Send {
                prompt: "reply".into(),
                client_id: "reply".into(),
                attachments: vec![],
                reply_to: Some(root),
            }),
        )
        .await
        .unwrap();
        let msg: Id = serde_json::from_value(reply["id"].clone()).unwrap();
        assert_eq!(
            message(&s.db, id, msg).await.unwrap().reply_to,
            Some(root.into())
        );
        assert!(parent(&s.db, 1.into(), Some(root)).await.is_err());
        assert!(parent(&s.db, id, Some(1.into())).await.is_err());
    }
}
