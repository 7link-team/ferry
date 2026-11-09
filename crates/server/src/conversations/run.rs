use super::*;
use ferry_protocol::agent::{Binding, Event, Input, Outcome, Request};
use std::time::Duration;
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
const LEASE: i64 = 30_000;

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
                if tasks.len()>=16 {continue;}
                let rows=turns::Entity::find().filter(sea_orm::Condition::any()
                    .add(turns::Column::State.eq("queued"))
                    .add(sea_orm::Condition::all().add(turns::Column::State.is_in(["running","stopping"])).add(turns::Column::ExpiresAt.lt(db::now()))))
                    .order_by_asc(turns::Column::Id).limit((16-tasks.len()) as u64).all(&s.db).await;
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
            row.error = crate::i18n::store_message(&Message::new("conversations.interrupted"));
            row.version += 1;
            row.updated_at = db::now();
            if !groups::finish(&tx, &row, None, true).await? {
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
async fn leased(
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
    Ok((tx, current, c))
}
pub(super) async fn append(
    s: &AppState,
    row: &turns::Model,
    owner: &str,
    event: Event,
) -> Result<()> {
    let payload = serde_json::to_string(&event).map_err(|_| invalid())?;
    if payload.len() > 1024 * 1024 {
        return Err(invalid());
    }
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
    current.error = if failure.starts_with("conversations.") {
        crate::i18n::store_message(&Message::new(failure))
    } else {
        failure.into()
    };
    current.expires_at = 0;
    current.version += 1;
    current.updated_at = db::now();
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
async fn execute(s: AppState, id: Id, mut stop: watch::Receiver<bool>) -> Result<()> {
    let Some((row, c, owner)) = claim(&s, id).await? else {
        return Ok(());
    };
    let prepare = async {
        let scope = actor(&s, &row, &c).await?;
        let config: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
        let binding: Binding = serde_json::from_str(&c.binding).map_err(|_| invalid())?;
        let host = crate::entities::hosts::Entity::find_by_id(config.host_id)
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if host.runner_key_hash != config.runner_key_hash || config.runner_key_hash.is_empty() {
            return Err(error(StatusCode::CONFLICT, "conversations.bindingChanged"));
        }
        let channel = crate::hosts::remote::open(&s, &scope.workspace_id, &config.host_id).await?;
        let probe = channel.agent_probe(config.runtime, &config.cwd).await?;
        if probe.execution_user != binding.execution_user
            || probe.canonical_directory.as_deref() != Some(&binding.canonical_directory)
            || probe.execution_identity != binding.execution_identity
            || probe.executable.as_deref() != Some(&binding.executable)
        {
            return Err(error(StatusCode::CONFLICT, "conversations.bindingChanged"));
        }
        if !config.authentication_usable(&probe.authentication) {
            return Err(error(StatusCode::CONFLICT, "conversations.loginRequired"));
        }
        let mut prompt = String::new();
        if !config.system_prompt.is_empty() {
            prompt.push_str(&config.system_prompt);
            prompt.push_str("\n\n");
        }
        prompt.push_str(&row.prompt);
        for a in serde_json::from_str::<Vec<Attachment>>(&row.attachments).map_err(|_| invalid())? {
            if !a.text.is_empty() {
                prompt.push_str(&format!(
                    "\n\n<attached_text name={:?}>\n{}\n</attached_text>",
                    a.name, a.text
                ));
            } else {
                prompt.push_str(&format!("\n\n<attachment name={:?} mime={:?} size={}>File stored in chat; its contents are NOT available to this text-only runtime. Do not claim to have read it.</attachment>",a.name,a.mime,a.size));
            }
        }
        if prompt.trim().is_empty() {
            prompt.push_str("A message with attachments was sent.");
        }
        if prompt.len() > 64 * 1024 {
            return Err(invalid());
        }
        let quoted = messages::reply_context(
            &s.db,
            &row,
            (64 * 1024usize)
                .saturating_sub(prompt.len() + 128)
                .min(8 * 1024),
        )
        .await?;
        prompt = format!("{quoted}{prompt}");
        let context = groups::context(
            &s.db,
            &row,
            (64 * 1024usize)
                .saturating_sub(prompt.len() + 128)
                .min(16 * 1024),
        )
        .await?;
        if !context.is_empty() {
            prompt = format!("{context}{prompt}");
        }
        Ok((
            channel,
            Request {
                binding: Some(binding),
                runtime: config.runtime,
                cwd: config.cwd,
                prompt,
                model: (!config.model.is_empty()).then_some(config.model),
                native_session_id: (!row.native_session_id.is_empty())
                    .then_some(row.native_session_id.clone()),
                permission: config.permission,
                timeout_seconds: 3600,
            },
        ))
    };
    // Keep admission bounded so it cannot dispatch after its lease expires.
    let prepared = tokio::time::timeout(Duration::from_secs(20), prepare).await;
    let (channel, request) = match prepared {
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
    // Do a fresh fenced check immediately before the first dispatch poll.
    let (must_stop, _) = heartbeat(&s, &row, &owner).await?;
    if must_stop || *stop.borrow() {
        finish(&s, &row, &owner, Some(Outcome::Cancelled), "", false).await?;
        return Ok(());
    }
    loop {
        tokio::select! {
            result=&mut running=>{
                while let Ok(event)=receiver.try_recv(){
                    if let Event::Finished{outcome:value,..}=&event {outcome=Some(*value);}
                    append(&s,&row,&owner,event).await?;
                }
                if let Err(e)=result{failure=e.stored();}
                break;
            }
            event=receiver.recv()=>if let Some(event)=event {
                received+=serde_json::to_vec(&event).map_err(|_|invalid())?.len();
                if received>18*1024*1024 {cancel.send_replace(true);failure="conversations.outputLimit".into();stopping=true;continue;}
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
                    if !stopping {for input in decisions {if inputs.try_send(input).is_err(){cancel.send_replace(true);stopping=true;}}}
                }
                Err(_)=>{
                    cancel.send_replace(true);
                    let _=tokio::time::timeout(Duration::from_secs(85),&mut running).await;
                    return Err(error(StatusCode::CONFLICT,"conversations.leaseLost"));
                }
            }
        }
    }
    finish(&s, &row, &owner, outcome, &failure, true).await
}
