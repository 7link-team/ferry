//! API-backed workflow Agents. Reuses the provider call journal and the same
//! live, read-only platform gateway as native Agents; never allocates a host.
use super::*;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::super::ai::{agent_response, agent_tools};

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    agent: &ai_agents::Model,
    params: &Value,
    items: &[Item],
    validator: &jsonschema::Validator,
    max_calls: usize,
    timeout: Duration,
    clock: Option<&Value>,
) -> Result<Map<String, Value>> {
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let profile = crate::agents::model_profile(
        &s.db,
        run.workspace_id.into(),
        agent.model_profile_id.into(),
    )
    .await?;
    let provider = crate::entities::ai_providers::Entity::find_by_id(profile.provider_id)
        .filter(crate::entities::ai_providers::Column::WorkspaceId.eq(run.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let anthropic = provider.protocol == "anthropic";
    if anthropic
        && params
            .get("max_tokens")
            .and_then(Value::as_u64)
            .is_none_or(|value| value == 0)
    {
        return Err(invalid(
            "workflow.agentMaxTokensRequired",
            "/params/max_tokens".into(),
        ));
    }
    let mut messages = vec![json!({"role":"user","content":compose(&agent.system_prompt,
        params["prompt"].as_str().unwrap_or_default(), items, Some(&params["output_schema"]), false, clock)?})];
    let mut log = NodeLog::with_limit(policy.log_output_bytes);
    let node_log = (node_run > 0).then_some(node_run);
    let mut active_round = None;
    let result = {
        let work = async {
            let backend = crate::artifact_store::ArtifactStore::for_state(s).await?;
            let storage = crate::content_assets::Store {
                db: &s.db,
                backend: &backend,
                vault: &s.vault,
                ids: &s.ids,
                quota: crate::content_assets::DEFAULT_QUOTA,
            };
            let mut used = 0usize;
            let mut received = 0usize;
            let mut corrected = false;
            // At least one tool is consumed in each nonfinal iteration. One
            // schema correction is allowed, matching the native Agent path.
            for round in 0..max_calls.saturating_add(2) {
                let scope = super::super::integration::execution_scope(s, run).await?;
                let current =
                    crate::conversations::available(&s.db, &scope, agent.id.into()).await?;
                let resolved = crate::agents::resolve_model(&s.db, &current).await?;
                if resolved.model_profile_id != agent.model_profile_id
                    || resolved.model != agent.model
                    || resolved.runtime != "api"
                {
                    return Err(invalid(
                        "workflow.agentUnavailable",
                        "/params/agent_id".into(),
                    ));
                }
                let mut p = json!({"provider_id":profile.provider_id.to_string(),"model":agent.model,
                    "prompt":"","output_field":"response",
                    "agent_messages":messages,"agent_tools":agent_tools(anthropic, false)});
                if let Some(max_tokens) = params.get("max_tokens").filter(|value| !value.is_null())
                {
                    p["max_tokens"] = max_tokens.clone();
                }
                within_policy(
                    serde_json::to_string(&p).map_err(|_| ApiError::missing())?,
                    &policy,
                )?;
                active_round = Some(round);
                let output = loop {
                    let result = super::super::ai::execute_indexed(
                        s,
                        &storage,
                        run,
                        node_run,
                        &json!({"id":"api_agent","type":"ai.llm.chat"}),
                        &[Item {
                            json: Map::new(),
                            binary: BTreeMap::new(),
                            paired: vec![],
                        }],
                        std::slice::from_ref(&p),
                        round,
                    )
                    .await;
                    match result {
                        // Local admission refused before sending. Wait for
                        // capacity within the node deadline; never retry an
                        // upstream request with an uncertain outcome.
                        Err(error)
                            if error
                                .2
                                .as_ref()
                                .is_some_and(|message| message.key == "workflow.aiLimit") =>
                        {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        other => break other?,
                    }
                };
                active_round = None;
                let body = &output[0].json["response"];
                received = received.saturating_add(
                    serde_json::to_vec(body)
                        .map_err(|_| ApiError::missing())?
                        .len(),
                );
                if !allows(policy.agent_turn_output_bytes, received) {
                    return Err(failed("workflow.agentOutputTooLarge"));
                }
                let (message, text, calls) = agent_response(body, anthropic)?;
                messages.push(message);
                if calls.is_empty() {
                    match accept_with_policy(&text, validator, &policy) {
                        Ok(object) => return Ok(object),
                        Err(error) if !corrected => {
                            corrected = true;
                            messages.push(json!({"role":"user","content":format!(
                                "Your final answer was rejected: {}. Reply with one JSON object matching the original output schema.",
                                error.1.chars().take(2000).collect::<String>())}));
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    used += calls.len();
                    if used > max_calls {
                        return Err(ApiError::localized(
                            StatusCode::BAD_REQUEST,
                            Message::new("workflow.agentToolLimit")
                                .with("max", max_calls.to_string()),
                        ));
                    }
                    let mut results = Vec::new();
                    for (id, name, arguments) in calls {
                        let (id, result, summary) = platform_call(
                            s.clone(),
                            run.clone(),
                            agent.id.into(),
                            id,
                            name,
                            arguments,
                        )
                        .await;
                        log.push("tool", summary.as_bytes());
                        log.flush(s, node_log).await?;
                        if anthropic {
                            results.push(json!({"type":"tool_result","tool_use_id":id,"content":result.to_string()}));
                        } else {
                            messages.push(json!({"role":"tool","tool_call_id":id,"content":result.to_string()}));
                        }
                    }
                    if anthropic {
                        messages.push(json!({"role":"user","content":results}));
                    }
                }
            }
            Err(failed("workflow.agentOutputTooLarge"))
        };
        tokio::pin!(work);
        let (_idle, idle) = watch::channel(false);
        let mut stop = crate::executions::stop_signal().unwrap_or(idle);
        let deadline = async {
            if timeout.is_zero() {
                std::future::pending().await
            } else {
                tokio::time::sleep(timeout).await
            }
        };
        tokio::pin!(deadline);
        // Keep database checks polled alongside the request. Awaiting a check
        // inside a select branch can deadlock a one-connection pool while the
        // suspended request owns a transaction on that connection.
        let monitor = async {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if crate::executions::stop_requested() {
                    return Err::<Map<String, Value>, ApiError>(crate::executions::ownership_lost());
                }
                let scope = super::super::integration::execution_scope(s, run).await?;
                crate::conversations::available(&s.db, &scope, agent.id.into()).await?;
            }
        };
        tokio::pin!(monitor);
        tokio::select! {
                result = &mut work => result,
                _ = &mut deadline => Err(ApiError::localized(StatusCode::GATEWAY_TIMEOUT,
                    Message::new("workflow.agentTimeout").with("seconds", timeout.as_secs().to_string()))),
                _ = stop.changed() => Err(crate::executions::ownership_lost()),
                result = &mut monitor => result,
        }
    };
    if let Some(round) = active_round {
        // The work future is dropped above, so it cannot race this cleanup.
        super::super::ai::interrupted(s, run, node_run, round).await?;
    }
    let _ = log.flush(s, node_log).await;
    result
}
