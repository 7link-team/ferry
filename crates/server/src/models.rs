//! Reusable API/CLI profiles. Credentials and provider policy remain in the existing provider registry.
#[cfg(test)]
mod tests;
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{ai_model_profiles as profiles, ai_providers as providers, credentials},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/ai-models", get(list).post(create))
        .route("/{workspace}/ai-models/model-discoveries", post(discover))
        .route("/{workspace}/ai-models/cli-checks", post(check_cli))
        .route("/{workspace}/ai-models/{id}", get(detail).patch(update))
}
const SPEC: Spec = Spec {
    search: &["name", "default_model"],
    filters: &["kind", "runtime", "state"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-updated_at",
};
/// One specific, actionable validation message.
fn bad(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new(key))
}
/// A save-time connection check failed; the client may retry with `skip_check`.
fn check_failed(cause: ApiError) -> ApiError {
    let reason = cause
        .2
        .clone()
        .unwrap_or_else(|| Message::new("models.discoveryFailed"));
    ApiError::localized(
        StatusCode::FAILED_DEPENDENCY,
        Message::new("models.connectionCheckFailed").with_message("reason", reason),
    )
}
fn changed() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("resource.conflict"))
}
#[derive(Clone, Deserialize, Serialize)]
struct Body {
    name: String,
    kind: String,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    provider_id: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    default_model: String,
    /// Optional API provider output parameter; absent leaves upstream defaults.
    #[serde(default)]
    max_output_tokens: Option<i64>,
    #[serde(default)]
    tool_version: String,
    state: String,
    #[serde(default)]
    version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connection: Option<crate::workflows::ai::Body>,
    /// CLI-only: upstream format and per-tool options. Null for native CLI configuration.
    #[serde(default)]
    cli_options: Value,
    /// CLI-only IANA zone its executions run in; empty follows the runner.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    timezone: String,
    /// CLI-only: Runner hosts to use, first preferred; empty = any ready Runner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hosts: Vec<Id>,
    /// Input only: save an API connection whose model list could not be loaded.
    #[serde(default, skip_serializing)]
    skip_check: bool,
}
/// Connections an Agent's CLI can run on (tests also use in-memory hosts).
pub(crate) fn runner_connections() -> &'static [&'static str] {
    if cfg!(test) {
        &["runner", "fake"]
    } else {
        &["runner"]
    }
}
/// The format a profile saved before formats existed implies: the connection's
/// protocol, narrowed to what this CLI can call (Codex only speaks Responses).
fn legacy_format(runtime: &str, protocol: &str) -> &'static str {
    let formats = ferry_protocol::agent::Runtime::parse(runtime)
        .map(|r| r.provider_formats())
        .unwrap_or_default();
    let wanted: &[&'static str] = if protocol == "anthropic" {
        &["anthropic"]
    } else {
        &["openai_chat", "openai_responses"]
    };
    wanted
        .iter()
        .copied()
        .find(|f| formats.contains(f))
        .unwrap_or(wanted[0])
}
/// Bounded, flat options: strings, numbers, booleans and a string-valued `headers` map.
fn cli_options(runtime: &str, provider: bool, options: &mut Value) -> Result<()> {
    if options.is_null() || !provider {
        *options = Value::Null;
        return Ok(());
    }
    let runtime = ferry_protocol::agent::Runtime::parse(runtime)
        .ok_or_else(|| bad("models.cliToolUnknown"))?;
    let map = options
        .as_object()
        .ok_or_else(|| bad("models.cliOptionsInvalid"))?;
    for (key, value) in map {
        let ok = key.len() <= 64
            && match value {
                Value::String(v) => v.len() <= 256 && !v.chars().any(char::is_control),
                Value::Bool(_) => true,
                Value::Number(n) => n.as_u64().is_some_and(|n| n <= 100_000_000),
                Value::Object(h) if key == "headers" => {
                    h.len() <= 32
                        && h.values().all(|v| {
                            v.as_str().is_some_and(|v| {
                                v.len() <= 4096 && !v.chars().any(char::is_control)
                            })
                        })
                }
                _ => false,
            };
        if !ok {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.cliOptionInvalid")
                    .with("option", key.chars().take(64).collect::<String>()),
            ));
        }
    }
    let format = map
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !runtime.provider_formats().contains(&format) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("models.cliFormatUnsupported"),
        ));
    }
    Ok(())
}
async fn provider(conn: &impl ConnectionTrait, workspace: Id, id: Id) -> Result<providers::Model> {
    providers::Entity::find_by_id(id)
        .filter(providers::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
impl Body {
    /// Checks needing no database or network, run before anything slower.
    fn check_fields(&mut self) -> Result<()> {
        if self
            .max_output_tokens
            .is_some_and(|value| !(1..=9_007_199_254_740_991).contains(&value))
        {
            return Err(bad("models.maxOutputInvalid"));
        }
        if self.kind != "api" && self.max_output_tokens.is_some() {
            return Err(bad("models.maxOutputApiOnly"));
        }
        self.name = self.name.trim().into();
        self.default_model = self.default_model.trim().into();
        self.tool_version = self.tool_version.trim().into();
        let mut seen = HashSet::new();
        self.models = self
            .models
            .iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty() && seen.insert(s.clone()))
            .collect();
        if !self.default_model.is_empty() && !self.models.contains(&self.default_model) {
            self.models.push(self.default_model.clone());
        }
        if self.kind == "api" && self.default_model.is_empty() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.defaultModelRequired"),
            ));
        }
        self.timezone = crate::hosts::runner::check_timezone(&self.timezone)?;
        if self.kind == "api" && !self.timezone.is_empty() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.timezoneCliOnly"),
            ));
        }
        let mut seen = HashSet::new();
        self.hosts
            .retain(|host| i64::from(*host) > 0 && seen.insert(*host));
        if self.kind == "api" && !self.hosts.is_empty() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.hostsCliOnly"),
            ));
        }
        if self.name.is_empty() {
            return Err(bad("models.nameRequired"));
        }
        if self.name.chars().count() > 128 || self.name.chars().any(char::is_control) {
            return Err(bad("models.nameInvalid"));
        }
        if !["api", "cli"].contains(&self.kind.as_str()) {
            return Err(bad("models.kindInvalid"));
        }
        if !["enabled", "disabled"].contains(&self.state.as_str()) {
            return Err(bad("models.stateInvalid"));
        }
        if let Some(model) = self
            .models
            .iter()
            .chain(std::iter::once(&self.default_model))
            .find(|s| s.len() > 256 || s.starts_with('-') || s.chars().any(char::is_control))
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.modelIdInvalid")
                    .with("model", model.chars().take(64).collect::<String>()),
            ));
        }
        if self.kind == "api" && (!self.runtime.is_empty() || !self.tool_version.is_empty()) {
            return Err(bad("models.apiCliFields"));
        }
        if self.kind == "cli" && ferry_protocol::agent::Runtime::parse(&self.runtime).is_none() {
            return Err(bad("models.cliToolUnknown"));
        }
        if self.tool_version.len() > 64
            || !self
                .tool_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
        {
            return Err(bad("models.toolVersionInvalid"));
        }
        Ok(())
    }
    async fn validate(&mut self, conn: &impl ConnectionTrait, scope: &Scope) -> Result<Id> {
        self.check_fields()?;
        for host in &self.hosts {
            let found = crate::entities::hosts::Entity::find_by_id(*host)
                .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(
                    crate::entities::hosts::Column::Connection
                        .is_in(runner_connections().iter().copied()),
                )
                .count(conn)
                .await?;
            if found == 0 {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("models.hostInvalid"),
                ));
            }
        }
        let id = Id::reference(&self.provider_id)?;
        if i64::from(id) != 0 {
            scope.require("credentials.read")?;
            provider(conn, scope.workspace_id, id).await?;
        } else if self.kind == "api" {
            return Err(bad("models.connectionRequired"));
        }
        if self.kind == "cli"
            && i64::from(id) != 0
            && ferry_protocol::agent::Runtime::parse(&self.runtime)
                .is_none_or(|r| r.provider_formats().is_empty())
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.cliFormatUnsupported"),
            ));
        }
        if self.kind == "cli" && i64::from(id) != 0 && self.cli_options.is_null() {
            // Profiles saved before formats existed: the connection protocol decides.
            let protocol = provider(conn, scope.workspace_id, id).await?.protocol;
            self.cli_options = json!({ "format": legacy_format(&self.runtime, &protocol) });
        }
        let runtime = self.runtime.clone();
        cli_options(
            &runtime,
            self.kind == "cli" && i64::from(id) != 0,
            &mut self.cli_options,
        )?;
        if self.kind == "cli" && i64::from(id) != 0 {
            // A referenced connection is never converted: its protocol must be the format.
            let protocol = provider(conn, scope.workspace_id, id).await?.protocol;
            let anthropic = self.cli_options["format"] == "anthropic";
            if anthropic != (protocol == "anthropic") {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("models.cliFormatUnsupported"),
                ));
            }
        }
        self.provider_id = id.to_string();
        Ok(id)
    }
}
fn view(p: &profiles::Model) -> Value {
    json!({"cli_options":p.cli_options.as_deref().and_then(|v|serde_json::from_str::<Value>(v).ok()).unwrap_or(Value::Null),"id":Id::from(p.id),"name":p.name,"kind":p.kind,"runtime":p.runtime,"provider_id":Id::from(p.provider_id),"models":serde_json::from_str::<Value>(&p.models).unwrap_or(json!([])),"default_model":p.default_model,"max_output_tokens":p.max_output_tokens,"tool_version":p.tool_version,"timezone":p.timezone,"hosts":host_ids(p).into_iter().map(Id::from).collect::<Vec<_>>(),"state":p.state,"revision_id":Id::from(p.revision_id),"version":p.version,"created_at":p.created_at,"updated_at":p.updated_at})
}
async fn find(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<profiles::Model> {
    scope.require("ai-models.read")?;
    profiles::Entity::find_by_id(id)
        .filter(profiles::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("ai-models.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<profiles::Entity, profiles::Model>(
        &s.db,
        &SPEC,
        profiles::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.iter().map(view).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    let row = find(&s.db, &scope, id).await?;
    let mut value = view(&row);
    if row.provider_id != 0 && scope.permissions.contains("credentials.read") {
        value["connection"] = crate::workflows::ai::view(
            &provider(&s.db, scope.workspace_id, Id::from(row.provider_id)).await?,
        );
        value["connection"].as_object_mut().unwrap().remove("id");
    }
    Ok(Json(value))
}
async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    Ok((
        StatusCode::CREATED,
        Json(save(&s, &scope, None, body).await?),
    ))
}
async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    Ok(Json(save(&s, &scope, Some(id), body).await?))
}
async fn save(s: &AppState, scope: &Scope, id: Option<Id>, mut body: Body) -> Result<Value> {
    let permission = if id.is_some() {
        "ai-models.update"
    } else {
        "ai-models.create"
    };
    scope.require(permission)?;
    body.check_fields()?;
    if body.kind == "api" && !body.skip_check {
        check_connection(s, scope, &body).await?;
    }
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    let current = crate::accounts::workspace_admin::authorize(&tx, scope, permission).await?;
    let old = if let Some(id) = id {
        Some(find(&tx, &current, id).await?)
    } else {
        None
    };
    if old.as_ref().is_some_and(|v| v.version != body.version) {
        return Err(changed());
    }
    if let Some(mut connection) = body.connection.take() {
        connection.name = body.name.clone();
        let mut provider_id = Id::reference(&body.provider_id)?;
        if body.kind == "cli" {
            // A CLI keeps its own connection. One still shared with another model
            // becomes independent instead of silently changing that model.
            connection.protocol = if body.cli_options["format"] == "anthropic" {
                "anthropic".into()
            } else {
                "openai".into()
            };
            if i64::from(provider_id) != 0
                && profiles::Entity::find()
                    .filter(profiles::Column::WorkspaceId.eq(scope.workspace_id))
                    .filter(profiles::Column::ProviderId.eq(i64::from(provider_id)))
                    .all(&tx)
                    .await?
                    .iter()
                    .any(|p| Some(Id::from(p.id)) != id)
            {
                if connection
                    .api_key
                    .as_deref()
                    .is_none_or(|k| k.trim().is_empty())
                {
                    return Err(ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("models.sharedConnectionNeedsKey"),
                    ));
                }
                connection.credential_id = Id::from(0);
                provider_id = Id::from(0);
            }
        }
        let saved = crate::workflows::ai::save_in(
            s,
            &current,
            &tx,
            (i64::from(provider_id) != 0).then_some(provider_id),
            connection,
        )
        .await?;
        body.provider_id = saved.id.to_string();
    }
    let provider_id = body.validate(&tx, &current).await?;
    let id = match id {
        Some(id) => id,
        None => s.next_id()?,
    };
    let now = db::now();
    let mut definition = serde_json::to_value(&body).map_err(crate::error::internal_fault)?;
    definition.as_object_mut().unwrap().remove("version");
    let revision = crate::definitions::record(
        s,
        &tx,
        scope.workspace_id,
        "model",
        id,
        &definition.to_string(),
        scope.user.id,
        None,
    )
    .await?;
    let row = profiles::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        name: body.name,
        kind: body.kind,
        runtime: body.runtime,
        provider_id: provider_id.into(),
        models: serde_json::to_string(&body.models).map_err(crate::error::internal_fault)?,
        default_model: body.default_model,
        max_output_tokens: body.max_output_tokens,
        tool_version: body.tool_version,
        state: body.state,
        cli_options: (!body.cli_options.is_null()).then(|| body.cli_options.to_string()),
        timezone: body.timezone,
        hosts: serde_json::to_string(&body.hosts).map_err(crate::error::internal_fault)?,
        revision_id: revision.id,
        version: old
            .as_ref()
            .map_or(Some(1), |o| o.version.checked_add(1))
            .ok_or_else(changed)?,
        created_at: old.as_ref().map_or(now, |v| v.created_at),
        updated_at: now,
    };
    if old.is_some() {
        row.clone()
            .into_active_model()
            .reset_all()
            .update(&tx)
            .await?;
    } else {
        profiles::Entity::insert(row.clone().into_active_model())
            .exec(&tx)
            .await?;
    }
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        if old.is_some() { "update" } else { "create" },
        "ai-models",
        &id,
        json!({"revision_id":Id::from(revision.id)}),
    )
    .await?;
    tx.commit().await?;
    Ok(view(&row))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Discovery {
    #[serde(default)]
    provider_id: Id,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    host_id: Id,
    #[serde(default)]
    connection: Option<crate::workflows::ai::Body>,
}
async fn discover(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Discovery>,
) -> Result<Json<Value>> {
    if !scope.permissions.contains("ai-models.create") {
        scope.require("ai-models.update")?;
    }
    if !body.runtime.is_empty() {
        scope.require("hosts.read")?;
        scope.require("ai-agents.probe")?;
        let runtime = ferry_protocol::agent::Runtime::parse(&body.runtime)
            .ok_or_else(|| bad("models.cliToolUnknown"))?;
        let host = crate::entities::hosts::Entity::find_by_id(body.host_id)
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(crate::entities::hosts::Column::Connection.eq("runner"))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        let channel = crate::hosts::remote::open(&s, &scope.workspace_id, &body.host_id).await?;
        let status = channel.agent_probe(runtime, "", None).await?;
        crate::accounts::workspace_admin::authorize(&s.db, &scope, "ai-agents.probe").await?;
        // The selected host cannot be silently replaced by another Runner.
        if crate::entities::hosts::Entity::find_by_id(host.id)
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .is_none()
        {
            return Err(ApiError::missing());
        }
        if status.models.is_empty() {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("models.nativeCatalogUnavailable"),
            ));
        }
        crate::hosts::activity(
            &s,
            &scope,
            "discover",
            "ai-models",
            &body.provider_id,
            json!({"host_id":body.host_id,"runtime":body.runtime}),
        )
        .await?;
        return Ok(Json(
            json!({"models":status.models,"has_more":false,"source":"native_catalog","host_id":body.host_id}),
        ));
    }
    scope.require("credentials.use")?;
    scope.require("credentials.read")?;
    let stored = if body.connection.is_none() {
        Some(provider(&s.db, scope.workspace_id, body.provider_id).await?)
    } else {
        None
    };
    let (credential_id, base_url, protocol) = if let Some(ref connection) = body.connection {
        (
            i64::from(connection.credential_id),
            connection.base_url.as_str(),
            connection.protocol.as_str(),
        )
    } else {
        let p = stored.as_ref().unwrap();
        (p.credential_id, p.base_url.as_str(), p.protocol.as_str())
    };
    if !matches!(protocol, "openai" | "anthropic") {
        return Err(bad("models.protocolInvalid"));
    }
    let supplied_key = body
        .connection
        .as_ref()
        .and_then(|c| c.api_key.as_deref())
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let secret = if let Some(key) = supplied_key {
        if key.len() > 16_384 || key.chars().any(char::is_control) {
            return Err(bad("models.apiKeyInvalid"));
        }
        key.to_owned()
    } else {
        let c = credentials::Entity::find_by_id(credential_id)
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if c.scope != "ai" || c.kind != "token" {
            return Err(bad("models.credentialWrongKind"));
        }
        let (_, _, secret) =
            crate::credentials::unseal(&s, &scope.workspace_id, &Id::from(credential_id)).await?;
        secret
    };
    crate::hosts::activity(
        &s,
        &scope,
        "discover",
        "ai-models",
        &body.provider_id,
        json!({"provider_id":body.provider_id}),
    )
    .await?;
    let (models, has_more) = fetch_models(
        base_url,
        protocol,
        &secret,
        &crate::accounts::resource_policy::load(&s.db).await?,
    )
    .await?;
    let current = Scope::for_actor(&s, scope.workspace_id, scope.user.id).await?;
    if !current.permissions.contains("ai-models.create") {
        current.require("ai-models.update")?;
    }
    current.require("credentials.use")?;
    current.require("credentials.read")?;
    if let Some(p) = stored {
        if provider(&s.db, scope.workspace_id, body.provider_id)
            .await?
            .version
            != p.version
        {
            return Err(changed());
        }
    }
    if supplied_key.is_none() {
        let c = credentials::Entity::find_by_id(credential_id)
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if c.scope != "ai" || c.kind != "token" {
            return Err(bad("models.credentialWrongKind"));
        }
    }
    Ok(Json(json!({"models":models,"has_more":has_more})))
}

