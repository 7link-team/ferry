//! Which Runner host runs a conversation, or one Agent in a room. Creating a
//! conversation chooses nothing; the first message binds a host, later
//! messages keep it while it works, and a host that stops working is replaced
//! — unless the conversation works in a chosen directory, whose files only
//! exist there, in which case the user decides (`rebind`).
use super::*;
use crate::{
    agent_hosts::{self, Reasons},
    entities::{
        ai_chat_agents as members, ai_chat_messages as chat, ai_chat_turns as links, hosts,
    },
    hosts::channel::HostChannel,
};
use ferry_protocol::agent::Binding;

/// Stored for a conversation (or room Agent) that has no host yet.
pub(super) const UNBOUND: &str = "null";

/// Where one turn runs.
pub(super) struct Placement {
    pub config: Configuration,
    pub binding: Binding,
    pub host: hosts::Model,
    pub channel: Box<dyn HostChannel>,
    /// The native CLI session cannot be resumed here: a new host or binding.
    pub fresh: bool,
    /// What the Runner reported for file output (see `Probe::write_workdir`).
    pub files: FileCapability,
}

/// Whether the Runner can confine a turn's writes, and its file tools.
#[derive(Clone, Debug, Default)]
pub(super) struct FileCapability {
    pub write_workdir: bool,
    pub tools: Vec<String>,
}
impl FileCapability {
    fn of(probe: &ferry_protocol::agent::Probe) -> Self {
        Self {
            write_workdir: probe.write_workdir,
            tools: probe.file_tools.clone(),
        }
    }
}

/// Why the bound host cannot run this turn.
struct Unusable {
    host: Option<hosts::Model>,
    reason: Message,
    /// The host itself still works (its CLI path or account changed): binding
    /// it again is allowed, even for a chosen directory.
    same_host_ok: bool,
}

fn parse_binding(text: &str) -> Result<Option<Binding>> {
    serde_json::from_str(text).map_err(|_| invalid())
}

impl Configuration {
    /// Whether the directory was chosen (on the Agent or at creation) rather
    /// than managed by the Runner. Conversations created before hosts were
    /// assigned lazily do not record it; a managed directory is recognised by
    /// its per-conversation name.
    pub(crate) fn directory_pinned(&self, conversation: i64, agent: i64) -> bool {
        self.pinned_directory.unwrap_or_else(|| {
            let managed = format!("{conversation}-{agent}");
            !self.cwd.is_empty() && !self.cwd.trim_end_matches(['/', '\\']).ends_with(&managed)
        })
    }
}

