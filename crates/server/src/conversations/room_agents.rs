//! Room Agents brought by individual developers (their sponsors). Each row is
//! one Agent in one room for one sponsor; the same Agent definition may be in
//! a room once per sponsor, so turns, approvals and addressing use the row id.
use super::*;
use crate::entities::ai_chat_agents as agents;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{workspace}/conversations/{id}/agents",
            axum::routing::post(add),
        )
        .route(
            "/{workspace}/conversations/{id}/agents/{agent}",
            axum::routing::delete(remove),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddBody {
    agent_id: Id,
}

/// Whether the caller takes part in the room (owner or human member).
async fn participant(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    c: &conversations::Model,
) -> Result<()> {
    if c.owner_id == i64::from(scope.user.id) || groups::member(conn, c, scope.user.id).await? {
        Ok(())
    } else {
        Err(ApiError::missing())
    }
}

pub(super) fn view(a: &agents::Model) -> Value {
    json!({"id":Id::from(a.id),"agent_id":Id::from(a.agent_id),"added_by":Id::from(a.added_by),
        "approval_mode":a.approval_mode,"state":a.state})
}

pub(super) async fn add(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<AddBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    let c = find(&s.db, &scope, id).await?;
    if !groups::is_group(&s.db, id).await? {
        return Err(ApiError::missing());
    }
    participant(&s.db, &scope, &c).await?;
    let agent = available(&s.db, &scope, body.agent_id).await?;
    // Private Agents cannot be exposed to other room participants.
    if agent.visibility != "workspace" {
        return Err(error(
            StatusCode::CONFLICT,
            "conversations.groupAgentPrivate",
        ));
    }
    let create: Create =
        serde_json::from_value(json!({"agent_id":body.agent_id})).map_err(|_| invalid())?;
    let config = prepare_execution(&s, &scope, &agent, &create).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    if c.state != "active" {
        return Err(error(StatusCode::CONFLICT, "conversations.archived"));
    }
    let rows = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(id))
        .all(&tx)
        .await?;
    if rows.iter().filter(|a| a.state != "removed").count() >= 8 {
        return Err(invalid());
    }
    let mode = super::approval::mode(&c.config)?;
    let row = match rows
        .into_iter()
        .find(|a| a.agent_id == agent.id && a.added_by == i64::from(scope.user.id))
    {
        Some(a) if a.state != "removed" => return Err(conflict()),
        // Re-adding starts a fresh context, as the owner's member edit does.
        Some(mut a) => {
            a.state = "active".into();
            a.config = serde_json::to_string(&config).unwrap();
            a.binding = super::placement::UNBOUND.into();
            a.revision_id = agent.revision_id;
            a.native_session_id.clear();
            a.approval_mode = mode;
            a.clone()
                .into_active_model()
                .reset_all()
                .update(&tx)
                .await?;
            a
        }
        None => {
            let a = agents::Model {
                id: s.next_id()?.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                agent_id: agent.id,
                revision_id: agent.revision_id,
                config: serde_json::to_string(&config).unwrap(),
                binding: super::placement::UNBOUND.into(),
                native_session_id: String::new(),
                state: "active".into(),
                added_by: scope.user.id.into(),
                approval_mode: mode,
            };
            agents::Entity::insert(a.clone().into_active_model())
                .exec(&tx)
                .await?;
            a
        }
    };
    super::introductions::added(&s, &tx, &scope, id, row.clone()).await?;
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "room_agent_add",
        "conversations",
        &id,
        json!({"room_agent_id":Id::from(row.id),"agent_id":Id::from(row.agent_id)}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view(&row))))
}

pub(super) async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, room_agent)): Path<(String, Id, Id)>,
) -> Result<StatusCode> {
    scope.require("conversations.run")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    participant(&tx, &scope, &c).await?;
    let mut a = agents::Entity::find_by_id(room_agent)
        .filter(agents::Column::ConversationId.eq(id))
        .filter(agents::Column::State.ne("removed"))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let user = i64::from(scope.user.id);
    if a.added_by != user && c.owner_id != user {
        return Err(ApiError::forbidden());
    }
    a.state = "removed".into();
    a.into_active_model().reset_all().update(&tx).await?;
    super::run::cancel_queued_for(&tx, room_agent, "conversations.agentRemoved").await?;
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "room_agent_remove",
        "conversations",
        &id,
        json!({"room_agent_id":room_agent}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The room Agents a human message addresses. `agents` names room rows;
/// legacy `targets` name Agent definitions and must match exactly one row.
pub(super) async fn resolve(
    conn: &impl ConnectionTrait,
    conversation: Id,
    room_agents: &[Id],
    targets: &[Id],
) -> Result<Vec<agents::Model>> {
    let active = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(conversation))
        .filter(agents::Column::State.ne("removed"))
        .all(conn)
        .await?;
    let mut selected = vec![];
    for id in room_agents {
        let a = active
            .iter()
            .find(|a| a.id == i64::from(*id))
            .ok_or_else(ApiError::missing)?;
        selected.push(a.clone());
    }
    for target in targets {
        let mut matches = active.iter().filter(|a| a.agent_id == i64::from(*target));
        let a = matches.next().ok_or_else(ApiError::missing)?;
        if matches.next().is_some() {
            return Err(error(StatusCode::CONFLICT, "conversations.ambiguousTarget"));
        }
        selected.push(a.clone());
    }
    selected.sort_by_key(|a| a.id);
    selected.dedup_by_key(|a| a.id);
    Ok(selected)
}
