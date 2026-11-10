//! Explicit room membership and per-Agent native histories; execution uses the existing scheduler.
use super::*;
use crate::entities::{
    ai_chat_agents as agents, ai_chat_members as members, ai_chat_messages as messages,
    ai_chat_turns as links, users, workspace_members,
};
use sea_orm::{Condition, QuerySelect, sea_query::Query as SqlQuery};
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/conversation-participants", get(participants))
        .route(
            "/{workspace}/conversations/{id}/messages",
            post(send_message),
        )
}
pub(super) fn visible(scope: &Scope) -> Condition {
    let sub = SqlQuery::select()
        .column(members::Column::ConversationId)
        .from(members::Entity)
        .and_where(members::Column::WorkspaceId.eq(scope.workspace_id))
        .and_where(members::Column::UserId.eq(scope.user.id))
        .to_owned();
    Condition::any()
        .add(conversations::Column::OwnerId.eq(scope.user.id))
        .add(conversations::Column::Id.in_subquery(sub))
}
pub(super) async fn member(
    conn: &impl ConnectionTrait,
    c: &conversations::Model,
    user: Id,
) -> Result<bool> {
    Ok(members::Entity::find()
        .filter(members::Column::ConversationId.eq(c.id))
        .filter(members::Column::UserId.eq(user))
        .count(conn)
        .await?
        > 0)
}
pub(super) async fn is_group(conn: &impl ConnectionTrait, id: Id) -> Result<bool> {
    Ok(members::Entity::find()
        .filter(members::Column::ConversationId.eq(id))
        .count(conn)
        .await?
        > 0)
}
async fn participants(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    if scope.require("conversations.update").is_err() {
        scope.require("conversations.create")?;
    }
    let member_ids: Vec<i64> = workspace_members::Entity::find()
        .select_only()
        .column(workspace_members::Column::UserId)
        .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workspace_members::Column::State.eq("active"))
        .into_tuple()
        .all(&s.db)
        .await?;
    let rows = users::Entity::find()
        .filter(users::Column::Id.is_in(member_ids))
        .filter(users::Column::Id.ne(scope.user.id))
        .filter(users::Column::Active.eq(1))
        .order_by_asc(users::Column::Name)
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"items":rows.iter().map(|u|json!({"id":Id::from(u.id),"name":u.name})).collect::<Vec<_>>(),"total":rows.len(),"page":1,"page_size":rows.len()}),
    ))
}
pub(super) async fn prepare(
    s: &AppState,
    scope: &Scope,
    body: &Create,
    first: &ai_agents::Model,
    config: &Configuration,
    binding: &ferry_protocol::agent::Binding,
    conversation_id: Id,
) -> Result<Vec<agents::Model>> {
    if body.kind != "group" {
        if !body.agent_ids.is_empty() || !body.member_ids.is_empty() {
            return Err(invalid());
        }
        return Ok(vec![]);
    }
    let mut ids = body.agent_ids.clone();
    ids.push(first.id.into());
    ids.sort();
    ids.dedup();
    if ids.len() > 8 {
        return Err(invalid());
    }
    let mut result = vec![];
    for id in ids {
        let agent = available(&s.db, scope, id).await?;
        // Private Agents cannot be exposed to other room participants by invitation.
        if agent.visibility != "workspace" {
            return Err(error(
                StatusCode::CONFLICT,
                "conversations.groupAgentPrivate",
            ));
        }
        let (config, binding) = if agent.id == first.id {
            (config.clone(), binding.clone())
        } else {
            prepare_execution(s, scope, &agent, body, conversation_id).await?
        };
        result.push(agents::Model {
            id: s.next_id()?.into(),
            workspace_id: scope.workspace_id.into(),
            conversation_id: 0,
            agent_id: agent.id,
            revision_id: agent.revision_id,
            config: serde_json::to_string(&config).unwrap(),
            binding: serde_json::to_string(&binding).unwrap(),
            native_session_id: String::new(),
            state: "active".into(),
        });
    }
    Ok(result)
}
pub(super) async fn insert(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    id: Id,
    body: &Create,
    rows: Vec<agents::Model>,
) -> Result<()> {
    if body.kind != "group" {
        return Ok(());
    }
    let mut ids = body.member_ids.clone();
    ids.push(scope.user.id);
    ids.sort();
    ids.dedup();
    for user in ids {
        let membership = workspace_members::Entity::find()
            .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(workspace_members::Column::UserId.eq(user))
            .filter(workspace_members::Column::State.eq("active"))
            .count(tx)
            .await?
            > 0;
        if !membership && !(user == scope.user.id && scope.user.is_superadmin) {
            return Err(ApiError::missing());
        }
        members::Entity::insert(
            members::Model {
                id: s.next_id()?.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                user_id: user.into(),
            }
            .into_active_model(),
        )
        .exec(tx)
        .await?;
    }
    for mut a in rows {
        if available(tx, scope, a.agent_id.into()).await?.revision_id != a.revision_id {
            return Err(conflict());
        }
        a.conversation_id = id.into();
        agents::Entity::insert(a.into_active_model())
            .exec(tx)
            .await?;
    }
    Ok(())
}
pub(super) async fn detail(
    conn: &impl ConnectionTrait,
    c: &conversations::Model,
    value: &mut Value,
) -> Result<()> {
    let people = members::Entity::find()
        .filter(members::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    value["members_version"] = json!(Id::from(people.iter().map(|m| m.id).max().unwrap_or(0)));
    value["kind"] = json!(if people.is_empty() {
        "private"
    } else {
        "group"
    });
    let users = users::Entity::find()
        .filter(
            users::Column::Id.is_in(
                people
                    .iter()
                    .map(|m| m.user_id)
                    .chain(std::iter::once(c.owner_id)),
            ),
        )
        .all(conn)
        .await?;
    value["members"] = json!(
        users
            .iter()
            .map(|u| json!({"id":Id::from(u.id),"name":u.name}))
            .collect::<Vec<_>>()
    );
    let snapshots = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    value["agents"]=json!(snapshots.iter().filter(|a|a.state!="removed").map(|a|json!({"id":Id::from(a.agent_id),"name":serde_json::from_str::<Value>(&a.config).ok().and_then(|v|v["name"].as_str().map(str::to_owned)),"state":a.state})).collect::<Vec<_>>());
    let chat = messages::Entity::find()
        .filter(messages::Column::ConversationId.eq(c.id))
        .order_by_asc(messages::Column::Id)
        .all(conn)
        .await?;
    value["messages"]=json!(chat.iter().map(|m|json!({"id":Id::from(m.id),"owner_id":Id::from(m.owner_id),"role":m.role,"turn_id":m.turn_id.map(Id::from),"reply_to":m.reply_to.map(Id::from),"agent_id":Id::from(m.agent_id),"sender_name":m.sender_name,"prompt":m.prompt,"attachments":serde_json::from_str::<Value>(&m.attachments).unwrap_or(json!([])),"targets":serde_json::from_str::<Value>(&m.targets).unwrap_or(json!([])),"created_at":m.created_at})).collect::<Vec<_>>());
    let links = links::Entity::find()
        .filter(links::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    if let Some(turns) = value["turns"].as_array_mut() {
        for t in turns {
            if let Some(l) = links.iter().find(|l| json!(Id::from(l.id)) == t["id"]) {
                t["message_id"] = json!(Id::from(l.message_id));
                if let Some(a) = snapshots.iter().find(|a| a.id == l.chat_agent_id) {
                    t["agent_id"] = json!(Id::from(a.agent_id));
                }
            }
        }
    }
    Ok(())
}
pub(super) async fn execution(
    conn: &impl ConnectionTrait,
    row: &turns::Model,
    mut c: conversations::Model,
) -> Result<conversations::Model> {
    if let Some(link) = links::Entity::find_by_id(row.id).one(conn).await? {
        let a = agents::Entity::find_by_id(link.chat_agent_id)
            .filter(agents::Column::ConversationId.eq(c.id))
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        if a.state != "active" {
            return Err(error(StatusCode::CONFLICT, "conversations.interrupted"));
        }
        c.agent_id = a.agent_id;
        c.revision_id = a.revision_id;
        c.config = super::appearance::preserve(&c.config, &a.config);
        c.binding = a.binding;
        c.native_session_id = a.native_session_id;
    }
    Ok(c)
}
pub(super) async fn finish(
    tx: &sea_orm::DatabaseTransaction,
    row: &turns::Model,
    outcome: Option<ferry_protocol::agent::Outcome>,
    started: bool,
) -> Result<bool> {
    if let Some(link) = links::Entity::find_by_id(row.id).one(tx).await? {
        let mut a = agents::Entity::find_by_id(link.chat_agent_id)
            .one(tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if matches!(outcome, Some(ferry_protocol::agent::Outcome::Completed)) {
            a.native_session_id = row.candidate_session.clone();
        } else if started && !matches!(outcome, Some(ferry_protocol::agent::Outcome::Cancelled)) {
            a.state = "interrupted".into();
            a.native_session_id.clear();
        }
        a.into_active_model().reset_all().update(tx).await?;
        return Ok(true);
    }
    Ok(false)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MessageBody {
    prompt: String,
    client_id: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
    #[serde(default)]
    targets: Vec<Id>,
    #[serde(default)]
    reply_to: Option<Id>,
    #[serde(default)]
    mentions: Vec<Id>,
}
pub(super) async fn send_message(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<MessageBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    find(&s.db, &scope, id).await?;
    if (body.prompt.trim().is_empty() && body.attachments.is_empty())
        || body.prompt.len() > 48 * 1024
        || body.prompt.contains('\0')
        || body.client_id.is_empty()
        || body.client_id.len() > 128
        || body.client_id.chars().any(char::is_control)
        || body.attachments.len() > 10
        || body.targets.len() > 8
    {
        return Err(invalid());
    }
    files::canonicalize(&s, &scope, id, &mut body.attachments).await?;
    if body.attachments.iter().any(|a| {
        a.name.is_empty()
            || a.name.len() > 255
            || a.name.chars().any(char::is_control)
            || a.text.contains('\0')
    }) || body.attachments.iter().map(|a| a.text.len()).sum::<usize>() + body.prompt.len()
        > 48 * 1024
    {
        return Err(invalid());
    }
    body.targets.sort();
    body.targets.dedup();
    body.mentions.sort();
    body.mentions.dedup();
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    super::interactions::parent(&tx, id, body.reply_to).await?;
    if body.mentions.len() > 100 {
        return Err(invalid());
    }
    for user in &body.mentions {
        if !member(&tx, &c, *user).await? {
            return Err(ApiError::missing());
        }
    }
    files::validate_refs(&tx, &scope, id, &body.attachments).await?;
    if !is_group(&tx, id).await? {
        return Err(invalid());
    }
    let attachments = serde_json::to_string(&body.attachments).unwrap();
    let targets = serde_json::to_string(&body.targets).unwrap();
    if let Some(old) = messages::Entity::find()
        .filter(messages::Column::ConversationId.eq(id))
        .filter(messages::Column::Role.eq("user"))
        .filter(messages::Column::OwnerId.eq(scope.user.id))
        .filter(messages::Column::ClientId.eq(&body.client_id))
        .one(&tx)
        .await?
    {
        if old.prompt != body.prompt
            || old.attachments != attachments
            || old.targets != targets
            || old.reply_to != body.reply_to.map(i64::from)
            || old.mentions != serde_json::to_string(&body.mentions).unwrap()
        {
            return Err(conflict());
        }
        return Ok((StatusCode::OK, Json(json!({"id":Id::from(old.id)}))));
    }
    if c.state != "active" {
        return Err(error(StatusCode::CONFLICT, "conversations.archived"));
    }
    let mut selected = vec![];
    for target in &body.targets {
        available(&tx, &scope, *target).await?;
        let a = agents::Entity::find()
            .filter(agents::Column::ConversationId.eq(id))
            .filter(agents::Column::AgentId.eq(*target))
            .one(&tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if a.state != "active" {
            return Err(error(StatusCode::CONFLICT, "conversations.interrupted"));
        }
        let config: Configuration = serde_json::from_str(&a.config).map_err(|_| invalid())?;
        if config.system_prompt.len()
            + body.prompt.len()
            + body
                .attachments
                .iter()
                .map(|a| a.text.len() + a.name.len() + 256)
                .sum::<usize>()
            > 63 * 1024
        {
            return Err(invalid());
        }
        selected.push(a);
    }
    let message = s.next_id()?;
    let now = db::now();
    messages::Entity::insert(
        messages::Model {
            id: message.into(),
            workspace_id: scope.workspace_id.into(),
            conversation_id: id.into(),
            owner_id: scope.user.id.into(),
            created_at: now,
            client_id: body.client_id,
            prompt: body.prompt.clone(),
            attachments: attachments.clone(),
            targets,
            role: "user".into(),
            agent_id: 0,
            turn_id: None,
            reply_to: body.reply_to.map(i64::from),
            sender_name: scope.user.name.clone(),
            text_items: None,
            version: 1,
            edited_at: None,
            retracted_at: None,
            mentions: serde_json::to_string(&body.mentions).unwrap(),
        }
        .into_active_model(),
    )
    .exec(&tx)
    .await?;
    for a in selected {
        let turn = s.next_id()?;
        turns::Entity::insert(
            turns::Model {
                id: turn.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                owner_id: scope.user.id.into(),
                state: "queued".into(),
                lease_owner: String::new(),
                expires_at: 0,
                version: 1,
                client_id: turn.to_string(),
                prompt: body.prompt.clone(),
                attachments: attachments.clone(),
                native_session_id: String::new(),
                candidate_session: String::new(),
                error: String::new(),
                usage: "null".into(),
                decisions: "{}".into(),
                created_at: now,
                updated_at: now,
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
        links::Entity::insert(
            links::Model {
                id: turn.into(),
                workspace_id: scope.workspace_id.into(),
                conversation_id: id.into(),
                message_id: message.into(),
                chat_agent_id: a.id,
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
    }
    c.updated_at = now;
    c.version += 1;
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "send",
        "conversations",
        &id,
        json!({"message_id":message,"target_count":body.targets.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"id":message}))))
}
pub(super) async fn delete(tx: &sea_orm::DatabaseTransaction, id: Id) -> Result<()> {
    members::Entity::delete_many()
        .filter(members::Column::ConversationId.eq(id))
        .exec(tx)
        .await?;
    agents::Entity::delete_many()
        .filter(agents::Column::ConversationId.eq(id))
        .exec(tx)
        .await?;
    messages::Entity::delete_many()
        .filter(messages::Column::ConversationId.eq(id))
        .exec(tx)
        .await?;
    links::Entity::delete_many()
        .filter(links::Column::ConversationId.eq(id))
        .exec(tx)
        .await?;
    Ok(())
}
/// Only prior messages from this room are included; no private conversation history.
pub(super) async fn context(
    conn: &impl ConnectionTrait,
    row: &turns::Model,
    budget: usize,
) -> Result<String> {
    let Some(link) = links::Entity::find_by_id(row.id).one(conn).await? else {
        return Ok(String::new());
    };
    let mut history = messages::Entity::find()
        .filter(messages::Column::Role.eq("user"))
        .filter(messages::Column::RetractedAt.is_null())
        .filter(messages::Column::ConversationId.eq(row.conversation_id))
        .filter(messages::Column::Id.lt(link.message_id))
        .order_by_desc(messages::Column::Id)
        .limit(20)
        .all(conn)
        .await?;
    history.reverse();
    let mut output = String::new();
    for m in history {
        output.push_str(&format!("\nMember {}: {}", m.owner_id, m.prompt));
        let turns = links::Entity::find()
            .filter(links::Column::MessageId.eq(m.id))
            .order_by_asc(links::Column::Id)
            .all(conn)
            .await?;
        for previous in turns {
            let Some(turn) = super::turns::Entity::find_by_id(previous.id)
                .one(conn)
                .await?
            else {
                continue;
            };
            if turn.state != "completed" {
                continue;
            }
            let reply = messages::Entity::find()
                .filter(messages::Column::ConversationId.eq(row.conversation_id))
                .filter(messages::Column::TurnId.eq(previous.id))
                .filter(messages::Column::Role.eq("assistant"))
                .one(conn)
                .await?;
            output.push_str(&format!(
                "\nAgent {}: {}",
                previous.chat_agent_id,
                reply.map(|message| message.prompt).unwrap_or_default()
            ));
            if output.len() > budget {
                let mut start = output.len() - budget;
                while !output.is_char_boundary(start) {
                    start += 1;
                }
                output = output[start..].to_owned();
            }
        }
        // Keep only a bounded recent suffix, always at a UTF-8 boundary.
        if output.len() > budget {
            let mut start = output.len() - budget;
            while !output.is_char_boundary(start) {
                start += 1;
            }
            output = output[start..].to_owned();
        }
    }
    if output.is_empty() {
        Ok(output)
    } else {
        Ok(format!("<group_history>\n{output}\n</group_history>\n\n"))
    }
}
