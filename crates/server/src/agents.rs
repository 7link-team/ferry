//! Workspace Agent configuration. Execution and conversation ownership live in P2.
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{ai_agents as agents, hosts, users, workspace_members},
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
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/ai-agents", get(list).post(create))
        .route("/{workspace}/ai-agents/{id}", get(detail).patch(update))
        .route("/{workspace}/ai-agents/{id}/probes", post(probe))
        .route("/{workspace}/ai-agents/{id}/revisions", get(revisions))
}
const SPEC: Spec = Spec {
    search: &["name", "role", "description"],
    filters: &["runtime", "state", "visibility", "host_id", "owner_id"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-updated_at",
};
fn invalid() -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new("agents.invalid"))
}
fn changed() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("agents.changed"))
}
fn manages(scope: &Scope) -> bool {
    scope.permissions.contains("ai-agents.update")
}
fn visible(scope: &Scope) -> Condition {
    if manages(scope) {
        Condition::all()
    } else {
        Condition::any()
            .add(agents::Column::Visibility.eq("workspace"))
            .add(agents::Column::OwnerId.eq(scope.user.id))
    }
}
async fn find(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<agents::Model> {
    scope.require("ai-agents.read")?;
    agents::Entity::find_by_id(id)
        .filter(agents::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(visible(scope))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Body {
    #[serde(flatten, default, skip_serializing)]
    metadata: HashMap<String, Value>,
    name: String,
    #[serde(default)]
    avatar: Option<crate::avatar::Avatar>,
    #[serde(default)]
    role: String,
    #[serde(default)]
    platform_role_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    model_profile_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    host_id: Id,
    owner_id: Id,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default = "default_permission")]
    permission: String,
    visibility: String,
    state: String,
    #[serde(default)]
    version: i64,
}
fn default_permission() -> String {
    "read_only".into()
}
impl Body {
    fn definition(&self) -> Result<String> {
        let mut value = serde_json::to_value(self).map_err(|_| invalid())?;
        value.as_object_mut().unwrap().remove("version");
        Ok(value.to_string())
    }
    async fn validate(
        &mut self,
        conn: &impl ConnectionTrait,
        scope: &Scope,
        old: Option<&agents::Model>,
    ) -> Result<()> {
        if self.metadata.keys().any(|k| {
            ![
                "id",
                "host_name",
                "owner_name",
                "model_profile_name",
                "platform_role_name",
                "effective_model",
                "revision_id",
                "created_at",
                "updated_at",
            ]
            .contains(&k.as_str())
        }) {
            return Err(invalid());
        }
        self.name = self.name.trim().into();
        if self.avatar.is_none() {
            self.avatar = old.and_then(|row| row.avatar.as_ref()).and_then(|value| serde_json::from_str(value).ok());
        }
        if let Some(avatar) = &self.avatar { avatar.validate()?; }
        if self.platform_role_id.is_empty() {
            if let Some(old) = old {
                self.platform_role_id =
                    crate::platform_tools::role_id(conn, old).await?.to_string();
            }
        }
        let role_id = Id::reference(&self.platform_role_id)?;
        self.platform_role_id = role_id.to_string();
        let old_role = if let Some(old) = old {
            crate::platform_tools::role_id(conn, old).await?
        } else {
            Id::default()
        };
        if role_id != old_role {
            scope.require("roles.update")?;
        }
        if i64::from(role_id) != 0 {
            crate::entities::workspace_roles::Entity::find_by_id(role_id)
                .filter(
                    crate::entities::workspace_roles::Column::WorkspaceId.eq(scope.workspace_id),
                )
                .one(conn)
                .await?
                .ok_or_else(invalid)?;
        }
        let profile_id = Id::reference(&self.model_profile_id)?;
        self.model_profile_id = profile_id.to_string();
        if i64::from(profile_id) != 0 {
            scope.require("ai-models.read")?;
            let profile = model_profile(conn, scope.workspace_id, profile_id).await?;
            if profile.state != "enabled" {
                return Err(invalid());
            }
            let catalog: Vec<String> =
                serde_json::from_str(&profile.models).map_err(|_| invalid())?;
            if !self.model.is_empty()
                && !catalog.contains(&self.model)
                && self.model != profile.default_model
            {
                return Err(invalid());
            }
            self.runtime = if profile.kind == "api" {
                "api".into()
            } else {
                profile.runtime
            };
            self.host_id = 0.into();
            self.cwd.clear();
            self.permission = default_permission();
        }
        if self.name.is_empty()
            || self.name.len() > 128
            || self.role.len() > 256
            || self.description.len() > 4096
            || self.system_prompt.len() > 32 * 1024
            || self.cwd.len() > 4096
            || self.model.len() > 256
            || self.model.starts_with('-')
            || self.model.chars().any(char::is_control)
            || (self.runtime != "api"
                && ferry_protocol::agent::Runtime::parse(&self.runtime).is_none())
            || (self.runtime == "api" && i64::from(profile_id) == 0)
            || !["read_only", "on_request"].contains(&self.permission.as_str())
            || !["owner", "workspace"].contains(&self.visibility.as_str())
            || !["enabled", "disabled", "archived"].contains(&self.state.as_str())
            || [
                &self.name,
                &self.role,
                &self.description,
                &self.cwd,
                &self.system_prompt,
            ]
            .iter()
            .any(|s| s.contains('\0'))
        {
            return Err(invalid());
        }
        if !self.cwd.is_empty() {
            crate::hosts::channel::check_path(&self.cwd)?;
        }
        // Delegating an executable identity requires host administration, even
        // though later Agent use never grants an interactive terminal.
        let execution_changed = old.is_none_or(|o| {
            o.host_id != i64::from(self.host_id)
                || o.cwd != self.cwd
                || o.runtime != self.runtime
                || o.permission != self.permission
                || o.system_prompt != self.system_prompt
                || o.visibility != self.visibility
                || o.owner_id != i64::from(self.owner_id)
        });
        if execution_changed && i64::from(profile_id) == 0 {
            scope.require("hosts.update")?;
        }
        if i64::from(profile_id) == 0 {
            hosts::Entity::find_by_id(self.host_id)
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(hosts::Column::Connection.eq("runner"))
                .one(conn)
                .await?
                .ok_or_else(invalid)?;
        }
        workspace_members::Entity::find()
            .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(workspace_members::Column::UserId.eq(self.owner_id))
            .filter(workspace_members::Column::State.eq("active"))
            .one(conn)
            .await?
            .ok_or_else(invalid)?;
        users::Entity::find_by_id(self.owner_id)
            .filter(users::Column::Active.eq(1))
            .one(conn)
            .await?
            .ok_or_else(invalid)?;
        Ok(())
    }
    fn model(&self, id: Id, workspace: Id, now: i64) -> agents::Model {
        agents::Model {
            id: id.into(),
            model_profile_id: Id::reference(&self.model_profile_id)
                .unwrap_or_default()
                .into(),
            workspace_id: workspace.into(),
            name: self.name.clone(),
            avatar: self.avatar.as_ref().map(|value| serde_json::to_string(value).expect("serializable avatar")),
            role: self.role.clone(),
            description: self.description.clone(),
            runtime: self.runtime.clone(),
            model: self.model.clone(),
            host_id: self.host_id.into(),
            owner_id: self.owner_id.into(),
            cwd: self.cwd.clone(),
            system_prompt: self.system_prompt.clone(),
            permission: self.permission.clone(),
            state: self.state.clone(),
            visibility: self.visibility.clone(),
            revision_id: 0,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }
}
fn json(row: agents::Model) -> Value {
    json!({"avatar":row.avatar.as_ref().and_then(|v|serde_json::from_str::<Value>(v).ok()),"id":Id::from(row.id),"model_profile_id":Id::from(row.model_profile_id),"name":row.name,"role":row.role,"description":row.description,"runtime":row.runtime,"model":row.model,"host_id":Id::from(row.host_id),"owner_id":Id::from(row.owner_id),"cwd":row.cwd,"system_prompt":row.system_prompt,"permission":row.permission,"visibility":row.visibility,"state":row.state,"revision_id":Id::from(row.revision_id),"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at})
}
async fn views(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    rows: Vec<agents::Model>,
) -> Result<Vec<Value>> {
    use crate::resources::names::{self, Kind, of};
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let role_id = crate::platform_tools::role_id(conn, &row).await?;
        let mut value = json(row);
        value["platform_role_id"] = json!(role_id);
        if i64::from(role_id) != 0 {
            if let Some(role) = crate::entities::workspace_roles::Entity::find_by_id(role_id)
                .filter(
                    crate::entities::workspace_roles::Column::WorkspaceId.eq(scope.workspace_id),
                )
                .one(conn)
                .await?
            {
                value["platform_role_name"] = json!(role.name);
            }
        }
        values.push(value);
    }
    names::attach(
        conn,
        scope.workspace_id,
        &mut values,
        &[
            of("host_id", "host_name", Kind::Host),
            of("owner_id", "owner_name", Kind::User),
        ],
    )
    .await?;
    for value in &mut values {
        if let Some(id) = value["model_profile_id"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|id| *id != 0)
        {
            if let Some(profile) = crate::entities::ai_model_profiles::Entity::find_by_id(id)
                .filter(
                    crate::entities::ai_model_profiles::Column::WorkspaceId.eq(scope.workspace_id),
                )
                .one(conn)
                .await?
            {
                value["model_profile_name"] = json!(profile.name);
                value["effective_model"] = json!(if value["model"]
                    .as_str()
                    .is_some_and(|model| !model.is_empty())
                {
                    value["model"].as_str().unwrap().to_owned()
                } else {
                    profile.default_model
                });
            }
        }
    }
    Ok(values)
}

pub(crate) async fn model_profile(
    conn: &impl ConnectionTrait,
    workspace: Id,
    id: Id,
) -> Result<crate::entities::ai_model_profiles::Model> {
    use crate::entities::ai_model_profiles as profiles;
    profiles::Entity::find_by_id(id)
        .filter(profiles::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

pub(crate) async fn resolve_model(
    conn: &impl ConnectionTrait,
    agent: &agents::Model,
) -> Result<agents::Model> {
    let mut resolved = agent.clone();
    if agent.model_profile_id != 0 {
        let profile = model_profile(
            conn,
            agent.workspace_id.into(),
            agent.model_profile_id.into(),
        )
        .await?;
        if profile.state != "enabled" {
            return Err(invalid());
        }
        let catalog: Vec<String> = serde_json::from_str(&profile.models).map_err(|_| invalid())?;
        if !resolved.model.is_empty()
            && !catalog.contains(&resolved.model)
            && resolved.model != profile.default_model
        {
            return Err(invalid());
        }
        resolved.runtime = if profile.kind == "api" {
            "api".into()
        } else {
            profile.runtime
        };
        if resolved.model.is_empty() {
            resolved.model = profile.default_model;
        }
        resolved.host_id = 0;
        resolved.cwd.clear();
        resolved.permission = default_permission();
    }
    Ok(resolved)
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("ai-agents.read")?;
    let for_use = raw.remove("for_use").is_some_and(|value| value == "true");
    let visibility = if for_use {
        scope.require("ai-agents.use")?;
        Condition::all()
            .add(agents::Column::State.eq("enabled"))
            .add(
                Condition::any()
                    .add(agents::Column::Visibility.eq("workspace"))
                    .add(agents::Column::OwnerId.eq(scope.user.id)),
            )
    } else {
        visible(&scope)
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<agents::Entity, agents::Model>(
        &s.db,
        &SPEC,
        agents::Entity::find().filter(visibility),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":views(&s.db,&scope,rows).await?,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
pub async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    Ok(Json(
        views(&s.db, &scope, vec![find(&s.db, &scope, id).await?])
            .await?
            .remove(0),
    ))
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(mut body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("ai-agents.create")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    body.validate(&tx, &scope, None).await?;
    let id = s.next_id()?;
    let mut row = body.model(id, scope.workspace_id, db::now());
    let revision = crate::definitions::record(
        &s,
        &tx,
        scope.workspace_id,
        "agent",
        id,
        &body.definition()?,
        scope.user.id,
        None,
    )
    .await?;
    row.revision_id = revision.id;
    agents::Entity::insert(row.into_active_model())
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "ai-agents",
        &id,
        json!({"name":body.name,"revision_id":Id::from(revision.id)}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("ai-agents.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = find(&tx, &scope, id).await?;
    if old.version != body.version {
        return Err(changed());
    }
    body.validate(&tx, &scope, Some(&old)).await?;
    let mut row = body.model(id, scope.workspace_id, db::now());
    row.created_at = old.created_at;
    row.version = old.version.checked_add(1).ok_or_else(changed)?;
    // Version is concurrency metadata, not part of the executable definition.
    let revision = crate::definitions::record(
        &s,
        &tx,
        scope.workspace_id,
        "agent",
        id,
        &body.definition()?,
        scope.user.id,
        None,
    )
    .await?;
    row.revision_id = revision.id;
    let mut active = row.into_active_model().reset_all();
    active.id = sea_orm::ActiveValue::Unchanged(id.into());
    agents::Entity::update(active).exec(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "ai-agents",
        &id,
        json!({"name":body.name,"revision_id":Id::from(revision.id),"state":body.state}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn revisions(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    find(&s.db, &scope, id).await?;
    crate::definitions::list(&s.db, scope.workspace_id, "agent", id, raw).await
}
async fn probe(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("ai-agents.probe")?;
    let row = find(&s.db, &scope, id).await?;
    let mut execution = resolve_model(&s.db, &row).await?;
    if execution.runtime == "api" {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("conversations.apiPending"),
        ));
    }
    if execution.host_id == 0 {
        use sea_orm::QueryOrder;
        execution.host_id = hosts::Entity::find()
            .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(hosts::Column::Connection.eq("runner"))
            .filter(hosts::Column::State.eq("online"))
            .order_by_asc(hosts::Column::Id)
            .one(&s.db)
            .await?
            .ok_or_else(|| {
                ApiError::localized(StatusCode::CONFLICT, Message::new("conversations.noRunner"))
            })?
            .id;
    }
    let runtime = ferry_protocol::agent::Runtime::parse(&execution.runtime).ok_or_else(invalid)?;
    let channel =
        crate::hosts::remote::open(&s, &scope.workspace_id, &execution.host_id.into()).await?;
    let status = channel.agent_probe(runtime, &execution.cwd).await?;
    crate::hosts::activity(
        &s,
        &scope,
        "probe",
        "ai-agents",
        &id,
        json!({"revision_id":Id::from(row.revision_id)}),
    )
    .await?;
    Ok(Json(
        json!({"revision_id":Id::from(row.revision_id),"checked_at":db::now(),"status":status}),
    ))
}

#[cfg(test)]
pub(crate) mod tests;