/// A profile's stored preferred hosts, in order.
fn host_ids(p: &profiles::Model) -> Vec<i64> {
    serde_json::from_str::<Vec<Id>>(&p.hosts)
        .unwrap_or_default()
        .into_iter()
        .map(i64::from)
        .collect()
}

/// The Runner hosts an Agent's current CLI model prefers, first preferred;
/// empty means any Runner that has the CLI ready.
pub(crate) async fn cli_hosts(
    conn: &impl ConnectionTrait,
    workspace: Id,
    agent: &crate::entities::ai_agents::Model,
) -> Result<Vec<i64>> {
    Ok(profiles::Entity::find_by_id(agent.model_profile_id)
        .filter(profiles::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .filter(|profile| profile.kind == "cli")
        .map(|profile| host_ids(&profile))
        .unwrap_or_default())
}

/// A confirmed workspace zone controls CLI executions; unconfirmed workspaces
/// retain their model/runner settings. A required zone refuses runners that
/// cannot apply it rather than silently running in another zone.
pub(crate) async fn cli_timezone(
    s: &AppState,
    workspace: Id,
    agent_id: i64,
    host: &crate::entities::hosts::Model,
) -> Result<Option<String>> {
    let zone = crate::accounts::workspace::business_timezone(&s.db, workspace).await?;
    cli_timezone_at(
        s,
        workspace,
        agent_id,
        host,
        zone.as_ref().map(|zone| zone.name()),
    )
    .await
}

/// Workflow executions supply their frozen workspace zone, including None for legacy runs.
pub(crate) async fn cli_timezone_at(
    s: &AppState,
    workspace: Id,
    agent_id: i64,
    host: &crate::entities::hosts::Model,
    workspace_timezone: Option<&str>,
) -> Result<Option<String>> {
    if let Some(zone) = workspace_timezone {
        return timezone_for(zone.to_owned(), host);
    }
    let Some(agent) = crate::entities::ai_agents::Entity::find_by_id(agent_id)
        .filter(crate::entities::ai_agents::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
    else {
        return Ok(None);
    };
    let zone = profiles::Entity::find_by_id(agent.model_profile_id)
        .filter(profiles::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
        .map(|profile| profile.timezone)
        .unwrap_or_default();
    timezone_for(zone, host)
}

fn timezone_for(zone: String, host: &crate::entities::hosts::Model) -> Result<Option<String>> {
    if zone.is_empty() {
        return Ok(None);
    }
    if host.timezone_supported == 0 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("runner.timezoneUpgrade").with("timezone", zone),
        ));
    }
    Ok(Some(zone))
}

