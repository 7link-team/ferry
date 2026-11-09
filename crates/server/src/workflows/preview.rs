//! `host.preview` (docs/preview-automation-design.md section 6): optionally
//! start a service on a runner host, open a preview of its port, and with
//! `review: human` park the execution until a person approves or rejects it.
//! The service outlives the node and ends with the execution: `run::finish`
//! and the immediate cancellation call `stop`, and each service also stops on
//! its own once it sees the execution in a terminal state.
use super::journal::{Output, Port};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{holes, hosts, workflow_executions as runs, workflow_node_runs as nodes},
    error::Result,
    holes::{OwnedRequest, close_all, open_owned, revoke_source, wait_ready, wait_review},
    hosts::channel::HostChannel,
    snowflake::Id,
    state::AppState,
};
use bytes::Bytes;
use ferry_protocol::sandbox::Item;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};

/// Longer than any preview lives (24 h), so the preview ends first.
const SERVICE_LIMIT: Duration = Duration::from_secs(25 * 3600);
/// Startup output goes to the node log while the node waits for the port.
const LOG_WINDOW: Duration = Duration::from_secs(600 + 30);
const CLEANUP: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const WATCH_EVERY: Duration = Duration::from_secs(5);
#[cfg(test)]
const WATCH_EVERY: Duration = Duration::from_millis(100);
const TERMINAL: [&str; 3] = ["succeeded", "failed", "cancelled"];

/// What the node does next: park until the review deadline, or complete.
pub(super) enum Step {
    Wait(i64),
    Done(Output),
}

/// Frozen parameters, with the contract's defaults applied.
struct Request {
    host_id: i64,
    command: Option<String>,
    input_env: Value,
    port: i64,
    name: String,
    strip_prefix: bool,
    ready_seconds: u64,
    human: bool,
    review_minutes: i64,
}

impl Request {
    fn parse(params: &Value) -> Result<Self> {
        let number = |key: &str, default: i64| params[key].as_i64().unwrap_or(default);
        Ok(Self {
            host_id: params["host_id"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .filter(|id| *id > 0)
                .ok_or_else(super::host::invalid)?,
            command: params["command"]
                .as_str()
                .filter(|command| !command.trim().is_empty())
                .map(str::to_owned),
            input_env: params.get("input_env").cloned().unwrap_or(json!({})),
            port: params["port"].as_i64().ok_or_else(super::host::invalid)?,
            name: params["name"].as_str().unwrap_or_default().to_owned(),
            strip_prefix: params["strip_prefix"].as_bool().unwrap_or(true),
            ready_seconds: u64::try_from(number("ready_timeout_seconds", 60))
                .map_err(|_| super::host::invalid())?,
            human: human(params),
            review_minutes: number("review_timeout_minutes", 60),
        })
    }
}

fn human(params: &Value) -> bool {
    params["review"] == "human"
}

/// The output ports a node declares for its review mode.
fn port_names(params: &Value) -> &'static [&'static str] {
    if human(params) {
        &["approved", "rejected"]
    } else {
        &["main"]
    }
}

/// Every port skipped, for a muted node or skipped input.
pub(super) fn skipped(spec: &Value) -> Output {
    Output {
        version: 1,
        ports: port_names(&spec["params"])
            .iter()
            .map(|name| Port {
                name: (*name).into(),
                index: 0,
                skipped: true,
                items: vec![],
            })
            .collect(),
    }
}

/// Graph rule: a node only connects the ports of its review mode.
pub(super) fn port_allowed(params: &Value, port: &str) -> bool {
    port_names(params).contains(&port)
}

pub(super) async fn execute(
    s: &AppState,
    run: &runs::Model,
    node: &str,
    params: &Value,
) -> Result<Step> {
    super::host::active(s, run).await?;
    let request = Request::parse(params)?;
    // A resumed attempt (review wake-up or recovery) never starts a second service.
    if let Some(hole) = latest(&s.db, run.id, node).await? {
        return settle(s, hole.id, request.human).await;
    }
    let scope = Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
    start(s, &scope, run.id, node, &request).await
}

async fn start(
    s: &AppState,
    scope: &Scope,
    execution: i64,
    node: &str,
    request: &Request,
) -> Result<Step> {
    // Checks holes.create and that the host is a runner host of this workspace.
    let hole = open_owned(
        s,
        scope,
        OwnedRequest {
            host_id: Id::from(request.host_id),
            port: request.port,
            name: request.name.clone(),
            strip_prefix: request.strip_prefix,
            source_kind: "workflow",
            source_id: execution,
            source_ref: node.into(),
            review_minutes: request.human.then_some(request.review_minutes),
        },
    )
    .await?;
    if let Some(command) = &request.command {
        if let Err(error) = serve(
            s,
            scope.workspace_id.into(),
            execution,
            node,
            hole.host_id,
            command,
            &request.input_env,
        )
        .await
        {
            close_all(&revoke_source(&s.db, "workflow", execution, Some(node)).await?);
            return Err(error);
        }
    }
    wait_ready(s, &hole, Duration::from_secs(request.ready_seconds)).await?;
    settle(s, hole.id, request.human).await
}

