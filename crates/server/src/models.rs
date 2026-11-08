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
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/ai-models", get(list).post(create))
        .route("/{workspace}/ai-models/model-discoveries", post(discover))
        .route("/{workspace}/ai-models/{id}", get(detail).patch(update))
}
const SPEC: Spec = Spec {
    search: &["name", "default_model"],
    filters: &["kind", "runtime", "state"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-updated_at",
};
fn invalid() -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new("models.invalid"))
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
    #[serde(default)]
    tool_version: String,
    state: String,
    #[serde(default)]
    version: i64,
}
async fn provider(conn: &impl ConnectionTrait, workspace: Id, id: Id) -> Result<providers::Model> {
    providers::Entity::find_by_id(id)
        .filter(providers::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
impl Body {
    async fn validate(&mut self, conn: &impl ConnectionTrait, scope: &Scope) -> Result<Id> {
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
        if self.name.is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || !["api", "cli"].contains(&self.kind.as_str())
            || !["enabled", "disabled"].contains(&self.state.as_str())
            || self.models.len() > 2000
            || self
                .models
                .iter()
                .chain(std::iter::once(&self.default_model))
                .any(|s| s.len() > 256 || s.starts_with('-') || s.chars().any(char::is_control))
            || (self.kind == "api"
                && (!self.runtime.is_empty()
                    || !self.tool_version.is_empty()
                    || self.default_model.is_empty()))
            || (self.kind == "cli" && !["codex", "claude"].contains(&self.runtime.as_str()))
            || self.tool_version.len() > 64
            || !self
                .tool_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
        {
            return Err(invalid());
        }
        let id = Id::reference(&self.provider_id)?;
        if i64::from(id) != 0 {
            scope.require("credentials.read")?;
            provider(conn, scope.workspace_id, id).await?;
        } else if self.kind == "api" {
            return Err(invalid());
        }
        self.provider_id = id.to_string();
        Ok(id)
    }
}
fn view(p: &profiles::Model) -> Value {
    json!({"id":Id::from(p.id),"name":p.name,"kind":p.kind,"runtime":p.runtime,"provider_id":Id::from(p.provider_id),"models":serde_json::from_str::<Value>(&p.models).unwrap_or(json!([])),"default_model":p.default_model,"tool_version":p.tool_version,"state":p.state,"revision_id":Id::from(p.revision_id),"version":p.version,"created_at":p.created_at,"updated_at":p.updated_at})
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
    Ok(Json(view(&find(&s.db, &scope, id).await?)))
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
    let provider_id = body.validate(&tx, &current).await?;
    let id = match id {
        Some(id) => id,
        None => s.next_id()?,
    };
    let now = db::now();
    let mut definition = serde_json::to_value(&body).map_err(|_| invalid())?;
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
        models: serde_json::to_string(&body.models).map_err(|_| invalid())?,
        default_model: body.default_model,
        tool_version: body.tool_version,
        state: body.state,
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
    provider_id: Id,
}
async fn discover(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Discovery>,
) -> Result<Json<Value>> {
    if !scope.permissions.contains("ai-models.create") {
        scope.require("ai-models.update")?;
    }
    scope.require("credentials.use")?;
    scope.require("credentials.read")?;
    let p = provider(&s.db, scope.workspace_id, body.provider_id).await?;
    let c = credentials::Entity::find_by_id(p.credential_id)
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if c.scope != "ai" || c.kind != "token" {
        return Err(invalid());
    }
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &Id::from(p.credential_id)).await?;
    crate::hosts::activity(
        &s,
        &scope,
        "discover",
        "ai-models",
        &body.provider_id,
        json!({"provider_id":body.provider_id}),
    )
    .await?;
    let unavailable = || {
        ApiError::localized(
            StatusCode::BAD_GATEWAY,
            Message::new("models.discoveryFailed"),
        )
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| unavailable())?;
    let url = reqwest::Url::parse(&p.base_url).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let request = client.get(format!("{}/models", p.base_url.trim_end_matches('/')));
    let request = if p.protocol == "anthropic" {
        request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(secret)
    };
    let mut response = request.send().await.map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    let rows = data["data"].as_array().ok_or_else(unavailable)?;
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
    if models.is_empty() || models.len() > 2000 {
        return Err(unavailable());
    }
    let current = Scope::for_actor(&s, scope.workspace_id, scope.user.id).await?;
    if !current.permissions.contains("ai-models.create") {
        current.require("ai-models.update")?;
    }
    current.require("credentials.use")?;
    current.require("credentials.read")?;
    if provider(&s.db, scope.workspace_id, body.provider_id)
        .await?
        .version
        != p.version
    {
        return Err(changed());
    }
    Ok(Json(
        json!({"models":models,"has_more":data["has_more"].as_bool().unwrap_or(false)}),
    ))
}
