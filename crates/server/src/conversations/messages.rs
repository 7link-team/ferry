//! Chat content is durable independently of execution events and diagnostics.
use super::*;
use crate::entities::{
    ai_chat_agents as snapshots, ai_chat_messages as chat, ai_chat_turns as links,
};
use sea_orm::DbErr;

#[derive(Serialize, Deserialize)]
struct TextPart {
    id: String,
    text: String,
}
fn apply(parts: &mut Vec<TextPart>, id: &str, text: &str, replace: bool) {
    let index = if let Some(index) = parts.iter().position(|part| part.id == id) {
        index
    } else {
        parts.push(TextPart {
            id: id.into(),
            text: String::new(),
        });
        parts.len() - 1
    };
    if replace {
        parts[index].text = text.into();
    } else {
        parts[index].text.push_str(text);
    }
}
fn content(parts: &[TextPart]) -> String {
    parts
        .iter()
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn private_message(turn: &turns::Model) -> chat::Model {
    chat::Model {
        id: turn.id,
        workspace_id: turn.workspace_id,
        conversation_id: turn.conversation_id,
        owner_id: turn.owner_id,
        created_at: turn.created_at,
        client_id: turn.client_id.clone(),
        prompt: turn.prompt.clone(),
        attachments: turn.attachments.clone(),
        targets: "[]".into(),
        role: "user".into(),
        agent_id: 0,
        turn_id: Some(turn.id),
        reply_to: None,
        sender_name: String::new(),
        text_items: None,
        version: 1,
        edited_at: None,
        retracted_at: None,
        mentions: "[]".into(),
    }
}
pub(super) async fn insert_private(conn: &impl ConnectionTrait, turn: &turns::Model) -> Result<()> {
    let mut message = private_message(turn);
    message.sender_name = crate::entities::users::Entity::find_by_id(turn.owner_id)
        .one(conn)
        .await?
        .map(|u| u.name)
        .unwrap_or_default();
    chat::Entity::insert(message.into_active_model())
        .exec(conn)
        .await?;
    Ok(())
}
pub(super) async fn reply_context(
    conn: &impl ConnectionTrait,
    turn: &turns::Model,
    budget: usize,
) -> Result<String> {
    let message = links::Entity::find_by_id(turn.id)
        .one(conn)
        .await?
        .map_or(turn.id, |link| link.message_id);
    let Some(parent) = chat::Entity::find_by_id(message)
        .one(conn)
        .await?
        .and_then(|m| m.reply_to)
    else {
        return Ok(String::new());
    };
    let Some(parent) = chat::Entity::find_by_id(parent)
        .filter(chat::Column::ConversationId.eq(turn.conversation_id))
        .one(conn)
        .await?
    else {
        return Ok(String::new());
    };
    if parent.retracted_at.is_some() || budget < 256 {
        return Ok(String::new());
    }
    // JSON escaping bounds the quoted material and distinguishes it from instructions.
    let mut text = parent.prompt;
    let limit = budget.saturating_sub(128) / 6;
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    Ok(format!(
        "Quoted chat message {} (conversation content): {}\n\n",
        parent.id,
        serde_json::to_string(&text).unwrap()
    ))
}
fn assistant_message(
    turn: &turns::Model,
    c: &conversations::Model,
    id: i64,
    created_at: i64,
    reply_to: i64,
) -> chat::Model {
    chat::Model {
        id,
        workspace_id: turn.workspace_id,
        conversation_id: turn.conversation_id,
        owner_id: turn.owner_id,
        created_at,
        client_id: turn.id.to_string(),
        prompt: String::new(),
        attachments: "[]".into(),
        targets: "[]".into(),
        role: "assistant".into(),
        agent_id: c.agent_id,
        turn_id: Some(turn.id),
        reply_to: Some(reply_to),
        sender_name: serde_json::from_str::<Value>(&c.config)
            .ok()
            .and_then(|v| v["name"].as_str().map(str::to_owned))
            .unwrap_or_default(),
        text_items: Some("[]".into()),
        version: 1,
        edited_at: None,
        retracted_at: None,
        mentions: "[]".into(),
    }
}
pub(super) async fn append_text(
    conn: &impl ConnectionTrait,
    turn: &turns::Model,
    c: &conversations::Model,
    event_id: i64,
    item_id: &str,
    text: &str,
    replace: bool,
) -> Result<()> {
    let old = chat::Entity::find()
        .filter(chat::Column::ConversationId.eq(c.id))
        .filter(chat::Column::TurnId.eq(turn.id))
        .filter(chat::Column::Role.eq("assistant"))
        .one(conn)
        .await?;
    let exists = old.is_some();
    let parent = links::Entity::find_by_id(turn.id)
        .one(conn)
        .await?
        .map(|link| link.message_id)
        .unwrap_or(turn.id);
    let mut row = old.unwrap_or_else(|| assistant_message(turn, c, event_id, db::now(), parent));
    let mut parts: Vec<TextPart> =
        serde_json::from_str(row.text_items.as_deref().unwrap_or("[]")).map_err(|_| invalid())?;
    apply(&mut parts, item_id, text, replace);
    row.prompt = content(&parts);
    row.text_items = Some(serde_json::to_string(&parts).map_err(|_| invalid())?);
    if exists {
        row.into_active_model().reset_all().update(conn).await?;
    } else {
        chat::Entity::insert(row.into_active_model())
            .exec(conn)
            .await?;
    }
    Ok(())
}

/// Upgrade existing conversations once. Original turns/events remain separate;
/// globally issued turn/event IDs give migrated messages stable positive IDs.
pub(crate) async fn backfill(conn: &impl ConnectionTrait) -> std::result::Result<(), DbErr> {
    let mut after = 0i64;
    loop {
        let batch = turns::Entity::find()
            .filter(turns::Column::Id.gt(after))
            .order_by_asc(turns::Column::Id)
            .limit(100)
            .all(conn)
            .await?;
        if batch.is_empty() {
            break;
        }
        for turn in batch {
            after = turn.id;
            let link = links::Entity::find_by_id(turn.id).one(conn).await?;
            let parent = link.as_ref().map(|link| link.message_id).unwrap_or(turn.id);
            if link.is_none() && chat::Entity::find_by_id(turn.id).one(conn).await?.is_none() {
                chat::Entity::insert(private_message(&turn).into_active_model())
                    .exec(conn)
                    .await?;
            }
            if chat::Entity::find()
                .filter(chat::Column::TurnId.eq(turn.id))
                .filter(chat::Column::Role.eq("assistant"))
                .one(conn)
                .await?
                .is_some()
            {
                continue;
            }
            let Some(mut c) = conversations::Entity::find_by_id(turn.conversation_id)
                .one(conn)
                .await?
            else {
                continue;
            };
            if let Some(link) = link {
                if let Some(snapshot) = snapshots::Entity::find_by_id(link.chat_agent_id)
                    .one(conn)
                    .await?
                {
                    c.agent_id = snapshot.agent_id;
                    c.config = snapshot.config;
                }
            }
            let mut parts = vec![];
            let mut first = None;
            for event in events::Entity::find()
                .filter(events::Column::TurnId.eq(turn.id))
                .order_by_asc(events::Column::Sequence)
                .all(conn)
                .await?
            {
                if let Ok(ferry_protocol::agent::Event::Text {
                    item_id,
                    text,
                    replace,
                }) = serde_json::from_str(&event.payload)
                {
                    first.get_or_insert((event.id, event.created_at));
                    apply(&mut parts, &item_id, &text, replace);
                }
            }
            if let Some((id, created_at)) = first {
                let mut row = assistant_message(&turn, &c, id, created_at, parent);
                row.prompt = content(&parts);
                row.text_items = Some(
                    serde_json::to_string(&parts)
                        .map_err(|_| DbErr::Custom("Invalid chat text parts".into()))?,
                );
                chat::Entity::insert(row.into_active_model())
                    .exec(conn)
                    .await?;
            }
        }
    }
    Ok(())
}
