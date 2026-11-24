//! Threads in rooms for pipelines and workflows created or run there. Links are
//! recorded from the room's own successful platform calls, so pipeline and
//! workflow modules know nothing about conversations. A sweep posts finished
//! runs into their thread and wakes the room Agent that started them.
use super::*;
use crate::entities::{
    ai_chat_agents as agents, ai_chat_run_links as run_links, ai_chat_thread_items as items,
    ai_chat_threads as threads, ai_chat_turns as links, pipeline_runs, pipelines,
    workflow_executions, workflows,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/{workspace}/resource-conversations/{kind}/{id}",
        get(origin),
    )
}

/// Run states after which nothing more happens to a run.
const FINISHED: [&str; 8] = [
    "succeeded",
    "success",
    "failed",
    "cancelled",
    "canceled",
    "timed_out",
    "error",
    "interrupted",
];

enum Recorded {
    Created(&'static str, i64),
    Started(&'static str, i64, i64),
}

fn classify(method: &str, path: &str, data: &Value) -> Option<Recorded> {
    if method != "POST" {
        return None;
    }
    let id: i64 = data["id"].as_str()?.parse().ok()?;
    let parts: Vec<&str> = path
        .split('?')
        .next()?
        .trim_matches('/')
        .split('/')
        .collect();
    match parts.as_slice() {
        ["pipelines"] => Some(Recorded::Created("pipeline", id)),
        ["workflows"] => Some(Recorded::Created("workflow", id)),
        ["pipelines", resource, "runs"] => {
            Some(Recorded::Started("pipeline", resource.parse().ok()?, id))
        }
        ["workflows", resource, "executions"] => {
            Some(Recorded::Started("workflow", resource.parse().ok()?, id))
        }
        _ => None,
    }
}

/// Records what a room turn's successful platform call created or started.
/// Never fails the call itself.
pub(super) async fn record(
    s: &AppState,
    turn: &turns::Model,
    method: &str,
    path: &str,
    result: &Value,
) {
    if result["ok"] != true {
        return;
    }
    let Some(recorded) = classify(method, path, &result["data"]) else {
        return;
    };
    if let Err(error) = record_in(s, turn, recorded).await {
        tracing::warn!(%error, turn = turn.id, "Could not link a room thread");
    }
}

async fn record_in(s: &AppState, turn: &turns::Model, recorded: Recorded) -> Result<()> {
    let tx = crate::variables::transaction(s, turn.workspace_id.into()).await?;
    let Some(link) = links::Entity::find_by_id(turn.id).one(&tx).await? else {
        return Ok(());
    };
    let (kind, resource, run) = match recorded {
        Recorded::Created(kind, resource) => (kind, resource, None),
        Recorded::Started(kind, resource, run) => (kind, resource, Some(run)),
    };
    let thread = ensure(s, &tx, turn, kind, resource, run.is_none()).await?;
    let message = match run {
        None => Message::new("conversations.threadCreated"),
        Some(run) => {
            run_links::Entity::insert(
                run_links::Model {
                    id: run,
                    workspace_id: turn.workspace_id,
                    conversation_id: turn.conversation_id,
                    thread_id: thread,
                    kind: if kind == "pipeline" {
                        "pipeline_run".into()
                    } else {
                        "workflow_execution".into()
                    },
                    turn_id: turn.id,
                    chat_agent_id: link.chat_agent_id,
                    hop: link.hop,
                    notified_at: None,
                }
                .into_active_model(),
            )
            .exec(&tx)
            .await?;
            Message::new("conversations.threadRunStarted").with("run", Id::from(run).to_string())
        }
    };
    post(
        s,
        &tx,
        turn.workspace_id,
        turn.conversation_id,
        turn.owner_id,
        thread,
        message,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The room's thread for a resource; marks it linked when the room created it.
async fn ensure(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    turn: &turns::Model,
    kind: &str,
    resource: i64,
    created_here: bool,
) -> Result<i64> {
    if let Some(mut existing) = threads::Entity::find()
        .filter(threads::Column::ConversationId.eq(turn.conversation_id))
        .filter(threads::Column::Kind.eq(kind))
        .filter(threads::Column::ResourceId.eq(resource))
        .one(tx)
        .await?
    {
        if created_here && existing.linked == 0 {
            existing.linked = 1;
            existing
                .clone()
                .into_active_model()
                .reset_all()
                .update(tx)
                .await?;
        }
        return Ok(existing.id);
    }
    let id: i64 = s.next_id()?.into();
    threads::Entity::insert(
        threads::Model {
            id,
            workspace_id: turn.workspace_id,
            conversation_id: turn.conversation_id,
            kind: kind.into(),
            resource_id: resource,
            linked: i16::from(created_here),
            created_by: turn.owner_id,
            created_at: db::now(),
        }
        .into_active_model(),
    )
    .exec(tx)
    .await?;
    Ok(id)
}

/// A platform line inside a thread.
async fn post(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    workspace: i64,
    conversation: i64,
    owner: i64,
    thread: i64,
    message: Message,
) -> Result<i64> {
    let id = super::placement::notice(s, tx, workspace, conversation, owner, 0, message).await?;
    items::Entity::insert(
        items::Model {
            message_id: id.into(),
            thread_id: thread,
        }
        .into_active_model(),
    )
    .exec(tx)
    .await?;
    Ok(id.into())
}

/// Posts finished runs into their thread once and queues a follow-up turn for
/// the room Agent that started them. Returns how many runs were reported.
pub async fn sweep(s: &AppState) -> Result<usize> {
    let pending = run_links::Entity::find()
        .filter(run_links::Column::NotifiedAt.is_null())
        .all(&s.db)
        .await?;
    let mut reported = 0;
    for link in pending {
        let Some(summary) = finished(&s.db, &link).await? else {
            continue;
        };
        let tx = crate::variables::transaction(s, link.workspace_id.into()).await?;
        // Single winner: only the transaction that sets notified_at reports.
        let claimed = run_links::Entity::update_many()
            .col_expr(
                run_links::Column::NotifiedAt,
                sea_orm::sea_query::Expr::value(db::now()),
            )
            .filter(run_links::Column::Id.eq(link.id))
            .filter(run_links::Column::NotifiedAt.is_null())
            .exec(&tx)
            .await?;
        if claimed.rows_affected != 1 {
            continue;
        }
        let owner = conversations::Entity::find_by_id(link.conversation_id)
            .one(&tx)
            .await?
            .map(|c| c.owner_id);
        let Some(owner) = owner else {
            tx.commit().await?;
            continue;
        };
        let message = Message::new("conversations.threadRunFinished")
            .with("run", Id::from(link.id).to_string())
            .with("state", summary.state.clone());
        let notice = post(
            s,
            &tx,
            link.workspace_id,
            link.conversation_id,
            owner,
            link.thread_id,
            message,
        )
        .await?;
        if link.chat_agent_id != 0
            && let Some(agent) = agents::Entity::find_by_id(link.chat_agent_id)
                .filter(agents::Column::State.eq("active"))
                .one(&tx)
                .await?
        {
            let policy = crate::accounts::conversation_policy::Configuration::load(&tx).await?;
            let hop = link.hop + 1;
            let room = conversations::Entity::find_by_id(link.conversation_id)
                .one(&tx)
                .await?;
            let relay_on = room
                .and_then(|c| serde_json::from_str::<Value>(&c.config).ok())
                .and_then(|c| c["relay_enabled"].as_bool())
                .unwrap_or(true);
            let held = hop > policy.agent_relay_max_hops || !relay_on;
            let prompt = format!(
                "The {} you started from this room finished with state {}. {}Read details with ferry_request {}. Continue the task, or tell the room what is needed.",
                if link.kind == "pipeline_run" {
                    "pipeline run"
                } else {
                    "workflow execution"
                },
                summary.state,
                summary.detail,
                if link.kind == "pipeline_run" {
                    format!("GET /pipeline-runs/{}/logs", Id::from(link.id))
                } else {
                    format!("GET /workflow-executions/{}/node-runs", Id::from(link.id))
                },
            );
            // The follow-up hangs off the thread's result line.
            groups::queue_turn(
                s,
                &tx,
                &agent,
                groups::Queued {
                    message: notice,
                    prompt: &prompt,
                    attachments: "[]",
                    state: if held { "held" } else { "queued" },
                    hop,
                    caller: 0,
                },
            )
            .await?;
        }
        tx.commit().await?;
        reported += 1;
    }
    Ok(reported)
}

struct Finished {
    state: String,
    detail: String,
}

async fn finished(
    conn: &impl ConnectionTrait,
    link: &run_links::Model,
) -> Result<Option<Finished>> {
    let state = if link.kind == "pipeline_run" {
        pipeline_runs::Entity::find_by_id(link.id)
            .one(conn)
            .await?
            .map(|r| r.state)
    } else {
        workflow_executions::Entity::find_by_id(link.id)
            .one(conn)
            .await?
            .map(|r| r.state)
    };
    Ok(match state {
        // A deleted run still ends the wait, so the thread is not stuck.
        None => Some(Finished {
            state: "deleted".into(),
            detail: String::new(),
        }),
        Some(state) if FINISHED.contains(&state.as_str()) => Some(Finished {
            detail: if matches!(state.as_str(), "succeeded" | "success") {
                String::new()
            } else {
                "It did not succeed. ".into()
            },
            state,
        }),
        Some(_) => None,
    })
}

/// Thread data for room detail: name, latest run and whether the resource still exists.
pub(super) async fn detail(
    conn: &impl ConnectionTrait,
    c: &conversations::Model,
    value: &mut Value,
) -> Result<()> {
    let rows = threads::Entity::find()
        .filter(threads::Column::ConversationId.eq(c.id))
        .order_by_asc(threads::Column::Id)
        .all(conn)
        .await?;
    let mut out = vec![];
    for t in rows {
        let name = if t.kind == "pipeline" {
            pipelines::Entity::find_by_id(t.resource_id)
                .one(conn)
                .await?
                .map(|p| p.name)
        } else {
            workflows::Entity::find_by_id(t.resource_id)
                .one(conn)
                .await?
                .map(|w| w.name)
        };
        let latest = run_links::Entity::find()
            .filter(run_links::Column::ThreadId.eq(t.id))
            .order_by_desc(run_links::Column::Id)
            .one(conn)
            .await?;
        let latest = match latest {
            None => Value::Null,
            Some(l) => {
                let state = if l.kind == "pipeline_run" {
                    pipeline_runs::Entity::find_by_id(l.id)
                        .one(conn)
                        .await?
                        .map(|r| r.state)
                } else {
                    workflow_executions::Entity::find_by_id(l.id)
                        .one(conn)
                        .await?
                        .map(|r| r.state)
                };
                json!({"id":Id::from(l.id),"kind":l.kind,"state":state})
            }
        };
        out.push(
            json!({"id":Id::from(t.id),"kind":t.kind,"resource_id":Id::from(t.resource_id),
            "name":name,"deleted":name.is_none(),"linked":t.linked != 0,"latest_run":latest}),
        );
    }
    value["threads"] = json!(out);
    let thread_of: std::collections::HashMap<i64, i64> = items::Entity::find()
        .filter(
            items::Column::ThreadId.is_in(
                value["threads"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(|t| t["id"].as_str()?.parse::<i64>().ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            ),
        )
        .all(conn)
        .await?
        .into_iter()
        .map(|i| (i.message_id, i.thread_id))
        .collect();
    if let Some(list) = value["messages"].as_array_mut() {
        for m in list {
            let id: Option<i64> = m["id"].as_str().and_then(|v| v.parse().ok());
            m["thread_id"] = id
                .and_then(|id| thread_of.get(&id))
                .map(|t| json!(Id::from(*t)))
                .unwrap_or(Value::Null);
        }
    }
    Ok(())
}

/// Puts a human room message into one of the room's threads.
pub(super) async fn file_message(
    tx: &sea_orm::DatabaseTransaction,
    conversation: Id,
    thread: Id,
    message: i64,
) -> Result<()> {
    threads::Entity::find_by_id(thread)
        .filter(threads::Column::ConversationId.eq(conversation))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    items::Entity::insert(
        items::Model {
            message_id: message,
            thread_id: thread.into(),
        }
        .into_active_model(),
    )
    .exec(tx)
    .await?;
    Ok(())
}

/// Deleting a room removes only its threads and links, never the resources.
pub(super) async fn delete(tx: &sea_orm::DatabaseTransaction, conversation: Id) -> Result<()> {
    let ids: Vec<i64> = threads::Entity::find()
        .select_only()
        .column(threads::Column::Id)
        .filter(threads::Column::ConversationId.eq(conversation))
        .into_tuple()
        .all(tx)
        .await?;
    items::Entity::delete_many()
        .filter(items::Column::ThreadId.is_in(ids))
        .exec(tx)
        .await?;
    run_links::Entity::delete_many()
        .filter(run_links::Column::ConversationId.eq(conversation))
        .exec(tx)
        .await?;
    threads::Entity::delete_many()
        .filter(threads::Column::ConversationId.eq(conversation))
        .exec(tx)
        .await?;
    Ok(())
}

/// Rooms the viewer can see that created this pipeline or workflow.
async fn origin(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind, id)): Path<(String, String, Id)>,
) -> Result<Json<Value>> {
    scope.require("conversations.read")?;
    if !["pipeline", "workflow"].contains(&kind.as_str()) {
        return Err(invalid());
    }
    let rows = threads::Entity::find()
        .filter(threads::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(threads::Column::Kind.eq(kind))
        .filter(threads::Column::ResourceId.eq(id))
        .filter(threads::Column::Linked.eq(1))
        .all(&s.db)
        .await?;
    let mut out = vec![];
    for t in rows {
        if let Ok(c) = find(&s.db, &scope, t.conversation_id.into()).await {
            out.push(json!({"conversation_id":Id::from(c.id),"title":c.title,"thread_id":Id::from(t.id)}));
        }
    }
    Ok(Json(json!({"items":out})))
}
