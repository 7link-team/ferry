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
        // Each Agent gets its own host at its first reply; none is chosen here.
        let config = if agent.id == first.id {
            config.clone()
        } else {
            prepare_execution(s, scope, &agent, body).await?
        };
        result.push(agents::Model {
            id: s.next_id()?.into(),
            workspace_id: scope.workspace_id.into(),
            conversation_id: 0,
            agent_id: agent.id,
            revision_id: agent.revision_id,
            config: serde_json::to_string(&config).unwrap(),
            binding: super::placement::UNBOUND.into(),
            native_session_id: String::new(),
            state: "active".into(),
            added_by: scope.user.id.into(),
            approval_mode: super::approval::default_mode(),
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
    let avatars =
        crate::accounts::preferences::avatars(conn, users.iter().map(|u| u.id).collect()).await?;
    value["members"] = json!(
        users
            .iter()
            .map(|u| json!({"id":Id::from(u.id),"name":u.name,"avatar":avatars.get(&u.id)}))
            .collect::<Vec<_>>()
    );
    let snapshots = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    let identities = ai_agents::Entity::find()
        .filter(ai_agents::Column::WorkspaceId.eq(c.workspace_id))
        .filter(
            ai_agents::Column::Id.is_in(
                snapshots
                    .iter()
                    .map(|a| a.agent_id)
                    .chain(std::iter::once(c.agent_id)),
            ),
        )
        .all(conn)
        .await?;
    let avatar = |id| {
        identities
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.avatar.as_deref())
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
    };
    value["agent_avatar"] = json!(avatar(c.agent_id));
    // Without a configured model the Agent cannot run; the header says so.
    let has_model = |id| {
        identities
            .iter()
            .find(|a| a.id == id)
            .is_some_and(|a| a.model_profile_id != 0)
    };
    value["agent_has_model"] = json!(has_model(c.agent_id));
    let sponsors = users::Entity::find()
        .filter(users::Column::Id.is_in(snapshots.iter().map(|a| a.added_by)))
        .all(conn)
        .await?;
    let sponsor_name = |id: i64| {
        sponsors
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.name.clone())
            .unwrap_or_default()
    };
    let mut listed = vec![];
    for a in snapshots.iter().filter(|a| a.state != "removed") {
        let config: Value = serde_json::from_str(&a.config).unwrap_or(Value::Null);
        let host = super::placement::host_view(conn, &a.config, &a.binding).await?;
        // `id` stays the Agent definition for older clients; `room_agent_id` is the row.
        listed.push(json!({"id":Id::from(a.agent_id),"room_agent_id":Id::from(a.id),"added_by":Id::from(a.added_by),
            "sponsor_name":sponsor_name(a.added_by),"approval_mode":a.approval_mode,"name":config["name"],"state":a.state,
            "runtime":config["runtime"],"model":config["model"],"revision_id":Id::from(a.revision_id),"avatar":avatar(a.agent_id),"has_model":has_model(a.agent_id),"host":host}));
    }
    value["agents"] = json!(listed);
    let chat = messages::Entity::find()
        .filter(messages::Column::ConversationId.eq(c.id))
        .order_by_asc(messages::Column::Id)
        .all(conn)
        .await?;
    // Platform notices are stored as messages and read in the reader's language.
    let text = |m: &messages::Model| {
        if m.role == "notice" {
            crate::i18n::render_stored(crate::i18n::current_locale(), &m.prompt)
        } else {
            m.prompt.clone()
        }
    };
    value["messages"]=json!(chat.iter().map(|m|json!({"id":Id::from(m.id),"owner_id":Id::from(m.owner_id),"role":m.role,"turn_id":m.turn_id.map(Id::from),"reply_to":m.reply_to.map(Id::from),"agent_id":Id::from(m.agent_id),"sender_name":m.sender_name,"prompt":text(m),"attachments":serde_json::from_str::<Value>(&m.attachments).unwrap_or(json!([])),"targets":serde_json::from_str::<Value>(&m.targets).unwrap_or(json!([])),"created_at":m.created_at})).collect::<Vec<_>>());
    let links = links::Entity::find()
        .filter(links::Column::ConversationId.eq(c.id))
        .all(conn)
        .await?;
    if let Some(turns) = value["turns"].as_array_mut() {
        for t in turns {
            if let Some(l) = links.iter().find(|l| json!(Id::from(l.id)) == t["id"]) {
                t["message_id"] = if l.message_id == 0 {
                    Value::Null
                } else {
                    json!(Id::from(l.message_id))
                };
                if let Some(a) = snapshots.iter().find(|a| a.id == l.chat_agent_id) {
                    t["agent_id"] = json!(Id::from(a.agent_id));
                    t["room_agent_id"] = json!(Id::from(a.id));
                }
                t["hop"] = json!(l.hop);
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
    let mut approval_mode = super::approval::mode(&c.config)?;
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
        // Each room Agent keeps its sponsor's approval mode, never the room's.
        approval_mode = a.approval_mode;
    }
    let mut config: Value = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    config["approval_mode"] = json!(approval_mode);
    c.config = config.to_string();
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
    /// Room Agent row ids; preferred over `targets` (Agent definition ids).
    #[serde(default)]
    agents: Vec<Id>,
}
pub(super) async fn send_message(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<MessageBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    find(&s.db, &scope, id).await?;
    if body.targets.len() + body.agents.len() > 8 {
        return Err(bad("conversations.tooManyTargets"));
    }
    check_message(&body.prompt, &body.client_id, &body.attachments)?;
    super::check_message_resources(
        &body.prompt,
        &body.attachments,
        &crate::accounts::resource_policy::load(&s.db).await?,
    )?;
    files::canonicalize(&s, &scope, id, &mut body.attachments).await?;
    check_message(&body.prompt, &body.client_id, &body.attachments)?;
    super::check_message_resources(
        &body.prompt,
        &body.attachments,
        &crate::accounts::resource_policy::load(&s.db).await?,
    )?;
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
    let candidates = super::room_agents::resolve(&tx, id, &body.agents, &body.targets).await?;
    // Stored for display: the Agent definitions addressed (rows are in turn links).
    let mut addressed: Vec<Id> = candidates.iter().map(|a| Id::from(a.agent_id)).collect();
    addressed.sort();
    addressed.dedup();
    let targets = serde_json::to_string(&addressed).unwrap();
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
    for a in candidates {
        // The sponsor's use of the Agent is checked when the turn starts.
        if a.added_by == i64::from(scope.user.id) {
            available(&tx, &scope, a.agent_id.into()).await?;
        }
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
        queue_turn(
            &s,
            &tx,
            &a,
            Queued {
                message: message.into(),
                prompt: &body.prompt,
                attachments: &attachments,
                state: "queued",
                hop: 0,
                caller: 0,
            },
        )
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
    context_in(conn, row, budget, Some(20)).await
}
/// API history is canonical and uses no CLI prompt-window truncation.
pub(super) async fn api_context(conn: &impl ConnectionTrait, row: &turns::Model) -> Result<String> {
    context_in(conn, row, usize::MAX, None).await
}
async fn context_in(
    conn: &impl ConnectionTrait,
    row: &turns::Model,
    budget: usize,
    limit: Option<u64>,
) -> Result<String> {
    let Some(link) = links::Entity::find_by_id(row.id).one(conn).await? else {
        return Ok(String::new());
    };
    let mut history = messages::Entity::find()
        .filter(messages::Column::Role.is_in(["user", "relay"]))
        .filter(messages::Column::RetractedAt.is_null())
        .filter(messages::Column::ConversationId.eq(row.conversation_id))
        .filter(messages::Column::Id.lt(link.message_id))
        .order_by_desc(messages::Column::Id)
        .limit(limit)
        .all(conn)
        .await?;
    history.reverse();
    let mut output = String::new();
    for m in history {
        if m.role == "relay" {
            // Agent-to-Agent messages, so each Agent sees what others asked.
            output.push_str(&format!("\nAgent {}: {}", m.sender_name, m.prompt));
        } else {
            output.push_str(&format!("\nMember {}: {}", m.owner_id, m.prompt));
        }
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
    let roster = roster(conn, row.conversation_id, link.chat_agent_id).await?;
    if output.is_empty() {
        Ok(roster)
    } else {
        Ok(format!(
            "{roster}<group_history>\n{output}\n</group_history>\n\n"
        ))
    }
}

/// The other room Agents this Agent can hand work to, and how.
async fn roster(conn: &impl ConnectionTrait, conversation: i64, me: i64) -> Result<String> {
    let rows = agents::Entity::find()
        .filter(agents::Column::ConversationId.eq(conversation))
        .filter(agents::Column::State.eq("active"))
        .filter(agents::Column::Id.ne(me))
        .order_by_asc(agents::Column::Id)
        .all(conn)
        .await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let sponsors = users::Entity::find()
        .filter(users::Column::Id.is_in(rows.iter().map(|a| a.added_by)))
        .all(conn)
        .await?;
    let mut out = String::from("<room_agents>\n");
    for a in &rows {
        let name = serde_json::from_str::<Value>(&a.config)
            .ok()
            .and_then(|c| c["name"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let sponsor = sponsors
            .iter()
            .find(|u| u.id == a.added_by)
            .map(|u| u.name.as_str())
            .unwrap_or("");
        out.push_str(&format!(
            "- room Agent {}: {name} (added by {sponsor})\n",
            Id::from(a.id)
        ));
    }
    out.push_str(&format!(
        "To hand work to another Agent (report a failing API call, announce a contract change, ask for a fix), call ferry_request POST /conversations/{}/relay with {{\"agents\":[room Agent ids],\"text\":\"...\"}}. It needs no approval. Their reply arrives later as a new message to you; do not poll or wait for it.\n</room_agents>\n\n",
        Id::from(conversation)
    ));
    Ok(out)
}

/// One turn for a room Agent, linked to the message that asked for it.
pub(super) struct Queued<'a> {
    pub message: i64,
    pub prompt: &'a str,
    pub attachments: &'a str,
    /// `queued`, or `held` while a relay waits for a human.
    pub state: &'static str,
    pub hop: i64,
    pub caller: i64,
}
pub(super) async fn queue_turn(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    a: &agents::Model,
    q: Queued<'_>,
) -> Result<Id> {
    let turn = s.next_id()?;
    let now = db::now();
    turns::Entity::insert(
        turns::Model {
            purpose: "message".into(),
            introduction_id: 0,
            introduction_locale: String::new(),
            id: turn.into(),
            workspace_id: a.workspace_id,
            conversation_id: a.conversation_id,
            // Room Agents always work as their sponsor, whoever addressed them.
            owner_id: a.added_by,
            state: q.state.into(),
            lease_owner: String::new(),
            expires_at: 0,
            version: 1,
            client_id: turn.to_string(),
            prompt: q.prompt.into(),
            attachments: q.attachments.into(),
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
    .exec(tx)
    .await?;
    links::Entity::insert(
        links::Model {
            id: turn.into(),
            workspace_id: a.workspace_id,
            conversation_id: a.conversation_id,
            message_id: q.message,
            chat_agent_id: a.id,
            hop: q.hop,
            caller_chat_agent_id: q.caller,
        }
        .into_active_model(),
    )
    .exec(tx)
    .await?;
    Ok(turn)
}
