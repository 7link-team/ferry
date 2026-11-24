use super::*;
use ferry_protocol::agent::{Event, Input, Outcome, Permission, Request};
use std::time::Duration;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
const LEASE: i64 = 30_000;
/// Longest admission, including choosing a host among several Runners.
const ADMISSION: Duration = Duration::from_secs(390);

/// Runs admission while keeping the turn's lease, which lasts only `LEASE`.
/// Renewal runs beside the work, never inside it, so a transaction the work
/// holds cannot block it. A requested stop is acted on right after admission.
async fn renewing<T>(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let renew = {
        let (s, row, owner) = (s.clone(), row.clone(), owner.to_owned());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis((LEASE / 3) as u64));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                if heartbeat(&s, &row, &owner).await.is_err() {
                    break;
                }
            }
        })
    };
    let result = work.await;
    renew.abort();
    let _ = renew.await;
    result
}
/// Standing guidance for every turn: a workflow the user asks for must fetch and
/// process its own data, so it keeps working without the conversation.

#[cfg(test)]
mod platform_tests;

/// Both conversation engines receive the workspace business clock. This only
/// supplies prompt context and never changes the server or host operating system.
pub(super) async fn workspace_clock_context(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    at: i64,
) -> Result<String> {
    clock_context(
        crate::accounts::workspace::business_timezone(conn, workspace).await?,
        at,
    )
}

fn clock_context(zone: Option<chrono_tz::Tz>, at: i64) -> Result<String> {
    let Some(zone) = zone else {
        return Ok(String::new());
    };
    let (now, today) =
        crate::workflows::expression::clock_with_timezone(&json!({}), Some(zone.name()), at)?;
    Ok(format!(
        "<workspace_runtime_context source=\"workspace business clock\">\n{}\n</workspace_runtime_context>\n\n",
        json!({"$now":now,"$today":today,"frozen_at_ms":at,"timezone":zone.name()})
    ))
}

/// One turn's text: platform rules, then the Agent's configured instructions and
/// the user's message, each delimited and labelled so the model never mistakes
/// the configuration (set by an administrator) for the user's own words.
pub(super) fn turn_prompt(
    platform: &str,
    system: &str,
    message: &str,
    attachments: &[Attachment],
) -> String {
    let mut prompt = if platform.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<platform_instructions source=\"platform settings\">\n{}\n</platform_instructions>\n\n",
            platform.trim()
        )
    };
    if !system.trim().is_empty() {
        prompt.push_str("<agent_instructions source=\"agent configuration set by a workspace administrator, not the user's words\">\n");
        prompt.push_str(system.trim_end());
        prompt.push_str("\n</agent_instructions>\n\n");
    }
    let mut user = message.to_owned();
    for a in attachments {
        if !a.text.is_empty() {
            user.push_str(&format!(
                "\n\n<attached_text name={:?}>\n{}\n</attached_text>",
                a.name, a.text
            ));
        } else {
            user.push_str(&format!("\n\n<attachment name={:?} mime={:?} size={}>File stored in chat; its contents are NOT available to this text-only runtime. Do not claim to have read it.</attachment>",a.name,a.mime,a.size));
        }
    }
    if user.trim().is_empty() {
        user.push_str("A message with attachments was sent.");
    }
    prompt.push_str("<user_message>\n");
    prompt.push_str(user.trim_start_matches('\n'));
    prompt.push_str("\n</user_message>");
    prompt
}

