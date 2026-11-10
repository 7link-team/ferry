//! Workspace notification resources. Credentials stay in the vault; all callers
//! use the same permission checks and durable idempotency boundary.
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        notification_channels as channels, notification_deliveries as deliveries,
        notification_templates as templates, workspace_members, workspace_roles,
    },
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, Iterable, QueryFilter,
    QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};

#[cfg(test)]
mod tests;

const CHANNELS: Spec = Spec {
    search: &["name"],
    filters: &["provider", "enabled"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-created_at",
};
const TEMPLATES: Spec = Spec {
    search: &["name"],
    filters: &["language", "format"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "-created_at",
};
const DELIVERIES: Spec = Spec {
    search: &["subject"],
    filters: &["channel_id", "state", "source", "agent_id"],
    sortable: &["created_at", "updated_at"],
    default_sort: "-created_at",
};
fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("notification.invalid"),
    )
}
fn conflict() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("notification.conflict"))
}
fn aad(workspace: i64, id: i64) -> String {
    format!("notification-channel:{workspace}:{id}")
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Transport {
    /// Flat SMS fields: sms_access_key, sms_secret_key, sms_region,
    /// sms_app_id, sms_sign_name, sms_template_code (strings), and
    /// sms_parameter_names (ordered string array). Recipients use E.164.
    /// The provider sends its approved template, not arbitrary body text.
    #[serde(flatten)]
    pub sms: super::sms::Config,
    pub language: String,
    pub recipients: Vec<String>,
    pub webhook: String,
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: String,
    pub password: String,
    pub from: String,
    #[serde(flatten)]
    pub options: super::channel::Options,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            sms: Default::default(),
            language: "en-US".into(),
            recipients: Vec::new(),
            webhook: String::new(),
            host: String::new(),
            port: 587,
            security: "starttls".into(),
            username: String::new(),
            password: String::new(),
            from: String::new(),
            options: Default::default(),
        }
    }
}
impl Transport {
    fn validate(&self, provider: &str) -> Result<()> {
        if !crate::i18n::Locale::ALL
            .iter()
            .any(|locale| locale.tag() == self.language)
        {
            return Err(invalid());
        }
        if serde_json::to_vec(self).map_err(|_| invalid())?.len() > 65536 {
            return Err(invalid());
        }
        if provider == "SMTP" {
            if self.recipients.len() > 20
                || self
                    .recipients
                    .iter()
                    .any(|v| v.len() > 320 || v.parse::<lettre::message::Mailbox>().is_err())
            {
                return Err(invalid());
            }
            if self.host.is_empty()
                || self.host.len() > 253
                || self
                    .host
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || self.port == 0
                || !matches!(self.security.as_str(), "tls" | "starttls" | "none")
                || self.from.parse::<lettre::message::Mailbox>().is_err()
            {
                return Err(invalid());
            }
        } else if super::sms::is_sms(provider) {
            if !self.sms.validate(provider, &self.recipients) {
                return Err(invalid());
            }
        } else if self.webhook.len() > 4096 || self.options.resolve(&self.webhook).is_err() {
            return Err(invalid());
        }
        Ok(())
    }
    fn preserve(&mut self, old: &Self) {
        if self.sms.sms_secret_key.is_empty() {
            self.sms.sms_secret_key = old.sms.sms_secret_key.clone();
        }
        if self.webhook.is_empty() {
            self.webhook = old.webhook.clone()
        }
        if self.password.is_empty() {
            self.password = old.password.clone()
        }
        if self.options.signing_secret.is_empty() {
            self.options.signing_secret = old.options.signing_secret.clone()
        }
        if self.options.bot_token.is_empty() {
            self.options.bot_token = old.options.bot_token.clone()
        }
        if self.options.bearer_token.is_empty() {
            self.options.bearer_token = old.options.bearer_token.clone()
        }
    }
}
#[derive(Deserialize)]
pub struct ChannelBody {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub provider: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub role_ids: Vec<Id>,
    #[serde(default)]
    pub version: i64,
    #[serde(flatten)]
    pub config: Transport,
}
fn enabled() -> bool {
    true
}
pub(crate) fn decode(s: &AppState, row: &channels::Model) -> Result<Transport> {
    serde_json::from_str(
        &s.vault
            .open_secret(&row.config, &aad(row.workspace_id, row.id))?,
    )
    .map_err(|_| invalid())
}