fn pending(row: &holes::Model, now: i64) -> bool {
    row.review_state == "waiting" && row.revoked_at == 0 && row.expires_at > now
}

async fn latest(
    db: &impl ConnectionTrait,
    execution: i64,
    node: &str,
) -> Result<Option<holes::Model>> {
    Ok(holes::Entity::find()
        .filter(holes::Column::SourceKind.eq("workflow"))
        .filter(holes::Column::SourceId.eq(execution))
        .filter(holes::Column::SourceRef.eq(node))
        .order_by_desc(holes::Column::Id)
        .one(db)
        .await?)
}

/// Waits while the review is open; otherwise routes approved to `approved`
/// and rejected, timed out or revoked to `rejected`.
async fn settle(s: &AppState, hole: i64, human: bool) -> Result<Step> {
    let row = holes::Entity::find_by_id(hole).one(&s.db).await?;
    if human && row.as_ref().is_some_and(|row| pending(row, db::now())) {
        return Ok(Step::Wait(row.unwrap().expires_at));
    }
    let review = if human {
        wait_review(s, hole).await?
    } else {
        "none".into()
    };
    let (url, host) = match &row {
        Some(row) => (
            crate::holes::link(s, row).await?,
            hosts::Entity::find_by_id(row.host_id).one(&s.db).await?,
        ),
        None => (String::new(), None),
    };
    let mut item = Map::new();
    item.insert("url".into(), json!(url));
    item.insert("hole_id".into(), json!(Id::from(hole)));
    item.insert("review_state".into(), json!(review));
    item.insert(
        "execution_host".into(),
        json!({"id": host.as_ref().map(|h| Id::from(h.id)), "name": host.map(|h| h.name)}),
    );
    let taken = match (human, review.as_str()) {
        (false, _) => "main",
        (true, "approved") => "approved",
        (true, _) => "rejected",
    };
    let names: &[&str] = if human {
        &["approved", "rejected"]
    } else {
        &["main"]
    };
    Ok(Step::Done(Output {
        version: 1,
        ports: names
            .iter()
            .map(|name| Port {
                name: (*name).into(),
                index: 0,
                skipped: *name != taken,
                items: if *name == taken {
                    vec![Item {
                        json: item.clone(),
                        binary: Default::default(),
                        paired: vec![],
                    }]
                } else {
                    vec![]
                },
            })
            .collect(),
    }))
}

/// Waiting preview attempts whose review closed, so `Journal::wake` resumes
/// them before their deadline.
pub(super) async fn due(
    db: &impl ConnectionTrait,
    execution: i64,
    graph: &Value,
) -> Result<Vec<i64>> {
    let waiting = nodes::Entity::find()
        .filter(nodes::Column::ExecutionId.eq(execution))
        .filter(nodes::Column::State.eq("waiting"))
        .all(db)
        .await?;
    let now = db::now();
    let mut due = Vec::new();
    for node in waiting {
        let preview = graph["nodes"].as_array().is_some_and(|specs| {
            specs
                .iter()
                .any(|spec| spec["id"] == node.node_id && spec["type"] == "host.preview")
        });
        if preview
            && latest(db, execution, &node.node_id)
                .await?
                .is_none_or(|hole| !pending(&hole, now))
        {
            due.push(node.id);
        }
    }
    Ok(due)
}

/// Called after a review, revocation or deletion of a workflow's preview: a
/// parked execution is claimed again now instead of at the review deadline.
pub(crate) async fn nudge(db: &impl ConnectionTrait, hole: &holes::Model) -> Result<()> {
    if hole.source_kind == "workflow" {
        runs::Entity::update_many()
            .col_expr(runs::Column::NotBefore, Expr::value(Some(db::now())))
            .filter(runs::Column::Id.eq(hole.source_id))
            .filter(runs::Column::State.eq("waiting"))
            .exec(db)
            .await?;
    }
    Ok(())
}

// ------------------------------------------------------------ services

type Sessions = HashMap<i64, Vec<(u64, watch::Sender<bool>)>>;
/// Execution id -> cancel switches of the services its preview nodes started.
static SERVICES: LazyLock<Mutex<Sessions>> = LazyLock::new(Default::default);
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) fn register(execution: i64, cancel: watch::Sender<bool>) -> u64 {
    let key = NEXT.fetch_add(1, Ordering::Relaxed);
    SERVICES
        .lock()
        .unwrap()
        .entry(execution)
        .or_default()
        .push((key, cancel));
    key
}