async fn platform_call_with_slots(
    s: AppState,
    row: turns::Model,
    owner: String,
    request_id: String,
    name: String,
    arguments: Value,
    mut approved: Option<bool>,
    slots: Option<std::sync::Arc<tokio::sync::Semaphore>>,
) -> (String, Value) {
    let _slot = match slots.as_ref() {
        Some(slots) => match slots.acquire().await {
            Ok(slot) => Some(slot),
            Err(_) => {
                return (
                    request_id,
                    json!({"ok":false,"error":"tool execution closed"}),
                );
            }
        },
        None => None,
    };
    // What was asked, kept with the result for a readable history: never the
    // body, and the path without its query string.
    let summary = json!({
        "tool": name,
        "method": arguments["method"].as_str().unwrap_or(if name == "ferry_discover" { "GET" } else { "" }),
        "path": arguments["path"].as_str().map(|path| path.split('?').next().unwrap_or("").to_owned()),
    });
    let result: Result<Option<Value>> = async {
        if request_id.is_empty() || request_id.len() > 128 || !["ferry_discover", "ferry_request", "ferry_ask_user"].contains(&name.as_str()) { return Err(invalid()); }
        let (tx, current, c) = leased(&s, &row, &owner).await?;
        if current.state != "running" { return Err(conflict()); }
        if super::questions::pending(&current) { return Err(error(StatusCode::CONFLICT,"conversations.questionPending")); }
        let execution = groups::execution(&tx, &current, c).await?;
        tx.rollback().await?;
        let scope = actor(&s, &current, &execution).await?;
        let agent = available(&s.db, &scope, execution.agent_id.into()).await?;
        if name == "ferry_ask_user" {
            return Ok(Some(super::questions::record(&s,&row,&owner,&request_id,arguments).await?));
        }
        let scope = crate::platform_tools::authorized_scope(&s, scope, &agent).await?;
        if name == "ferry_discover" {
            return Ok(Some(crate::platform_tools::catalogue(&scope, arguments["search"].as_str().unwrap_or(""), arguments["offset"].as_u64().unwrap_or(0).min(10000) as usize)));
        }
        let call: crate::platform_tools::Call = serde_json::from_value(arguments.clone()).map_err(|_| invalid())?;
        call.validate()?;
        if !call.read_only() && approved.is_none() {
            let aad = format!("platform:{}:{}:{}",row.workspace_id,row.id,request_id);
            let sealed = s.vault.seal(&arguments.to_string(), &aad)?;
            let mut display = arguments;
            crate::platform_tools::redact(&mut display);
            display["_sealed"] = json!(sealed);
            append(&s, &row, &owner, Event::Approval { request_id:request_id.clone(), tool:"ferry_request".into(), input:display }).await?;
            let mode=super::approval::mode(&execution.config)?;
            if super::approval::automatic(&mode,&call) {
                super::approval::decide_automatically(&s,&row,&owner,&request_id,&mode).await?;
                approved=Some(true);
            } else { return Ok(None); }
        }
        if approved == Some(false) {
            return Ok(Some(json!({"status":403,"ok":false,"error":"The user declined this platform operation. No operation was executed."})));
        }
        let value = crate::platform_tools::dispatch_as_agent(&s, scope, call, agent.id.into()).await?;
        Ok(Some(value))
    }.await;
    let value = match result {
        Ok(Some(value)) => value,
        Ok(None) => return (request_id, Value::Null),
        Err(error) => crate::platform_tools::denial(&error),
    };
    if let Some(allowed) = approved {
        let _ = append(
            &s,
            &row,
            &owner,
            Event::DecisionSent {
                request_id: request_id.clone(),
                allowed,
            },
        )
        .await;
    }
    let _ = append(
        &s,
        &row,
        &owner,
        Event::PlatformResult {
            request_id: request_id.clone(),
            result: value.clone(),
            call: Some(summary),
        },
    )
    .await;
    (request_id, value)
}

pub(super) async fn platform_call(
    s: AppState,
    row: turns::Model,
    owner: String,
    request_id: String,
    name: String,
    arguments: Value,
    approved: Option<bool>,
) -> (String, Value) {
    platform_call_with_slots(s, row, owner, request_id, name, arguments, approved, None).await
}

pub(super) async fn approved_platform_call(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    id: &str,
) -> Result<Option<Value>> {
    let (_tx, current, _) = leased(s, row, owner).await?;
    let decisions: Value = serde_json::from_str(&current.decisions).map_err(|_| invalid())?;
    let item = &decisions[id];
    if item["tool"] != "ferry_request" || !item["input"]["_sealed"].is_string() {
        return Ok(None);
    }
    if item["dispatching"] != true || item["sent"] == true {
        return Err(conflict());
    }
    let aad = format!("platform:{}:{}:{}", row.workspace_id, row.id, id);
    let sealed = item["input"]["_sealed"].as_str().ok_or_else(invalid)?;
    let plain = s.vault.open_secret(sealed, &aad)?;
    Ok(Some(serde_json::from_str(&plain).map_err(|_| invalid())?))
}

