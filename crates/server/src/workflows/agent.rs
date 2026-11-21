//! AI agent node (`ai.agent.run`, PRD §26.2). One execution runs the selected
//! Agent through its model API or a native CLI on a Runner, read-only, with
//! platform tools. Platform reads run under (execution identity ∩ Agent role),
//! rechecked for every call; any write is answered with a 403 tool result,
//! never an approval prompt. The final answer must be one JSON object that
//! validates against the node's output schema; it becomes the single output item.
use super::host::{self, NodeLog};
use crate::{
    accounts::workspace::Scope,
    entities::{ai_agents, hosts, workflow_executions},
    error::{ApiError, Result},
    hosts::{channel::HostChannel, execution::Lease},
    i18n::Message,
    platform_tools::{self, Call},
    snowflake::Id,
    state::AppState,
};
use axum::http::StatusCode;
use ferry_protocol::{
    agent::{Binding, Event, Input, Outcome, Permission, Request, Runtime},
    sandbox::{Asset, Item, Paired},
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
    time::Instant,
};
mod api;

const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
/// The Runner refuses longer prompts (crates/runner agent request validation).
const RUNNER_PROMPT_BYTES: usize = 64 * 1024;
const MAX_TOOL_CALLS: usize = 128;
const MAX_RECEIVED_BYTES: usize = 18 * 1024 * 1024;
const MAX_CONCURRENT_CALLS: usize = 16;
const PREVIEW_CHARS: usize = 500;
/// Remote cancellation and cleanup normally finish well within this.
const CANCEL_GRACE: Duration = Duration::from_secs(90);
const READ_ONLY: &str = "This workflow AI agent node is read-only and unattended: only GET operations of the platform catalogue are allowed. No operation was executed.";

pub(super) fn supports(spec: &Value) -> bool {
    spec["type"] == "ai.agent.run"
}

fn invalid(key: &'static str, path: String) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new(key).with("path", path),
    )
}

fn failed(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::BAD_GATEWAY, Message::new(key))
}

fn preview(text: &str) -> String {
    let text = text.trim();
    let mut preview: String = text.chars().take(PREVIEW_CHARS).collect();
    if preview.len() < text.len() {
        preview.push('…');
    }
    preview
}

/// Literal checks shared by save time and execution. A prompt expression is
/// checked again once resolved.
pub(super) fn validate(params: &Value, base: &str) -> Result<()> {
    let prompt = params["prompt"].as_str().unwrap_or_default();
    if !prompt.starts_with('=') && (prompt.trim().is_empty() || prompt.len() > MAX_PROMPT_BYTES) {
        return Err(invalid(
            "workflow.agentPrompt",
            format!("{base}/params/prompt"),
        ));
    }
    // With file output the JSON answer is optional; without it, it is the output.
    let files = super::agent_files::mode(params, base)?;
    if files.is_none() || !params["output_schema"].is_null() {
        output_schema(&params["output_schema"], base)?;
    }
    Ok(())
}

/// The user's output contract: a compilable JSON Schema whose root type is object.
fn output_schema(schema: &Value, base: &str) -> Result<jsonschema::Validator> {
    let error = || {
        invalid(
            "workflow.agentSchema",
            format!("{base}/params/output_schema"),
        )
    };
    if schema["type"] != "object"
        || serde_json::to_vec(schema).map_err(|_| error())?.len() > MAX_SCHEMA_BYTES
    {
        return Err(error());
    }
    jsonschema::options()
        .with_retriever(super::run::NoExternalSchema)
        .build(schema)
        .map_err(|_| error())
}

fn read_only_runtime(runtime: Runtime) -> bool {
    // Kimi and Grok cannot run without write access (conversations.rs).
    !matches!(runtime, Runtime::Kimi | Runtime::Grok)
}

fn agent_id(params: &Value, path: &str) -> Result<Id> {
    params["agent_id"]
        .as_str()
        .and_then(|id| Id::reference(id).ok())
        .filter(|id| i64::from(*id) > 0)
        .ok_or_else(|| invalid("workflow.agentUnavailable", path.to_owned()))
}