/// Some Windows tools read `TZ` as a POSIX rule or ignore it, so a zone that
/// differs from a Windows host's own is applied but worth a warning.
pub(crate) fn windows_timezone_caveat(zone: &str, host: &crate::entities::hosts::Model) -> bool {
    host.os.to_ascii_lowercase().contains("windows") && host.detected_timezone != zone
}

/// The custom upstream for a CLI execution, or None when the Agent's CLI model
/// deliberately keeps the CLI's own login/configuration. An Agent without an
/// enabled, matching CLI model is refused. Resolved from its current profile.
pub(crate) async fn cli_provider(
    s: &AppState,
    workspace: Id,
    agent_id: i64,
    runtime: ferry_protocol::agent::Runtime,
) -> Result<Option<ferry_protocol::agent::Provider>> {
    let Some(agent) = crate::entities::ai_agents::Entity::find_by_id(agent_id)
        .filter(crate::entities::ai_agents::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
    else {
        return Ok(None);
    };
    // Re-checked on every run: a conversation created earlier must not keep
    // running once its Agent has no usable model.
    let Some(profile) = profiles::Entity::find_by_id(agent.model_profile_id)
        .filter(profiles::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
    else {
        return Err(crate::agents::model_required());
    };
    if profile.state != "enabled"
        || profile.kind != "cli"
        || ferry_protocol::agent::Runtime::parse(&profile.runtime) != Some(runtime)
    {
        return Err(crate::agents::model_required());
    }
    // A CLI model without a custom API uses that CLI's own configuration on the host.
    if profile.provider_id == 0 {
        return Ok(None);
    }
    let connection = provider(&s.db, workspace, Id::from(profile.provider_id)).await?;
    let options: Value = profile
        .cli_options
        .as_deref()
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_else(
            || json!({"format": legacy_format(&profile.runtime, &connection.protocol)}),
        );
    let (_, _, api_key) =
        crate::credentials::unseal(s, &workspace, &Id::from(connection.credential_id)).await?;
    Ok(Some(ferry_protocol::agent::Provider {
        key: profile.id.to_string(),
        format: options["format"].as_str().unwrap_or_default().to_owned(),
        base_url: connection.base_url,
        api_key,
        models: serde_json::from_str(&profile.models).unwrap_or_default(),
        options,
    }))
}

/// Loads a connection's model list; every failure names its cause.
pub(crate) async fn fetch_models(
    base_url: &str,
    protocol: &str,
    secret: &str,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<(Vec<String>, bool)> {
    if base_url.len() > 512 {
        return Err(bad("models.baseUrlInvalid"));
    }
    let unavailable = || {
        ApiError::localized(
            StatusCode::BAD_GATEWAY,
            Message::new("models.discoveryFailed"),
        )
    };
    let client = crate::accounts::resource_policy::http(policy)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable())?;
    let url = reqwest::Url::parse(base_url).map_err(|_| bad("models.baseUrlInvalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(bad("models.baseUrlInvalid"));
    }
    let request = client.get(format!("{}/models", base_url.trim_end_matches('/')));
    let request = if protocol == "anthropic" {
        request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(secret)
    };
    let host = crate::upstream::host(base_url);
    let mut response = request.send().await.map_err(|e| {
        crate::upstream::unreachable("model-list", &host, &e).unwrap_or_else(unavailable)
    })?;
    if !response.status().is_success() {
        return Err(crate::upstream::status(
            "model-list",
            &host,
            response.status().as_u16(),
            true,
        ));
    }
    let format = || {
        tracing::warn!(host, "model list response is not a model catalog");
        ApiError::localized(
            StatusCode::BAD_GATEWAY,
            Message::new("models.catalogFormat").with("host", host.clone()),
        )
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if !ferry_protocol::resources::allows(
            policy.tool_response_bytes,
            bytes.len().saturating_add(chunk.len()),
        ) {
            return Err(format());
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| format())?;
    let rows = data["data"].as_array().ok_or_else(format)?;
    let mut models = rows
        .iter()
        .filter_map(|v| v["id"].as_str())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 256
                && !s.starts_with('-')
                && !s.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err(ApiError::localized(
            StatusCode::BAD_GATEWAY,
            Message::new("models.catalogEmpty").with("host", host.clone()),
        ));
    }
    Ok((models, data["has_more"].as_bool().unwrap_or(false)))
}

/// An API model's connection is proven when it is saved: its model list must load.
/// Only a new or changed address, protocol or key is checked.
async fn check_connection(s: &AppState, scope: &Scope, body: &Body) -> Result<()> {
    let Some(connection) = &body.connection else {
        return Ok(());
    };
    let provider_id = Id::reference(&body.provider_id)?;
    let stored = if i64::from(provider_id) != 0 {
        Some(provider(&s.db, scope.workspace_id, provider_id).await?)
    } else {
        None
    };
    let supplied = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    let unchanged = stored
        .as_ref()
        .is_some_and(|p| p.base_url == connection.base_url && p.protocol == connection.protocol)
        && supplied.is_none();
    if unchanged {
        return Ok(());
    }
    let secret = match (supplied, &stored) {
        (Some(key), _) => key.to_owned(),
        (None, Some(p)) if p.credential_id != 0 => {
            crate::credentials::unseal(s, &scope.workspace_id, &Id::from(p.credential_id))
                .await?
                .2
        }
        // No key to check with: saving reports the missing key itself.
        _ => return Ok(()),
    };
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let work = fetch_models(&connection.base_url, &connection.protocol, &secret, &policy);
    let result = match policy.task_timeout_seconds {
        Some(seconds) => tokio::time::timeout(std::time::Duration::from_secs(seconds), work).await,
        None => Ok(work.await),
    };
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(cause)) => Err(check_failed(cause)),
        Err(_) => Err(check_failed(ApiError::localized(
            StatusCode::GATEWAY_TIMEOUT,
            Message::new("models.checkTimeout")
                .with("host", crate::upstream::host(&connection.base_url)),
        ))),
    }
}

#[derive(Deserialize)]
struct CliCheck {
    runtime: String,
    /// The model's execution hosts; empty checks every online Runner.
    #[serde(default)]
    hosts: Vec<Id>,
}
/// Checks, without saving anything, whether each execution host can run the
/// CLI: where it was found and for which user, or why it was not.
async fn check_cli(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<CliCheck>,
) -> Result<Json<Value>> {
    if !scope.permissions.contains("ai-models.create") {
        scope.require("ai-models.update")?;
    }
    scope.require("hosts.read")?;
    scope.require("ai-agents.probe")?;
    let runtime = ferry_protocol::agent::Runtime::parse(&body.runtime)
        .ok_or_else(|| bad("models.cliToolUnknown"))?;
    let runners = || {
        crate::entities::hosts::Entity::find()
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(
                crate::entities::hosts::Column::Connection
                    .is_in(runner_connections().iter().copied()),
            )
    };
    let hosts = if body.hosts.is_empty() {
        runners()
            .filter(crate::entities::hosts::Column::State.eq("online"))
            .order_by_asc(crate::entities::hosts::Column::Id)
            .all(&s.db)
            .await?
    } else {
        let mut rows = vec![];
        for id in &body.hosts {
            rows.extend(
                runners()
                    .filter(crate::entities::hosts::Column::Id.eq(*id))
                    .one(&s.db)
                    .await?,
            );
        }
        rows
    };
    let cli = crate::agent_hosts::cli_name(runtime);
    let locale = crate::i18n::current_locale();
    let checks = hosts.into_iter().map(|host| {
        let s = s.clone();
        let cli = cli.clone();
        let workspace = scope.workspace_id;
        async move {
            let id = Id::from(host.id);
            let probe = async {
                let channel = crate::hosts::remote::open(&s, &workspace, &id).await?;
                channel.agent_probe(runtime, "", None).await
            };
            let result = tokio::time::timeout(std::time::Duration::from_secs(20), probe).await;
            let reason = |message: &Message| crate::i18n::render(locale, message);
            match result {
                Ok(Ok(probe)) if probe.executable.is_some() && probe.error.is_none() => json!({
                    "host_id": id, "host_name": host.name, "ok": true,
                    "executable": probe.executable, "user": probe.execution_user,
                    "version": probe.version, "authentication": probe.authentication,
                }),
                Ok(Ok(probe)) => json!({
                    "host_id": id, "host_name": host.name, "ok": false,
                    "user": probe.execution_user,
                    "reason": reason(&crate::agent_hosts::cli_problem(
                        probe.error.as_deref().unwrap_or_default(),
                        &cli,
                        &probe.execution_user,
                    )),
                }),
                Ok(Err(error)) => json!({
                    "host_id": id, "host_name": host.name, "ok": false,
                    "reason": reason(&crate::agent_hosts::reason_from(&error)),
                }),
                Err(_) => json!({
                    "host_id": id, "host_name": host.name, "ok": false,
                    "reason": reason(&Message::new("models.checkHostTimeout")),
                }),
            }
        }
    });
    let results = futures::future::join_all(checks).await;
    crate::hosts::activity(
        &s,
        &scope,
        "check",
        "ai-models",
        &Id::from(0),
        json!({"runtime": body.runtime, "hosts": results.len()}),
    )
    .await?;
    Ok(Json(json!({ "items": results })))
}