/// The host for this turn, binding or moving it as needed and recording that
/// before the turn starts.
pub(super) async fn place(
    s: &AppState,
    scope: &Scope,
    row: &turns::Model,
    c: &conversations::Model,
    agent: &ai_agents::Model,
    login_usable: impl Fn(&str) -> bool + Copy,
) -> Result<Placement> {
    let mut stored_binding = c.binding.clone();
    let mut stored_config = c.config.clone();
    // A concurrent worker may bind first; then its binding is used.
    for _ in 0..2 {
        let config: Configuration = serde_json::from_str(&stored_config).map_err(|_| invalid())?;
        let pinned = config.directory_pinned(c.id, agent.id);
        // A host released by `rebind` is not chosen again.
        let mut moved_from =
            (i64::from(config.previous_host) != 0).then(|| i64::from(config.previous_host));
        let mut exclude: Vec<i64> = moved_from.into_iter().collect();
        // The bound host still works but its binding changed: try it first,
        // and only it when the directory was chosen.
        let mut prefer = None;
        let mut only_preferred = false;
        // The bound host's own reason leads the error if no other host works.
        let mut bound_failure = None;
        if let Some(binding) = parse_binding(&stored_binding)? {
            match current(s, scope, &config, &binding, login_usable).await {
                Ok((host, channel, files)) => {
                    return Ok(Placement {
                        config,
                        binding,
                        host,
                        channel,
                        fresh: false,
                        files,
                    });
                }
                Err(unusable) => {
                    let host_id = unusable
                        .host
                        .as_ref()
                        .map_or(i64::from(config.host_id), |h| h.id);
                    if pinned && !unusable.same_host_ok {
                        let name = unusable
                            .host
                            .as_ref()
                            .map_or_else(|| host_id.to_string(), |h| h.name.clone());
                        return Err(ApiError::localized(
                            StatusCode::CONFLICT,
                            Message::new("conversations.hostUnavailable")
                                .with("host", name)
                                .with_message("reason", unusable.reason),
                        ));
                    }
                    if unusable.same_host_ok {
                        prefer = unusable.host;
                        only_preferred = pinned;
                    } else {
                        exclude.push(host_id);
                        let name = match unusable.host.as_ref() {
                            Some(host) => host.name.clone(),
                            None => hosts::Entity::find_by_id(host_id)
                                .one(&s.db)
                                .await?
                                .map_or_else(|| host_id.to_string(), |h| h.name),
                        };
                        bound_failure = Some((name, unusable.reason));
                    }
                    moved_from = Some(host_id);
                }
            }
        }
        let mut candidates = if only_preferred {
            vec![]
        } else {
            let requested =
                (i64::from(config.requested_host) != 0).then(|| config.requested_host.into());
            agent_hosts::candidates(&s.db, scope.workspace_id, requested, agent, &exclude).await?
        };
        if let Some(host) = prefer {
            candidates.retain(|known| known.id != host.id);
            candidates.insert(0, host);
        }
        let (host, channel, binding, files) = match allocate(
            s,
            scope,
            c,
            agent,
            &config,
            pinned,
            candidates,
            login_usable,
        )
        .await
        {
            Ok(found) => found,
            Err(Unplaced::Reasons(reasons)) => {
                return Err(match bound_failure {
                    Some((name, reason)) => reasons.into_failover_error(name, reason),
                    None => reasons.into_error(),
                });
            }
            Err(Unplaced::Error(error)) => return Err(error),
        };
        let mut next = config.clone();
        next.host_id = host.id.into();
        next.cwd = binding.canonical_directory.clone();
        next.runner_key_hash = host.runner_key_hash.clone();
        next.previous_host = Id::from(0);
        let from = match moved_from.filter(|id| *id != host.id) {
            Some(id) => Some(
                hosts::Entity::find_by_id(id)
                    .one(&s.db)
                    .await?
                    .map_or_else(|| id.to_string(), |h| h.name),
            ),
            None => None,
        };
        match store(s, row, c, &stored_binding, &host, &binding, from.as_deref()).await? {
            Stored::Bound => {
                return Ok(Placement {
                    config: next,
                    binding,
                    host,
                    channel,
                    fresh: true,
                    files,
                });
            }
            Stored::Changed(config, binding) => {
                stored_config = config;
                stored_binding = binding;
            }
        }
    }
    Err(conflict())
}

/// Checks the bound host still runs this CLI exactly as bound.
/// How recently the bound host must have been seen to wait for it to reconnect.
const RECONNECT_GRACE_WINDOW_MS: i64 = 120_000;
/// How long to wait for a recently seen bound host before failing over.
const RECONNECT_GRACE: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(600)
} else {
    std::time::Duration::from_secs(20)
};
const RECONNECT_POLL: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(100)
} else {
    std::time::Duration::from_secs(2)
};