/// An Agent usable by `scope` with an API model or a read-only capable CLI.
async fn usable(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    id: Id,
    path: &str,
    files: bool,
) -> Result<(ai_agents::Model, ai_agents::Model)> {
    let agent = crate::conversations::available(conn, scope, id)
        .await
        .map_err(|error| match error.0 {
            StatusCode::NOT_FOUND | StatusCode::CONFLICT => {
                invalid("workflow.agentUnavailable", path.to_owned())
            }
            _ => error,
        })?;
    let resolved = crate::agents::resolve_model(conn, &agent).await?;
    if resolved.runtime == "api" {
        if files {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("workflow.agentFilesRuntime")
                    .with("path", path.to_owned())
                    .with("cli", "API"),
            ));
        }
        return Ok((agent, resolved));
    }
    let runtime = Runtime::parse(&resolved.runtime)
        .ok_or_else(|| invalid("workflow.agentUnavailable", path.to_owned()))?;
    if !read_only_runtime(runtime) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.agentReadOnly")
                .with("path", path.to_owned())
                .with("cli", resolved.runtime.clone()),
        ));
    }
    // Only CLIs whose writes the Runner can confine may produce files unattended.
    if files && !super::agent_files::runtime_supported(runtime) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.agentFilesRuntime")
                .with("path", path.to_owned())
                .with("cli", resolved.runtime.clone()),
        ));
    }
    Ok((agent, resolved))
}