pub async fn scheduler(s: AppState, mut stop: watch::Receiver<bool>) {
    let mut tasks = JoinSet::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            Some(result)=tasks.join_next(),if !tasks.is_empty()=>if let Err(e)=result{tracing::error!(error=%e,"Conversation worker stopped");},
            _=tick.tick()=>{
                if *stop.borrow(){break;}
                let policy = match crate::accounts::resource_policy::load(&s.db).await { Ok(policy) => policy, Err(error) => {tracing::warn!(%error,"Cannot read conversation resource policy");continue;} };
                if policy.conversation_concurrent_runs.is_some_and(|limit| tasks.len() as u64 >= limit) {continue;}
                let mut query=turns::Entity::find().filter(sea_orm::Condition::any()
                    .add(turns::Column::State.eq("queued"))
                    .add(sea_orm::Condition::all().add(turns::Column::State.is_in(["running","stopping"])).add(turns::Column::ExpiresAt.lt(db::now()))))
                    .order_by_asc(turns::Column::Id);
                if let Some(limit)=policy.conversation_concurrent_runs { query=query.limit(limit.saturating_sub(tasks.len() as u64)); }
                let rows=query.all(&s.db).await;
                match rows {
                    Ok(rows)=>for row in rows {
                        let s=s.clone();let stop=stop.clone();
                        tasks.spawn(async move {if let Err(e)=execute(s,row.id.into(),stop).await {tracing::warn!(error=%e.1,"Conversation execution interrupted");}});
                    },
                    Err(e)=>tracing::warn!(error=%e,"Cannot poll conversation turns"),
                }
            }
        }
    }
    // Drain includes the remote cancellation/cleanup receipt, independent of HTTP.
    while tasks.join_next().await.is_some() {}
}
pub(super) async fn claim(
    s: &AppState,
    id: Id,
) -> Result<Option<(turns::Model, conversations::Model, String)>> {
    let Some(initial) = turns::Entity::find_by_id(id).one(&s.db).await? else {
        return Ok(None);
    };
    let tx = crate::variables::transaction(s, initial.workspace_id.into()).await?;
    let mut row = turns::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut c = conversations::Entity::find_by_id(row.conversation_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.state != "queued" {
        if active(&row.state) && row.expires_at < db::now() {
            row.state = "interrupted".into();
            mark_ended_undecided(&mut row);
            row.error = crate::i18n::store_message(&Message::new("conversations.interrupted"));
            row.version += 1;
            row.updated_at = db::now();
            let api = groups::execution(&tx, &row, c.clone())
                .await
                .ok()
                .and_then(|execution| serde_json::from_str::<Configuration>(&execution.config).ok())
                .is_some_and(|config| config.runtime == Runtime::Api);
            if api {
                crate::workflows::ai::conversation_interrupted_in(&tx, &row).await?;
            } else if !groups::finish(&tx, &row, None, true).await? {
                c.state = "interrupted".into();
                c.native_session_id.clear();
            }
            c.version += 1;
            row.into_active_model().reset_all().update(&tx).await?;
            c.into_active_model().reset_all().update(&tx).await?;
            tx.commit().await?;
        }
        return Ok(None);
    }
    if row.purpose == "introduction"
        && !crate::accounts::conversation_policy::Configuration::load(&tx)
            .await?
            .introduction_enabled
    {
        row.state = "cancelled".into();
        row.error = crate::i18n::store_message(&Message::new("conversations.introductionDisabled"));
        row.version += 1;
        row.updated_at = db::now();
        row.into_active_model().reset_all().update(&tx).await?;
        tx.commit().await?;
        return Ok(None);
    }
    if let Err(e) = super::introductions::validate(&tx, &row, &c).await {
        row.state = "failed".into();
        row.error = e.stored();
        row.version += 1;
        row.updated_at = db::now();
        row.into_active_model().reset_all().update(&tx).await?;
        tx.commit().await?;
        return Ok(None);
    }
    // Serialize room replies, including queued replies from earlier messages.
    if turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(c.id))
        .filter(turns::Column::Id.ne(row.id))
        .filter(
            sea_orm::Condition::any()
                .add(turns::Column::State.is_in(["running", "stopping"]))
                .add(
                    sea_orm::Condition::all()
                        .add(turns::Column::State.eq("queued"))
                        .add(turns::Column::Id.lt(row.id)),
                ),
        )
        .count(&tx)
        .await?
        > 0
    {
        return Ok(None);
    }
    let execution = match groups::execution(&tx, &row, c.clone()).await {
        Ok(value) => value,
        Err(e) => {
            row.state = "failed".into();
            row.error = e.stored();
            row.version += 1;
            row.into_active_model().reset_all().update(&tx).await?;
            tx.commit().await?;
            return Ok(None);
        }
    };
    row.native_session_id = execution.native_session_id.clone();
    let owner = uuid::Uuid::new_v4().to_string();
    row.state = "running".into();
    row.lease_owner = owner.clone();
    row.expires_at = db::now() + LEASE;
    row.version += 1;
    row.updated_at = db::now();
    row.clone()
        .into_active_model()
        .reset_all()
        .update(&tx)
        .await?;
    tx.commit().await?;
    Ok(Some((row, execution, owner)))
}
async fn actor(s: &AppState, row: &turns::Model, c: &conversations::Model) -> Result<Scope> {
    let scope = Scope::for_actor(s, row.workspace_id.into(), row.owner_id.into()).await?;
    scope.require("conversations.run")?;
    scope.require("conversations.read")?;
    find(&s.db, &scope, c.id.into()).await?;
    available(&s.db, &scope, c.agent_id.into()).await?;
    Ok(scope)
}
pub(super) async fn leased(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
) -> Result<(
    sea_orm::DatabaseTransaction,
    turns::Model,
    conversations::Model,
)> {
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let current = turns::Entity::find_by_id(row.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.lease_owner != owner || current.expires_at <= db::now() || !active(&current.state) {
        return Err(error(StatusCode::CONFLICT, "conversations.leaseLost"));
    }
    let c = conversations::Entity::find_by_id(row.conversation_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    super::introductions::validate(&tx, &current, &c).await?;
    Ok((tx, current, c))
}
/// Live chat authority, also used immediately before API transport dispatch.
pub(crate) async fn api_authorized(s: &AppState, row: &turns::Model, owner: &str) -> Result<Scope> {
    let (tx, current, c) = leased(s, row, owner).await?;
    if current.state != "running" {
        return Err(conflict());
    }
    let scope = api_authorized_in(&tx, &current, &c).await?;
    tx.rollback().await?;
    Ok(scope)
}
pub(super) async fn api_authorized_in(
    conn: &impl ConnectionTrait,
    row: &turns::Model,
    c: &conversations::Model,
) -> Result<Scope> {
    let scope = Scope::for_actor_in(conn, row.workspace_id.into(), row.owner_id.into()).await?;
    scope.require("conversations.run")?;
    scope.require("credentials.use")?;
    scope.require("conversations.read")?;
    find(conn, &scope, c.id.into()).await?;
    let execution = groups::execution(conn, row, c.clone()).await?;
    usable(conn, &scope, execution.agent_id.into()).await?;
    Ok(scope)
}
pub(super) async fn append(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    mut event: Event,
) -> Result<()> {
    if let Event::Tool {
        input: Some(input), ..
    } = &mut event
    {
        crate::platform_tools::redact(input);
    }
    let mut public_event = serde_json::to_value(&event).map_err(|_| invalid())?;
    if let Some(input) = public_event.get_mut("input").and_then(Value::as_object_mut) {
        input.remove("_sealed");
    }
    let payload = serde_json::to_string(&public_event).map_err(|_| invalid())?;
    let (tx, mut current, mut c) = leased(s, row, owner).await?;
    let event_id = s.next_id()?;
    match &event {
        Event::Text {
            item_id,
            text,
            replace,
        } => {
            let execution = groups::execution(&tx, &current, c.clone()).await?;
            messages::append_text(
                &tx,
                &current,
                &execution,
                event_id.into(),
                item_id,
                text,
                *replace,
            )
            .await?;
        }
        Event::Session { native_session_id } => {
            current.candidate_session = native_session_id.clone()
        }
        Event::Approval {
            request_id,
            tool,
            input,
        } => {
            if tool == "ferry_ask_user" && super::questions::pending(&current) {
                return Err(error(StatusCode::CONFLICT, "conversations.questionPending"));
            }
            let mut decisions: Value =
                serde_json::from_str(&current.decisions).map_err(|_| invalid())?;
            if decisions.get(request_id).is_some() {
                return Err(invalid());
            }
            decisions[request_id] =
                json!({"tool":tool,"input":input,"decision":null,"dispatching":false,"sent":false});
            current.decisions = decisions.to_string();
        }
        Event::DecisionSent {
            request_id,
            allowed,
        } => {
            let mut decisions: Value =
                serde_json::from_str(&current.decisions).map_err(|_| invalid())?;
            let item = decisions.get_mut(request_id).ok_or_else(invalid)?;
            if item["decision"] != json!(allowed) || item["dispatching"] != true {
                return Err(invalid());
            }
            item["sent"] = json!(true);
            current.decisions = decisions.to_string();
        }
        Event::Finished { usage, .. } => current.usage = serde_json::to_string(usage).unwrap(),
        _ => {}
    }
    c.next_sequence = c.next_sequence.checked_add(1).ok_or_else(invalid)?;
    c.updated_at = db::now();
    events::Entity::insert(
        events::Model {
            id: event_id.into(),
            workspace_id: row.workspace_id,
            conversation_id: c.id,
            turn_id: row.id,
            sequence: c.next_sequence,
            payload,
            created_at: db::now(),
        }
        .into_active_model(),
    )
    .exec(&tx)
    .await?;
    current.version += 1;
    current.updated_at = db::now();
    current.into_active_model().reset_all().update(&tx).await?;
    c.into_active_model().reset_all().update(&tx).await?;
    tx.commit().await?;
    Ok(())
}
pub(super) async fn heartbeat(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
) -> Result<(bool, Vec<Input>)> {
    let (tx, mut current, c) = leased(s, row, owner).await?;
    // Resolve the current account and grants in the same serialized transaction.
    let scope =
        Scope::for_actor_in(&tx, current.workspace_id.into(), current.owner_id.into()).await?;
    let execution = groups::execution(&tx, &current, c.clone()).await;
    let permitted = scope
        .require("conversations.run")
        .and_then(|_| scope.require("conversations.read"))
        .is_ok()
        && find(&tx, &scope, c.id.into()).await.is_ok()
        && match execution {
            Ok(c) => usable(&tx, &scope, c.agent_id.into()).await.is_ok(),
            Err(_) => false,
        };
    let cancel = current.state == "stopping" || !permitted;
    let mut decisions: Value = serde_json::from_str(&current.decisions).map_err(|_| invalid())?;
    let mut inputs = vec![];
    if !cancel {
        for (id, item) in decisions.as_object_mut().ok_or_else(invalid)? {
            if let Some(allow) = item["decision"].as_bool() {
                if item["dispatching"] == false {
                    item["dispatching"] = json!(true);
                    inputs.push(Input::Decision {
                        request_id: id.clone(),
                        allow,
                    });
                }
            }
        }
    }
    current.decisions = decisions.to_string();
    current.expires_at = db::now() + LEASE;
    current.version += 1;
    if cancel {
        current.state = "stopping".into();
    }
    current.into_active_model().reset_all().update(&tx).await?;
    tx.commit().await?;
    Ok((cancel, inputs))
}
/// "{cli} failed on {host}" for a private conversation's bound host; a group
/// turn or an unbound conversation gets the general message.
async fn failed_on(conn: &impl ConnectionTrait, c: &conversations::Model) -> Message {
    let general = Message::new("conversations.executionFailed");
    if groups::is_group(conn, c.id.into()).await.unwrap_or(true) {
        return general;
    }
    let Ok(config) = serde_json::from_str::<Configuration>(&c.config) else {
        return general;
    };
    let Ok(runtime) = config.runtime.cli() else {
        return general;
    };
    let host = crate::entities::hosts::Entity::find_by_id(i64::from(config.host_id))
        .filter(crate::entities::hosts::Column::WorkspaceId.eq(c.workspace_id))
        .one(conn)
        .await
        .ok()
        .flatten();
    match host {
        Some(host) => Message::new("conversations.executionFailedOn")
            .with("cli", crate::agent_hosts::cli_name(runtime))
            .with("host", host.name),
        None => general,
    }
}
/// An approval nobody decided before its turn ended was never executed.
/// Record that on the decision itself, so the conversation can say so and
/// offer to continue, instead of inferring it from the turn's state.
pub(super) fn mark_ended_undecided(turn: &mut turns::Model) {
    let Ok(mut decisions) = serde_json::from_str::<Value>(&turn.decisions) else {
        return;
    };
    let mut changed = false;
    for item in decisions
        .as_object_mut()
        .into_iter()
        .flat_map(|map| map.values_mut())
    {
        if item["decision"].is_null() && item["ended_undecided"] != true {
            item["ended_undecided"] = json!(true);
            changed = true;
        }
    }
    if changed {
        turn.decisions = decisions.to_string();
    }
}

pub(super) async fn finish(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    outcome: Option<Outcome>,
    failure: &str,
    started: bool,
) -> Result<()> {
    let (tx, mut current, mut c) = leased(s, row, owner).await?;
    current.state = match outcome {
        Some(Outcome::Completed) => "completed",
        Some(Outcome::Cancelled) => "cancelled",
        Some(Outcome::TimedOut) => "timed_out",
        Some(Outcome::Failed) => "failed",
        None if started => "interrupted",
        None => "failed",
    }
    .into();
    let unexplained = failure.is_empty() && matches!(outcome, Some(Outcome::Failed));
    current.error = if unexplained {
        // The Runner gave no reason: name the CLI and host so the user knows where to look.
        let message = failed_on(&tx, &c).await;
        tracing::warn!(
            conversation = c.id,
            turn = current.id,
            "CLI turn failed without a reported reason"
        );
        crate::i18n::store_message(&message)
    } else if failure.starts_with("conversations.") {
        crate::i18n::store_message(&Message::new(failure))
    } else {
        failure.into()
    };
    current.expires_at = 0;
    current.version += 1;
    current.updated_at = db::now();
    mark_ended_undecided(&mut current);
    if matches!(outcome, Some(Outcome::Completed)) && current.candidate_session.is_empty() {
        return Err(error(StatusCode::CONFLICT, "conversations.interrupted"));
    }
    let group = groups::finish(&tx, &current, outcome, started).await?;
    if !group && matches!(outcome, Some(Outcome::Completed)) {
        if current.candidate_session.is_empty() {
            return Err(error(StatusCode::CONFLICT, "conversations.interrupted"));
        }
        c.native_session_id = current.candidate_session.clone();
    } else if !group && started && !matches!(outcome, Some(Outcome::Cancelled)) {
        // A failed/unknown turn may have modified native history or external data.
        c.state = "interrupted".into();
        c.native_session_id.clear();
    }
    c.updated_at = db::now();
    c.version += 1;
    current.into_active_model().reset_all().update(&tx).await?;
    c.into_active_model().reset_all().update(&tx).await?;
    tx.commit().await?;
    Ok(())
}
fn account_event(
    policy: &ferry_protocol::resources::ResourcePolicy,
    event: &Event,
    received: &mut usize,
    platform_calls: &mut usize,
) -> Result<bool> {
    let bytes = serde_json::to_vec(event).map_err(|_| invalid())?.len();
    *received = received.checked_add(bytes).ok_or_else(invalid)?;
    if matches!(event, Event::PlatformCall { .. }) {
        *platform_calls = platform_calls.checked_add(1).ok_or_else(invalid)?;
    }
    Ok(
        ferry_protocol::resources::allows(policy.agent_event_bytes, bytes)
            && ferry_protocol::resources::allows(policy.agent_turn_output_bytes, *received)
            && ferry_protocol::resources::allows(policy.agent_tool_calls, *platform_calls),
    )
}

pub(super) async fn execute(s: AppState, id: Id, mut stop: watch::Receiver<bool>) -> Result<()> {
    let Some((row, c, owner)) = claim(&s, id).await? else {
        return Ok(());
    };
    let config: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    if config.runtime == Runtime::Api {
        return super::api::execute(s, row, c, owner, stop).await;
    }
    let prepare = async {
        let scope = actor(&s, &row, &c).await?;
        let agent = available(&s.db, &scope, c.agent_id.into()).await?;
        let stored: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
        let provider =
            crate::models::cli_provider(&s, scope.workspace_id, c.agent_id, stored.runtime.cli()?)
                .await?;
        let custom_upstream = provider.is_some();
        // A custom upstream replaces the CLI's own login for this execution.
        let login_usable =
            |authentication: &str| custom_upstream || stored.authentication_usable(authentication);
        let placement::Placement {
            config,
            binding,
            host,
            channel,
            fresh,
            files: capability,
        } = placement::place(&s, &scope, &row, &c, &agent, login_usable).await?;
        // Files the user asks for are written to ./output of the Runner-managed
        // directory (never a chosen one) and attached to the reply. A read-only
        // conversation may then write only inside that directory.
        let confinable = crate::workflows::agent_files::runtime_supported(config.runtime.cli()?);
        let pinned = config.directory_pinned(c.id, c.agent_id);
        let files = capability.write_workdir && confinable && !pinned;
        let confined = files && config.permission == Permission::ReadOnly;
        // A new host or binding cannot resume the old native session; the
        // conversation's history in Ferry is sent instead.
        let native_session_id = if fresh {
            String::new()
        } else {
            row.native_session_id.clone()
        };
        let business_timezone =
            crate::accounts::workspace::business_timezone(&s.db, scope.workspace_id).await?;
        let timezone = crate::models::cli_timezone_at(
            &s,
            scope.workspace_id,
            c.agent_id,
            &host,
            business_timezone.as_ref().map(|zone| zone.name()),
        )
        .await?;
        let policy = crate::accounts::conversation_policy::Configuration::load(&s.db).await?;
        let mut prompt =
            super::introductions::prompt(&s, &row, &c, &scope, &policy, &config).await?;
        let clock = clock_context(business_timezone, db::now())?;
        prompt = format!("{clock}{prompt}");
        let resources = crate::accounts::resource_policy::load(&s.db).await?;
        let prompt_limit = ferry_protocol::resources::byte_limit(resources.agent_prompt_bytes);
        if !ferry_protocol::resources::allows(resources.agent_prompt_bytes, prompt.len()) {
            return Err(invalid());
        }
        let quoted = messages::reply_context(
            &s.db,
            &row,
            prompt_limit.saturating_sub(prompt.len().saturating_add(128)),
        )
        .await?;
        prompt = format!("{quoted}{prompt}");
        let context = groups::context(
            &s.db,
            &row,
            prompt_limit.saturating_sub(prompt.len().saturating_add(128)),
        )
        .await?;
        if !context.is_empty() {
            prompt = format!("{context}{prompt}");
        }
        if native_session_id.is_empty() && !groups::is_group(&s.db, c.id.into()).await? {
            let history = messages::recovery_context(
                &s.db,
                &row,
                prompt_limit.saturating_sub(prompt.len().saturating_add(256)),
            )
            .await?;
            prompt = format!("{history}{prompt}");
        }
        // Either how to deliver files, or why this conversation cannot, so the
        // Agent explains it instead of pretending.
        let contract = if files {
            crate::workflows::agent_files::conversation_contract(
                config.runtime.cli()?,
                confined,
                &capability.tools,
            )
        } else {
            crate::workflows::agent_files::conversation_unavailable(if !confinable {
                "this Agent's CLI cannot be confined to a working directory; only Codex and Claude Code agents can deliver files"
            } else if pinned {
                "this conversation works in a directory chosen for it, where files are not collected; start a conversation without a chosen directory"
            } else {
                "the Runner on this host is too old to deliver files; it must be upgraded"
            })
        };
        prompt = format!("{contract}{prompt}");
        if !ferry_protocol::resources::allows(resources.agent_prompt_bytes, prompt.len()) {
            return Err(invalid());
        }
        Ok((
            channel,
            files,
            Request {
                resource_policy: Some(resources),
                binding: Some(binding),
                runtime: config.runtime.cli()?,
                cwd: config.cwd,
                prompt,
                model: (!config.model.is_empty()).then_some(config.model),
                native_session_id: (!native_session_id.is_empty()).then_some(native_session_id),
                permission: config.permission,
                timeout_seconds: 0,
                platform_tools: true,
                provider,
                timezone,
                write_workdir: confined,
            },
        ))
    };
    // Admission is bounded; choosing a host may probe several Runners, so the
    // lease is renewed meanwhile and it cannot dispatch after losing it.
    let prepared = tokio::time::timeout(ADMISSION, renewing(&s, &row, &owner, prepare)).await;
    let (channel, files, request) = match prepared {
        Ok(Ok(value)) => value,
        Ok(Err(e)) => {
            finish(&s, &row, &owner, None, &e.stored(), false).await?;
            return Ok(());
        }
        Err(_) => {
            finish(
                &s,
                &row,
                &owner,
                None,
                "conversations.preflightFailed",
                false,
            )
            .await?;
            return Ok(());
        }
    };
    let policy = request.resource_policy.clone().unwrap_or_default();
    let tool_slots = policy.agent_tool_concurrency.map(|limit| {
        std::sync::Arc::new(tokio::sync::Semaphore::new(
            ferry_protocol::resources::byte_limit(Some(limit))
                .min(tokio::sync::Semaphore::MAX_PERMITS),
        ))
    });
    let mut pending_inputs = std::collections::VecDeque::new();
    let workdir = request.cwd.clone();
    let (cancel, cancelled) = watch::channel(false);
    let (events, mut receiver) = mpsc::channel(32);
    let (inputs, commands) = mpsc::channel(16);
    let running = channel.agent(request, events, commands, cancelled);
    tokio::pin!(running);
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outcome = None;
    let mut failure = String::new();
    let mut stopping = false;
    let mut received = 0usize;
    let mut platform_calls = 0usize;
    let mut resource_limited = false;
    let mut tool_tasks: JoinSet<(String, Value)> = JoinSet::new();
    // Do a fresh fenced check immediately before the first dispatch poll.
    let (must_stop, _) = heartbeat(&s, &row, &owner).await?;
    if must_stop || *stop.borrow() {
        finish(&s, &row, &owner, Some(Outcome::Cancelled), "", false).await?;
        return Ok(());
    }
    loop {
        tokio::select! {
            permit=inputs.reserve(),if !pending_inputs.is_empty() && !stopping => match permit {
                Ok(permit) => { permit.send(pending_inputs.pop_front().unwrap()); },
                Err(_) => { cancel.send_replace(true); stopping=true; }
            },
            Some(result)=tool_tasks.join_next(),if !tool_tasks.is_empty()=> {
                if let Ok((request_id,result))=result {
                    if !result.is_null() { pending_inputs.push_back(Input::PlatformResult{request_id,result}); }
                } else { cancel.send_replace(true);stopping=true; }
            },
            result=&mut running=>{
                while let Ok(event)=receiver.try_recv(){
                    if resource_limited { continue; }
                    if !account_event(&policy, &event, &mut received, &mut platform_calls)? {
                        resource_limited = true;
                        cancel.send_replace(true);
                        failure = "conversations.outputLimit".into();
                        outcome = Some(Outcome::Failed);
                        stopping = true;
                        continue;
                    }
                    // Calls already emitted by the CLI still need a receipt. In particular,
                    // a queued write must persist its approval before finish marks it expired.
                    if let Event::PlatformCall { request_id, name, arguments } = event {
                        if !stopping {
                            tool_tasks.spawn(platform_call_with_slots(s.clone(), row.clone(), owner.clone(), request_id, name, arguments, None, tool_slots.clone()));
                        }
                        continue;
                    }
                    if let Event::Finished{outcome:value,..}=&event {outcome=Some(*value);}
                    append(&s,&row,&owner,event).await?;
                }
                if let Err(e)=result{if !resource_limited {failure=e.stored();}}
                break;
            }
            event=receiver.recv()=>if let Some(event)=event {
                if resource_limited { continue; }
                if !account_event(&policy, &event, &mut received, &mut platform_calls)? {
                    resource_limited = true;
                    cancel.send_replace(true);
                    failure = "conversations.outputLimit".into();
                    outcome = Some(Outcome::Failed);
                    stopping = true;
                    continue;
                }
                if let Event::PlatformCall{request_id,name,arguments} = event {
                    if !stopping {
                        tool_tasks.spawn(platform_call_with_slots(s.clone(),row.clone(),owner.clone(),request_id,name,arguments,None,tool_slots.clone()));
                    } else { cancel.send_replace(true);stopping=true; }
                    continue;
                }
                if let Event::Finished{outcome:value,..}=&event {outcome=Some(*value);}
                if append(&s,&row,&owner,event).await.is_err(){
                    cancel.send_replace(true);
                    let _=tokio::time::timeout(Duration::from_secs(85),&mut running).await;
                    return Err(error(StatusCode::CONFLICT,"conversations.leaseLost"));
                }
            },
            _=stop.changed(),if !stopping=>{cancel.send_replace(true);stopping=true;failure="conversations.interrupted".into();},
            _=tick.tick()=>match heartbeat(&s,&row,&owner).await {
                Ok((must_stop,decisions))=>{
                    if must_stop {cancel.send_replace(true);stopping=true;}
                    if !stopping {for input in decisions {
                        if let Input::Decision{ref request_id,allow} = input {
                            if let Some(arguments)=approved_platform_call(&s,&row,&owner,request_id).await? {
                                tool_tasks.spawn(platform_call_with_slots(s.clone(),row.clone(),owner.clone(),request_id.clone(),"ferry_request".into(),arguments,Some(allow),tool_slots.clone()));
                                continue;
                            }
                        }
                        pending_inputs.push_back(input);
                    }}
                }
                Err(_)=>{
                    cancel.send_replace(true);
                    let _=tokio::time::timeout(Duration::from_secs(85),&mut running).await;
                    return Err(error(StatusCode::CONFLICT,"conversations.leaseLost"));
                }
            }
        }
    }
    // Do not cancel a task between receiving a call and storing its approval.
    // The CLI has ended, so results cannot be delivered to it, but the durable
    // receipt must survive for the owner to continue in another turn.
    renewing(&s, &row, &owner, async {
        while tool_tasks.join_next().await.is_some() {}
        Ok(())
    })
    .await?;
    if files && failure.is_empty() && matches!(outcome, Some(Outcome::Completed)) {
        deliver_files(&s, &row, &owner, channel.as_ref(), &workdir).await?;
    }
    finish(&s, &row, &owner, outcome, &failure, true).await
}

/// Attaches what the Agent left in ./output to its reply and removes it from
/// the host. A limit or type problem becomes a notice; the reply stays.
async fn deliver_files(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    channel: &dyn crate::hosts::channel::HostChannel,
    workdir: &str,
) -> Result<()> {
    use crate::workflows::agent_files;
    let mut mode = agent_files::FileMode::conversation();
    agent_files::apply_policy(
        &mut mode,
        &crate::accounts::resource_policy::load(&s.db).await?,
    );
    let produced = match agent_files::collect_any(channel, workdir, &mode).await {
        Ok(produced) => produced,
        Err(error) => {
            tracing::warn!(turn = row.id, error = %error.1, "agent files rejected");
            let (tx, current, c) = leased(s, row, owner).await?;
            let execution = groups::execution(&tx, &current, c).await?;
            let message = error
                .2
                .clone()
                .unwrap_or_else(|| Message::new("workflow.agentFilesTotal"));
            placement::notice(
                s,
                &tx,
                current.workspace_id,
                current.conversation_id,
                current.owner_id,
                execution.agent_id,
                message,
            )
            .await?;
            tx.commit().await?;
            return Ok(());
        }
    };
    if produced.is_empty() {
        return Ok(());
    }
    let mut attachments = Vec::new();
    for file in &produced {
        attachments.push(files::produced(s, row, file).await?);
    }
    let (tx, current, c) = leased(s, row, owner).await?;
    let execution = groups::execution(&tx, &current, c).await?;
    messages::attach_files(&tx, &current, &execution, s.next_id()?.into(), attachments).await?;
    tx.commit().await?;
    agent_files::remove(channel, workdir, &produced).await;
    Ok(())
}

/// Cancel a room Agent's turns that have not started (queued or held), for
/// example when it is removed or its sponsor can no longer run it.
pub(super) async fn cancel_queued_for(
    conn: &impl ConnectionTrait,
    room_agent: Id,
    key: &'static str,
) -> Result<()> {
    let ids: Vec<i64> = crate::entities::ai_chat_turns::Entity::find()
        .select_only()
        .column(crate::entities::ai_chat_turns::Column::Id)
        .filter(crate::entities::ai_chat_turns::Column::ChatAgentId.eq(room_agent))
        .into_tuple()
        .all(conn)
        .await?;
    if ids.is_empty() {
        return Ok(());
    }
    turns::Entity::update_many()
        .col_expr(
            turns::Column::State,
            sea_orm::sea_query::Expr::value("cancelled"),
        )
        .col_expr(
            turns::Column::Error,
            sea_orm::sea_query::Expr::value(crate::i18n::store_message(&Message::new(key))),
        )
        .col_expr(turns::Column::Version, {
            use sea_orm::sea_query::ExprTrait;
            sea_orm::sea_query::Expr::col(turns::Column::Version).add(1_i64)
        })
        .col_expr(
            turns::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(db::now()),
        )
        .filter(turns::Column::Id.is_in(ids))
        .filter(turns::Column::State.is_in(["queued", "held"]))
        .exec(conn)
        .await?;
    Ok(())
}