pub(crate) async fn import_channel(
    s: &AppState,
    tx: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    name: String,
    provider: String,
    enabled: bool,
    config: Transport,
) -> Result<Id> {
    config.validate(&provider)?;
    let id = s.next_id()?;
    let now = db::now();
    channels::Entity::insert(
        channels::Model {
            id: id.into(),
            workspace_id: workspace.into(),
            name,
            description: String::new(),
            provider,
            enabled: i64::from(enabled),
            config: s.vault.seal(
                &serde_json::to_string(&config).map_err(|_| invalid())?,
                &aad(workspace.into(), id.into()),
            )?,
            role_ids: "[]".into(),
            version: 1,
            created_at: now,
            created_by: 0,
            updated_at: now,
            updated_by: 0,
        }
        .into_active_model(),
    )
    .exec(tx)
    .await?;
    Ok(id)
}
fn public(row: &channels::Model, transport: &Transport) -> Value {
    json!({"id":Id::from(row.id),"name":row.name,"description":row.description,"provider":row.provider,"language":transport.language,"recipients":transport.recipients,"enabled":row.enabled==1,"role_ids":serde_json::from_str::<Value>(&row.role_ids).unwrap_or(json!([])),"host":transport.host,"port":transport.port,"security":transport.security,"username":transport.username,"from":transport.from,"chat_id":transport.options.chat_id,"thread_id":transport.options.thread_id,"sms_access_key":transport.sms.sms_access_key,"sms_region":transport.sms.sms_region,"sms_app_id":transport.sms.sms_app_id,"sms_sign_name":transport.sms.sms_sign_name,"sms_template_code":transport.sms.sms_template_code,"sms_parameter_names":transport.sms.sms_parameter_names,"has_sms_secret":!transport.sms.sms_secret_key.is_empty(),"has_webhook":!transport.webhook.is_empty(),"has_secret":!transport.password.is_empty()||!transport.options.bot_token.is_empty()||!transport.options.signing_secret.is_empty()||!transport.options.bearer_token.is_empty(),"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at})
}
pub(crate) async fn channel(s: &AppState, workspace: Id, id: Id) -> Result<channels::Model> {
    channel_in(&s.db, workspace, id).await
}
pub(crate) async fn channel_in(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    id: Id,
) -> Result<channels::Model> {
    channels::Entity::find_by_id(id)
        .filter(channels::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(crate) async fn authorize(s: &AppState, scope: &Scope, row: &channels::Model) -> Result<()> {
    authorize_in(&s.db, scope, row).await
}
pub(crate) async fn authorize_in(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    row: &channels::Model,
) -> Result<()> {
    scope.require("notifications.send")?;
    if row.workspace_id != i64::from(scope.workspace_id) {
        return Err(ApiError::missing());
    }
    if row.enabled != 1 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("notification.disabled"),
        ));
    }
    let roles: Vec<Id> = serde_json::from_str(&row.role_ids).map_err(|_| invalid())?;
    if roles.is_empty() || scope.user.is_superadmin {
        return Ok(());
    }
    let member = workspace_members::Entity::find()
        .filter(workspace_members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(workspace_members::Column::UserId.eq(scope.user.id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    if !roles.contains(&Id::from(member.role_id)) {
        return Err(ApiError::forbidden());
    }
    if let Some(agent) = crate::platform_tools::current_agent() {
        let agent = crate::entities::ai_agents::Entity::find_by_id(agent)
            .filter(crate::entities::ai_agents::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(ApiError::forbidden)?;
        if !roles.contains(&crate::platform_tools::role_id(conn, &agent).await?) {
            return Err(ApiError::forbidden());
        }
    }
    Ok(())
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let q = ListQuery::parse(&CHANNELS, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &CHANNELS,
        channels::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let items = rows
        .iter()
        .map(|r| Ok(public(r, &decode(&s, r)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let row = channel(&s, scope.workspace_id, id).await?;
    Ok(Json(public(&row, &decode(&s, &row)?)))
}
async fn save(
    s: &AppState,
    scope: &Scope,
    id: Id,
    mut body: ChannelBody,
    old: Option<channels::Model>,
) -> Result<()> {
    if body.name.trim().is_empty()
        || body.name.chars().count() > 120
        || body.description.len() > 2048
        || body.role_ids.len() > 100
        || ![
            "SMTP",
            "Aliyun SMS",
            "Tencent SMS",
            "WeCom",
            "DingTalk",
            "Feishu",
            "Lark",
            "Telegram",
            "Slack",
            "Discord",
            "Google Chat",
            "Webhook",
        ]
        .contains(&body.provider.as_str())
    {
        return Err(invalid());
    }
    for role in &body.role_ids {
        if workspace_roles::Entity::find_by_id(*role)
            .filter(workspace_roles::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&s.db)
            .await?
            .is_none()
        {
            return Err(ApiError::missing());
        }
    }
    if let Some(old) = &old {
        if old.version != body.version {
            return Err(conflict());
        }
        if old.provider == body.provider {
            body.config.preserve(&decode(s, old)?);
        }
    }
    body.config.options.provider = body.provider.clone();
    body.config.recipients = body
        .config
        .recipients
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect();
    body.config.sms.sms_parameter_names = body
        .config
        .sms
        .sms_parameter_names
        .into_iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect();
    body.config.validate(&body.provider)?;
    let now = db::now();
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = channels::Model {
        id: i64::from(id),
        workspace_id: i64::from(scope.workspace_id),
        name: body.name.trim().into(),
        description: body.description,
        provider: body.provider,
        enabled: i64::from(body.enabled),
        config: s.vault.seal(
            &serde_json::to_string(&body.config).map_err(|_| invalid())?,
            &aad(i64::from(scope.workspace_id), i64::from(id)),
        )?,
        role_ids: json!(body.role_ids).to_string(),
        version: old.as_ref().map_or(1, |r| r.version + 1),
        created_at: old.as_ref().map_or(now, |r| r.created_at),
        created_by: old
            .as_ref()
            .map_or(i64::from(scope.user.id), |r| r.created_by),
        updated_at: now,
        updated_by: i64::from(scope.user.id),
    };
    if let Some(previous) = old {
        let changed = channels::Entity::update_many()
            .set(row.clone().into_active_model().reset_all())
            .filter(channels::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(channels::Column::Id.eq(id))
            .filter(channels::Column::Version.eq(previous.version))
            .exec(&tx)
            .await?;
        if changed.rows_affected != 1 {
            return Err(conflict());
        }
    } else {
        channels::Entity::insert(row.clone().into_active_model())
            .exec(&tx)
            .await?;
    }
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        "save",
        "notification-channels",
        &id,
        json!({"name":row.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<ChannelBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("notifications.create")?;
    let id = s.next_id()?;
    save(&s, &scope, id, body, None).await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<ChannelBody>,
) -> Result<Json<Value>> {
    scope.require("notifications.update")?;
    let old = channel(&s, scope.workspace_id, id).await?;
    save(&s, &scope, id, body, Some(old)).await?;
    get(State(s), scope, Path((String::new(), id))).await
}
pub async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("notifications.delete")?;
    channel(&s, scope.workspace_id, id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    crate::repositories::monitor::ensure_channel_unreferenced(&s, &tx, scope.workspace_id, id)
        .await?;
    if crate::entities::alert_rules::Entity::find()
        .filter(crate::entities::alert_rules::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::alert_rules::Column::NotificationChannelId.eq(id))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("notification.inUse"),
        ));
    }
    channels::Entity::delete_many()
        .filter(channels::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(channels::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "notification-channels",
        &id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TemplateBody {
    pub name: String,
    pub language: String,
    pub subject: String,
    pub body: String,
    pub format: String,
    #[serde(default)]
    pub version: i64,
}
fn valid_message(subject: &str, body: &str, format: &str) -> Result<()> {
    if subject.chars().count() > 256
        || subject.contains(['\r', '\n'])
        || body.len() > 48000
        || !matches!(format, "text" | "html")
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
pub async fn list_templates(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let q = ListQuery::parse(&TEMPLATES, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &TEMPLATES,
        templates::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.iter().map(template_public).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
fn template_public(row: &templates::Model) -> Value {
    json!({"id":Id::from(row.id),"name":row.name,"language":row.language,"subject":row.subject,"body":row.body,"format":row.format,"version":row.version,"created_at":row.created_at,"updated_at":row.updated_at})
}
async fn template(s: &AppState, scope: &Scope, id: Id) -> Result<templates::Model> {
    templates::Entity::find_by_id(id)
        .filter(templates::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}
pub async fn get_template(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    Ok(Json(template_public(&template(&s, &scope, id).await?)))
}
async fn save_template(
    s: &AppState,
    scope: &Scope,
    id: Id,
    body: TemplateBody,
    old: Option<templates::Model>,
) -> Result<()> {
    valid_message(&body.subject, &body.body, &body.format)?;
    if body.name.trim().is_empty()
        || body.name.chars().count() > 120
        || !crate::i18n::Locale::ALL
            .iter()
            .any(|l| l.tag() == body.language)
    {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let now = db::now();
    let row = templates::Model {
        id: i64::from(id),
        workspace_id: i64::from(scope.workspace_id),
        name: body.name,
        language: body.language,
        subject: body.subject,
        body: body.body,
        format: body.format,
        version: old.as_ref().map_or(1, |r| r.version + 1),
        created_at: old.as_ref().map_or(now, |r| r.created_at),
        created_by: old
            .as_ref()
            .map_or(i64::from(scope.user.id), |r| r.created_by),
        updated_at: now,
        updated_by: i64::from(scope.user.id),
    };
    if let Some(old) = old {
        if old.version != body.version {
            return Err(conflict());
        }
        let changed = templates::Entity::update_many()
            .set(row.into_active_model().reset_all())
            .filter(templates::Column::Id.eq(id))
            .filter(templates::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(templates::Column::Version.eq(old.version))
            .exec(&tx)
            .await?;
        if changed.rows_affected != 1 {
            return Err(conflict());
        }
    } else {
        templates::Entity::insert(row.into_active_model())
            .exec(&tx)
            .await?;
    }
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        "save",
        "notification-templates",
        &id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn create_template(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<TemplateBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("notifications.create")?;
    let id = s.next_id()?;
    save_template(&s, &scope, id, body, None).await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
pub async fn update_template(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<TemplateBody>,
) -> Result<Json<Value>> {
    scope.require("notifications.update")?;
    let old = template(&s, &scope, id).await?;
    save_template(&s, &scope, id, body, Some(old)).await?;
    get_template(State(s), scope, Path((String::new(), id))).await
}
pub async fn delete_template(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("notifications.delete")?;
    template(&s, &scope, id).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    templates::Entity::delete_many()
        .filter(templates::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(templates::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "notification-templates",
        &id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SendBody {
    pub client_id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "text")]
    pub format: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub template_id: Option<Id>,
    #[serde(default)]
    pub variables: std::collections::BTreeMap<String, String>,
}
fn text() -> String {
    "text".into()
}
fn render(
    input: &str,
    vars: &std::collections::BTreeMap<String, String>,
    html: bool,
) -> Result<String> {
    let mut result = String::new();
    let mut remaining = input;
    while let Some(start) = remaining.find("{{") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];
        let end = remaining.find("}}").ok_or_else(invalid)?;
        let value = vars.get(remaining[..end].trim()).ok_or_else(invalid)?;
        if html {
            result.push_str(
                &value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;"),
            )
        } else {
            result.push_str(value)
        }
        remaining = &remaining[end + 2..];
        if result.len() > 48000 {
            return Err(invalid());
        }
    }
    result.push_str(remaining);
    Ok(result)
}
async fn message(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    mut body: SendBody,
    sms: bool,
) -> Result<SendBody> {
    body.to = body
        .to
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect();
    if body.client_id.is_empty()
        || body.client_id.len() > 128
        || !body.client_id.bytes().all(|c| c.is_ascii_graphic())
        || body.to.len() > 20
        || body.to.iter().any(|v| {
            if sms {
                !super::sms::phone(v)
            } else {
                v.len() > 320 || v.parse::<lettre::message::Mailbox>().is_err()
            }
        })
        || body.variables.len() > 100
        || body
            .variables
            .iter()
            .any(|(k, v)| k.len() > 120 || v.len() > 48000)
    {
        return Err(invalid());
    }
    if sms && (body.template_id.is_some() || body.format == "html") {
        return Err(invalid());
    }
    if let Some(id) = body.template_id {
        let template = templates::Entity::find_by_id(id)
            .filter(templates::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        body.subject = render(&template.subject, &body.variables, false)?;
        body.body = render(&template.body, &body.variables, template.format == "html")?;
        body.format = template.format;
    }
    valid_message(&body.subject, &body.body, &body.format)?;
    if !sms && body.body.trim().is_empty() {
        return Err(invalid());
    }
    Ok(body)
}
fn delivery_public(row: &deliveries::Model, detail: bool) -> Value {
    let mut value = json!({"id":Id::from(row.id),"channel_id":Id::from(row.channel_id),"template_id":Id::from(row.template_id),"actor_id":Id::from(row.actor_id),"agent_id":Id::from(row.agent_id),"source":row.source,"source_id":row.source_id,"state":row.state,"subject":row.subject,"result":crate::i18n::display_stored(&row.result),"created_at":row.created_at,"updated_at":row.updated_at});
    if detail {
        value["body"] = json!(row.body)
    }
    value
}
pub async fn list_deliveries(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let q = ListQuery::parse(&DELIVERIES, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &DELIVERIES,
        deliveries::Entity::find()
            .select_only()
            .columns(deliveries::Column::iter().filter(|c| !matches!(c, deliveries::Column::Body)))
            .expr_as(Expr::value(""), deliveries::Column::Body),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.iter().map(|r|delivery_public(r,false)).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_delivery(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let row = deliveries::Entity::find_by_id(id)
        .filter(deliveries::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(delivery_public(&row, true)))
}

pub async fn preview(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<SendBody>,
) -> Result<Json<Value>> {
    scope.require("notifications.read")?;
    let row = channel(&s, scope.workspace_id, id).await?;
    let mut body = message(&s.db, &scope, body, super::sms::is_sms(&row.provider)).await?;
    if super::sms::is_sms(&row.provider) {
        let config = decode(&s, &row)?;
        let parameters = sms_message(&row.provider, &config, &mut body)?;
        return Ok(Json(
            json!({"provider":row.provider,"sms_sign_name":config.sms.sms_sign_name,"sms_template_code":config.sms.sms_template_code,"parameters":parameters,"to":body.to}),
        ));
    }
    if row.provider == "SMTP" {
        return Ok(Json(
            json!({"provider":"SMTP","subject":body.subject,"body":body.body,"format":body.format,"to":body.to}),
        ));
    }
    let notice = super::webhook::Notice {
        title: &body.subject,
        accent: None,
        field_links: vec![],
        field_accents: vec![],
        fields: vec![(String::new(), body.body.clone())],
        generic: json!({"subject":body.subject,"body":body.body,"format":body.format}),
    };
    Ok(Json(
        super::webhook::preview(&row.provider, &notice).map_err(|_| invalid())?,
    ))
}
pub async fn send(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<SendBody>,
) -> Result<Json<Value>> {
    scope.require("notifications.send")?;
    let source = if crate::platform_tools::current_agent().is_some() {
        "mcp"
    } else {
        "manual"
    };
    let result = send_message(&s, &scope, id, body, source, "").await?;
    Ok(Json(result))
}
pub(crate) async fn send_message(
    s: &AppState,
    scope: &Scope,
    id: Id,
    body: SendBody,
    source: &str,
    source_id: &str,
) -> Result<Value> {
    if source.len() > 32
        || source_id.len() > 128
        || body.client_id.is_empty()
        || body.client_id.len() > 128
        || !body.client_id.bytes().all(|c| c.is_ascii_graphic())
    {
        return Err(invalid());
    }
    let row = channel(s, scope.workspace_id, id).await?;
    authorize(s, scope, &row).await?;
    send_authorized(s, scope, row, body, source, source_id, None).await
}

/// Business subscriptions are authorized when configured. Background delivery
/// retains their system identity; this entry point is never an HTTP handler.
pub(crate) async fn send_system_notice(
    s: &AppState,
    workspace: Id,
    channel_id: Id,
    client_id: String,
    source: &str,
    source_id: &str,
    notice: &super::webhook::Notice<'_>,
) -> Result<String> {
    let row = channel(s, workspace, channel_id).await?;
    if row.enabled != 1 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("notification.disabled"),
        ));
    }
    let scope = Scope {
        workspace_id: workspace,
        user: crate::accounts::auth::User {
            id: Id::default(),
            email: String::new(),
            name: "system".into(),
            is_superadmin: false,
        },
        permissions: Default::default(),
    };
    let body = SendBody {
        client_id,
        subject: notice.title.into(),
        body: notice
            .fields
            .iter()
            .map(|(label, value)| {
                if label.is_empty() {
                    value.clone()
                } else {
                    format!("{label}: {value}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        format: text(),
        ..Default::default()
    };
    let sent = send_authorized(s, &scope, row, body, source, source_id, Some(notice)).await?;
    let id: Id = serde_json::from_value(sent["id"].clone()).map_err(|_| invalid())?;
    let record = deliveries::Entity::find_by_id(id)
        .filter(deliveries::Column::WorkspaceId.eq(workspace))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(record.result)
}

async fn send_authorized(
    s: &AppState,
    scope: &Scope,
    row: channels::Model,
    body: SendBody,
    source: &str,
    source_id: &str,
    business_notice: Option<&super::webhook::Notice<'_>>,
) -> Result<Value> {
    let id = Id::from(row.id);
    let hash = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(scope.user.id, crate::platform_tools::current_agent(), &body))
                .map_err(|_| invalid())?
        )
    );
    let prior = deliveries::Entity::find()
        .filter(deliveries::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(deliveries::Column::ChannelId.eq(id))
        .filter(deliveries::Column::ClientId.eq(&body.client_id));
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    if let Some(old) = prior.clone().one(&tx).await? {
        if old.request_hash != hash {
            return Err(conflict());
        }
        return Ok(delivery_public(&old, true));
    }
    let mut body = message(&tx, scope, body, super::sms::is_sms(&row.provider)).await?;
    let transport = decode(s, &row)?;
    if row.provider == "SMTP" && body.to.is_empty() {
        body.to = transport.recipients.clone();
    }
    if row.provider == "SMTP" && body.to.is_empty() {
        return Err(invalid());
    }
    let sms_parameters = if super::sms::is_sms(&row.provider) {
        Some(sms_message(&row.provider, &transport, &mut body)?)
    } else {
        None
    };
    let now = db::now();
    let delivery_id = s.next_id()?;
    let record = deliveries::Model {
        id: i64::from(delivery_id),
        workspace_id: i64::from(scope.workspace_id),
        channel_id: i64::from(id),
        template_id: body.template_id.map_or(0, i64::from),
        actor_id: i64::from(scope.user.id),
        agent_id: crate::platform_tools::current_agent().map_or(0, i64::from),
        source: source.into(),
        source_id: source_id.into(),
        client_id: body.client_id.clone(),
        request_hash: hash,
        state: "sending".into(),
        subject: body.subject.clone(),
        body: body.body.clone(),
        result: String::new(),
        version: 1,
        created_at: now,
        created_by: i64::from(scope.user.id),
        updated_at: now,
        updated_by: i64::from(scope.user.id),
    };
    deliveries::Entity::insert(record.into_active_model())
        .exec(&tx)
        .await?;
    tx.commit().await?;
    let (result, state) = if row.provider == "SMTP" {
        smtp(&transport, &body).await
    } else if let Some(parameters) = sms_parameters {
        super::sms::send(&row.provider, &transport.sms, &body.to, &parameters).await
    } else {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid())?;
        let notice = super::webhook::Notice {
            title: &body.subject,
            accent: None,
            field_links: vec![],
            field_accents: vec![],
            fields: vec![(String::new(), body.body.clone())],
            generic: json!({"subject":body.subject,"body":body.body,"format":body.format}),
        };
        let result = super::webhook::deliver_configured(
            &client,
            &transport.webhook,
            &transport.options,
            business_notice.unwrap_or(&notice),
        )
        .await;
        let key = crate::i18n::decode_stored(&result)
            .map(|m| m.key)
            .unwrap_or_default();
        let state = if matches!(key.as_str(), "webhook.delivered" | "webhook.deliveredHttp") {
            "succeeded"
        } else if key.contains("Timeout")
            || key == "webhook.failed"
            || key == "webhook.providerTimeout"
        {
            "unknown"
        } else {
            "failed"
        };
        (result, state)
    };
    deliveries::Entity::update_many()
        .col_expr(deliveries::Column::State, Expr::value(state))
        .col_expr(deliveries::Column::Result, Expr::value(result))
        .col_expr(deliveries::Column::UpdatedAt, Expr::value(db::now()))
        .filter(deliveries::Column::Id.eq(delivery_id))
        .filter(deliveries::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&s.db)
        .await?;
    let record = deliveries::Entity::find_by_id(delivery_id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(delivery_public(&record, true))
}
fn sms_message(
    provider: &str,
    transport: &Transport,
    body: &mut SendBody,
) -> Result<std::collections::BTreeMap<String, String>> {
    if body.to.is_empty() {
        body.to = transport.recipients.clone();
    }
    body.to.sort();
    body.to.dedup();
    if body.to.is_empty() || !transport.sms.validate(provider, &body.to) {
        return Err(invalid());
    }
    let mut variables = body.variables.clone();
    variables
        .entry("subject".into())
        .or_insert(body.subject.clone());
    variables.entry("body".into()).or_insert(body.body.clone());
    let parameters = transport.sms.parameters(&variables).ok_or_else(invalid)?;
    if body.subject.trim().is_empty() {
        body.subject = format!("{provider} · {}", transport.sms.sms_template_code);
    }
    // This is the submitted provider template, not a fabricated SMS preview.
    body.body =
        json!({"template":transport.sms.sms_template_code,"parameters":parameters}).to_string();
    Ok(parameters)
}
async fn smtp(config: &Transport, body: &SendBody) -> (String, &'static str) {
    use lettre::{
        AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
        message::{Message, MultiPart, SinglePart, header::ContentType},
        transport::smtp::authentication::Credentials,
    };
    let invalid = || {
        (
            crate::i18n::store_message(&crate::i18n::Message::new("notification.sendFailed")),
            "failed",
        )
    };
    let Ok(from) = config.from.parse() else {
        return invalid();
    };
    let mut message = Message::builder().from(from).subject(&body.subject);
    for to in &body.to {
        let Ok(to) = to.parse() else { return invalid() };
        message = message.to(to);
    }
    let Ok(message) = (if body.format == "html" {
        message.multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(crate::accounts::mail::html_text(
                    &body.body,
                )))
                .singlepart(SinglePart::html(body.body.clone())),
        )
    } else {
        message
            .header(ContentType::TEXT_PLAIN)
            .body(body.body.clone())
    }) else {
        return invalid();
    };
    let builder = match config.security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
        "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host),
        _ => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &config.host,
        )),
    };
    let Ok(mut builder) = builder else {
        return invalid();
    };
    builder = builder
        .port(config.port)
        .timeout(Some(Duration::from_secs(15)));
    if !config.username.is_empty() {
        builder = builder.credentials(Credentials::new(
            config.username.clone(),
            config.password.clone(),
        ))
    }
    match tokio::time::timeout(Duration::from_secs(20), builder.build().send(message)).await {
        Ok(Ok(_)) => (
            crate::i18n::store_message(&crate::i18n::Message::new("notification.sent")),
            "succeeded",
        ),
        Ok(Err(_)) => (
            crate::i18n::store_message(&crate::i18n::Message::new("notification.unknown")),
            "unknown",
        ),
        Err(_) => (
            crate::i18n::store_message(&crate::i18n::Message::new("notification.unknown")),
            "unknown",
        ),
    }
}

/// Startup never replays in-flight messages: delivery may already have happened.
pub(crate) async fn prepare(s: &AppState) -> Result<()> {
    deliveries::Entity::update_many()
        .col_expr(deliveries::Column::State, Expr::value("unknown"))
        .col_expr(
            deliveries::Column::Result,
            Expr::value(crate::i18n::store_message(&Message::new(
                "notification.unknown",
            ))),
        )
        .col_expr(deliveries::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(deliveries::Column::UpdatedBy, Expr::value(0))
        .filter(deliveries::Column::State.eq("sending"))
        .filter(deliveries::Column::CreatedAt.lte(db::now() - 60_000))
        .exec(&s.db)
        .await?;
    Ok(())
}