/// Save-time checks that need the workspace: every literal Agent reference is
/// visible to the saving user, enabled, and uses a supported API or CLI runtime.
pub(super) async fn validate_agents(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    definition: &str,
) -> Result<()> {
    let graph: Value = serde_json::from_str(definition)
        .map_err(|_| invalid("workflow.graphSchema", "/".into()))?;
    let mut layers = vec![(graph, String::new())];
    while let Some((graph, base)) = layers.pop() {
        for (index, node) in graph["nodes"].as_array().into_iter().flatten().enumerate() {
            let path = format!("{base}/nodes/{index}");
            if node["type"] == "core.loop" {
                layers.push((
                    node["params"]["body"].clone(),
                    format!("{path}/params/body"),
                ));
            }
            if supports(node) {
                let field = format!("{path}/params/agent_id");
                let files = super::agent_files::mode(&node["params"], &path)?.is_some();
                usable(
                    conn,
                    scope,
                    agent_id(&node["params"], &field)?,
                    &field,
                    files,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Instructions, the user's task, the input items and the output contract in one
/// prompt; the Runner's prompt limit applies to the whole text.
fn compose(
    system: &str,
    prompt: &str,
    items: &[Item],
    schema: Option<&Value>,
    files: bool,
) -> Result<String> {
    let rows: Vec<_> = items.iter().map(|item| &item.json).collect();
    let mut text = String::new();
    // Labelled so the model never mistakes the Agent's configuration for the task.
    if !system.trim().is_empty() {
        text.push_str("<agent_instructions source=\"agent configuration set by a workspace administrator\">\n");
        text.push_str(system.trim_end());
        text.push_str("\n</agent_instructions>\n\n");
    }
    text.push_str("<workflow_task source=\"this workflow node's prompt\">\n");
    text.push_str(prompt);
    text.push_str("\n</workflow_task>");
    text.push_str(&format!(
        "\n\n<workflow_input_items count=\"{}\">\n{}\n</workflow_input_items>\n\n",
        rows.len(),
        serde_json::to_string(&rows).map_err(|_| ApiError::missing())?
    ));
    text.push_str("<output_contract>\nThis is an unattended workflow step. Read workspace data only with the ferry_discover and ferry_request tools (GET operations only; anything else is refused with 403). ");
    text.push_str(if files {
        "Write files only as described in <file_output> below; change nothing else. "
    } else {
        "Do not modify files. "
    });
    match schema {
        Some(schema) => {
            text.push_str("Finish with exactly one JSON object that validates against the JSON Schema below, either as the whole final message or in a ```json fenced block. No other text is required.\n");
            text.push_str(&serde_json::to_string(schema).map_err(|_| ApiError::missing())?);
            text.push('\n');
        }
        None => text.push_str("Finish with a short summary of what you produced.\n"),
    }
    text.push_str("</output_contract>");
    within_limit(text)
}

/// The Runner refuses longer prompts.
fn within_limit(text: String) -> Result<String> {
    if text.len() > RUNNER_PROMPT_BYTES {
        return Err(ApiError::localized(
            StatusCode::PAYLOAD_TOO_LARGE,
            Message::new("workflow.agentInputTooLarge")
                .with("bytes", text.len().to_string())
                .with("limit", RUNNER_PROMPT_BYTES.to_string()),
        ));
    }
    Ok(text)
}

/// The final answer as a JSON object: the whole message, or the last fenced
/// ```json (or unlabelled) block that holds one.
pub(super) fn extract(text: &str) -> Option<Map<String, Value>> {
    if let Ok(Value::Object(object)) = serde_json::from_str(text.trim()) {
        return Some(object);
    }
    let mut found = None;
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let Some(line) = after.find('\n') else {
            break;
        };
        let language = after[..line].trim();
        let body = &after[line + 1..];
        let Some(end) = body.find("```") else {
            break;
        };
        if (language.is_empty() || language.eq_ignore_ascii_case("json"))
            && let Ok(Value::Object(object)) = serde_json::from_str(body[..end].trim())
        {
            found = Some(object);
        }
        rest = &body[end + 3..];
    }
    found
}

/// Accept the answer or explain, in a localized error, why it was rejected.
fn accept(text: &str, validator: &jsonschema::Validator) -> Result<Map<String, Value>> {
    let Some(object) = extract(text) else {
        return Err(ApiError::localized(
            StatusCode::UNPROCESSABLE_ENTITY,
            Message::new("workflow.agentNoJson").with("text", preview(text)),
        ));
    };
    if serde_json::to_vec(&object)
        .map_err(|_| ApiError::missing())?
        .len()
        > MAX_OUTPUT_BYTES
    {
        return Err(ApiError::localized(
            StatusCode::PAYLOAD_TOO_LARGE,
            Message::new("workflow.agentOutputTooLarge"),
        ));
    }
    let value = Value::Object(object);
    if let Some(error) = validator.iter_errors(&value).next() {
        let path = error.instance_path().to_string();
        return Err(ApiError::localized(
            StatusCode::UNPROCESSABLE_ENTITY,
            Message::new("workflow.agentInvalidOutput")
                .with("path", if path.is_empty() { "/".into() } else { path })
                .with(
                    "reason",
                    error.to_string().chars().take(300).collect::<String>(),
                )
                .with("text", preview(text)),
        ));
    }
    let Value::Object(object) = value else {
        unreachable!()
    };
    Ok(object)
}

/// One platform tool call, authorized afresh: the execution identity must still
/// be allowed to run this workflow, the Agent must still be usable by it, and
/// the Agent role bounds the delegated permissions. Only catalogue GETs pass.
async fn platform_call(
    s: AppState,
    run: workflow_executions::Model,
    agent: Id,
    request_id: String,
    name: String,
    arguments: Value,
) -> (String, Value, String) {
    let mut summary = name.clone();
    let result: Result<Value> = async {
        if request_id.is_empty()
            || request_id.len() > 128
            || !["ferry_discover", "ferry_request"].contains(&name.as_str())
        {
            return Err(ApiError::forbidden());
        }
        let call = if name == "ferry_request" {
            let call: Call =
                serde_json::from_value(arguments.clone()).map_err(|_| ApiError::forbidden())?;
            summary = format!(
                "{} {}",
                call.method.chars().take(16).collect::<String>(),
                call.path.chars().take(512).collect::<String>()
            );
            call.validate()?;
            if !call.read_only()
                || call.transfer.is_some()
                || !call.body.is_null()
                || !platform_tools::read_path(&call.path)
            {
                return Ok(json!({"status":403,"ok":false,"error":READ_ONLY}));
            }
            Some(call)
        } else {
            None
        };
        let scope = super::integration::execution_scope(&s, &run).await?;
        let agent = crate::conversations::available(&s.db, &scope, agent).await?;
        let scope = platform_tools::authorized_scope(&s, scope, &agent).await?;
        match call {
            None => Ok(platform_tools::read_catalogue(
                &scope,
                arguments["search"].as_str().unwrap_or(""),
                arguments["offset"].as_u64().unwrap_or(0).min(10000) as usize,
            )),
            Some(call) => platform_tools::dispatch_as_agent(&s, scope, call, agent.id.into()).await,
        }
    }
    .await;
    let value = result.unwrap_or_else(|error| platform_tools::denial(&error));
    let status = value["status"].as_u64().unwrap_or(200);
    let bytes = serde_json::to_vec(&value).map_or(0, |bytes| bytes.len());
    (
        request_id,
        value,
        format!("{summary} -> {status} ({bytes} bytes)"),
    )
}

/// Everything one execution shares across its (at most two) native turns.
struct Session<'a> {
    s: &'a AppState,
    run: &'a workflow_executions::Model,
    agent: Id,
    channel: &'a dyn HostChannel,
    lease: Arc<Lease>,
    log: NodeLog,
    node_run: Option<i64>,
    calls: usize,
    max_calls: usize,
    received: usize,
    deadline: Instant,
    timeout: Duration,
}

struct Answer {
    text: String,
    session: Option<String>,
}

impl Session<'_> {
    /// Drive one native turn to completion; any limit, revocation or
    /// cancellation stops the CLI and fails the node.
    async fn turn(&mut self, request: Request) -> Result<Answer> {
        let (cancel, cancelled) = watch::channel(false);
        let (events_tx, mut events) = mpsc::channel(32);
        let (inputs, commands) = mpsc::channel(16);
        let channel = self.channel;
        let running = channel.agent(request, events_tx, commands, cancelled);
        tokio::pin!(running);
        let (_idle, idle) = watch::channel(false);
        let mut stop = crate::executions::stop_signal().unwrap_or(idle);
        let mut tasks: JoinSet<(String, Value, String)> = JoinSet::new();
        let mut texts: Vec<(String, String)> = Vec::new();
        let mut session = None;
        let mut outcome = None;
        let mut failure: Option<ApiError> = None;
        let mut check = tokio::time::interval(Duration::from_millis(500));
        let mut renewal = tokio::time::interval(Duration::from_secs(10));
        let mut flush = tokio::time::interval(Duration::from_millis(250));
        for timer in [&mut check, &mut renewal, &mut flush] {
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        }
        let deadline = tokio::time::sleep_until(self.deadline);
        tokio::pin!(deadline);
        // Armed once cancellation is requested: a CLI that never acknowledges is abandoned.
        let grace = tokio::time::sleep(Duration::from_secs(86_400));
        tokio::pin!(grace);
        macro_rules! fail {
            ($error:expr) => {{
                if failure.is_none() {
                    failure = Some($error);
                    cancel.send_replace(true);
                    grace.as_mut().reset(Instant::now() + CANCEL_GRACE);
                }
            }};
        }
        loop {
            tokio::select! {
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => match joined {
                    Ok((request_id, result, line)) => {
                        self.log.push("tool", line.as_bytes());
                        if failure.is_none() && inputs.try_send(Input::PlatformResult { request_id, result }).is_err() {
                            fail!(failed("workflow.agentFailed"));
                        }
                    }
                    Err(_) => fail!(failed("workflow.agentFailed")),
                },
                result = &mut running => {
                    while let Ok(event) = events.try_recv() {
                        self.observe(event, &mut texts, &mut session, &mut outcome);
                    }
                    if let Err(error) = result {
                        failure.get_or_insert(error);
                    }
                    break;
                }
                event = events.recv() => if let Some(event) = event {
                    if let Event::PlatformCall { request_id, name, arguments } = event {
                        self.calls += 1;
                        self.received += serde_json::to_vec(&arguments).map_or(usize::MAX, |bytes| bytes.len());
                        if self.calls > self.max_calls || tasks.len() >= MAX_CONCURRENT_CALLS {
                            fail!(ApiError::localized(StatusCode::TOO_MANY_REQUESTS,
                                Message::new("workflow.agentToolLimit").with("max", self.max_calls.to_string())));
                        } else if self.received > MAX_RECEIVED_BYTES {
                            fail!(ApiError::localized(StatusCode::PAYLOAD_TOO_LARGE, Message::new("workflow.agentOutputLimit")));
                        } else if failure.is_none() {
                            tasks.spawn(platform_call(self.s.clone(), self.run.clone(), self.agent, request_id, name, arguments));
                        }
                        continue;
                    }
                    self.received += serde_json::to_vec(&event).map_or(usize::MAX, |bytes| bytes.len());
                    if self.received > MAX_RECEIVED_BYTES {
                        fail!(ApiError::localized(StatusCode::PAYLOAD_TOO_LARGE, Message::new("workflow.agentOutputLimit")));
                    }
                    if matches!(event, Event::Approval { .. }) {
                        // Read-only CLIs never ask; an unattended node cannot answer.
                        fail!(failed("workflow.agentApproval"));
                    }
                    self.observe(event, &mut texts, &mut session, &mut outcome);
                },
                _ = &mut deadline, if failure.is_none() => fail!(ApiError::localized(
                    StatusCode::GATEWAY_TIMEOUT,
                    Message::new("workflow.agentTimeout").with("seconds", self.timeout.as_secs().to_string()),
                )),
                _ = stop.changed(), if failure.is_none() => fail!(crate::executions::ownership_lost()),
                _ = check.tick(), if failure.is_none() => if let Err(error) = host::active(self.s, self.run).await {
                    fail!(error);
                },
                _ = renewal.tick() => if !self.lease.renew(&self.s.db).await.unwrap_or(false) {
                    fail!(crate::executions::ownership_lost());
                },
                _ = flush.tick(), if self.log.dirty => {
                    let _ = self.log.flush(self.s, self.node_run).await;
                }
                _ = &mut grace, if failure.is_some() => break,
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        if let Some(error) = failure {
            return Err(error);
        }
        match outcome {
            Some(Outcome::Completed) => {}
            Some(Outcome::TimedOut) => {
                return Err(ApiError::localized(
                    StatusCode::GATEWAY_TIMEOUT,
                    Message::new("workflow.agentTimeout")
                        .with("seconds", self.timeout.as_secs().to_string()),
                ));
            }
            _ => return Err(failed("workflow.agentFailed")),
        }
        let text = texts
            .into_iter()
            .rev()
            .map(|(_, text)| text)
            .find(|text| !text.trim().is_empty())
            .unwrap_or_default();
        Ok(Answer { text, session })
    }

    fn observe(
        &mut self,
        event: Event,
        texts: &mut Vec<(String, String)>,
        session: &mut Option<String>,
        outcome: &mut Option<Outcome>,
    ) {
        match event {
            // Message text folds per item: deltas append, a replacement is authoritative.
            Event::Text {
                item_id,
                text,
                replace,
            } => match texts.iter_mut().find(|(id, _)| *id == item_id) {
                Some((_, current)) if replace => *current = text,
                Some((_, current)) => current.push_str(&text),
                None => texts.push((item_id, text)),
            },
            Event::Session { native_session_id } => *session = Some(native_session_id),
            Event::Finished { outcome: value, .. } => *outcome = Some(value),
            Event::Tool { name, state, .. } => {
                self.log.push(
                    "cli",
                    format!("{name} {state}")
                        .chars()
                        .take(300)
                        .collect::<String>()
                        .as_bytes(),
                );
            }
            _ => {}
        }
    }
}

/// Execution limits; tests shorten the timeout below the contract's minimum.
pub(super) struct Limits {
    pub timeout: Duration,
}

pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    node: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    let timeout = params
        .first()
        .and_then(|params| super::graph::port_index(&params["timeout_s"]))
        .filter(|seconds| (30..=1800).contains(seconds))
        .ok_or_else(|| invalid("workflow.graphSchema", "/params/timeout_s".into()))?;
    execute_with(
        s,
        run,
        node_run,
        node,
        items,
        params,
        Limits {
            timeout: Duration::from_secs(timeout),
        },
    )
    .await
}

pub(super) async fn execute_with(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    node: &Value,
    items: &[Item],
    params: &[Value],
    limits: Limits,
) -> Result<Vec<Item>> {
    // One run per node execution, over all input items.
    let [params] = params else {
        return Err(invalid("workflow.graphSchema", "/params".into()));
    };
    if items.is_empty() {
        return Ok(vec![]);
    }
    let files = super::agent_files::mode(params, "")?;
    // Without file output the JSON answer is the node's output and is required.
    let validator = if files.is_none() || !params["output_schema"].is_null() {
        Some(output_schema(&params["output_schema"], "")?)
    } else {
        None
    };
    let max_calls = super::graph::port_index(&params["max_tool_calls"])
        .and_then(|calls| usize::try_from(calls).ok())
        .filter(|calls| (1..=MAX_TOOL_CALLS).contains(calls))
        .ok_or_else(|| invalid("workflow.graphSchema", "/params/max_tool_calls".into()))?;
    let prompt = params["prompt"].as_str().unwrap_or_default();
    if prompt.trim().is_empty() || prompt.len() > MAX_PROMPT_BYTES {
        return Err(invalid("workflow.agentPrompt", "/params/prompt".into()));
    }
    let scope = super::integration::execution_scope(s, run).await?;
    let (agent, resolved) = usable(
        &s.db,
        &scope,
        agent_id(params, "/params/agent_id")?,
        "/params/agent_id",
        files.is_some(),
    )
    .await?;
    if resolved.runtime == "api" {
        let object = api::execute(
            s,
            run,
            node_run,
            &resolved,
            params,
            items,
            validator.as_ref().ok_or_else(ApiError::missing)?,
            max_calls,
            limits.timeout,
        )
        .await?;
        return Ok(vec![Item {
            json: object,
            binary: BTreeMap::new(),
            paired: items
                .iter()
                .flat_map(|item| item.paired.iter().cloned())
                .collect(),
        }]);
    }
    let config = crate::conversations::Configuration::from_agent(&resolved)?;
    let prompt = compose(
        &config.system_prompt,
        prompt,
        items,
        validator.as_ref().map(|_| &params["output_schema"]),
        files.is_some(),
    )?;
    let node_name = node["name"].as_str().unwrap_or_default().to_owned();
    let deadline = Instant::now() + limits.timeout;
    let node_run = (node_run > 0).then_some(node_run);
    let paired: Vec<Paired> = items
        .iter()
        .flat_map(|item| item.paired.iter().cloned())
        .collect();
    let attempt = |target: (i64, bool)| {
        let prompt = prompt.clone();
        let (resolved, config, validator, files, node_name) = (
            &resolved,
            &config,
            validator.as_ref(),
            files.as_ref(),
            &node_name,
        );
        host::on_host(s, run, node, target, move |row, lease| async move {
            on_runner(
                s,
                run,
                node_run,
                resolved,
                config,
                row,
                lease,
                prompt,
                validator,
                files.map(|mode| (mode, node_name.as_str())),
                max_calls,
                deadline,
                limits.timeout,
            )
            .await
        })
    };
    let item = |(object, binary): (Map<String, Value>, BTreeMap<String, Asset>)| {
        vec![Item {
            json: object,
            binary,
            paired: paired.clone(),
        }]
    };
    // An explicit node (or workflow default) target is used as is.
    if let Some(target) = host::target(node) {
        return match attempt(target).await? {
            OnHost::Done(output) => Ok(item(output)),
            OnHost::Unusable(error) => Err(error),
        };
    }
    // Otherwise the same hosts, in the same order, a conversation would use.
    let cli = resolved.runtime.clone();
    let mut reasons = crate::agent_hosts::Reasons::default();
    let mut last = None;
    for row in
        crate::agent_hosts::candidates(&s.db, Id::from(run.workspace_id), None, &agent, &[]).await?
    {
        if row.state != "online" {
            reasons.push(&row, Message::new("models.reasonOffline"));
            continue;
        }
        match attempt((row.id, false)).await? {
            OnHost::Done(output) => return Ok(item(output)),
            OnHost::Unusable(error) => {
                reasons.push(&row, unusable_reason(&error, &cli));
                last = Some(error);
            }
        }
    }
    // One host tried: its own error already names the host and the CLI.
    match last {
        Some(error) if reasons.len() == 1 => Err(error),
        _ => Err(reasons.into_error()),
    }
}

/// What one candidate host did with the node.
enum OnHost {
    /// The output item's JSON and its files (empty without file output).
    Done((Map<String, Value>, BTreeMap<String, Asset>)),
    /// The host cannot run this CLI (offline, missing, not signed in...); the
    /// next candidate is tried. Failures after the CLI started are errors.
    Unusable(ApiError),
}

/// The host-list reason for a host that could not run the CLI.
fn unusable_reason(error: &ApiError, cli: &str) -> Message {
    use crate::agent_hosts::reason;
    match error.2.as_ref().map(|message| message.key.as_str()) {
        Some("workflow.agentNotRunner") => Message::new("models.reasonNotRunner"),
        Some("workflow.agentDirectoryMissing") => Message::new("models.reasonDirectory"),
        Some("workflow.agentCliMissing") => reason("models.reasonCliMissing", cli),
        Some("workflow.agentRunnerUpgrade") => Message::new("models.reasonRunnerOld"),
        Some("workflow.agentLoginRequired") => reason("models.reasonLogin", cli),
        Some("workflow.agentFilesRunnerUpgrade") => Message::new("models.reasonRunnerOld"),
        _ => crate::agent_hosts::reason_from(error),
    }
}

fn runner_error(key: &'static str, host: &hosts::Model, cli: &str) -> ApiError {
    ApiError::localized(
        StatusCode::CONFLICT,
        Message::new(key)
            .with("host", host.name.clone())
            .with("cli", cli.to_owned()),
    )
}

#[allow(clippy::too_many_arguments)]
async fn on_runner(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: Option<i64>,
    agent: &ai_agents::Model,
    config: &crate::conversations::Configuration,
    row: hosts::Model,
    lease: Arc<Lease>,
    prompt: String,
    validator: Option<&jsonschema::Validator>,
    files: Option<(&super::agent_files::FileMode, &str)>,
    max_calls: usize,
    deadline: Instant,
    timeout: Duration,
) -> Result<OnHost> {
    let runtime = config.runtime.cli()?;
    // The resolved CLI (a model profile may select another one than the Agent row).
    let cli = agent.runtime.clone();
    let unusable = |key| Ok(OnHost::Unusable(runner_error(key, &row, &cli)));
    if !crate::models::runner_connections().contains(&row.connection.as_str()) {
        return unusable("workflow.agentNotRunner");
    }
    let channel =
        match crate::hosts::remote::open(s, &Id::from(run.workspace_id), &Id::from(row.id)).await {
            Ok(channel) => channel,
            Err(error) => return Ok(OnHost::Unusable(error)),
        };
    // A managed workspace per workflow and Agent; keys are numeric IDs. File
    // output always gets a fresh one per node execution, never the Agent's own
    // directory, so nothing outside it can be written or collected.
    let probe = if files.is_some() {
        channel
            .agent_workspace(
                runtime,
                &format!("{}-{}", run.id, node_run.unwrap_or(run.id)),
            )
            .await
    } else if config.cwd.is_empty() {
        channel
            .agent_workspace(runtime, &format!("{}-{}", run.workflow_id, agent.id))
            .await
    } else {
        channel.agent_probe(runtime, &config.cwd, None).await
    };
    let probe = match probe {
        Ok(probe) => probe,
        Err(error) => return Ok(OnHost::Unusable(error)),
    };
    let Some(directory) = probe.canonical_directory.filter(|_| probe.directory_exists) else {
        return unusable("workflow.agentDirectoryMissing");
    };
    let Some(executable) = probe.executable.filter(|_| probe.error.is_none()) else {
        return unusable("workflow.agentCliMissing");
    };
    if !probe.platform_tools
        || probe.execution_user.is_empty()
        || probe.execution_identity.is_empty()
    {
        return unusable("workflow.agentRunnerUpgrade");
    }
    if files.is_some() && !probe.write_workdir {
        return unusable("workflow.agentFilesRunnerUpgrade");
    }
    let prompt = match files {
        Some((mode, _)) => within_limit(format!(
            "{prompt}\n\n{}",
            super::agent_files::contract(mode, runtime, &probe.file_tools)
        ))?,
        None => prompt,
    };
    let provider =
        crate::models::cli_provider(s, Id::from(run.workspace_id), agent.id, runtime).await?;
    if provider.is_none() && !config.authentication_usable(&probe.authentication) {
        return unusable("workflow.agentLoginRequired");
    }
    let timezone =
        match crate::models::cli_timezone(s, Id::from(run.workspace_id), agent.id, &row).await {
            Ok(timezone) => timezone,
            Err(error) => return Ok(OnHost::Unusable(error)),
        };
    let binding = Binding {
        execution_user: probe.execution_user,
        execution_identity: probe.execution_identity,
        canonical_directory: directory.clone(),
        executable,
    };
    let mut session = Session {
        s,
        run,
        agent: Id::from(agent.id),
        channel: channel.as_ref(),
        lease,
        log: NodeLog::default(),
        node_run,
        calls: 0,
        max_calls,
        received: 0,
        deadline,
        timeout,
    };
    session.log.push(
        "agent",
        format!(
            "{} · {} · {} · {}",
            agent.name,
            cli,
            if config.model.is_empty() {
                "-"
            } else {
                &config.model
            },
            row.name
        )
        .as_bytes(),
    );
    if let Some(zone) = &timezone {
        let caveat = if crate::models::windows_timezone_caveat(zone, &row) {
            " (Windows host in another zone: some Windows tools ignore TZ)"
        } else {
            ""
        };
        session
            .log
            .push("agent", format!("time zone {zone}{caveat}").as_bytes());
    }
    let request = |prompt: String, native_session_id: Option<String>| Request {
        binding: Some(binding.clone()),
        runtime: runtime,
        cwd: directory.clone(),
        prompt,
        model: (!config.model.is_empty()).then(|| config.model.clone()),
        native_session_id,
        permission: Permission::ReadOnly,
        timeout_seconds: deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .clamp(1, 3600),
        platform_tools: true,
        provider: provider.clone(),
        timezone: timezone.clone(),
        write_workdir: files.is_some(),
    };
    if files.is_some() {
        let output = super::agent_files::join(&directory, super::agent_files::OUTPUT_DIR);
        let _ = session.channel.create_dir(&output).await;
    }
    let result = async {
        let answer = session.turn(request(prompt, None)).await?;
        let Some(validator) = validator else {
            // File output without a JSON contract: the final message is the summary.
            let mut object = Map::new();
            object.insert(
                "summary".into(),
                json!(answer.text.trim().chars().take(4000).collect::<String>()),
            );
            return Ok(object);
        };
        match accept(&answer.text, validator) {
            Ok(object) => Ok(object),
            // One correction in the same native session, within the same limits.
            Err(rejection) => {
                let Some(native) = answer.session.filter(|_| Instant::now() < deadline) else {
                    return Err(rejection);
                };
                session.log.push("agent", format!("output rejected: {}", rejection.1).as_bytes());
                let correction = format!(
                    "Your final answer was rejected: {}\nReply again with only one JSON object that validates against the output schema given earlier, in a ```json fenced block. Use read-only tools only if data is missing.",
                    rejection.1.chars().take(2000).collect::<String>()
                );
                let answer = session.turn(request(correction, Some(native))).await?;
                accept(&answer.text, validator)
            }
        }
    }
    .await;
    // Files left under ./output become workflow files; the working directory
    // is removed afterwards whether or not the node succeeded.
    let result = match (result, files) {
        (Ok(mut object), Some((mode, node_name))) => {
            let collected = async {
                let found = super::agent_files::collect(session.channel, &directory, mode).await?;
                let mut listed = Vec::new();
                let mut binary = BTreeMap::new();
                for (index, file) in found.iter().enumerate() {
                    let (reference, info) =
                        super::agent_files::store(s, run, node_name, file).await?;
                    session.log.push(
                        "agent",
                        format!(
                            "file: {} ({} bytes, {})",
                            file.name,
                            file.bytes.len(),
                            file.mime
                        )
                        .as_bytes(),
                    );
                    binary.insert(format!("file_{}", index + 1), reference);
                    listed.push(info);
                }
                Ok::<_, ApiError>((listed, binary))
            }
            .await;
            super::agent_files::cleanup(session.channel, &directory).await;
            collected.map(|(listed, binary)| {
                object.insert("files".into(), Value::Array(listed));
                (object, binary)
            })
        }
        (result, files) => {
            if files.is_some() {
                super::agent_files::cleanup(session.channel, &directory).await;
            }
            result.map(|object| (object, BTreeMap::new()))
        }
    };
    match &result {
        Ok((object, _)) => session.log.push(
            "agent",
            format!(
                "output: {}",
                preview(&Value::Object(object.clone()).to_string())
            )
            .as_bytes(),
        ),
        Err(error) => session
            .log
            .push("agent", format!("failed: {}", error.1).as_bytes()),
    }
    let _ = session.log.flush(s, node_run).await;
    result.map(OnHost::Done)
}

#[cfg(test)]
mod tests;
