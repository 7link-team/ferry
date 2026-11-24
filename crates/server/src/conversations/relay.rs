//! Agent-to-Agent relay inside a room. A running room Agent posts
//! `ferry_request POST /conversations/{id}/relay {agents, text}`; the server
//! records the message and queues a turn for each addressed room Agent as its
//! own sponsor. It is a conversation action, not a workspace write, so it needs
//! no approval. Chains are bounded by `agent_relay_max_hops`; beyond that, or
//! while relay is off for the room, deliveries are held until a human continues.
use super::*;
use crate::entities::{
    ai_chat_agents as agents, ai_chat_messages as messages, ai_chat_turns as links,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{workspace}/conversations/{id}/relay",
            axum::routing::patch(switch),
        )
        .route(
            "/{workspace}/conversations/{id}/relay/continue",
            axum::routing::post(resume),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayBody {
    agents: Vec<Id>,
    text: String,
}

/// Handles a platform call that is a relay from the running turn. Returns the
/// tool result for any `POST /conversations/{id}/relay`, `None` otherwise.
pub(super) async fn intercept(
    s: &AppState,
    turn: &turns::Model,
    call: &crate::platform_tools::Call,
) -> Result<Option<Value>> {
    let Some(target) = call
        .path
        .strip_prefix("/conversations/")
        .and_then(|rest| rest.strip_suffix("/relay"))
    else {
        return Ok(None);
    };
    if call.method != "POST" {
        return Ok(None);
    }
    let result = if target != Id::from(turn.conversation_id).to_string() {
        Err(ApiError::forbidden())
    } else {
        match serde_json::from_value::<RelayBody>(call.body.clone()) {
            Ok(body) => relay(s, turn, body).await,
            Err(_) => Err(invalid()),
        }
    };
    Ok(Some(match result {
        Ok(data) => json!({"status":200,"ok":true,"data":data}),
        Err(e) => {
            json!({"status":e.0.as_u16(),"ok":false,"data":{"error":e.1,"key":e.2.map(|m| m.key)}})
        }
    }))
}

async fn relay(s: &AppState, turn: &turns::Model, body: RelayBody) -> Result<Value> {
    let mut targets = body.agents;
    targets.sort();
    targets.dedup();
    if targets.is_empty() || targets.len() > 8 {
        return Err(bad("conversations.tooManyTargets"));
    }
    if body.text.trim().is_empty() || body.text.len() > 63 * 1024 {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(s, turn.workspace_id.into()).await?;
    let link = links::Entity::find_by_id(turn.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let caller = agents::Entity::find_by_id(link.chat_agent_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let mut c = conversations::Entity::find_by_id(turn.conversation_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let rows = super::room_agents::resolve(&tx, turn.conversation_id.into(), &targets, &[]).await?;
    if rows
        .iter()
        .any(|a| a.id == caller.id || a.state != "active")
    {
        return Err(error(StatusCode::CONFLICT, "conversations.interrupted"));
    }
    let policy = crate::accounts::conversation_policy::Configuration::load(&tx).await?;
    let hop = link.hop + 1;
    let held = hop > policy.agent_relay_max_hops || !enabled(&c.config);
    let name = serde_json::from_str::<Value>(&caller.config)
        .ok()
        .and_then(|config| config["name"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let name_for_prompt = name.clone();
    let mut addressed: Vec<Id> = rows.iter().map(|a| Id::from(a.agent_id)).collect();
    addressed.sort();
    addressed.dedup();
    // Each target's sponsor is notified like a mention.
    let mut sponsors: Vec<Id> = rows.iter().map(|a| Id::from(a.added_by)).collect();
    sponsors.sort();
    sponsors.dedup();
    let message = s.next_id()?;
    messages::Entity::insert(
        messages::Model {
            id: message.into(),
            workspace_id: turn.workspace_id,
            conversation_id: turn.conversation_id,
            owner_id: caller.added_by,
            created_at: db::now(),
            client_id: format!("relay-{message}"),
            prompt: body.text.clone(),
            attachments: "[]".into(),
            targets: serde_json::to_string(&addressed).unwrap(),
            role: "relay".into(),
            agent_id: caller.agent_id,
            // turn_id marks a turn's own reply (unique per turn and role);
            // the calling turn is recorded on the delivered turns' links.
            turn_id: None,
            reply_to: None,
            sender_name: name.clone(),
            text_items: None,
            version: 1,
            edited_at: None,
            retracted_at: None,
            mentions: serde_json::to_string(&sponsors).unwrap(),
        }
        .into_active_model(),
    )
    .exec(&tx)
    .await?;
    // Tell the target who wrote and how to answer.
    let prompt = format!(
        "Message from Agent {} (room Agent {}). Reply with a relay to that room Agent when you need it to act.

{}",
        name_for_prompt,
        Id::from(caller.id),
        body.text
    );
    let mut queued = vec![];
    for a in &rows {
        let id = groups::queue_turn(
            s,
            &tx,
            a,
            groups::Queued {
                message: message.into(),
                prompt: &prompt,
                attachments: "[]",
                state: if held { "held" } else { "queued" },
                hop,
                caller: caller.id,
            },
        )
        .await?;
        queued.push(json!({"room_agent_id":Id::from(a.id),"turn_id":id}));
    }
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    tx.commit().await?;
    Ok(if held {
        json!({"message_id":message,"queued":[],"held":queued,
            "note":"Delivery is held: the relay limit was reached or relay is off for this room. A human must continue it."})
    } else {
        json!({"message_id":message,"queued":queued,"held":[]})
    })
}

/// Relay is on for rooms unless a member turned it off.
fn enabled(config: &str) -> bool {
    serde_json::from_str::<Value>(config)
        .ok()
        .and_then(|config| config["relay_enabled"].as_bool())
        .unwrap_or(true)
}

async fn participant_room(
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    id: Id,
) -> Result<conversations::Model> {
    let c = find(tx, scope, id).await?;
    if !groups::is_group(tx, id).await? {
        return Err(ApiError::missing());
    }
    Ok(c)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SwitchBody {
    enabled: bool,
}

/// Any room participant may turn Agent-to-Agent relay on or off.
pub(super) async fn switch(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<SwitchBody>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    if crate::platform_tools::current_agent().is_some() {
        return Err(ApiError::forbidden());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = participant_room(&tx, &scope, id).await?;
    let mut config: Value = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    config["relay_enabled"] = json!(body.enabled);
    c.config = config.to_string();
    c.version += 1;
    c.updated_at = db::now();
    let version = c.version;
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "conversations",
        &id,
        json!({"changes":{"relay_enabled":body.enabled}}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"relay_enabled":body.enabled,"version":version}),
    ))
}

/// Delivers held relays now, as a new chain (hop 0).
pub(super) async fn resume(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    if crate::platform_tools::current_agent().is_some() {
        return Err(ApiError::forbidden());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = participant_room(&tx, &scope, id).await?;
    let held: Vec<i64> = turns::Entity::find()
        .select_only()
        .column(turns::Column::Id)
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::State.eq("held"))
        .into_tuple()
        .all(&tx)
        .await?;
    if !held.is_empty() {
        turns::Entity::update_many()
            .col_expr(
                turns::Column::State,
                sea_orm::sea_query::Expr::value("queued"),
            )
            .col_expr(
                turns::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(db::now()),
            )
            .filter(turns::Column::Id.is_in(held.clone()))
            .exec(&tx)
            .await?;
        links::Entity::update_many()
            .col_expr(links::Column::Hop, sea_orm::sea_query::Expr::value(0_i64))
            .filter(links::Column::Id.is_in(held.clone()))
            .exec(&tx)
            .await?;
        c.version += 1;
        c.updated_at = db::now();
        c.into_active_model().reset_all().update(&tx).await?;
        crate::hosts::activity_in(
            &s,
            &tx,
            &scope,
            "relay_continue",
            "conversations",
            &id,
            json!({"turns":held.iter().map(|t| Id::from(*t)).collect::<Vec<_>>()}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"queued":held.len()})))
}
