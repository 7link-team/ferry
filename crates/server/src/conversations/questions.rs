//! Durable user questions. Answering is a human action, never an Agent grant.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionItem {
    id: String,
    label: String,
    #[serde(default)]
    description: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Question {
    title: String,
    question: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    options: Vec<OptionItem>,
    #[serde(default)]
    recommended_option_id: Option<String>,
}
impl Question {
    fn validate(&self) -> Result<()> {
        let bounded = |value: &str, max: usize| {
            !value.trim().is_empty() && !value.contains('\0') && value.chars().count() <= max
        };
        if !bounded(&self.title, 120)
            || !bounded(&self.question, 2048)
            || self.reason.len() > 4096
            || self.options.len() > 6
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > 24 * 1024
        {
            return Err(bad("conversations.questionInvalid"));
        }
        let mut ids = std::collections::HashSet::new();
        for option in &self.options {
            if !bounded(&option.id, 64)
                || !bounded(&option.label, 160)
                || option.description.len() > 2048
                || !ids.insert(&option.id)
            {
                return Err(bad("conversations.questionInvalid"));
            }
        }
        if self
            .recommended_option_id
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
        {
            return Err(bad("conversations.questionInvalid"));
        }
        Ok(())
    }
}

pub(super) fn pending(turn: &turns::Model) -> bool {
    serde_json::from_str::<Value>(&turn.decisions)
        .ok()
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|items| {
            items
                .values()
                .any(|item| item["tool"] == "ferry_ask_user" && !item["resolved"].is_string())
        })
}

pub(super) async fn record(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    request: &str,
    arguments: Value,
) -> Result<Value> {
    let question: Question =
        serde_json::from_value(arguments).map_err(|_| bad("conversations.questionInvalid"))?;
    question.validate()?;
    run::append(
        s,
        row,
        owner,
        ferry_protocol::agent::Event::Approval {
            request_id: request.into(),
            tool: "ferry_ask_user".into(),
            input: serde_json::to_value(question).map_err(|_| invalid())?,
        },
    )
    .await?;
    Ok(
        json!({"status":200,"ok":true,"data":{"state":"awaiting_user","request_id":request,
        "instruction":"The question is saved in this conversation. End this turn and wait for the user's reply. Do not perform further operations or ask again. The user can choose a suggestion, enter arbitrary text, pause, or abandon. An answer continues in this same conversation; it does not approve a platform write."}}),
    )
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    request_id: String,
    action: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    option_id: Option<String>,
}