async fn current(
    s: &AppState,
    scope: &Scope,
    config: &Configuration,
    binding: &Binding,
    login_usable: impl Fn(&str) -> bool,
) -> std::result::Result<(hosts::Model, Box<dyn HostChannel>, FileCapability), Unusable> {
    let runtime = config.runtime.cli().map_err(|_| Unusable {
        host: None,
        reason: Message::new("conversations.invalid"),
        same_host_ok: false,
    })?;
    let cli = agent_hosts::cli_name(runtime);
    let host = hosts::Entity::find_by_id(config.host_id)
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await
        .ok()
        .flatten();
    let Some(host) = host else {
        return Err(Unusable {
            host: None,
            reason: Message::new("models.reasonOffline"),
            same_host_ok: false,
        });
    };
    let unusable = |host: &hosts::Model, reason: Message, same_host_ok: bool| Unusable {
        host: Some(host.clone()),
        reason,
        same_host_ok,
    };
    if host.runner_key_hash.is_empty() || host.runner_key_hash != config.runner_key_hash {
        // Re-enrolled: the same machine may still run it, with a new binding.
        let reason = Message::new("conversations.bindingChanged");
        return Err(unusable(&host, reason, !host.runner_key_hash.is_empty()));
    }
    // A Runner that was online moments ago is usually just reconnecting
    // (a network blip or an update restart takes a few seconds). Moving the
    // conversation loses the CLI's own context, so give its host a short grace
    // period before failing over.
    let recently_seen = crate::db::now() - host.seen_at < RECONNECT_GRACE_WINDOW_MS;
    let deadline = tokio::time::Instant::now() + RECONNECT_GRACE;
    // Reaching the Runner (opening a channel and probing over it) is retried;
    // what the probe reports about the CLI is an answer, not retried.
    let (channel, probe) = loop {
        let reached =
            match crate::hosts::remote::open(s, &scope.workspace_id, &config.host_id).await {
                Ok(channel) => match channel
                    .agent_probe(runtime, &config.cwd, Some(binding))
                    .await
                {
                    Ok(probe) => Ok((channel, probe)),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
        match reached {
            Ok(reached) => break reached,
            Err(error) => {
                if !recently_seen || tokio::time::Instant::now() >= deadline {
                    return Err(unusable(&host, offline(&error), false));
                }
                tokio::time::sleep(RECONNECT_POLL).await;
            }
        }
    };
    let files = FileCapability::of(&probe);
    let checked = match agent_hosts::check(probe, &cli, &login_usable) {
        Ok(checked) => checked,
        Err(reason) => return Err(unusable(&host, reason, false)),
    };
    if checked.execution_user != binding.execution_user
        || checked.canonical_directory != binding.canonical_directory
        || checked.execution_identity != binding.execution_identity
        || checked.executable != binding.executable
    {
        let reason = Message::new("conversations.bindingChanged");
        return Err(unusable(&host, reason, true));
    }
    Ok((host, channel, files))
}

fn offline(error: &ApiError) -> Message {
    Message::new("models.reasonUnreachable").with_message("detail", agent_hosts::reason_from(error))
}

/// Why no candidate could be used: every host's reason, or a platform error.
enum Unplaced {
    Reasons(Reasons),
    Error(ApiError),
}
impl From<ApiError> for Unplaced {
    fn from(error: ApiError) -> Self {
        Self::Error(error)
    }
}

/// The first candidate host that runs this CLI, or every host and its reason.
#[allow(clippy::too_many_arguments)]
async fn allocate(
    s: &AppState,
    scope: &Scope,
    c: &conversations::Model,
    agent: &ai_agents::Model,
    config: &Configuration,
    pinned: bool,
    candidates: Vec<hosts::Model>,
    login_usable: impl Fn(&str) -> bool + Copy,
) -> std::result::Result<(hosts::Model, Box<dyn HostChannel>, Binding, FileCapability), Unplaced> {
    let cli = agent_hosts::cli_name(config.runtime.cli()?);
    let mut reasons = Reasons::default();
    for host in candidates {
        if host.state != "online" {
            reasons.push(&host, Message::new("models.reasonOffline"));
            continue;
        }
        if host.runner_key_hash.is_empty() {
            reasons.push(&host, Message::new("models.reasonNotRunner"));
            continue;
        }
        if let Err(error) =
            crate::models::cli_timezone(s, scope.workspace_id, agent.id, &host).await
        {
            reasons.push(&host, agent_hosts::reason_from(&error));
            continue;
        }
        let channel =
            match crate::hosts::remote::open(s, &scope.workspace_id, &host.id.into()).await {
                Ok(channel) => channel,
                Err(error) => {
                    reasons.push(&host, offline(&error));
                    continue;
                }
            };
        let probe = if pinned && !config.cwd.is_empty() {
            channel
                .agent_probe(config.runtime.cli()?, &config.cwd, None)
                .await
        } else {
            channel
                .agent_workspace(config.runtime.cli()?, &format!("{}-{}", c.id, agent.id))
                .await
        };
        let probe = match probe {
            Ok(probe) => probe,
            Err(error) => {
                reasons.push(&host, agent_hosts::reason_from(&error));
                continue;
            }
        };
        let files = FileCapability::of(&probe);
        match agent_hosts::check(probe, &cli, login_usable) {
            Ok(binding) => return Ok((host, channel, binding, files)),
            Err(reason) => reasons.push(&host, reason),
        }
    }
    Err(Unplaced::Reasons(reasons))
}

enum Stored {
    Bound,
    /// Someone else bound or released it first: its current config and binding.
    Changed(String, String),
}

/// Records the new binding if the stored one is still `expected`, clears the
/// native session (it lives on the old host) and tells the conversation when
/// execution moved from another host.
async fn store(
    s: &AppState,
    row: &turns::Model,
    c: &conversations::Model,
    expected: &str,
    host: &hosts::Model,
    binding: &Binding,
    moved_from: Option<&str>,
) -> Result<Stored> {
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let binding_text = serde_json::to_string(binding).map_err(|_| invalid())?;
    let patch = |config: &str| -> Result<String> {
        let mut value: Value = serde_json::from_str(config).map_err(|_| invalid())?;
        value["host_id"] = json!(Id::from(host.id));
        value["cwd"] = json!(binding.canonical_directory);
        value["runner_key_hash"] = json!(host.runner_key_hash);
        if let Some(object) = value.as_object_mut() {
            object.remove("previous_host");
        }
        Ok(value.to_string())
    };
    let agent_id;
    if let Some(link) = links::Entity::find_by_id(row.id).one(&tx).await? {
        let mut member = members::Entity::find_by_id(link.chat_agent_id)
            .one(&tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if member.binding != expected {
            return Ok(Stored::Changed(member.config, member.binding));
        }
        agent_id = member.agent_id;
        member.config = patch(&member.config)?;
        member.binding = binding_text;
        member.native_session_id.clear();
        member.into_active_model().reset_all().update(&tx).await?;
    } else {
        let mut current = conversations::Entity::find_by_id(c.id)
            .one(&tx)
            .await?
            .ok_or_else(ApiError::missing)?;
        if current.binding != expected {
            return Ok(Stored::Changed(current.config, current.binding));
        }
        agent_id = current.agent_id;
        current.config = patch(&current.config)?;
        current.binding = binding_text;
        current.native_session_id.clear();
        current.version += 1;
        current.into_active_model().reset_all().update(&tx).await?;
    }
    if let Some(from) = moved_from {
        notice(
            s,
            &tx,
            row.workspace_id,
            c.id,
            row.owner_id,
            agent_id,
            Message::new("conversations.hostSwitched")
                .with("from", from.to_owned())
                .with("to", host.name.clone()),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Stored::Bound)
}

/// A line in the conversation from the platform itself, rendered per reader.
pub(super) async fn notice(
    s: &AppState,
    conn: &impl ConnectionTrait,
    workspace: i64,
    conversation: i64,
    owner: i64,
    agent: i64,
    message: Message,
) -> Result<Id> {
    let id = s.next_id()?;
    chat::Entity::insert(
        chat::Model {
            id: id.into(),
            workspace_id: workspace,
            conversation_id: conversation,
            owner_id: owner,
            created_at: db::now(),
            client_id: format!("notice-{id}"),
            prompt: crate::i18n::store_message(&message),
            attachments: "[]".into(),
            targets: "[]".into(),
            role: "notice".into(),
            agent_id: agent,
            turn_id: None,
            reply_to: None,
            sender_name: String::new(),
            text_items: None,
            version: 1,
            edited_at: None,
            retracted_at: None,
            mentions: "[]".into(),
        }
        .into_active_model(),
    )
    .exec(conn)
    .await?;
    Ok(id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rebind {
    /// The room Agent to move; omitted for a private conversation.
    #[serde(default)]
    agent_id: Option<Id>,
    /// The room Agent row; required when several members added the same Agent.
    #[serde(default)]
    room_agent_id: Option<Id>,
}

/// Releases the current host so the next message runs on another one, in a
/// Runner-managed directory there (a chosen directory's files stay behind).
pub(super) async fn rebind(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Rebind>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    if has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    let release = |config: &str| -> Result<(String, i64)> {
        let mut value: Value = serde_json::from_str(config).map_err(|_| invalid())?;
        let host = value["host_id"]
            .as_str()
            .and_then(|v| v.parse::<i64>().ok())
            .or_else(|| value["host_id"].as_i64())
            .unwrap_or(0);
        value["previous_host"] = json!(Id::from(host));
        value["host_id"] = json!(Id::from(0));
        value["cwd"] = json!("");
        value["pinned_directory"] = json!(false);
        value["runner_key_hash"] = json!("");
        Ok((value.to_string(), host))
    };
    let (host, agent) = if groups::is_group(&tx, id).await? {
        let rooms: Vec<Id> = body.room_agent_id.into_iter().collect();
        let targets: Vec<Id> = if rooms.is_empty() {
            vec![body.agent_id.ok_or_else(invalid)?]
        } else {
            vec![]
        };
        let mut member = super::room_agents::resolve(&tx, id, &rooms, &targets)
            .await?
            .pop()
            .ok_or_else(ApiError::missing)?;
        let agent = member.agent_id;
        if member.binding == UNBOUND {
            return Err(conflict());
        }
        let (config, host) = release(&member.config)?;
        member.config = config;
        member.binding = UNBOUND.into();
        member.native_session_id.clear();
        member.state = "active".into();
        member.into_active_model().reset_all().update(&tx).await?;
        (host, i64::from(agent))
    } else {
        if body
            .agent_id
            .is_some_and(|agent| i64::from(agent) != c.agent_id)
        {
            return Err(invalid());
        }
        if c.owner_id != i64::from(scope.user.id) {
            return Err(ApiError::missing());
        }
        if c.binding == UNBOUND {
            return Err(conflict());
        }
        let (config, host) = release(&c.config)?;
        c.config = config;
        c.binding = UNBOUND.into();
        c.native_session_id.clear();
        if c.state == "interrupted" {
            c.state = "active".into();
        }
        (host, c.agent_id)
    };
    let name = hosts::Entity::find_by_id(host)
        .one(&tx)
        .await?
        .map_or_else(|| host.to_string(), |h| h.name);
    notice(
        &s,
        &tx,
        scope.workspace_id.into(),
        id.into(),
        scope.user.id.into(),
        agent,
        Message::new("conversations.hostReleased").with("host", name.clone()),
    )
    .await?;
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "rebind",
        "conversations",
        &id,
        json!({"host":name,"agent_id":Id::from(agent)}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

/// Host names for the conversation header: each bound host, or none yet.
pub(super) async fn host_view(
    conn: &impl ConnectionTrait,
    config: &str,
    binding: &str,
) -> Result<Value> {
    if binding == UNBOUND {
        return Ok(Value::Null);
    }
    let config: Value = serde_json::from_str(config).unwrap_or(Value::Null);
    let Some(id) = config["host_id"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok())
        .or_else(|| config["host_id"].as_i64())
        .filter(|id| *id > 0)
    else {
        return Ok(Value::Null);
    };
    let name = hosts::Entity::find_by_id(id)
        .one(conn)
        .await?
        .map(|h| h.name)
        .unwrap_or_default();
    Ok(json!({"id":Id::from(id),"name":name}))
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
