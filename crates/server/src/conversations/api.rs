//! Private canonical API context, keyed by workspace + application room + Agent.
//! Provider messages and pending tool arguments never enter config/binding/events.
use super::*;
use crate::entities::{ai_chat_api_sessions as sessions, ai_providers as providers};
use ferry_protocol::agent::{Event, Input, Outcome, Usage};
use ferry_protocol::resources::{ResourcePolicy, allows};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Serialize, Deserialize, Default)]
struct History {
    messages: Vec<Value>,
    protocol: String,
    pending: Vec<Pending>,
    turn_id: i64,
    round: usize,
    request_inflight: bool,
    usage: Option<Usage>,
    usage_known: bool,
    #[serde(default)]
    response_bytes: usize,
    #[serde(default)]
    tool_calls: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    id: String,
    name: String,
    arguments: Value,
    dispatching: bool,
}
fn aad(session: &sessions::Model) -> String {
    format!(
        "chat-api:{}:{}:{}:{}:{}:{}",
        session.workspace_id,
        session.conversation_id,
        session.agent_id,
        session.model_profile_id,
        session.provider_id,
        session.model
    )
}
fn context_changed() -> ApiError {
    error(StatusCode::CONFLICT, "conversations.apiContextChanged")
}
async fn configuration(
    conn: &impl ConnectionTrait,
    c: &conversations::Model,
) -> Result<(ai_agents::Model, providers::Model)> {
    let agent = ai_agents::Entity::find_by_id(c.agent_id)
        .filter(ai_agents::Column::WorkspaceId.eq(c.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    let resolved = crate::agents::resolve_model(conn, &agent).await?;
    let config: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    if resolved.runtime != "api" || resolved.model != config.model {
        return Err(context_changed());
    }
    let profile =
        crate::agents::model_profile(conn, c.workspace_id.into(), agent.model_profile_id.into())
            .await?;
    let provider = providers::Entity::find_by_id(profile.provider_id)
        .filter(providers::Column::WorkspaceId.eq(c.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    if !["openai", "anthropic"].contains(&provider.protocol.as_str()) {
        return Err(invalid());
    }
    Ok((resolved, provider))
}
async fn load(
    s: &AppState,
    turn: &turns::Model,
    c: &conversations::Model,
    owner: &str,
) -> Result<(sessions::Model, History, bool)> {
    let (tx, current, room) = run::leased(s, turn, owner).await?;
    if current.state != "running" {
        return Err(conflict());
    }
    run::api_authorized_in(&tx, &current, &room).await?;
    let (agent, provider) = configuration(&tx, c).await?;
    let old = sessions::Entity::find()
        .filter(sessions::Column::WorkspaceId.eq(turn.workspace_id))
        .filter(sessions::Column::ConversationId.eq(turn.conversation_id))
        .filter(sessions::Column::AgentId.eq(c.agent_id))
        .one(&tx)
        .await?;
    let session = if let Some(old) = old {
        if old.model_profile_id != agent.model_profile_id
            || old.provider_id != provider.id
            || old.model != agent.model
        {
            return Err(context_changed());
        }
        old
    } else {
        let now = db::now();
        let mut session = sessions::Model {
            id: s.next_id()?.into(),
            workspace_id: turn.workspace_id,
            conversation_id: turn.conversation_id,
            agent_id: c.agent_id,
            model_profile_id: agent.model_profile_id,
            provider_id: provider.id,
            model: agent.model,
            encrypted_history: String::new(),
            last_turn_id: 0,
            version: 1,
            created_at: now,
            created_by: turn.owner_id,
            updated_at: now,
            updated_by: turn.owner_id,
        };
        session.encrypted_history = s.vault.seal(
            &serde_json::to_string(&History {
                protocol: provider.protocol.clone(),
                ..Default::default()
            })
            .map_err(|_| invalid())?,
            &aad(&session),
        )?;
        sessions::Entity::insert(session.clone().into_active_model())
            .exec(&tx)
            .await?;
        session
    };
    let plain = s
        .vault
        .open_secret(&session.encrypted_history, &aad(&session))?;
    let history: History = serde_json::from_str(&plain).map_err(|_| invalid())?;
    if history.protocol != provider.protocol {
        return Err(context_changed());
    }
    tx.commit().await?;
    Ok((session, history, provider.protocol == "anthropic"))
}
/// Every canonical update checks current lease, user/member/Agent authorization,
/// model identity and version under the same workspace-serialized transaction.
async fn checkpoint(
    s: &AppState,
    turn: &turns::Model,
    c: &conversations::Model,
    owner: &str,
    session: &mut sessions::Model,
    history: &History,
    final_text: Option<&str>,
    usage: Option<&Usage>,
) -> Result<()> {
    let (tx, mut current, mut room) = run::leased(s, turn, owner).await?;
    if current.state != "running" {
        return Err(conflict());
    }
    run::api_authorized_in(&tx, &current, &room).await?;
    let (agent, provider) = configuration(&tx, c).await?;
    if history.protocol != provider.protocol
        || agent.model_profile_id != session.model_profile_id
        || provider.id != session.provider_id
        || agent.model != session.model
    {
        return Err(context_changed());
    }
    let encrypted = s.vault.seal(
        &serde_json::to_string(history).map_err(|_| invalid())?,
        &aad(session),
    )?;
    let changed = sessions::Entity::update_many()
        .col_expr(
            sessions::Column::EncryptedHistory,
            sea_orm::sea_query::Expr::value(encrypted.clone()),
        )
        .col_expr(
            sessions::Column::LastTurnId,
            sea_orm::sea_query::Expr::value(turn.id),
        )
        .col_expr(
            sessions::Column::Version,
            sea_orm::sea_query::Expr::value(session.version + 1),
        )
        .col_expr(
            sessions::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(db::now()),
        )
        .col_expr(
            sessions::Column::UpdatedBy,
            sea_orm::sea_query::Expr::value(turn.owner_id),
        )
        .filter(sessions::Column::Id.eq(session.id))
        .filter(sessions::Column::Version.eq(session.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed != 1 {
        return Err(conflict());
    }
    if let Some(text) = final_text {
        let id = s.next_id()?;
        let execution = groups::execution(&tx, &current, room.clone()).await?;
        messages::append_text(
            &tx,
            &current,
            &execution,
            id.into(),
            "api-final",
            text,
            true,
        )
        .await?;
        current.state = "completed".into();
        current.expires_at = 0;
        current.updated_at = db::now();
        current.version += 1;
        let usage = usage.cloned();
        current.usage = serde_json::to_string(&usage).map_err(|_| invalid())?;
        run::mark_ended_undecided(&mut current);
        current.into_active_model().reset_all().update(&tx).await?;
        room.next_sequence += 1;
        events::Entity::insert(
            events::Model {
                id: id.into(),
                workspace_id: turn.workspace_id,
                conversation_id: room.id,
                turn_id: turn.id,
                sequence: room.next_sequence,
                payload: serde_json::to_string(&Event::Text {
                    item_id: "api-final".into(),
                    text: text.into(),
                    replace: true,
                })
                .map_err(|_| invalid())?,
                created_at: db::now(),
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
        room.next_sequence += 1;
        events::Entity::insert(
            events::Model {
                id: s.next_id()?.into(),
                workspace_id: turn.workspace_id,
                conversation_id: room.id,
                turn_id: turn.id,
                sequence: room.next_sequence,
                payload: serde_json::to_string(&Event::Finished {
                    outcome: Outcome::Completed,
                    usage,
                })
                .map_err(|_| invalid())?,
                created_at: db::now(),
            }
            .into_active_model(),
        )
        .exec(&tx)
        .await?;
        room.updated_at = db::now();
        room.version += 1;
        room.into_active_model().reset_all().update(&tx).await?;
    }
    tx.commit().await?;
    session.version += 1;
    session.encrypted_history = encrypted;
    session.last_turn_id = turn.id;
    Ok(())
}
fn accumulate_usage(history: &mut History, usage: Option<&Value>) {
    if !history.usage_known {
        return;
    }
    let tokens = usage.and_then(|usage| {
        Some((
            usage["prompt_tokens"]
                .as_u64()
                .or_else(|| usage["input_tokens"].as_u64())?,
            usage["completion_tokens"]
                .as_u64()
                .or_else(|| usage["output_tokens"].as_u64())?,
        ))
    });
    let totals = tokens.and_then(|(input, output)| {
        let previous = history.usage.as_ref();
        Some(Usage {
            input_tokens: previous
                .map_or(0, |usage| usage.input_tokens)
                .checked_add(input)?,
            output_tokens: previous
                .map_or(0, |usage| usage.output_tokens)
                .checked_add(output)?,
        })
    });
    // Missing/partial/overflowing telemetry never changes execution success.
    history.usage_known = totals.is_some();
    history.usage = totals;
}

fn tool_result(history: &mut History, anthropic: bool, id: &str, result: Value) {
    if anthropic {
        history.messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":result.to_string()}]}));
    } else {
        history
            .messages
            .push(json!({"role":"tool","tool_call_id":id,"content":result.to_string()}));
    }
}
/// The platform gateway records its completed result before returning to the
/// provider loop. Recover that receipt if a crash preceded the context commit.
async fn recorded_tool_result(
    conn: &impl ConnectionTrait,
    workspace: i64,
    turn: i64,
    request: &str,
) -> Result<Option<Value>> {
    let rows = events::Entity::find()
        .filter(events::Column::WorkspaceId.eq(workspace))
        .filter(events::Column::TurnId.eq(turn))
        .order_by_desc(events::Column::Sequence)
        .all(conn)
        .await?;
    for event in rows {
        let value: Value = serde_json::from_str(&event.payload).map_err(|_| invalid())?;
        if value["type"] == "platform_result" && value["request_id"] == request {
            return Ok(value.get("result").cloned());
        }
    }
    Ok(None)
}
async fn work(
    s: &AppState,
    row: &turns::Model,
    c: &conversations::Model,
    owner: &str,
    inputs: &mut mpsc::UnboundedReceiver<Input>,
    resources: &ResourcePolicy,
) -> Result<()> {
    let scope = run::api_authorized(s, row, owner).await?;
    available(&s.db, &scope, c.agent_id.into()).await?;
    let (mut session, mut history, anthropic) = load(s, row, c, owner).await?;
    if history.turn_id != row.id {
        // Never silently replay a tool which may have crossed the side-effect boundary.
        for pending in std::mem::take(&mut history.pending) {
            if let Some(result) =
                recorded_tool_result(&s.db, row.workspace_id, history.turn_id, &pending.id).await?
            {
                tool_result(&mut history, anthropic, &pending.id, result);
                continue;
            }
            tool_result(
                &mut history,
                anthropic,
                &pending.id,
                json!({"ok":false,"status":409,"error": if pending.dispatching {
                "The preceding turn was interrupted while this operation was in flight. Its outcome is unknown. Do not repeat it automatically. Ask the user to verify the result."
            } else { "The preceding turn ended before this operation was dispatched. No automatic replay is permitted." }}),
            );
        }
        if history.request_inflight {
            let previous = turns::Entity::find_by_id(history.turn_id)
                .one(&s.db)
                .await?;
            let recorded = match &previous {
                Some(previous) => {
                    crate::workflows::ai::conversation_outcome(
                        s,
                        previous,
                        c.agent_id,
                        history.round,
                    )
                    .await?
                }
                None => crate::workflows::ai::ConversationOutcome::Unsent,
            };
            if let crate::workflows::ai::ConversationOutcome::Known(response) = recorded {
                match crate::workflows::ai::agent_response(&response, anthropic) {
                    Ok((message, _, calls)) => {
                        history.messages.push(message);
                        for (id, _, _) in calls {
                            tool_result(&mut history, anthropic, &id, json!({"ok":false,"status":409,"error":"The preceding response was recovered from the local journal after interruption. This operation was not dispatched and must not be replayed automatically."}));
                        }
                    }
                    Err(_) => history.messages.push(json!({"role":"user","content":"The preceding model request returned an unusable response, retained in the private call journal. No request was retried."})),
                }
            } else {
                if let Some(previous) = previous {
                    crate::workflows::ai::conversation_interrupted(s, &previous).await?;
                }
                let notice = if matches!(
                    recorded,
                    crate::workflows::ai::ConversationOutcome::Unknown
                ) {
                    "The preceding model request was interrupted with an unknown outcome. Its response is unavailable; no request was automatically retried."
                } else {
                    "The preceding model request was interrupted before provider dispatch. No request was sent or automatically retried."
                };
                history
                    .messages
                    .push(json!({"role":"user","content":notice}));
            }
        }
        history.turn_id = row.id;
        history.round = 0;
        history.request_inflight = false;
        history.usage = None;
        history.usage_known = true;
        history.response_bytes = 0;
        history.tool_calls = 0;
        let config: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
        let policy = crate::accounts::conversation_policy::Configuration::load(&s.db).await?;
        let mut prompt = super::introductions::prompt(s, row, c, &scope, &policy, &config).await?;
        let clock =
            super::run::workspace_clock_context(&s.db, scope.workspace_id, db::now()).await?;
        prompt = format!("{clock}{prompt}");
        let quote = messages::reply_context(&s.db, row, usize::MAX).await?;
        if !quote.is_empty() {
            prompt = format!("{quote}{prompt}");
        }
        let context = groups::api_context(&s.db, row).await?;
        if !context.is_empty() {
            prompt = format!("{context}{prompt}");
        }
        if !allows(resources.agent_prompt_bytes, prompt.len()) {
            return Err(invalid());
        }
        history
            .messages
            .push(json!({"role":"user","content":prompt}));
        checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
    }
    loop {
        // Persist full native assistant/tool-call ids before any dispatch.
        while !history.pending.is_empty() {
            let pending = history.pending[0].clone();
            if !allows(
                resources.tool_request_bytes,
                serde_json::to_vec(&pending.arguments)
                    .map_err(|_| invalid())?
                    .len(),
            ) {
                return Err(error(StatusCode::BAD_REQUEST, "conversations.outputLimit"));
            }
            history.pending[0].dispatching = true;
            checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
            let (_, mut result) = run::platform_call(
                s.clone(),
                row.clone(),
                owner.into(),
                pending.id.clone(),
                pending.name.clone(),
                pending.arguments.clone(),
                None,
            )
            .await;
            if result.is_null() {
                history.pending[0].dispatching = false;
                checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
                loop {
                    match inputs.recv().await.ok_or_else(conflict)? {
                        Input::Decision { request_id, allow } if request_id == pending.id => {
                            let arguments = run::approved_platform_call(s, row, owner, &request_id)
                                .await?
                                .ok_or_else(invalid)?;
                            history.pending[0].dispatching = true;
                            checkpoint(s, row, c, owner, &mut session, &history, None, None)
                                .await?;
                            result = run::platform_call(
                                s.clone(),
                                row.clone(),
                                owner.into(),
                                request_id,
                                "ferry_request".into(),
                                arguments,
                                Some(allow),
                            )
                            .await
                            .1;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            if !allows(
                resources.tool_response_bytes,
                serde_json::to_vec(&result).map_err(|_| invalid())?.len(),
            ) {
                return Err(error(StatusCode::BAD_REQUEST, "conversations.outputLimit"));
            }
            tool_result(&mut history, anthropic, &pending.id, result.clone());
            history.pending.remove(0);
            if pending.name == "ferry_ask_user" && result["ok"] == true {
                // End at the human question. Supply explicit results for later calls in this same response.
                for extra in std::mem::take(&mut history.pending) {
                    tool_result(
                        &mut history,
                        anthropic,
                        &extra.id,
                        json!({"ok":false,"status":409,"error":"Waiting for the user's answer; this operation was not dispatched."}),
                    );
                }
                let text = pending.arguments["question"].as_str().unwrap_or_default();
                return checkpoint(
                    s,
                    row,
                    c,
                    owner,
                    &mut session,
                    &history,
                    Some(text),
                    history.usage.as_ref(),
                )
                .await;
            }
            checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
        }
        let profile = crate::agents::model_profile(
            &s.db,
            row.workspace_id.into(),
            session.model_profile_id.into(),
        )
        .await?;
        let params = json!({"provider_id":session.provider_id.to_string(),"model":session.model,
            "agent_messages":history.messages,"agent_tools":crate::workflows::ai::agent_tools(anthropic, true),"max_tokens":profile.max_output_tokens});
        if anthropic && profile.max_output_tokens.is_none() {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "conversations.apiOutputRequired",
            ));
        }
        // The full native history is the actual provider prompt, including
        // previous tool results; never silently truncate canonical context.
        if !allows(
            resources.agent_prompt_bytes,
            serde_json::to_vec(&history.messages)
                .map_err(|_| invalid())?
                .len(),
        ) {
            return Err(invalid());
        }
        history.request_inflight = true;
        checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
        let response = loop {
            match crate::workflows::ai::conversation_round(s, row, c, owner, history.round, &params)
                .await
            {
                Err(error)
                    if error
                        .2
                        .as_ref()
                        .is_some_and(|message| message.key == "workflow.aiLimit") =>
                {
                    // This error is issued only by local reservation before a
                    // provider request exists. Sent/unknown calls never retry.
                    run::api_authorized(s, row, owner).await?;
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                other => break other?,
            }
        };
        history.response_bytes = history
            .response_bytes
            .checked_add(serde_json::to_vec(&response).map_err(|_| invalid())?.len())
            .ok_or_else(invalid)?;
        if !allows(resources.agent_turn_output_bytes, history.response_bytes) {
            return Err(error(StatusCode::BAD_REQUEST, "conversations.outputLimit"));
        }
        let (message, text, calls) = crate::workflows::ai::agent_response(&response, anthropic)?;
        history.tool_calls = history
            .tool_calls
            .checked_add(calls.len())
            .ok_or_else(invalid)?;
        if !allows(resources.agent_tool_calls, history.tool_calls) {
            return Err(error(StatusCode::BAD_REQUEST, "conversations.outputLimit"));
        }
        history.request_inflight = false;
        accumulate_usage(&mut history, response.get("usage"));
        history.round = history.round.checked_add(1).ok_or_else(invalid)?;
        history.messages.push(message);
        if calls.is_empty() {
            return checkpoint(
                s,
                row,
                c,
                owner,
                &mut session,
                &history,
                Some(&text),
                history.usage.as_ref(),
            )
            .await;
        }
        history.pending = calls
            .into_iter()
            .map(|(id, name, arguments)| Pending {
                id,
                name,
                arguments,
                dispatching: false,
            })
            .collect();
        checkpoint(s, row, c, owner, &mut session, &history, None, None).await?;
    }
}
pub(super) async fn execute(
    s: AppState,
    row: turns::Model,
    c: conversations::Model,
    owner: String,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let (send, mut inputs) = mpsc::unbounded_channel();
    let monitor = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let (cancel, decisions) = run::heartbeat(&s, &row, &owner).await?;
            if cancel {
                return Err::<(), ApiError>(error(
                    StatusCode::CONFLICT,
                    "conversations.interrupted",
                ));
            }
            for input in decisions {
                let _ = send.send(input);
            }
        }
    };
    let result = async {
        let resources = crate::accounts::resource_policy::load(&s.db).await?;
        let deadline = async {
            match resources.task_timeout_seconds {
                Some(seconds) => tokio::time::sleep(std::time::Duration::from_secs(seconds)).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(deadline);
        tokio::select! {
            result = work(&s, &row, &c, &owner, &mut inputs, &resources) => result,
            result = monitor => result,
            _ = &mut deadline => Err(error(StatusCode::GATEWAY_TIMEOUT, "workflow.aiTimeout")),
            _ = stop.changed() => Err(error(StatusCode::CONFLICT, "conversations.interrupted")),
        }
    }
    .await;
    if let Err(error) = result {
        crate::workflows::ai::conversation_interrupted(&s, &row).await?;
        // API contexts survive failure/cancellation; only current-turn state changes.
        let (tx, mut current, mut room) = run::leased(&s, &row, &owner).await?;
        current.state = if current.state == "stopping" {
            "cancelled"
        } else if error
            .2
            .as_ref()
            .is_some_and(|message| message.key == "workflow.aiTimeout")
        {
            "timed_out"
        } else {
            "failed"
        }
        .into();
        current.error = error.stored();
        current.expires_at = 0;
        current.version += 1;
        current.updated_at = db::now();
        run::mark_ended_undecided(&mut current);
        current.into_active_model().reset_all().update(&tx).await?;
        room.updated_at = db::now();
        room.version += 1;
        room.into_active_model().reset_all().update(&tx).await?;
        tx.commit().await?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::admin;
    use crate::conversations::tests::{seeded, sent};
    use sea_orm::sea_query::Expr;
    use std::sync::{Arc, Mutex};

    async fn upstream(
        responses: Vec<Value>,
    ) -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let responses = Arc::new(Mutex::new(std::collections::VecDeque::from(responses)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let handler = move |axum::Json(body): axum::Json<Value>| {
                captured.lock().unwrap().push(body);
                let response = responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected resend");
                async move { axum::Json(response) }
            };
            axum::serve(
                listener,
                axum::Router::new()
                    .route("/chat/completions", axum::routing::post(handler.clone()))
                    .route("/messages", axum::routing::post(handler)),
            )
            .await
            .unwrap();
        });
        (url, task, seen)
    }
    fn answer(text: &str) -> Value {
        json!({"choices":[{"message":{"role":"assistant","content":text}}],"usage":{"prompt_tokens":7,"completion_tokens":5}})
    }
    async fn resource_policy(s: &AppState, policy: Value) {
        crate::entities::platform_state::Entity::update_many()
            .col_expr(
                crate::entities::platform_state::Column::ResourceConfig,
                Expr::value(policy.to_string()),
            )
            .exec(&s.db)
            .await
            .unwrap();
    }
    fn calls(count: usize, write: bool) -> Value {
        json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":(0..count).map(|i|json!({"id":format!("native-{i}"),"type":"function","function":{"name":"ferry_request","arguments":if write {
            json!({"method":"POST","path":"/node-groups","body":{"name":format!("must-not-write-{i}"),"parent_id":"0","position":0}}).to_string()
        }else{json!({"method":"GET","path":"/node-groups"}).to_string()}}})).collect::<Vec<_>>()}}],"usage":{"prompt_tokens":1,"completion_tokens":1}})
    }
    #[tokio::test]
    async fn api_resource_default_is_unlimited_and_ignores_legacy_provider_quotas() {
        let (url, server, requests) =
            upstream(vec![calls(160, false), answer(&"x".repeat(96 * 1024))]).await;
        let (_dir, s, room) = fixture(&url).await;
        crate::entities::ai_providers::Entity::update_many()
            .col_expr(crate::entities::ai_providers::Column::Qps, Expr::value(1))
            .col_expr(
                crate::entities::ai_providers::Column::Concurrency,
                Expr::value(1),
            )
            .col_expr(
                crate::entities::ai_providers::Column::MonthlyLimitMicros,
                Expr::value(1),
            )
            .exec(&s.db)
            .await
            .unwrap();
        let row = execute_turn(&s, room, "unlimited-native-tools").await;
        assert_eq!(row.state, "completed", "{}", row.error);
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[1]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "tool")
                .count(),
            160
        );
        assert!(seen.iter().all(|r| r.get("max_tokens").is_none()));
        server.abort();
    }
    #[tokio::test]
    async fn api_resource_configured_tool_and_output_caps_keep_known_responses_without_dispatch() {
        let (url, server, requests) =
            upstream(vec![calls(2, true), answer(&"x".repeat(1024))]).await;
        let (_dir, s, room) = fixture(&url).await;
        resource_policy(&s, json!({"agent_tool_calls":1})).await;
        let first = execute_turn(&s, room, "limited-tools").await;
        assert_eq!(first.state, "failed");
        assert!(
            first.decisions == "{}",
            "no approval or write may be dispatched"
        );
        assert_eq!(
            crate::entities::host_groups::Entity::find()
                .filter(crate::entities::host_groups::Column::Name.like("must-not-write-%"))
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        resource_policy(&s, json!({"agent_turn_output_bytes":64})).await;
        let second = execute_turn(&s, room, "limited-output").await;
        assert_eq!(second.state, "failed");
        let journal = crate::entities::ai_calls::Entity::find()
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(journal.len(), 2);
        assert!(
            journal
                .iter()
                .all(|c| c.state == "succeeded" && c.result.is_some())
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), second.id.into(), signal)
            .await
            .unwrap();
        assert_eq!(
            requests.lock().unwrap().len(),
            2,
            "failed calls are never automatically resent"
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_resource_prompt_fails_before_send_and_transport_limit_never_replays_unknown_response()
     {
        let (url, server, requests) =
            upstream(vec![answer("too large for the configured transport")]).await;
        let (_dir, s, room) = fixture(&url).await;
        // Queue under the original policy, then tighten it before execution.
        let turn = sent(&s, room, "prompt-policy-change").await;
        resource_policy(&s, json!({"agent_prompt_bytes":1})).await;
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), turn, signal).await.unwrap();
        assert!(requests.lock().unwrap().is_empty());
        assert_eq!(
            crate::entities::ai_calls::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        resource_policy(&s, json!({"file_upload_bytes":8})).await;
        let failed = execute_turn(&s, room, "transport-ceiling").await;
        assert_eq!(failed.state, "failed");
        let journal = crate::entities::ai_calls::Entity::find()
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(journal.state, "unknown");
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), failed.id.into(), signal)
            .await
            .unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
        server.abort();
    }
    #[tokio::test]
    async fn api_resource_local_admission_wait_obeys_stop_without_provider_dispatch() {
        let (url, server, requests) = upstream(vec![answer("first known result")]).await;
        let (_dir, s, room) = fixture(&url).await;
        assert_eq!(execute_turn(&s, room, "seed-call").await.state, "completed");
        crate::entities::ai_calls::Entity::update_many()
            .col_expr(
                crate::entities::ai_calls::Column::State,
                Expr::value("unknown"),
            )
            .exec(&s.db)
            .await
            .unwrap();
        resource_policy(&s, json!({"provider_concurrency":1})).await;
        let turn = sent(&s, room, "wait-for-admission").await;
        let (_stop, signal) = watch::channel(false);
        let mut running = tokio::spawn(run::execute(s.clone(), turn, signal));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(400), &mut running)
                .await
                .is_err(),
            "local admission waits rather than failing"
        );
        cancel(State(s.clone()), admin(1), Path(("1".into(), room, turn)))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            turns::Entity::find_by_id(turn)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .state,
            "cancelled"
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(
            crate::entities::ai_calls::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            1
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_resource_optional_deadline_stops_a_sent_call_and_never_resends_it() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/chat/completions",
                    axum::routing::post(move || {
                        let captured = captured.clone();
                        async move {
                            captured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            std::future::pending::<axum::Json<Value>>().await
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let (_dir, s, room) = fixture(&url).await;
        resource_policy(&s, json!({"task_timeout_seconds":1})).await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            execute_turn(&s, room, "optional-deadline"),
        )
        .await
        .unwrap();
        assert_eq!(result.state, "timed_out", "{}", result.error);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        let journal = crate::entities::ai_calls::Entity::find()
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(journal.state, "unknown");
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), result.id.into(), signal)
            .await
            .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }
    #[tokio::test]
    async fn api_resource_cumulative_output_budget_is_persisted_and_resets_only_for_a_new_turn() {
        let first = calls(1, false);
        let final_response = answer("first final answer");
        let next = answer("next turn is allowed");
        let cap = serde_json::to_vec(&first).unwrap().len()
            + serde_json::to_vec(&final_response).unwrap().len()
            - 1;
        let (url, server, requests) =
            upstream(vec![first.clone(), final_response, next.clone()]).await;
        let (_dir, s, room) = fixture(&url).await;
        resource_policy(&s, json!({"agent_turn_output_bytes":cap})).await;
        let failed = execute_turn(&s, room, "cumulative-output").await;
        assert_eq!(failed.state, "failed");
        let session = sessions::Entity::find()
            .filter(sessions::Column::ConversationId.eq(room))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let history: History = serde_json::from_str(
            &s.vault
                .open_secret(&session.encrypted_history, &aad(&session))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            history.response_bytes,
            serde_json::to_vec(&first).unwrap().len()
        );
        assert_eq!(history.tool_calls, 1);
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert_eq!(
            execute_turn(&s, room, "budget-new-turn").await.state,
            "completed"
        );
        let session = sessions::Entity::find_by_id(session.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let history: History = serde_json::from_str(
            &s.vault
                .open_secret(&session.encrypted_history, &aad(&session))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            history.response_bytes,
            serde_json::to_vec(&next).unwrap().len()
        );
        assert_eq!(history.tool_calls, 0);
        assert_eq!(requests.lock().unwrap().len(), 3);
        server.abort();
    }
    #[tokio::test]
    async fn introduction_api_real_provider_result_continues_after_restart_without_public_instructions()
     {
        let (url, task, requests) = upstream(vec![
            answer("Generated first introduction"),
            answer("I remember my introduction"),
        ])
        .await;
        let (_dir, mut s, old) = fixture(&url).await;
        crate::entities::platform_state::Entity::update_many().col_expr(crate::entities::platform_state::Column::ConversationConfig,
            Expr::value(json!({"introduction_enabled":true,"introduction_instructions":"PRIVATE INTRO POLICY"}).to_string())).exec(&s.db).await.unwrap();
        let c = find(&s.db, &admin(1), old).await.unwrap();
        let (_, Json(created)) = create(
            State(s.clone()),
            admin(1),
            Json(serde_json::from_value(json!({"agent_id":Id::from(c.agent_id)})).unwrap()),
        )
        .await
        .unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let intro = turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(id))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), intro.id.into(), signal)
            .await
            .unwrap();
        let completed = turns::Entity::find_by_id(intro.id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.state, "completed", "{}", completed.error);
        s = restart(s).await;
        assert_eq!(
            execute_turn(&s, id, "first-human-after-intro").await.state,
            "completed"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("<automatic_task")
        );
        assert!(seen[0].to_string().contains("PRIVATE INTRO POLICY"));
        assert!(!seen[0].to_string().contains("<user_message>"));
        assert_eq!(
            seen[1]["messages"][1],
            json!({"role":"assistant","content":"Generated first introduction"})
        );
        drop(seen);
        let Json(public) = detail(State(s.clone()), admin(1), Path(("1".into(), id)))
            .await
            .unwrap();
        assert!(!public.to_string().contains("PRIVATE INTRO POLICY"));
        assert_eq!(
            public["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user")
                .count(),
            1
        );
        assert!(
            public["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["prompt"] == "Generated first introduction")
                .unwrap()["reply_to"]
                .is_null()
        );
        assert_eq!(
            crate::entities::ai_calls::Entity::find()
                .filter(crate::entities::ai_calls::Column::ConversationId.eq(id))
                .count(&s.db)
                .await
                .unwrap(),
            2
        );
        task.abort();
    }
    async fn fixture(url: &str) -> (tempfile::TempDir, AppState, Id) {
        let (dir, s, id) = seeded().await;
        let result = crate::platform_tools::dispatch(&s, admin(1), crate::platform_tools::Call {
            method:"POST".into(), path:"/ai-models".into(),
            body:json!({"name":"Chat API","kind":"api","skip_check":true,"state":"enabled","default_model":"m",
                "connection":{"name":"Chat upstream","api_key":"private-test-key","protocol":"openai","base_url":url,
                "qps":1000,"concurrency":10,"monthly_limit_micros":0,"prices":[]}}), ..Default::default()
        }).await.unwrap();
        assert_eq!(result["status"], 201, "{result}");
        let profile = result["data"]["id"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        let mut c = conversations::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        ai_agents::Entity::update_many()
            .col_expr(ai_agents::Column::ModelProfileId, Expr::value(profile))
            .col_expr(ai_agents::Column::Model, Expr::value("m"))
            .col_expr(ai_agents::Column::HostId, Expr::value(0_i64))
            .filter(ai_agents::Column::Id.eq(c.agent_id))
            .exec(&s.db)
            .await
            .unwrap();
        let agent = ai_agents::Entity::find_by_id(c.agent_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let resolved = crate::agents::resolve_model(&s.db, &agent).await.unwrap();
        c.config = serde_json::to_string(&Configuration::from_agent(&resolved).unwrap()).unwrap();
        c.binding = "null".into();
        c.into_active_model()
            .reset_all()
            .update(&s.db)
            .await
            .unwrap();
        let mut revision =
            crate::entities::definition_revisions::Entity::find_by_id(agent.revision_id)
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
        (dir, s, id)
    }
    async fn restart(old: AppState) -> AppState {
        let directory = old.data_dir.clone();
        let ids = old.ids.clone();
        old.db.clone().close().await.unwrap();
        drop(old);
        Arc::new(crate::state::State {
            ids,
            db: sea_orm::Database::connect(format!(
                "sqlite://{}?mode=rwc",
                directory.join("test.sqlite").display()
            ))
            .await
            .unwrap(),
            vault: crate::vault::Vault::open(&directory, false).unwrap(),
            data_dir: directory,
            instance_id: "api-chat-restarted".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
        })
    }
    async fn execute_turn(s: &AppState, room: Id, client: &str) -> turns::Model {
        let id = sent(s, room, client).await;
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), id, signal).await.unwrap();
        turns::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap()
    }
    #[tokio::test]
    async fn api_chat_roundtrips_a_b_a_private_history_and_real_provenance() {
        let (url, task, requests) = upstream(vec![
            answer("A secret"),
            answer("B secret"),
            answer("A remembered"),
        ])
        .await;
        let (_dir, mut s, a) = fixture(&url).await;
        let original = conversations::Entity::find_by_id(a)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let (_, Json(created)) = create(
            State(s.clone()),
            admin(1),
            Json(
                serde_json::from_value(json!({"agent_id":Id::from(original.agent_id),"title":"B"}))
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
        let b: Id = serde_json::from_value(created["id"].clone()).unwrap();
        assert_eq!(execute_turn(&s, a, "a-first").await.state, "completed");
        assert_eq!(execute_turn(&s, b, "b-first").await.state, "completed");
        s = restart(s).await;
        assert_eq!(execute_turn(&s, a, "a-second").await.state, "completed");
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(seen[1]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            seen[2]["messages"][1],
            json!({"role":"assistant","content":"A secret"})
        );
        assert!(!seen[2].to_string().contains("B secret"));
        assert!(
            seen.iter()
                .all(|request| request.get("max_tokens").is_none())
        );
        drop(seen);
        let sessions = sessions::Entity::find().all(&s.db).await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(
            sessions
                .iter()
                .all(|session| !session.encrypted_history.contains("secret"))
        );
        let calls = crate::entities::ai_calls::Entity::find()
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|call| call.execution_id == 0
            && call.node_run_id == 0
            && call.conversation_id > 0
            && call.conversation_turn_id > 0));
        let public = detail(State(s.clone()), admin(1), Path(("1".into(), a)))
            .await
            .unwrap()
            .0;
        assert!(public["config"].get("encrypted_history").is_none());
        assert_eq!(public["binding"], Value::Null);
        assert!(
            public["turns"]
                .as_array()
                .unwrap()
                .iter()
                .all(|turn| turn["usage"]["input_tokens"] == 7)
        );
        assert!(
            events::Entity::find()
                .all(&s.db)
                .await
                .unwrap()
                .iter()
                .all(|event| !event.payload.contains("private-test-key")
                    && !event.payload.contains("agent_messages"))
        );
        task.abort();
    }
    #[tokio::test]
    async fn api_chat_preserves_native_tool_ids_and_question_continues_later() {
        let tool = json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"native-question-id","type":"function","function":{"name":"ferry_ask_user","arguments":json!({"title":"Choose","question":"Which environment?"}).to_string()}}]}}],"usage":{"prompt_tokens":2,"completion_tokens":3}});
        let (url, task, requests) = upstream(vec![tool, answer("Human answer remembered")]).await;
        let (_dir, s, room) = fixture(&url).await;
        let first = execute_turn(&s, room, "ask").await;
        assert_eq!(first.state, "completed", "{}", first.error);
        assert_eq!(
            serde_json::from_str::<Value>(&first.usage).unwrap(),
            json!({"input_tokens":2,"output_tokens":3}),
            "question turn retains its provider usage"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "question ends model loop"
        );
        let response=crate::platform_tools::dispatch(&s,admin(1),crate::platform_tools::Call {
            method:"POST".into(),path:format!("/conversations/{room}/turns/{}/question-answers",first.id),
            body:json!({"request_id":"native-question-id","action":"continue","answer":"Staging"}),..Default::default()
        }).await.unwrap();
        assert_eq!(response["status"], 200, "{response}");
        let next: Id = serde_json::from_value(response["data"]["id"].clone()).unwrap();
        let (_stop, signal) = watch::channel(false);
        run::execute(s.clone(), next, signal).await.unwrap();
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[1]["messages"][1]["tool_calls"][0]["id"],
            "native-question-id"
        );
        assert_eq!(seen[1]["messages"][2]["role"], "tool");
        assert_eq!(seen[1]["messages"][2]["tool_call_id"], "native-question-id");
        assert!(
            seen[1]["messages"][3]["content"]
                .as_str()
                .unwrap()
                .contains("Staging")
        );
        task.abort();
    }
    #[tokio::test]
    async fn api_chat_stale_lease_cannot_advance_history_or_emit_final() {
        let (_dir, s, room) = fixture("http://127.0.0.1:9").await;
        let turn = sent(&s, room, "lease").await;
        let (row, c, owner) = run::claim(&s, turn).await.unwrap().unwrap();
        let (mut session, mut history, _) = load(&s, &row, &c, &owner).await.unwrap();
        let before = session.encrypted_history.clone();
        turns::Entity::update_many()
            .col_expr(turns::Column::ExpiresAt, Expr::value(0_i64))
            .filter(turns::Column::Id.eq(row.id))
            .exec(&s.db)
            .await
            .unwrap();
        history
            .messages
            .push(json!({"role":"assistant","content":"must never publish"}));
        assert!(
            checkpoint(
                &s,
                &row,
                &c,
                &owner,
                &mut session,
                &history,
                Some("must never publish"),
                None
            )
            .await
            .is_err()
        );
        assert_eq!(
            sessions::Entity::find_by_id(session.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .encrypted_history,
            before
        );
        assert_eq!(
            events::Entity::find()
                .filter(events::Column::TurnId.eq(row.id))
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn api_chat_uncertain_upstream_is_never_resent_and_revocation_blocks_final() {
        let received = Arc::new(tokio::sync::Notify::new());
        let blocked = Arc::new(tokio::sync::Notify::new());
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (seen, wait, calls) = (received.clone(), blocked.clone(), count.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/chat/completions",
                    axum::routing::post(move || {
                        let (seen, wait, calls) = (seen.clone(), wait.clone(), calls.clone());
                        async move {
                            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            seen.notify_one();
                            wait.notified().await;
                            axum::Json(answer("forbidden final"))
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let (_dir, s, room) = fixture(&url).await;
        let turn = sent(&s, room, "revoke").await;
        let (stop, signal) = watch::channel(false);
        let running = tokio::spawn(run::execute(s.clone(), turn, signal));
        tokio::time::timeout(std::time::Duration::from_secs(5), received.notified())
            .await
            .unwrap();
        crate::entities::workspace_roles::Entity::update_many()
            .col_expr(
                crate::entities::workspace_roles::Column::Permissions,
                Expr::value("[]"),
            )
            .filter(crate::entities::workspace_roles::Column::Id.eq(1_i64))
            .exec(&s.db)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        blocked.notify_one();
        let row = turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "cancelled");
        let call = crate::entities::ai_calls::Entity::find()
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call.state, "unknown");
        assert_eq!(
            events::Entity::find()
                .filter(events::Column::TurnId.eq(turn))
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        run::execute(s.clone(), turn, stop.subscribe())
            .await
            .unwrap();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_approval_waits_and_then_returns_the_native_result() {
        let tool = json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"native-write","type":"function","function":{"name":"ferry_request","arguments":json!({"method":"POST","path":"/node-groups","body":{"name":"API approved group","parent_id":"0","position":0}}).to_string()}}]}}],"usage":{"prompt_tokens":1,"completion_tokens":1}});
        let (url, server, requests) = upstream(vec![tool, answer("Created after approval")]).await;
        let (_dir, s, room) = fixture(&url).await;
        resource_policy(&s, json!({"agent_tool_calls":1})).await;
        let turn = sent(&s, room, "approve").await;
        let (_stop, signal) = watch::channel(false);
        let running = tokio::spawn(run::execute(s.clone(), turn, signal));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let row = turns::Entity::find_by_id(turn)
                    .one(&s.db)
                    .await
                    .unwrap()
                    .unwrap();
                if serde_json::from_str::<Value>(&row.decisions)
                    .unwrap()
                    .get("native-write")
                    .is_some()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(
            crate::entities::host_groups::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        decide(
            State(s.clone()),
            admin(1),
            Path(("1".into(), room, turn)),
            Json(
                serde_json::from_value(json!({"request_id":"native-write","allow":true})).unwrap(),
            ),
        )
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::entities::host_groups::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            1
        );
        let completed = turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            completed.state, "completed",
            "approval must not count one tool twice: {}",
            completed.error
        );
        let session = sessions::Entity::find()
            .filter(sessions::Column::ConversationId.eq(room))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let history: History = serde_json::from_str(
            &s.vault
                .open_secret(&session.encrypted_history, &aad(&session))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(history.tool_calls, 1);
        assert_eq!(
            serde_json::from_str::<Value>(&completed.usage).unwrap(),
            json!({"input_tokens":8,"output_tokens":6})
        );

        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1]["messages"][2]["tool_call_id"], "native-write");
        assert!(
            seen[1]["messages"][2]["content"]
                .as_str()
                .unwrap()
                .contains("201")
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_interrupted_pending_write_recovers_unknown_without_dispatch() {
        let (url, server, requests) = upstream(vec![answer("Unknown result acknowledged")]).await;
        let (_dir, s, room) = fixture(&url).await;
        let first = sent(&s, room, "unknown").await;
        let (row, c, owner) = run::claim(&s, first).await.unwrap().unwrap();
        let (mut session, mut history, _) = load(&s, &row, &c, &owner).await.unwrap();
        history.turn_id = row.id;
        history.messages = vec![
            json!({"role":"user","content":"Create group"}),
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"native-unknown","type":"function","function":{"name":"ferry_request","arguments":"{}"}}]}),
        ];
        history.pending = vec![Pending {
            id: "native-unknown".into(),
            name: "ferry_request".into(),
            arguments: json!({"method":"POST","path":"/node-groups","body":{"name":"must not replay"}}),
            dispatching: true,
        }];
        checkpoint(&s, &row, &c, &owner, &mut session, &history, None, None)
            .await
            .unwrap();
        turns::Entity::update_many()
            .col_expr(turns::Column::ExpiresAt, Expr::value(0_i64))
            .filter(turns::Column::Id.eq(row.id))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(run::claim(&s, first).await.unwrap().is_none());
        let row = execute_turn(&s, room, "recover").await;
        assert_eq!(row.state, "completed", "{}", row.error);
        assert_eq!(
            crate::entities::host_groups::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["messages"][2]["tool_call_id"], "native-unknown");
        assert!(
            seen[0]["messages"][2]["content"]
                .as_str()
                .unwrap()
                .contains("outcome is unknown")
        );
        server.abort();
    }

    async fn group_turn(s: &AppState, room: Id, targets: Vec<Id>, client: &str) {
        groups::send_message(
            State(s.clone()),
            admin(1),
            Path(("1".into(), room)),
            Json(
                serde_json::from_value(
                    json!({"prompt":"Room message","client_id":client,"targets":targets}),
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let turns = turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(room))
            .filter(turns::Column::State.eq("queued"))
            .order_by_asc(turns::Column::Id)
            .all(&s.db)
            .await
            .unwrap();
        let (_stop, signal) = watch::channel(false);
        for turn in turns {
            run::execute(s.clone(), turn.id.into(), signal.clone())
                .await
                .unwrap();
        }
    }
    #[tokio::test]
    async fn api_chat_group_agents_keep_separate_context_and_explicit_readd_resets_only_that_pair()
    {
        let (url, server, requests) = upstream(vec![
            answer("A native"),
            answer("B native"),
            answer("A continued"),
            answer("B readded"),
        ])
        .await;
        let (_dir, s, private) = fixture(&url).await;
        let c = conversations::Entity::find_by_id(private)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let a = ai_agents::Entity::find_by_id(c.agent_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let mut b = a.clone();
        b.id = s.next_id().unwrap().into();
        b.name = "Second Agent".into();
        ai_agents::Entity::insert(b.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let (_,Json(created))=create(State(s.clone()),admin(1),Json(serde_json::from_value(json!({"kind":"group","title":"API room","agent_ids":[Id::from(a.id),Id::from(b.id)]})).unwrap())).await.unwrap();
        let room: Id = serde_json::from_value(created["id"].clone()).unwrap();
        group_turn(&s, room, vec![a.id.into(), b.id.into()], "both").await;
        group_turn(&s, room, vec![a.id.into()], "a-followup").await;
        {
            let seen = requests.lock().unwrap();
            assert_eq!(seen.len(), 3);
            assert_eq!(seen[0]["messages"].as_array().unwrap().len(), 1);
            assert_eq!(seen[1]["messages"].as_array().unwrap().len(), 1);
            assert_eq!(seen[2]["messages"][1]["content"], "A native");
        }
        let detail = super::super::detail(State(s.clone()), admin(1), Path(("1".into(), room)))
            .await
            .unwrap()
            .0;
        interactions::members(State(s.clone()),admin(1),Path(("1".into(),room)),Json(serde_json::from_value(json!({"version":detail["members_version"],"member_ids":[],"agent_ids":[Id::from(a.id)]})).unwrap())).await.unwrap();
        assert_eq!(
            sessions::Entity::find()
                .filter(sessions::Column::ConversationId.eq(room))
                .count(&s.db)
                .await
                .unwrap(),
            2,
            "removal retains private rows"
        );
        let detail = super::super::detail(State(s.clone()), admin(1), Path(("1".into(), room)))
            .await
            .unwrap()
            .0;
        interactions::members(State(s.clone()),admin(1),Path(("1".into(),room)),Json(serde_json::from_value(json!({"version":detail["members_version"],"member_ids":[],"agent_ids":[Id::from(a.id),Id::from(b.id)]})).unwrap())).await.unwrap();
        assert_eq!(
            sessions::Entity::find()
                .filter(sessions::Column::ConversationId.eq(room))
                .count(&s.db)
                .await
                .unwrap(),
            1,
            "explicit re-add resets just B"
        );
        group_turn(&s, room, vec![b.id.into()], "b-readded").await;
        assert_eq!(
            requests.lock().unwrap()[3]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        server.abort();
    }

    #[tokio::test]
    async fn api_chat_recovers_a_known_provider_response_after_interrupted_canonical_commit() {
        let (url, server, requests) = upstream(vec![
            answer("Durably journaled answer"),
            answer("Continued after restart"),
        ])
        .await;
        let (_dir, s, room) = fixture(&url).await;
        let first = sent(&s, room, "journaled").await;
        let (row, c, owner) = run::claim(&s, first).await.unwrap().unwrap();
        let (mut session, mut history, _) = load(&s, &row, &c, &owner).await.unwrap();
        history.turn_id = row.id;
        history
            .messages
            .push(json!({"role":"user","content":"Original request"}));
        history.request_inflight = true;
        checkpoint(&s, &row, &c, &owner, &mut session, &history, None, None)
            .await
            .unwrap();
        let params = json!({"provider_id":session.provider_id.to_string(),"model":session.model,"agent_messages":history.messages,"agent_tools":crate::workflows::ai::agent_tools(false,true)});
        crate::workflows::ai::conversation_round(&s, &row, &c, &owner, 0, &params)
            .await
            .unwrap();
        turns::Entity::update_many()
            .col_expr(turns::Column::ExpiresAt, Expr::value(0_i64))
            .filter(turns::Column::Id.eq(row.id))
            .exec(&s.db)
            .await
            .unwrap();
        run::claim(&s, first).await.unwrap();
        assert_eq!(
            execute_turn(&s, room, "after-known-result").await.state,
            "completed"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(
            seen.len(),
            2,
            "known previous call is recovered, never resent"
        );
        assert_eq!(
            seen[1]["messages"][1],
            json!({"role":"assistant","content":"Durably journaled answer"})
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_model_change_retains_context_and_room_delete_cleans_private_history() {
        let (url, server, requests) = upstream(vec![answer("Original context")]).await;
        let (_dir, s, room) = fixture(&url).await;
        assert_eq!(execute_turn(&s, room, "original").await.state, "completed");
        let saved = sessions::Entity::find().one(&s.db).await.unwrap().unwrap();
        crate::entities::ai_model_profiles::Entity::update_many()
            .col_expr(
                crate::entities::ai_model_profiles::Column::Models,
                Expr::value("[\"m\",\"other\"]"),
            )
            .filter(crate::entities::ai_model_profiles::Column::Id.eq(saved.model_profile_id))
            .exec(&s.db)
            .await
            .unwrap();
        ai_agents::Entity::update_many()
            .col_expr(ai_agents::Column::Model, Expr::value("other"))
            .filter(ai_agents::Column::Id.eq(saved.agent_id))
            .exec(&s.db)
            .await
            .unwrap();
        let failed = execute_turn(&s, room, "changed-model").await;
        assert_eq!(failed.state, "failed");
        assert!(failed.error.contains("conversations.apiContextChanged"));
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "no call is sent with incompatible history"
        );
        assert_eq!(
            sessions::Entity::find_by_id(saved.id)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .encrypted_history,
            saved.encrypted_history
        );
        remove(State(s.clone()), admin(1), Path(("1".into(), room)))
            .await
            .unwrap();
        assert_eq!(sessions::Entity::find().count(&s.db).await.unwrap(), 0);
        assert_eq!(
            crate::entities::ai_calls::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            1,
            "usage provenance retains existing ledger retention"
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_unsent_interruption_is_distinct_from_an_unknown_sent_request() {
        let (url, server, requests) = upstream(vec![answer("New request only")]).await;
        let (_dir, s, room) = fixture(&url).await;
        let turn = sent(&s, room, "unsent").await;
        let (row, c, owner) = run::claim(&s, turn).await.unwrap().unwrap();
        let (mut session, mut history, _) = load(&s, &row, &c, &owner).await.unwrap();
        history.turn_id = row.id;
        history.request_inflight = true;
        history
            .messages
            .push(json!({"role":"user","content":"Interrupted before reserve"}));
        checkpoint(&s, &row, &c, &owner, &mut session, &history, None, None)
            .await
            .unwrap();
        turns::Entity::update_many()
            .col_expr(turns::Column::ExpiresAt, Expr::value(0_i64))
            .filter(turns::Column::Id.eq(row.id))
            .exec(&s.db)
            .await
            .unwrap();
        run::claim(&s, turn).await.unwrap();
        assert_eq!(
            execute_turn(&s, room, "after-unsent").await.state,
            "completed"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0]["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("before provider dispatch")
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_anthropic_requires_an_explicit_model_output_setting_without_sending() {
        let (url, server, requests) = upstream(vec![]).await;
        let (_dir, s, room) = fixture(&url).await;
        providers::Entity::update_many()
            .col_expr(providers::Column::Protocol, Expr::value("anthropic"))
            .exec(&s.db)
            .await
            .unwrap();
        let row = execute_turn(&s, room, "anthropic-config").await;
        assert_eq!(row.state, "failed");
        assert!(row.error.contains("conversations.apiOutputRequired"));
        assert_eq!(requests.lock().unwrap().len(), 0);
        assert_eq!(
            crate::entities::ai_calls::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        server.abort();
    }
    #[test]
    fn api_chat_usage_preserves_unknowns_and_marks_overflow_unknown() {
        let mut history = History {
            usage_known: true,
            ..Default::default()
        };
        accumulate_usage(
            &mut history,
            Some(&json!({"input_tokens":2,"output_tokens":3})),
        );
        accumulate_usage(&mut history, None);
        accumulate_usage(
            &mut history,
            Some(&json!({"prompt_tokens":7,"completion_tokens":5})),
        );
        assert!(history.usage.is_none());
        assert!(!history.usage_known);
        history.usage_known = true;
        history.usage = Some(Usage {
            input_tokens: u64::MAX,
            output_tokens: 0,
        });
        accumulate_usage(
            &mut history,
            Some(&json!({"input_tokens":1,"output_tokens":0})),
        );
        assert!(history.usage.is_none());
        assert!(!history.usage_known);
    }

    #[tokio::test]
    async fn api_chat_recovers_the_durable_platform_receipt_without_replaying_the_tool() {
        let (url, server, requests) =
            upstream(vec![answer("Known operation receipt preserved")]).await;
        let (_dir, s, room) = fixture(&url).await;
        let turn = sent(&s, room, "receipt").await;
        let (row, c, owner) = run::claim(&s, turn).await.unwrap().unwrap();
        let (mut session, mut history, _) = load(&s, &row, &c, &owner).await.unwrap();
        history.turn_id = row.id;
        history.messages = vec![
            json!({"role":"user","content":"Original request"}),
            json!({"role":"assistant","tool_calls":[{"id":"native-known-tool","type":"function","function":{"name":"ferry_request","arguments":"{}"}}]}),
        ];
        history.pending = vec![Pending {
            id: "native-known-tool".into(),
            name: "ferry_request".into(),
            arguments: json!({"method":"POST","path":"/node-groups","body":{"name":"must not replay"}}),
            dispatching: true,
        }];
        checkpoint(&s, &row, &c, &owner, &mut session, &history, None, None)
            .await
            .unwrap();
        let known = json!({"ok":true,"status":201,"data":{"id":"durable-operation-receipt"}});
        run::append(
            &s,
            &row,
            &owner,
            Event::PlatformResult {
                request_id: "native-known-tool".into(),
                result: known.clone(),
                call: None,
            },
        )
        .await
        .unwrap();
        turns::Entity::update_many()
            .col_expr(turns::Column::ExpiresAt, Expr::value(0_i64))
            .filter(turns::Column::Id.eq(row.id))
            .exec(&s.db)
            .await
            .unwrap();
        run::claim(&s, turn).await.unwrap();
        assert_eq!(
            execute_turn(&s, room, "after-tool-receipt").await.state,
            "completed"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let recovered =
            serde_json::from_str::<Value>(seen[0]["messages"][2]["content"].as_str().unwrap())
                .unwrap();
        assert_eq!(recovered, known);
        assert_eq!(
            crate::entities::host_groups::Entity::find()
                .count(&s.db)
                .await
                .unwrap(),
            0,
            "recovery never re-dispatches the tool"
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_priced_responses_without_usable_metering_still_keep_known_results() {
        let replies = vec![
            json!({"choices":[{"message":{"role":"assistant","content":"Missing usage is still a known answer"}}]}),
            json!({"choices":[{"message":{"role":"assistant","content":"Partial usage is still a known answer"}}],"usage":{"prompt_tokens":1}}),
            json!({"choices":[{"message":{"role":"assistant","content":"Cost overflow is observational"}}],"usage":{"prompt_tokens":i64::MAX,"completion_tokens":i64::MAX}}),
        ];
        let (url, server, requests) = upstream(replies).await;
        let (_dir, s, room) = fixture(&url).await;
        providers::Entity::update_many().col_expr(providers::Column::Prices,Expr::value(json!([{"model":"m","operation":"ai.llm.chat","input_micros":1_000_000_000_000_i64,"output_micros":1_000_000_000_000_i64,"unit_size":1}]).to_string())).exec(&s.db).await.unwrap();
        for client in ["missing-cost", "partial-cost", "overflow-cost"] {
            let row = execute_turn(&s, room, client).await;
            assert_eq!(
                row.state, "completed",
                "valid provider body must survive unavailable metering: {}",
                row.error
            );
        }
        let calls = crate::entities::ai_calls::Entity::find()
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(calls.len(), 3);
        assert!(
            calls
                .iter()
                .all(|call| call.state == "succeeded" && call.result.is_some())
        );
        let public = crate::platform_tools::dispatch(
            &s,
            admin(1),
            crate::platform_tools::Call {
                method: "GET".into(),
                path: "/ai-calls".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(public["status"], 200, "{public}");
        assert!(
            public["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|call| call["actual_micros"].is_null()
                    && call["cost_known"] == false
                    && call["price_known"] == true)
        );
        assert_eq!(requests.lock().unwrap().len(), 3);
        server.abort();
    }
    async fn output_setting(s: &AppState, room: Id, limit: i64) -> i64 {
        let c = conversations::Entity::find_by_id(room)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let agent = ai_agents::Entity::find_by_id(c.agent_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let path = format!("/ai-models/{}", agent.model_profile_id);
        let result = crate::platform_tools::dispatch(
            &s,
            admin(1),
            crate::platform_tools::Call {
                method: "GET".into(),
                path: path.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(result["status"], 200, "{result}");
        let mut body = result["data"].clone();
        body["max_output_tokens"] = json!(limit);
        body["skip_check"] = json!(true);
        let saved = crate::platform_tools::dispatch(
            &s,
            admin(1),
            crate::platform_tools::Call {
                method: "PATCH".into(),
                path,
                body,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(saved["status"], 200, "{saved}");
        assert_eq!(saved["data"]["max_output_tokens"], limit);
        let row = crate::entities::ai_model_profiles::Entity::find_by_id(agent.model_profile_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.max_output_tokens, Some(limit));
        let revision = crate::entities::definition_revisions::Entity::find_by_id(row.revision_id)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&revision.definition).unwrap()["max_output_tokens"],
            limit
        );
        row.id
    }
    #[tokio::test]
    async fn api_chat_anthropic_native_history_resumes_with_configured_model_output_tokens() {
        let (url,server,requests)=upstream(vec![
            json!({"content":[{"type":"tool_use","id":"anthropic-native-id","name":"ferry_discover","input":{"search":"workflows"}}],"usage":{"input_tokens":2,"output_tokens":3}}),
            json!({"content":[{"type":"text","text":"Anthropic first answer"}],"usage":{"input_tokens":5,"output_tokens":7}}),
            json!({"content":[{"type":"text","text":"Anthropic continued context"}],"usage":{"input_tokens":3,"output_tokens":4}})
        ]).await;
        let (_dir, s, room) = fixture(&url).await;
        providers::Entity::update_many()
            .col_expr(providers::Column::Protocol, Expr::value("anthropic"))
            .exec(&s.db)
            .await
            .unwrap();
        output_setting(&s, room, 256).await;
        let first = execute_turn(&s, room, "anthropic-first").await;
        assert_eq!(first.state, "completed", "{}", first.error);
        assert_eq!(
            serde_json::from_str::<Value>(&first.usage).unwrap(),
            json!({"input_tokens":7,"output_tokens":10})
        );
        output_setting(&s, room, 512).await;
        assert_eq!(
            execute_turn(&s, room, "anthropic-next").await.state,
            "completed"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0]["max_tokens"], 256);
        assert_eq!(seen[1]["max_tokens"], 256);
        assert_eq!(seen[2]["max_tokens"], 512);
        assert_eq!(seen[1]["messages"][1]["role"], "assistant");
        assert_eq!(
            seen[1]["messages"][1]["content"][0]["id"],
            "anthropic-native-id"
        );
        assert_eq!(seen[1]["messages"][2]["role"], "user");
        assert_eq!(
            seen[1]["messages"][2]["content"][0]["tool_use_id"],
            "anthropic-native-id"
        );
        assert_eq!(
            seen[2]["messages"][3]["content"][0]["text"],
            "Anthropic first answer"
        );
        assert!(
            seen[0]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "ferry_ask_user" && tool["input_schema"].is_object())
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_openai_sends_the_optional_model_output_parameter_when_configured() {
        let (url, server, requests) = upstream(vec![answer("Configured OpenAI output")]).await;
        let (_dir, s, room) = fixture(&url).await;
        output_setting(&s, room, 17).await;
        assert_eq!(
            execute_turn(&s, room, "openai-output").await.state,
            "completed"
        );
        assert_eq!(requests.lock().unwrap()[0]["max_tokens"], 17);
        server.abort();
    }

    #[tokio::test]
    async fn api_chat_price_provenance_distinguishes_explicit_zero_from_unpriced_or_unmatched_models()
     {
        let (url, server, _) = upstream(vec![
            answer("Unpriced"),
            answer("Explicit zero"),
            answer("Unmatched price"),
        ])
        .await;
        let (_dir, s, room) = fixture(&url).await;
        assert_eq!(execute_turn(&s, room, "unpriced").await.state, "completed");
        providers::Entity::update_many().col_expr(providers::Column::Prices,Expr::value(json!([{"model":"m","operation":"ai.llm.chat","input_micros":0,"output_micros":0,"unit_size":1}]).to_string())).exec(&s.db).await.unwrap();
        assert_eq!(
            execute_turn(&s, room, "explicit-zero").await.state,
            "completed"
        );
        providers::Entity::update_many().col_expr(providers::Column::Prices,Expr::value(json!([{"model":"other","operation":"ai.llm.chat","input_micros":2,"output_micros":3,"unit_size":1}]).to_string())).exec(&s.db).await.unwrap();
        assert_eq!(
            execute_turn(&s, room, "unmatched-price").await.state,
            "completed"
        );
        let public = crate::platform_tools::dispatch(
            &s,
            admin(1),
            crate::platform_tools::Call {
                method: "GET".into(),
                path: "/ai-calls".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let items = public["data"]["items"].as_array().unwrap();
        assert_eq!(
            items
                .iter()
                .filter(|call| call["cost_known"] == true
                    && call["price_known"] == true
                    && call["actual_micros"] == 0)
                .count(),
            1
        );
        assert_eq!(
            items
                .iter()
                .filter(|call| call["cost_known"] == false
                    && call["price_known"] == false
                    && call["actual_micros"].is_null())
                .count(),
            2
        );
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_live_protocol_change_blocks_final_history_and_public_assistant() {
        let received = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (seen, wait) = (received.clone(), release.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/chat/completions",
                    axum::routing::post(move || {
                        let (seen, wait) = (seen.clone(), wait.clone());
                        async move {
                            seen.notify_one();
                            wait.notified().await;
                            axum::Json(answer("must not commit after protocol edit"))
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let (_dir, s, room) = fixture(&url).await;
        let turn = sent(&s, room, "protocol-race").await;
        let (_stop, signal) = watch::channel(false);
        let running = tokio::spawn(run::execute(s.clone(), turn, signal));
        tokio::time::timeout(std::time::Duration::from_secs(5), received.notified())
            .await
            .unwrap();
        providers::Entity::update_many()
            .col_expr(providers::Column::Protocol, Expr::value("anthropic"))
            .exec(&s.db)
            .await
            .unwrap();
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let row = turns::Entity::find_by_id(turn)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "failed");
        assert!(row.error.contains("conversations.apiContextChanged"));
        assert_eq!(
            events::Entity::find()
                .filter(events::Column::TurnId.eq(turn))
                .count(&s.db)
                .await
                .unwrap(),
            0
        );
        let session = sessions::Entity::find().one(&s.db).await.unwrap().unwrap();
        let history: History = serde_json::from_str(
            &s.vault
                .open_secret(&session.encrypted_history, &aad(&session))
                .unwrap(),
        )
        .unwrap();
        assert!(
            history
                .messages
                .iter()
                .all(|message| message["role"] != "assistant")
        );
        let call = crate::entities::ai_calls::Entity::find()
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            call.state, "succeeded",
            "known provider response remains privately journaled"
        );
        assert!(call.result.is_some());
        server.abort();
    }
    #[tokio::test]
    async fn api_chat_overflowing_aggregate_token_usage_is_unknown_without_failing_known_output() {
        let first = json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"usage-overflow-tool","type":"function","function":{"name":"ferry_discover","arguments":"{}"}}]}}],"usage":{"prompt_tokens":u64::MAX,"completion_tokens":u64::MAX}});
        let (url, server, requests) = upstream(vec![
            first,
            answer("Known output survives token counter overflow"),
        ])
        .await;
        let (_dir, s, room) = fixture(&url).await;
        let row = execute_turn(&s, room, "usage-overflow").await;
        assert_eq!(
            row.state, "completed",
            "token usage is observational: {}",
            row.error
        );
        assert_eq!(
            serde_json::from_str::<Value>(&row.usage).unwrap(),
            Value::Null
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert!(
            crate::entities::ai_calls::Entity::find()
                .all(&s.db)
                .await
                .unwrap()
                .iter()
                .all(|call| call.state == "succeeded" && call.result.is_some())
        );
        server.abort();
    }
}