fn forget(execution: i64, key: u64) {
    let mut services = SERVICES.lock().unwrap();
    if let Some(list) = services.get_mut(&execution) {
        list.retain(|(candidate, _)| *candidate != key);
        if list.is_empty() {
            services.remove(&execution);
        }
    }
}

/// Cancels the execution's services; the runner kills each process group.
pub(super) fn stop(execution: i64) {
    let stopped = SERVICES.lock().unwrap().remove(&execution);
    for (_, cancel) in stopped.into_iter().flatten() {
        let _ = cancel.send(true);
    }
}

/// The terminal hooks' cleanup once their transaction committed.
pub(super) fn ended(execution: i64, revoked: &[i64]) {
    close_all(revoked);
    stop(execution);
}

async fn serve(
    s: &AppState,
    workspace: i64,
    execution: i64,
    node: &str,
    host: i64,
    command: &str,
    input_env: &Value,
) -> Result<()> {
    // Same contract as host.command: inputs are environment data, never script text.
    let script = super::host::command_script(&json!({
        "shell": "sh", "input_assets": {}, "input_env": input_env, "command": command
    }))?;
    let row = hosts::Entity::find_by_id(host)
        .filter(hosts::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
        .ok_or_else(super::host::invalid)?;
    let (ch, private) = super::host::private_channel(s, workspace, &row).await?;
    let path = format!("{private}/service.sh");
    let bytes = Bytes::from(script);
    if let Err(error) = ch
        .write_file(
            &path,
            0o700,
            Box::pin(futures::stream::once(async move { Ok(bytes) })),
        )
        .await
    {
        let _ = ch
            .exec(
                &format!("rm -rf -- {}", super::host::quote(&private)),
                CLEANUP,
            )
            .await;
        return Err(error);
    }
    let log = nodes::Entity::find()
        .filter(nodes::Column::ExecutionId.eq(execution))
        .filter(nodes::Column::NodeId.eq(node))
        .filter(nodes::Column::State.eq("running"))
        .order_by_desc(nodes::Column::Id)
        .one(&s.db)
        .await?
        .map(|row| row.id);
    spawn(
        s.clone(),
        execution,
        log,
        ch,
        private,
        format!("sh {}", super::host::quote(&path)),
    );
    Ok(())
}

/// Runs the service without host.command's 120 s limit until it exits, the
/// execution's services are stopped, or the execution is seen to have ended.
fn spawn(
    s: AppState,
    execution: i64,
    log: Option<i64>,
    ch: Box<dyn HostChannel>,
    private: String,
    command: String,
) {
    let (cancel, receiver) = watch::channel(false);
    let key = register(execution, cancel.clone());
    tokio::spawn(async move {
        let (sender, mut output) = mpsc::channel(16);
        let task = ch.exec_stream(&command, SERVICE_LIMIT, sender, receiver);
        tokio::pin!(task);
        let mut node_log = super::host::NodeLog::default();
        let logging_until = tokio::time::Instant::now() + LOG_WINDOW;
        let mut flush = tokio::time::interval(Duration::from_millis(250));
        let mut check =
            tokio::time::interval_at(tokio::time::Instant::now() + WATCH_EVERY, WATCH_EVERY);
        let mut open = true;
        loop {
            tokio::select! {
                result = &mut task => {
                    if let Err(error) = result {
                        tracing::warn!(execution, %error, "预览服务会话结束");
                    }
                    break;
                }
                chunk = output.recv(), if open => match chunk {
                    Some((kind, bytes)) => if tokio::time::Instant::now() < logging_until {
                        node_log.push(&kind, &bytes);
                    },
                    None => open = false,
                },
                _ = flush.tick(), if log.is_some() && node_log.dirty => {
                    let _ = node_log.flush(&s, log).await;
                }
                _ = check.tick() => if finished(&s, execution).await {
                    let _ = cancel.send(true);
                }
            }
        }
        let _ = node_log.flush(&s, log).await;
        let _ = ch
            .exec(
                &format!("rm -rf -- {}", super::host::quote(&private)),
                CLEANUP,
            )
            .await;
        forget(execution, key);
    });
}

/// Backstop for terminal writes on another server instance.
async fn finished(s: &AppState, execution: i64) -> bool {
    match runs::Entity::find_by_id(execution).one(&s.db).await {
        Ok(row) => row.is_none_or(|row| TERMINAL.contains(&row.state.as_str())),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