/// Human-only response to a saved Agent question: continue with arbitrary text
/// and/or a suggested option, pause, or abandon. Agents cannot answer themselves.
pub(super) async fn answer(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, turn)): Path<(String, Id, Id)>,
    Json(body): Json<Answer>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    if crate::platform_tools::current_agent().is_some() {
        return Err(ApiError::forbidden());
    }
    if !["continue", "pause", "abandon"].contains(&body.action.as_str())
        || body.answer.len() > 4096
        || body.answer.contains('\0')
    {
        return Err(bad("conversations.questionInvalid"));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    let mut row = find_turn(&tx, &c, turn).await?;
    if row.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    let execution = groups::execution(&tx, &row, c.clone()).await?;
    let mut decisions: Value = serde_json::from_str(&row.decisions).map_err(|_| invalid())?;
    let item = decisions
        .get_mut(&body.request_id)
        .filter(|item| item["tool"] == "ferry_ask_user")
        .ok_or_else(ApiError::missing)?;
    let question: Question =
        serde_json::from_value(item["input"].clone()).map_err(|_| invalid())?;
    let option = body
        .option_id
        .as_ref()
        .map(|id| {
            question
                .options
                .iter()
                .find(|option| &option.id == id)
                .ok_or_else(|| bad("conversations.questionInvalid"))
        })
        .transpose()?;
    if body.action == "continue" && body.answer.trim().is_empty() && option.is_none() {
        return Err(bad("conversations.questionInvalid"));
    }
    let response = serde_json::to_value(&body).map_err(|_| invalid())?;
    if let Some(previous) = item.get("response").filter(|value| !value.is_null()) {
        if previous == &response {
            if let Some(receipt) = item.get("receipt") {
                return Ok(Json(receipt.clone()));
            }
            if body.action != "continue" {
                return Ok(Json(json!({"ok":true,"state":item["resolved"]})));
            }
        } else if item["resolved"] != "paused" {
            return Err(error(
                StatusCode::CONFLICT,
                "conversations.questionAnswered",
            ));
        }
    }
    if body.action == "continue" && active(&row.state) {
        return Err(error(StatusCode::CONFLICT, "conversations.questionPending"));
    }
    if body.action == "continue" {
        usable(&tx, &scope, execution.agent_id.into()).await?;
    }
    let selected = option
        .map(|option| option.label.clone())
        .unwrap_or_default();
    let text = match (selected.is_empty(), body.answer.trim().is_empty()) {
        (false, false) => format!("{selected}\n{}", body.answer.trim()),
        (false, true) => selected,
        _ => body.answer.trim().to_owned(),
    };
    let prompt = format!("{}\n\n{text}", question.question);
    let client_id = format!(
        "question-{}",
        crate::hosts::runner::fingerprint(&format!(
            "{}:{}:{}:{}",
            scope.user.id, turn, body.request_id, response
        ))
    );
    item["response"] = response;
    item["decided_by"] = json!(scope.user.id);
    if body.action != "continue" {
        item["resolved"] = json!(if body.action == "pause" {
            "paused"
        } else {
            "abandoned"
        });
        if active(&row.state) {
            row.state = if row.state == "queued" {
                "cancelled"
            } else {
                "stopping"
            }
            .into();
        }
    } else {
        // The continuation is committed before normal turn admission. Clear a
        // prior pause so a failed send can only retry this exact response;
        // otherwise another answer could replace it and enqueue a second task.
        item["resolved"] = Value::Null;
    }
    row.decisions = decisions.to_string();
    row.version += 1;
    row.updated_at = db::now();
    row.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(&s,&tx,&scope,"decision","conversations",&id,
        json!({"kind":"question","turn_id":turn,"request_id":body.request_id,"action":body.action,"answer":body.answer,"option_id":body.option_id})).await?;
    tx.commit().await?;
    if body.action != "continue" {
        return Ok(Json(
            json!({"ok":true,"state":if body.action=="pause" {"paused"} else {"abandoned"}}),
        ));
    }

    // Existing send endpoints own admission, group targeting and idempotency.
    // Persisting the response first makes a retry after a network failure safe.
    let (_, Json(receipt)) = if groups::is_group(&s.db, id).await? {
        let message = serde_json::from_value(
            json!({"prompt":prompt,"client_id":client_id,"targets":[Id::from(execution.agent_id)]}),
        )
        .map_err(|_| invalid())?;
        groups::send_message(
            State(s.clone()),
            scope.clone(),
            Path((String::new(), id)),
            Json(message),
        )
        .await?
    } else {
        super::send(
            State(s.clone()),
            scope.clone(),
            Path((String::new(), id)),
            Json(Send {
                prompt,
                client_id,
                attachments: vec![],
                reply_to: None,
            }),
        )
        .await?
    };
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    let mut row = find_turn(&tx, &c, turn).await?;
    let mut decisions: Value = serde_json::from_str(&row.decisions).map_err(|_| invalid())?;
    let item = decisions.get_mut(&body.request_id).ok_or_else(invalid)?;
    item["resolved"] = json!("answered");
    item["receipt"] = receipt.clone();
    row.decisions = decisions.to_string();
    row.version += 1;
    row.updated_at = db::now();
    row.into_active_model().reset_all().update(&tx).await?;
    tx.commit().await?;
    Ok(Json(receipt))
}
