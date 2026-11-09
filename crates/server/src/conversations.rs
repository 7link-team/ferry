//! Private conversations; HTTP owns authorization, the scheduler owns execution.
mod files;
mod groups;
mod interactions;
pub(crate) mod messages;
mod run;
#[cfg(test)]
mod tests;
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        ai_agents, ai_conversations as conversations, ai_events as events, ai_turns as turns,
    },
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
pub use run::scheduler;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/conversations", get(list).post(create))
        .route(
            "/{workspace}/conversations/{id}",
            get(detail).patch(update).delete(remove),
        )
        .merge(files::routes())
        .merge(groups::routes())
        .merge(interactions::routes())
        .route("/{workspace}/conversations/{id}/events", get(history))
        .route("/{workspace}/conversations/{id}/turns", post(send))
        .route(
            "/{workspace}/conversations/{id}/turns/{turn}/cancellations",
            post(cancel),
        )
        .route(
            "/{workspace}/conversations/{id}/turns/{turn}/decisions",
            post(decide),
        )
}
pub(super) fn error(status: StatusCode, key: &'static str) -> ApiError {
    ApiError::localized(status, Message::new(key))
}
fn invalid() -> ApiError {
    error(StatusCode::BAD_REQUEST, "conversations.invalid")
}
fn conflict() -> ApiError {
    error(StatusCode::CONFLICT, "conversations.changed")
}
pub(super) fn active(state: &str) -> bool {
    matches!(state, "queued" | "running" | "stopping")
}
async fn find(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<conversations::Model> {
    scope.require("conversations.read")?;
    let c = conversations::Entity::find_by_id(id)
        .filter(conversations::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    if c.owner_id != i64::from(scope.user.id) && !groups::member(conn, &c, scope.user.id).await? {
        return Err(ApiError::missing());
    }
    Ok(c)
}
pub(super) async fn available(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<ai_agents::Model> {
    let row = usable(conn, scope, id).await?;
    if row.state != "enabled" {
        return Err(error(StatusCode::CONFLICT, "conversations.agentDisabled"));
    }
    Ok(row)
}
// Disabling blocks admission; existing execution still checks grants and scope.
pub(super) async fn usable(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<ai_agents::Model> {
    scope.require("ai-agents.read")?;
    scope.require("ai-agents.use")?;
    let row = ai_agents::Entity::find_by_id(id)
        .filter(ai_agents::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.visibility != "workspace" && row.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    Ok(row)
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Configuration {
    pub runtime: ferry_protocol::agent::Runtime,
    pub cwd: String,
    pub host_id: Id,
    pub model: String,
    pub system_prompt: String,
    pub permission: ferry_protocol::agent::Permission,
    pub name: String,
    #[serde(default)]
    pub runner_key_hash: String,
}
impl Configuration {
    fn authentication_usable(&self, authentication: &str) -> bool {
        authentication == "logged_in"
            || (authentication == "unknown"
                && matches!(
                    self.runtime,
                    ferry_protocol::agent::Runtime::Gemini
                        | ferry_protocol::agent::Runtime::Qwen
                        | ferry_protocol::agent::Runtime::Opencode
                        | ferry_protocol::agent::Runtime::Pi
                ))
    }
    fn for_conversation(a: &ai_agents::Model, body: &Create) -> Result<Self> {
        let mut config = Self::from_agent(a)?;
        if let Some(cwd) = &body.cwd {
            config.cwd = cwd.trim().to_owned();
        }
        if let Some(permission) = body.permission {
            config.permission = permission;
        }
        if config.runtime == ferry_protocol::agent::Runtime::Pi
            && config.permission != ferry_protocol::agent::Permission::ReadOnly
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "conversations.cliPermissionUnsupported",
            ));
        }
        if config.cwd.len() > 4096 || config.cwd.contains('\0') {
            return Err(invalid());
        }
        if !config.cwd.is_empty() {
            crate::hosts::channel::check_path(&config.cwd)?;
        }
        Ok(config)
    }
    fn from_agent(a: &ai_agents::Model) -> Result<Self> {
        Ok(Self {
            runtime: ferry_protocol::agent::Runtime::parse(&a.runtime).ok_or_else(invalid)?,
            cwd: a.cwd.clone(),
            host_id: a.host_id.into(),
            model: a.model.clone(),
            system_prompt: a.system_prompt.clone(),
            permission: match a.permission.as_str() {
                "read_only" => ferry_protocol::agent::Permission::ReadOnly,
                "on_request" => ferry_protocol::agent::Permission::OnRequest,
                _ => return Err(invalid()),
            },
            name: a.name.clone(),
            runner_key_hash: String::new(),
        })
    }
}
fn view(c: &conversations::Model) -> Value {
    let mut config = serde_json::from_str::<Value>(&c.config).unwrap_or(Value::Null);
    if let Some(object) = config.as_object_mut() {
        object.remove("runner_key_hash");
    }
    json!({"id":Id::from(c.id),"agent_id":Id::from(c.agent_id),"revision_id":Id::from(c.revision_id),"title":c.title,"state":c.state,
        "agent_name":serde_json::from_str::<Value>(&c.config).ok().and_then(|v|v["name"].as_str().map(str::to_owned)),
        "config":config,"binding":serde_json::from_str::<Value>(&c.binding).unwrap_or(Value::Null),
        "version":c.version,"created_at":c.created_at,"updated_at":c.updated_at})
}
pub(super) fn view_turn(t: &turns::Model) -> Value {
    json!({"id":Id::from(t.id),"state":t.state,"prompt":t.prompt,"attachments":serde_json::from_str::<Value>(&t.attachments).unwrap_or(json!([])),
        "error":crate::i18n::render_stored(crate::i18n::current_locale(),&t.error),"usage":serde_json::from_str::<Value>(&t.usage).unwrap_or(Value::Null),"decisions":serde_json::from_str::<Value>(&t.decisions).unwrap_or(json!({})),
        "owner_id":Id::from(t.owner_id),"created_at":t.created_at,"updated_at":t.updated_at})
}
async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("conversations.read")?;
    let spec = crate::resources::resource::Spec {
        search: &["title"],
        filters: &["state", "agent_id"],
        sortable: &["title", "created_at", "updated_at"],
        default_sort: "-updated_at",
    };
    let query = crate::resources::resource::ListQuery::parse(&spec, &raw)?;
    let (rows, total) =
        crate::resources::resource::list::<conversations::Entity, conversations::Model>(
            &s.db,
            &spec,
            conversations::Entity::find().filter(groups::visible(&scope)),
            &scope.workspace_id,
            &query,
        )
        .await?;
    let grouped = crate::entities::ai_chat_members::Entity::find()
        .select_only()
        .column(crate::entities::ai_chat_members::Column::ConversationId)
        .filter(
            crate::entities::ai_chat_members::Column::ConversationId
                .is_in(rows.iter().map(|r| r.id)),
        )
        .distinct()
        .into_tuple::<i64>()
        .all(&s.db)
        .await?;
    let counts = interactions::counts(&s.db, &scope, &rows).await?;
    let items = rows
        .iter()
        .map(|r| {
            let mut value = view(r);
            let (unread, mentions) = counts.get(&r.id).copied().unwrap_or_default();
            value["unread"] = json!(unread);
            value["mentions"] = json!(mentions);
            value["kind"] = json!(if grouped.contains(&r.id) {
                "group"
            } else {
                "private"
            });
            value
        })
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
#[derive(Deserialize)]
pub struct Create {
    #[serde(default)]
    host_id: Option<Id>,
    #[serde(default)]
    agent_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    permission: Option<ferry_protocol::agent::Permission>,
    #[serde(default = "private_kind")]
    kind: String,
    #[serde(default)]
    member_ids: Vec<Id>,
    #[serde(default)]
    agent_ids: Vec<Id>,
}
fn private_kind() -> String {
    "private".into()
}
async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.create")?;
    if !["private", "group"].contains(&body.kind.as_str())
        || body.member_ids.len() > 100
        || body.agent_ids.len() > 8
    {
        return Err(invalid());
    }
    // Group participants are peers: the UI submits one list, without requiring
    // users to nominate a primary Agent. Retain the old explicit field for clients.
    let agent_id = if body.agent_id.trim().is_empty() && body.kind == "group" {
        body.agent_ids.first().copied().ok_or_else(invalid)?
    } else {
        Id::reference(&body.agent_id)?
    };
    if i64::from(agent_id) <= 0 {
        return Err(invalid());
    }
    let agent = available(&s.db, &scope, agent_id).await?;
    let title = if body.title.trim().is_empty() {
        agent.name.clone()
    } else {
        body.title.trim().to_owned()
    };
    if title.len() > 256 || title.contains('\0') {
        return Err(invalid());
    }
    let id = s.next_id()?;
    let (config, binding) = prepare_execution(&s, &scope, &agent, &body, id).await?;
    let extra = groups::prepare(&s, &scope, &body, &agent, &config, &binding, id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    if available(&tx, &scope, agent_id).await?.revision_id != agent.revision_id {
        return Err(conflict());
    }
    let now = db::now();
    conversations::Entity::insert(
        conversations::Model {
            id: id.into(),
            workspace_id: scope.workspace_id.into(),
            owner_id: scope.user.id.into(),
            agent_id: agent.id,
            revision_id: agent.revision_id,
            title,
            state: "active".into(),
            config: serde_json::to_string(&config).unwrap(),
            binding: serde_json::to_string(&binding).unwrap(),
            native_session_id: String::new(),
            next_sequence: 0,
            version: 1,
            created_at: now,
            updated_at: now,
        }
        .into_active_model(),
    )
    .exec(&tx)
    .await?;
    groups::insert(&s, &tx, &scope, id, &body, extra).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "conversations",
        &id,
        json!({"agent_id":agent_id}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}

async fn prepare_execution(
    s: &AppState,
    scope: &Scope,
    agent: &ai_agents::Model,
    body: &Create,
    id: Id,
) -> Result<(Configuration, ferry_protocol::agent::Binding)> {
    let resolved = crate::agents::resolve_model(&s.db, agent).await?;
    if resolved.runtime == "api" {
        return Err(error(StatusCode::CONFLICT, "conversations.apiPending"));
    }
    let mut config = Configuration::for_conversation(&resolved, body)?;
    if let Some(host) = body.host_id.filter(|id| i64::from(*id) != 0) {
        scope.require("hosts.update")?;
        config.host_id = host;
    }
    if i64::from(config.host_id) != 0 {
        return bind_execution(s, scope, config, agent.id, id).await;
    }
    use crate::entities::hosts;
    let hosts = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Connection.eq("runner"))
        .filter(hosts::Column::State.eq("online"))
        .order_by_asc(hosts::Column::Id)
        .limit(100)
        .all(&s.db)
        .await?;
    let allocate = async {
        let mut last = error(StatusCode::CONFLICT, "conversations.noRunner");
        for host in hosts {
            let mut candidate = config.clone();
            candidate.host_id = host.id.into();
            match bind_execution(s, scope, candidate, agent.id, id).await {
                Ok(bound) => return Ok(bound),
                Err(e) => last = e,
            }
        }
        Err(last)
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), allocate)
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "conversations.noRunner"))?
}
async fn bind_execution(
    s: &AppState,
    scope: &Scope,
    mut config: Configuration,
    agent: i64,
    id: Id,
) -> Result<(Configuration, ferry_protocol::agent::Binding)> {
    let host = crate::entities::hosts::Entity::find_by_id(config.host_id)
        .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::hosts::Column::Connection.eq("runner"))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    config.runner_key_hash = host.runner_key_hash;
    let channel = crate::hosts::remote::open(s, &scope.workspace_id, &config.host_id).await?;
    let probe = if config.cwd.is_empty() {
        channel
            .agent_workspace(config.runtime, &format!("{id}-{agent}"))
            .await?
    } else {
        channel.agent_probe(config.runtime, &config.cwd).await?
    };
    let canonical = probe
        .canonical_directory
        .filter(|_| probe.directory_exists)
        .ok_or_else(|| error(StatusCode::CONFLICT, "conversations.directoryMissing"))?;
    let executable = probe
        .executable
        .filter(|_| probe.error.is_none())
        .ok_or_else(|| error(StatusCode::CONFLICT, "conversations.cliMissing"))?;
    if !config.authentication_usable(&probe.authentication) {
        return Err(error(StatusCode::CONFLICT, "conversations.loginRequired"));
    }
    if probe.execution_user.is_empty()
        || probe.execution_identity.is_empty()
        || config.runner_key_hash.is_empty()
    {
        return Err(error(StatusCode::CONFLICT, "conversations.bindingChanged"));
    }
    config.cwd = canonical.clone();
    let binding = ferry_protocol::agent::Binding {
        execution_user: probe.execution_user,
        execution_identity: probe.execution_identity,
        canonical_directory: canonical,
        executable,
    };
    Ok((config, binding))
}
async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    let c = find(&s.db, &scope, id).await?;
    let mut value = view(&c);
    value["owner_id"] = json!(Id::from(c.owner_id));
    value["can_manage"] = json!(
        c.owner_id == i64::from(scope.user.id) && scope.require("conversations.update").is_ok()
    );
    value["viewer_id"] = json!(scope.user.id);
    value["turns"] = json!(
        turns::Entity::find()
            .filter(turns::Column::ConversationId.eq(id))
            .order_by_asc(turns::Column::Id)
            .all(&s.db)
            .await?
            .iter()
            .map(view_turn)
            .collect::<Vec<_>>()
    );
    groups::detail(&s.db, &c, &mut value).await?;
    interactions::decorate(&s.db, &scope, &c, &mut value).await?;
    Ok(Json(value))
}
#[derive(Deserialize)]
pub struct Update {
    title: String,
    state: String,
    version: i64,
}
async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Update>,
) -> Result<Json<Value>> {
    scope.require("conversations.update")?;
    if body.title.trim().is_empty()
        || body.title.len() > 256
        || body.title.contains('\0')
        || !["active", "archived", "interrupted"].contains(&body.state.as_str())
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    if c.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    if c.version != body.version {
        return Err(conflict());
    }
    if c.state != "interrupted" && body.state == "interrupted" {
        return Err(conflict());
    }
    if has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    if c.state == "interrupted" && body.state == "active" {
        // Explicit recovery starts a fresh native session. Old attempts and
        // their uncertain effects remain recorded and are never replayed here.
        scope.require("conversations.run")?;
        c.native_session_id.clear();
    }
    c.title = body.title.trim().into();
    c.state = body.state.clone();
    c.version += 1;
    c.updated_at = db::now();
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "conversations",
        &id,
        json!({"state":body.state}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn has_active(conn: &impl ConnectionTrait, id: Id) -> Result<bool> {
    Ok(turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::State.is_in(["queued", "running", "stopping"]))
        .count(conn)
        .await?
        > 0)
}
async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("conversations.update")?;
    let backend = crate::artifact_store::ArtifactStore::for_state(&s).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    if c.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    if has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    files::detach_all(&s, &tx, id, &backend).await?;
    groups::delete(&tx, id).await?;
    interactions::delete(&tx, id).await?;
    events::Entity::delete_many()
        .filter(events::Column::ConversationId.eq(id))
        .exec(&tx)
        .await?;
    turns::Entity::delete_many()
        .filter(turns::Column::ConversationId.eq(id))
        .exec(&tx)
        .await?;
    conversations::Entity::delete_by_id(id).exec(&tx).await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "conversations", &id, json!({})).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    name: String,
    #[serde(default)]
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_id: Option<Id>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    mime: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    size: u64,
}
fn is_zero(n: &u64) -> bool {
    *n == 0
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    prompt: String,
    client_id: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
    #[serde(default)]
    reply_to: Option<Id>,
}
async fn send(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<Send>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    find(&s.db, &scope, id).await?;
    if groups::is_group(&s.db, id).await? {
        return Err(invalid());
    }
    files::canonicalize(&s, &scope, id, &mut body.attachments).await?;
    if (body.prompt.trim().is_empty() && body.attachments.is_empty())
        || body.prompt.contains('\0')
        || body.client_id.is_empty()
        || body.client_id.len() > 128
        || body.client_id.chars().any(char::is_control)
        || body.attachments.len() > 10
        || body.attachments.iter().any(|a| {
            a.name.is_empty()
                || a.name.len() > 255
                || a.name.chars().any(char::is_control)
                || a.text.contains('\0')
        })
        || body.prompt.len()
            + body
                .attachments
                .iter()
                .map(|a| a.text.len() + a.name.len())
                .sum::<usize>()
            > 48 * 1024
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut c = find(&tx, &scope, id).await?;
    available(&tx, &scope, c.agent_id.into()).await?;
    interactions::parent(&tx, id, body.reply_to).await?;
    files::validate_refs(&tx, &scope, id, &body.attachments).await?;
    let config: Configuration = serde_json::from_str(&c.config).map_err(|_| invalid())?;
    if config.system_prompt.len()
        + body.prompt.len()
        + body
            .attachments
            .iter()
            .map(|a| a.text.len() + a.name.len() + 128)
            .sum::<usize>()
        > 63 * 1024
    {
        return Err(invalid());
    }
    if let Some(existing) = turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::ClientId.eq(&body.client_id))
        .one(&tx)
        .await?
    {
        let original = crate::entities::ai_chat_messages::Entity::find_by_id(existing.id)
            .one(&tx)
            .await?;
        if existing.prompt != body.prompt
            || existing.attachments != serde_json::to_string(&body.attachments).unwrap()
            || original.and_then(|m| m.reply_to) != body.reply_to.map(i64::from)
        {
            return Err(conflict());
        }
        return Ok((StatusCode::OK, Json(json!({"id":Id::from(existing.id)}))));
    }
    if c.state != "active" {
        return Err(error(StatusCode::CONFLICT, "conversations.archived"));
    }
    if has_active(&tx, id).await? {
        return Err(error(StatusCode::CONFLICT, "conversations.busy"));
    }
    let turn = s.next_id()?;
    let now = db::now();
    let row = turns::Model {
        id: turn.into(),
        workspace_id: scope.workspace_id.into(),
        conversation_id: id.into(),
        owner_id: scope.user.id.into(),
        state: "queued".into(),
        lease_owner: String::new(),
        expires_at: 0,
        version: 1,
        client_id: body.client_id,
        prompt: body.prompt,
        attachments: serde_json::to_string(&body.attachments).unwrap(),
        native_session_id: c.native_session_id.clone(),
        candidate_session: String::new(),
        error: String::new(),
        usage: "null".into(),
        decisions: "{}".into(),
        created_at: now,
        updated_at: now,
    };
    messages::insert_private(&tx, &row).await?;
    if let Some(parent) = body.reply_to {
        use crate::entities::ai_chat_messages as chat;
        chat::Entity::update_many()
            .col_expr(
                chat::Column::ReplyTo,
                sea_orm::sea_query::Expr::value(i64::from(parent)),
            )
            .filter(chat::Column::Id.eq(turn))
            .exec(&tx)
            .await?;
    }
    turns::Entity::insert(row.into_active_model())
        .exec(&tx)
        .await?;
    c.updated_at = now;
    c.version += 1;
    c.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "run",
        "conversations",
        &id,
        json!({"turn_id":turn}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"id":turn}))))
}
#[derive(Deserialize)]
pub struct Cursor {
    #[serde(default)]
    after: i64,
}
async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<Value>> {
    find(&s.db, &scope, id).await?;
    if cursor.after < 0 {
        return Err(invalid());
    }
    let rows = events::Entity::find()
        .filter(events::Column::ConversationId.eq(id))
        .filter(events::Column::Sequence.gt(cursor.after))
        .order_by_asc(events::Column::Sequence)
        .limit(200)
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"items":rows.iter().map(|r|json!({"sequence":r.sequence,"turn_id":Id::from(r.turn_id),"event":serde_json::from_str::<Value>(&r.payload).unwrap_or(Value::Null),"created_at":r.created_at})).collect::<Vec<_>>(),
        "next":rows.last().map_or(cursor.after,|r|r.sequence),"has_more":rows.len()==200}),
    ))
}
async fn find_turn(
    conn: &impl ConnectionTrait,
    c: &conversations::Model,
    id: Id,
) -> Result<turns::Model> {
    turns::Entity::find_by_id(id)
        .filter(turns::Column::ConversationId.eq(c.id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn cancel(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, turn)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    let mut row = find_turn(&tx, &c, turn).await?;
    if row.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    if active(&row.state) {
        row.state = if row.state == "queued" {
            "cancelled"
        } else {
            "stopping"
        }
        .into();
        row.version += 1;
        row.updated_at = db::now();
        row.into_active_model().reset_all().update(&tx).await?;
        crate::hosts::activity_in(
            &s,
            &tx,
            &scope,
            "cancel",
            "conversations",
            &id,
            json!({"turn_id":turn}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    request_id: String,
    allow: bool,
}
async fn decide(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, turn)): Path<(String, Id, Id)>,
    Json(body): Json<Decision>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    let mut row = find_turn(&tx, &c, turn).await?;
    if row.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    let execution = groups::execution(&tx, &row, c).await?;
    usable(&tx, &scope, execution.agent_id.into()).await?;
    if row.state != "running" || row.expires_at <= db::now() {
        return Err(error(StatusCode::CONFLICT, "conversations.staleDecision"));
    }
    let mut decisions: Value = serde_json::from_str(&row.decisions).map_err(|_| invalid())?;
    let item = decisions
        .get_mut(&body.request_id)
        .ok_or_else(|| error(StatusCode::CONFLICT, "conversations.staleDecision"))?;
    if !item["decision"].is_null() {
        if item["decision"] == json!(body.allow) {
            return Ok(Json(json!({"ok":true})));
        }
        return Err(error(StatusCode::CONFLICT, "conversations.staleDecision"));
    }
    item["decision"] = json!(body.allow);
    row.decisions = decisions.to_string();
    row.version += 1;
    row.into_active_model().reset_all().update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "decision",
        "conversations",
        &id,
        json!({"turn_id":turn,"request_id":body.request_id,"allow":body.allow}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
