//! Browser authorization for external repositories. App secrets stay server-side.
use crate::{
    db,
    entities::{
        credentials, platform_state, repository_connections as connections,
        repository_oauth_flows as flows,
    },
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
    vault,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

struct Provider {
    client_id: String,
    client_secret: String,
    base: reqwest::Url,
    api: reqwest::Url,
}
fn endpoint(base: &reqwest::Url, path: &str) -> Result<reqwest::Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| ApiError::bad("授权接入点无效。"))?
        .pop_if_empty()
        .extend(path.split('/'));
    Ok(url)
}
const SETTINGS_AAD: &str = "platform-repository-oauth:1";
const PROVIDERS: [&str; 4] = ["github", "gitlab", "gitee", "gitea"];

#[derive(Default, Serialize, Deserialize)]
struct ProviderSettings {
    enabled: bool,
    client_id: String,
    client_secret: String,
    base_url: String,
    api_url: String,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct OAuthSettings {
    public_url: String,
    providers: std::collections::BTreeMap<String, ProviderSettings>,
}
fn default_base(kind: &str) -> &str {
    match kind {
        "github" => "https://github.com",
        "gitlab" => "https://gitlab.com",
        "gitea" => "",
        _ => "https://gitee.com",
    }
}
fn parse_endpoint(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(|_| ApiError::bad("授权接入点无效。"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad(
            "授权接入点必须为不带凭据或查询参数的HTTP(S)地址。",
        ));
    }
    Ok(url)
}
impl OAuthSettings {
    pub(crate) fn read(s: &AppState, stored: Option<&str>) -> Result<Self> {
        if let Some(stored) = stored {
            let mut config: Self =
                serde_json::from_str(&s.vault.open_secret(stored, SETTINGS_AAD)?)
                    .map_err(|_| ApiError::bad("平台网页授权配置损坏。"))?;
            for kind in PROVIDERS {
                config.providers.entry(kind.into()).or_default();
            }
            return Ok(config);
        }
        let mut providers = std::collections::BTreeMap::new();
        for kind in PROVIDERS {
            let prefix = format!("REPOSITORY_OAUTH_{}", kind.to_ascii_uppercase());
            let value = |key: &str| std::env::var(format!("{prefix}_{key}")).unwrap_or_default();
            let client_id = value("CLIENT_ID");
            let client_secret = value("CLIENT_SECRET");
            providers.insert(
                kind.into(),
                ProviderSettings {
                    enabled: !client_id.is_empty() && !client_secret.is_empty(),
                    client_id,
                    client_secret,
                    base_url: value("BASE_URL"),
                    api_url: value("API_URL"),
                },
            );
        }
        Ok(Self {
            public_url: std::env::var("REPOSITORY_OAUTH_PUBLIC_URL").unwrap_or_default(),
            providers,
        })
    }
    pub(crate) async fn load(s: &AppState, conn: &impl sea_orm::ConnectionTrait) -> Result<Self> {
        let row = platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        Self::read(s, row.repository_oauth.as_deref())
    }
    pub(crate) fn view(&self, s: &AppState, saved: bool) -> Value {
        let mut view = json!({"oauth_public_url":self.public_url,
            "oauth_callback_url":self.callback(s).unwrap_or_default(),
            "oauth_source":if saved { "平台设置（保存后立即生效）" } else { "服务端环境变量（网页保存后以平台设置为准）" }});
        for (kind, value) in &self.providers {
            for (key, field) in [
                ("enabled", json!(value.enabled)),
                ("client_id", json!(value.client_id)),
                ("has_client_secret", json!(!value.client_secret.is_empty())),
                ("base_url", json!(value.base_url)),
                ("api_url", json!(value.api_url)),
            ] {
                view[format!("oauth_{kind}_{key}")] = field;
            }
        }
        view
    }
    pub(crate) fn update(
        &mut self,
        s: &AppState,
        patch: &serde_json::Map<String, Value>,
    ) -> Result<()> {
        fn text(
            patch: &serde_json::Map<String, Value>,
            key: &str,
            target: &mut String,
            secret: bool,
        ) -> Result<()> {
            if let Some(value) = patch.get(key) {
                let value = value
                    .as_str()
                    .ok_or_else(|| ApiError::bad("网页授权配置字段类型无效。"))?;
                if value.len() > 8192 || value.chars().any(char::is_control) {
                    return Err(ApiError::bad("网页授权配置字段过长或包含控制字符。"));
                }
                if !secret || !value.is_empty() {
                    *target = if secret {
                        value.into()
                    } else {
                        value.trim().into()
                    };
                }
            }
            Ok(())
        }
        text(patch, "oauth_public_url", &mut self.public_url, false)?;
        if !self.public_url.is_empty() {
            self.callback(s)?;
        }
        for kind in PROVIDERS {
            let value = self
                .providers
                .get_mut(kind)
                .ok_or_else(|| ApiError::bad("平台网页授权配置损坏。"))?;
            let prefix = format!("oauth_{kind}_");
            if let Some(enabled) = patch.get(&(prefix.clone() + "enabled")) {
                value.enabled = enabled
                    .as_bool()
                    .ok_or_else(|| ApiError::bad("网页授权启用状态无效。"))?;
            }
            text(
                patch,
                &(prefix.clone() + "client_id"),
                &mut value.client_id,
                false,
            )?;
            text(
                patch,
                &(prefix.clone() + "client_secret"),
                &mut value.client_secret,
                true,
            )?;
            text(
                patch,
                &(prefix.clone() + "base_url"),
                &mut value.base_url,
                false,
            )?;
            text(patch, &(prefix + "api_url"), &mut value.api_url, false)?;
            for url in [&value.base_url, &value.api_url] {
                if !url.is_empty() {
                    parse_endpoint(url)?;
                }
            }
            if value.enabled {
                self.callback(s)?;
                self.provider(kind)?;
            }
        }
        Ok(())
    }
    pub(crate) fn seal(&self, s: &AppState) -> Result<String> {
        s.vault.seal(
            &serde_json::to_string(self).map_err(|_| ApiError::bad("网页授权配置编码失败。"))?,
            SETTINGS_AAD,
        )
    }
    fn provider(&self, kind: &str) -> Result<Provider> {
        let value = self
            .providers
            .get(kind)
            .ok_or_else(|| ApiError::bad("该平台请使用API令牌授权。"))?;
        if !value.enabled || value.client_id.is_empty() || value.client_secret.is_empty() {
            return Err(ApiError::bad(
                "平台尚未启用或配置该网页授权应用，请联系平台管理员。",
            ));
        }
        let base = parse_endpoint(if value.base_url.is_empty() {
            default_base(kind)
        } else {
            &value.base_url
        })?;
        let api_default = match kind {
            "github" if base.host_str() == Some("github.com") => {
                "https://api.github.com".to_owned()
            }
            "github" => endpoint(&base, "api/v3")?.to_string(),
            "gitlab" => endpoint(&base, "api/v4")?.to_string(),
            "gitea" => endpoint(&base, "api/v1")?.to_string(),
            _ => endpoint(&base, "api/v5")?.to_string(),
        };
        let api = parse_endpoint(if value.api_url.is_empty() {
            &api_default
        } else {
            &value.api_url
        })?;
        Ok(Provider {
            client_id: value.client_id.clone(),
            client_secret: value.client_secret.clone(),
            base,
            api,
        })
    }
    fn callback(&self, s: &AppState) -> Result<String> {
        let base = if self.public_url.is_empty() {
            format!("http://{}", s.bind)
        } else {
            self.public_url.clone()
        };
        let url = parse_endpoint(&base)?;
        if url
            .host_str()
            .is_none_or(|h| matches!(h, "0.0.0.0" | "[::]"))
        {
            return Err(ApiError::bad("请在平台设置中配置有效公开访问地址。"));
        }
        Ok(endpoint(&url, "api/repository-providers/callback")?.to_string())
    }
}
fn config_fingerprint(config: &Provider) -> String {
    hash(&json!([config.client_id, config.base.as_str(), config.api.as_str()]).to_string())
}
fn require_same_config(config: &Provider, fingerprint: &str) -> Result<()> {
    if config_fingerprint(config) != fingerprint {
        return Err(ApiError::bad("授权应用或接入点已变化，请重新授权。"));
    }
    Ok(())
}
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn flow_aad(workspace: Id, id: Id) -> String {
    format!("repository-oauth:{workspace}:{id}")
}
fn connection_aad(workspace: Id, id: Id) -> String {
    format!("repository-refresh:{workspace}:{id}")
}
async fn response_json(mut response: reqwest::Response) -> Result<Value> {
    if !response.status().is_success() {
        return Err(ApiError::bad(format!(
            "授权提供方返回HTTP {}，请重新授权。",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(part) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad("读取授权结果失败。"))?
    {
        if bytes.len() + part.len() > 1024 * 1024 {
            return Err(ApiError::bad("授权响应过大。"));
        }
        bytes.extend_from_slice(&part);
    }
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("授权提供方返回了无效结果。"))
}
fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("workspace-repository-authorization/1")
        .build()
        .map_err(|_| ApiError::bad("无法初始化授权连接。"))
}

// Codeup may return fewer items than requested; the documented response headers
// determine whether another page exists. Keep the row-count fallback for proxies
// that omit them, and for the other providers' existing contracts.
fn codeup_has_more(response: &reqwest::Response, page: u32) -> Option<bool> {
    if let Some(next) = response
        .headers()
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
    {
        if next.trim().is_empty() {
            return Some(false);
        }
        if let Ok(next) = next.trim().parse::<u32>() {
            return Some(next > page);
        }
    }
    response
        .headers()
        .get("x-total-pages")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .map(|total| page < total)
}

async fn repository_response(response: reqwest::Response, kind: &str) -> Result<Value> {
    if kind == "codeup" && !response.status().is_success() {
        let message = match response.status().as_u16() {
            401 => "Codeup令牌无效或已过期，请更新授权认证。",
            403 => "Codeup拒绝访问，请检查令牌的代码仓库、分支只读权限及账号的仓库权限。",
            404 => "Codeup资源不存在或不可见，请检查组织ID、仓库地址和API接入点。",
            429 => "Codeup请求过于频繁，请稍后重试。",
            500..=599 => "Codeup服务暂时不可用，请稍后重试。",
            300..=399 => "Codeup接入点返回重定向，请填写最终API地址；不会转发令牌。",
            _ => "Codeup请求失败，请检查接入点和授权配置。",
        };
        return Err(ApiError::bad(message));
    }
    response_json(response).await
}
fn token_data(value: &Value) -> Result<(&str, i64, Option<&str>)> {
    let valid = |v: &str| {
        !v.is_empty() && v.len() < 65536 && !v.chars().any(|c| c.is_control() || c.is_whitespace())
    };
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|v| valid(v))
        .ok_or_else(|| ApiError::bad("授权未返回有效访问令牌，请重新授权。"))?;
    let expires = match value.get("expires_in") {
        None => 0,
        Some(v) => {
            let seconds = v
                .as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| ApiError::bad("授权有效期无效。"))?;
            i64::try_from(seconds)
                .ok()
                .and_then(|v| v.checked_mul(1000))
                .and_then(|v| db::now().checked_add(v))
                .filter(|v| *v <= 8_640_000_000_000_000)
                .ok_or_else(|| ApiError::bad("授权有效期无效。"))?
        }
    };
    let refresh = match value.get("refresh_token") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .filter(|v| valid(v))
                .ok_or_else(|| ApiError::bad("授权刷新令牌无效。"))?,
        ),
    };
    Ok((access, expires, refresh))
}

pub async fn list(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let mut rows = Vec::new();
    let settings = OAuthSettings::load(&s, &s.db).await?;
    for kind in ["github", "gitlab", "gitee", "gitea", "codeup"] {
        let row = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(connections::Column::UserId.eq(scope.user.id))
            .filter(connections::Column::Provider.eq(kind))
            .filter(connections::Column::Name.eq(""))
            .one(&s.db)
            .await?;
        let configured = settings.provider(kind).is_ok() && settings.callback(&s).is_ok();
        let state = row
            .as_ref()
            .map(|r| {
                if r.state != "connected" {
                    r.state.as_str()
                } else if r.expires_at > 0 && r.expires_at <= db::now() {
                    if r.refresh_secret.is_empty() {
                        "expired"
                    } else {
                        "refreshable"
                    }
                } else {
                    r.state.as_str()
                }
            })
            .unwrap_or("not-connected");
        rows.push(json!({"id":kind,"provider":kind,"web_available":configured,"state":state,"credential_id":row.as_ref().map(|r|Id::from(r.credential_id)),"account":row.as_ref().map(|r|r.account.as_str()).unwrap_or(""),"expires_at":row.as_ref().map(|r|r.expires_at).unwrap_or(0),"last_error":row.as_ref().map(|r|r.last_error.as_str()).unwrap_or(""),"setup":if configured{""}else if kind=="codeup"{"使用Codeup个人访问令牌。"}else{"尚未配置网页授权应用，请联系平台管理员；也可使用API令牌。"}}));
    }
    Ok(Json(
        json!({"items":rows,"total":5,"page":1,"page_size":20}),
    ))
}
#[derive(Default, Deserialize)]
pub struct StartOptions {
    #[serde(default)]
    popup: bool,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    connection_id: String,
}
pub async fn start(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Query(options): Query<StartOptions>,
) -> Result<Json<Value>> {
    scope.require("credentials.create")?;
    if options.channel.len() > 64
        || !options
            .channel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ApiError::bad("授权窗口标识无效。"));
    }
    if !options.name.is_empty() {
        validate_connection_name(&options.name, &options.description)?;
        scope.require("repositories.create")?;
    }
    let target = Id::reference(&options.connection_id)?;
    if target != Id::from(0) {
        scope.require("repositories.update")?;
        let row = owned_connection(&s.db, &scope, target).await?;
        if row.provider != kind
            || row.name.is_empty()
            || options.name.is_empty()
            || row.token_config.is_some()
        {
            return Err(ApiError::bad("请选择同平台的网页授权连接重新授权。"));
        }
    }
    let settings = OAuthSettings::load(&s, &s.db).await?;
    let config = settings.provider(&kind)?;
    let redirect = settings.callback(&s)?;
    let state = random_token();
    let verifier = random_token();
    let id = s.next_id()?;
    let now = db::now();
    let proof = json!({"verifier":verifier,"redirect":redirect,"client_id":config.client_id,"base":config.base.as_str(),"config":config_fingerprint(&config),"popup":options.popup,"channel":options.channel,"name":options.name,"description":options.description,"connection_id":options.connection_id});
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    flows::Entity::delete_many()
        .filter(flows::Column::ExpiresAt.lt(now))
        .exec(&tx)
        .await?;
    flows::Entity::insert(flows::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        provider: Set(kind.clone()),
        state_hash: Set(hash(&state)),
        proof: Set(s
            .vault
            .seal(&proof.to_string(), &flow_aad(scope.workspace_id, id))?),
        expires_at: Set(now + 600_000),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    tx.commit().await?;
    let mut url = endpoint(
        &config.base,
        if kind == "github" {
            "login/oauth/authorize"
        } else if kind == "gitea" {
            "login/oauth/authorize"
        } else {
            "oauth/authorize"
        },
    )?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("response_type", "code")
            .append_pair("state", &state)
            .append_pair(
                "scope",
                match kind.as_str() {
                    "github" => "repo read:user",
                    "gitlab" => "read_user read_api read_repository",
                    "gitea" => "read:user read:repository",
                    _ => "user_info projects",
                },
            );
        if kind != "gitee" {
            query
                .append_pair("code_challenge_method", "S256")
                .append_pair(
                    "code_challenge",
                    &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                );
        }
    }
    Ok(Json(json!({"url":url.as_str()})))
}
#[derive(Deserialize)]
pub struct Callback {
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    error: String,
}
pub async fn complete(
    State(s): State<AppState>,
    account: crate::auth::Account,
    Query(b): Query<Callback>,
) -> Result<Redirect> {
    let pending = flows::Entity::find()
        .filter(flows::Column::StateHash.eq(hash(&b.state)))
        .filter(flows::Column::UserId.eq(account.user.id))
        .filter(flows::Column::ExpiresAt.gt(db::now()))
        .one(&s.db)
        .await?
        .ok_or_else(|| ApiError::bad("授权状态无效或已过期，请从原工作空间重新发起授权。"))?;
    let scope = Scope::for_actor(&s, pending.workspace_id.into(), account.user.id).await?;
    scope.require("credentials.create")?;
    let workspace = scope.workspace_id;
    let flow = {
        let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
        let row = flows::Entity::find()
            .filter(flows::Column::StateHash.eq(hash(&b.state)))
            .filter(flows::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(flows::Column::UserId.eq(scope.user.id))
            .filter(flows::Column::ExpiresAt.gt(db::now()))
            .one(&tx)
            .await?
            .ok_or_else(|| ApiError::bad("授权状态无效或已过期，请从原工作空间重新发起授权。"))?;
        flows::Entity::delete_by_id(row.id).exec(&tx).await?;
        tx.commit().await?;
        row
    };
    let proof: Value = serde_json::from_str(
        &s.vault
            .open_secret(&flow.proof, &flow_aad(scope.workspace_id, flow.id.into()))?,
    )
    .map_err(|_| ApiError::bad("授权状态损坏，请重新授权。"))?;
    let return_path = if proof["popup"].as_bool() == Some(true) {
        format!(
            "/workspaces/{workspace}/code-connections/new?oauth_popup=1&provider={}&oauth_channel={}",
            flow.provider,
            proof["channel"].as_str().unwrap_or("")
        )
    } else {
        format!(
            "/workspaces/{workspace}/code-connections/new?provider={}",
            flow.provider
        )
    };
    if !b.error.is_empty() || b.code.is_empty() {
        return Ok(Redirect::to(&format!(
            "{return_path}&authorization=cancelled"
        )));
    }
    let settings = OAuthSettings::load(&s, &s.db).await?;
    let config = settings.provider(&flow.provider)?;
    if proof["redirect"].as_str() != Some(settings.callback(&s)?.as_str())
        || proof["client_id"].as_str() != Some(&config.client_id)
        || proof["base"].as_str() != Some(config.base.as_str())
    {
        return Err(ApiError::bad("授权应用配置已变化，请重新授权。"));
    }
    require_same_config(&config, proof["config"].as_str().unwrap_or(""))?;
    let mut form = vec![
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
        ("code", b.code.as_str()),
        ("redirect_uri", proof["redirect"].as_str().unwrap_or("")),
        ("grant_type", "authorization_code"),
    ];
    if flow.provider != "gitee" {
        form.push(("code_verifier", proof["verifier"].as_str().unwrap_or("")));
    }
    let client = http()?;
    let tokens = response_json(
        client
            .post(endpoint(
                &config.base,
                if matches!(flow.provider.as_str(), "github" | "gitea") {
                    "login/oauth/access_token"
                } else {
                    "oauth/token"
                },
            )?)
            .header("Accept", "application/json")
            .form(&form)
            .send()
            .await
            .map_err(|_| ApiError::bad("授权交换失败，请重新授权。"))?,
    )
    .await?;
    let (access, expires, refresh) = token_data(&tokens)?;
    let mut request = client.get(endpoint(&config.api, "user")?);
    if flow.provider == "gitee" {
        request = request.query(&[("access_token", access)]);
    } else {
        request = request.bearer_auth(access);
    }
    let account = response_json(
        request
            .send()
            .await
            .map_err(|_| ApiError::bad("读取授权账号失败，请重新授权。"))?,
    )
    .await?;
    let account = account
        .get("login")
        .or_else(|| account.get("username"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() < 255)
        .ok_or_else(|| ApiError::bad("授权提供方未返回有效账号。"))?;
    let connection_name = proof["name"].as_str().unwrap_or("");
    let target = Id::reference(proof["connection_id"].as_str().unwrap_or(""))?;
    let (connection_id, _) = save_connection(
        &s,
        &scope,
        &flow.provider,
        &config,
        account,
        access,
        expires,
        refresh,
        None,
        if connection_name.is_empty() {
            None
        } else {
            Some((
                connection_name,
                proof["description"].as_str().unwrap_or(""),
                if target == Id::from(0) {
                    None
                } else {
                    Some(target)
                },
            ))
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "{return_path}&authorization=connected&connection_id={connection_id}"
    )))
}

async fn save_connection(
    s: &AppState,
    scope: &Scope,
    kind: &str,
    config: &Provider,
    account: &str,
    access: &str,
    expires: i64,
    refresh: Option<&str>,
    token_config: Option<String>,
    named: Option<(&str, &str, Option<Id>)>,
) -> Result<(Id, Id)> {
    let fresh = Scope::for_actor(&s, scope.workspace_id, scope.user.id).await?;
    fresh.require("credentials.create")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let now = db::now();
    let existing = match named {
        Some((_, _, Some(id))) => {
            fresh.require("repositories.update")?;
            let row = owned_connection(&tx, &fresh, id).await?;
            if row.provider != kind || row.token_config.is_some() {
                return Err(ApiError::bad("连接授权类型已变化，请重新发起授权。"));
            }
            Some(row)
        }
        Some(_) => {
            fresh.require("repositories.create")?;
            None
        }
        None => {
            connections::Entity::find()
                .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(connections::Column::UserId.eq(scope.user.id))
                .filter(connections::Column::Provider.eq(kind))
                .filter(connections::Column::Name.eq(""))
                .one(&tx)
                .await?
        }
    };
    let id = existing
        .as_ref()
        .map(|r| Id::from(r.id))
        .unwrap_or(s.next_id()?);
    let credential = existing
        .as_ref()
        .map(|r| Id::from(r.credential_id))
        .unwrap_or(s.next_id()?);
    let sealed = s.vault.seal(
        access,
        &vault::aad(
            &scope.workspace_id.to_string(),
            "credentials",
            &credential.to_string(),
        ),
    )?;
    if existing.is_some() {
        credentials::Entity::update_many()
            .col_expr(credentials::Column::Secret, Expr::value(sealed))
            .col_expr(credentials::Column::Username, Expr::value(account))
            .col_expr(credentials::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                credentials::Column::Version,
                Expr::col(credentials::Column::Version).add(1),
            )
            .filter(credentials::Column::Id.eq(credential))
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .exec(&tx)
            .await?;
        connections::Entity::delete_by_id(i64::from(id))
            .exec(&tx)
            .await?;
    } else {
        credentials::Entity::insert(credentials::ActiveModel {
            id: Set(credential.into()),
            workspace_id: Set(scope.workspace_id.into()),
            name: Set(format!("{} · {}", kind, account).chars().take(60).collect()),
            kind: Set("token".into()),
            username: Set(account.into()),
            secret: Set(sealed),
            description: Set("仓库连接管理；请在仓库表单中重新连接。".into()),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .exec(&tx)
        .await?;
    }
    connections::Entity::insert(connections::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        provider: Set(kind.to_owned()),
        name: Set(named.map(|v| v.0.trim().to_owned()).unwrap_or_default()),
        description: Set(named.map(|v| v.1.trim().to_owned()).unwrap_or_default()),
        credential_id: Set(credential.into()),
        account: Set(account.into()),
        refresh_secret: Set(match refresh {
            Some(v) => s.vault.seal(v, &connection_aad(scope.workspace_id, id))?,
            None => String::new(),
        }),
        expires_at: Set(expires),
        state: Set("connected".into()),
        last_error: Set(String::new()),
        config_fingerprint: Set(config_fingerprint(config)),
        client_id: Set(config.client_id.clone()),
        token_config: Set(token_config),
        version: Set(existing.as_ref().map(|r| r.version + 1).unwrap_or(1)),
        created_at: Set(existing.map(|r| r.created_at).unwrap_or(now)),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &fresh,
        "authorize",
        "credentials",
        &credential,
        json!({"connection_id":id,"account":account}),
    )
    .await?;
    tx.commit().await?;
    Ok((id, credential))
}

#[derive(Serialize, Deserialize)]
struct TokenConfig {
    base_url: String,
    api_url: String,
    #[serde(default)]
    organization: String,
}
impl TokenConfig {
    fn provider(&self) -> Result<Provider> {
        Ok(Provider {
            client_id: String::new(),
            client_secret: String::new(),
            base: parse_endpoint(&self.base_url)?,
            api: parse_endpoint(&self.api_url)?,
        })
    }
}
async fn connection_provider(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    row: &connections::Model,
) -> Result<Provider> {
    let config = if let Some(value) = &row.token_config {
        serde_json::from_str::<TokenConfig>(value)
            .map_err(|_| ApiError::bad("仓库连接配置损坏。"))?
            .provider()?
    } else {
        OAuthSettings::load(s, conn)
            .await?
            .provider(&row.provider)?
    };
    require_same_config(&config, &row.config_fingerprint)?;
    Ok(config)
}
#[derive(Deserialize)]
pub struct TokenConnect {
    token: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_url: String,
    #[serde(default)]
    organization: String,
}
fn token_config(kind: &str, body: &TokenConnect) -> Result<TokenConfig> {
    if !PROVIDERS.contains(&kind) && kind != "codeup" {
        return Err(ApiError::bad("请选择支持的代码平台。"));
    }
    if body.token.is_empty()
        || body.token.len() > 65536
        || body
            .token
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ApiError::bad("请填写有效个人访问令牌。"));
    }
    if kind == "codeup"
        && (body.username.trim().is_empty()
            || body.username.len() > 255
            || body.username.chars().any(char::is_control))
    {
        return Err(ApiError::bad("请填写Codeup的Git用户名，用于后续拉取仓库。"));
    }
    let base = parse_endpoint(if body.base_url.trim().is_empty() {
        if kind == "codeup" {
            "https://codeup.aliyun.com"
        } else {
            default_base(kind)
        }
    } else {
        body.base_url.trim()
    })?;
    let default_api = match kind {
        "github" if base.host_str() == Some("github.com") => "https://api.github.com".into(),
        "github" => endpoint(&base, "api/v3")?.to_string(),
        "gitlab" => endpoint(&base, "api/v4")?.to_string(),
        "gitea" => endpoint(&base, "api/v1")?.to_string(),
        "codeup" => "https://openapi-rdc.aliyuncs.com".into(),
        _ => endpoint(&base, "api/v5")?.to_string(),
    };
    let api = parse_endpoint(if body.api_url.trim().is_empty() {
        &default_api
    } else {
        body.api_url.trim()
    })?;
    if body.organization.len() > 255
        || body
            .organization
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '?' | '#'))
    {
        return Err(ApiError::bad("Codeup组织ID无效。"));
    }
    if kind == "codeup"
        && api.host_str() == Some("openapi-rdc.aliyuncs.com")
        && body.organization.trim().is_empty()
    {
        return Err(ApiError::bad("Codeup中心版需要填写组织ID。"));
    }
    Ok(TokenConfig {
        base_url: base.to_string(),
        api_url: api.to_string(),
        organization: body.organization.trim().into(),
    })
}
fn authorize_request(
    request: reqwest::RequestBuilder,
    kind: &str,
    token: &str,
    personal: bool,
) -> reqwest::RequestBuilder {
    match kind {
        "gitee" => request.query(&[("access_token", token)]),
        "codeup" => request.header("x-yunxiao-token", token),
        "gitlab" if personal => request.header("PRIVATE-TOKEN", token),
        "gitea" if personal => request.header("Authorization", format!("token {token}")),
        _ => request.bearer_auth(token),
    }
}
async fn remote_repositories(
    kind: &str,
    config: &Provider,
    access: &str,
    personal: bool,
    organization: &str,
    page: u32,
) -> Result<Value> {
    let mut url = match kind {
        "gitlab" => endpoint(&config.api, "projects")?,
        "codeup" => {
            let mut url = endpoint(&config.api, "oapi/v1/codeup")?;
            {
                let mut path = url
                    .path_segments_mut()
                    .map_err(|_| ApiError::bad("API地址无效。"))?;
                if !organization.is_empty() {
                    path.push("organizations").push(organization);
                }
                path.push("repositories");
            }
            url
        }
        _ => endpoint(&config.api, "user/repos")?,
    };
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("page", &page.to_string()).append_pair(
            if kind == "codeup" {
                "perPage"
            } else if kind == "gitea" {
                "limit"
            } else {
                "per_page"
            },
            "50",
        );
        if kind == "gitlab" {
            query.append_pair("membership", "true");
        }
    }
    let response = authorize_request(http()?.get(url), kind, access, personal)
        .send()
        .await
        .map_err(|_| ApiError::bad("读取代码平台仓库失败，请检查接入点和网络。"))?;
    let has_more = (kind == "codeup")
        .then(|| codeup_has_more(&response, page))
        .flatten();
    let value = repository_response(response, kind).await?;
    let rows = value
        .as_array()
        .ok_or_else(|| ApiError::bad("代码平台返回了无效仓库列表。"))?;
    let mut items = Vec::new();
    for row in rows {
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| row.get(k).and_then(Value::as_str))
                .unwrap_or("")
        };
        let clone_url = text(&[
            "clone_url",
            "http_url_to_repo",
            "httpCloneUrl",
            "httpUrlToRepo",
        ]);
        let url = if clone_url.is_empty() && kind == "codeup" {
            let web = text(&["webUrl"]);
            if web.is_empty() {
                return Err(ApiError::bad("Codeup未返回仓库地址。"));
            }
            format!("{}.git", web.trim_end_matches('/').trim_end_matches(".git"))
        } else {
            clone_url.to_owned()
        };
        let address = parse_endpoint(&url)?;
        if address.host_str() != config.base.host_str()
            || address.port_or_known_default() != config.base.port_or_known_default()
            || (config.base.scheme() == "https" && address.scheme() != "https")
        {
            return Err(ApiError::bad(
                "代码平台返回的仓库地址与配置的网站地址不匹配。",
            ));
        }
        items.push(json!({"name":text(&["name"]),"full_name":text(&["full_name","path_with_namespace","pathWithNamespace","name"]),
            "url":url,"default_branch":text(&["default_branch","defaultBranch"])}));
    }
    Ok(json!({"items":items,"page":page,"has_more":has_more.unwrap_or(rows.len()==50)}))
}
pub async fn connect_token(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Json(body): Json<TokenConnect>,
) -> Result<Json<Value>> {
    scope.require("credentials.create")?;
    scope.require("repositories.create")?;
    let settings = token_config(&kind, &body)?;
    let config = settings.provider()?;
    let account = if kind == "codeup" {
        body.username.trim().to_owned()
    } else {
        let value = response_json(
            authorize_request(
                http()?.get(endpoint(&config.api, "user")?),
                &kind,
                &body.token,
                true,
            )
            .send()
            .await
            .map_err(|_| ApiError::bad("无法验证代码平台令牌。"))?,
        )
        .await?;
        value
            .get("login")
            .or_else(|| value.get("username"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() < 255)
            .ok_or_else(|| ApiError::bad("代码平台未返回有效账号。"))?
            .to_owned()
    };
    // Do not save a credential or mark connected unless repository access also succeeds.
    let mut result =
        remote_repositories(&kind, &config, &body.token, true, &settings.organization, 1).await?;
    let stored =
        serde_json::to_string(&settings).map_err(|_| ApiError::bad("连接配置编码失败。"))?;
    let (_, id) = save_connection(
        &s,
        &scope,
        &kind,
        &config,
        &account,
        &body.token,
        0,
        None,
        Some(stored),
        None,
    )
    .await?;
    result["credential_id"] = json!(id);
    result["account"] = json!(account);
    Ok(Json(result))
}
#[derive(Deserialize)]
pub struct RepositoryPage {
    #[serde(default = "first_page")]
    page: u32,
}
fn first_page() -> u32 {
    1
}
pub async fn browse_repositories(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
    Query(page): Query<RepositoryPage>,
) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    scope.require("repositories.create")?;
    if page.page == 0 || page.page > 10000 {
        return Err(ApiError::bad("页码无效。"));
    }
    let row = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Provider.eq(&kind))
        .filter(connections::Column::Name.eq(""))
        .one(&s.db)
        .await?
        .ok_or_else(|| ApiError::bad("请先连接代码平台。"))?;
    let config = connection_provider(&s, &s.db, &row).await?;
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
    let token = row
        .token_config
        .as_deref()
        .map(serde_json::from_str::<TokenConfig>)
        .transpose()
        .map_err(|_| ApiError::bad("连接配置损坏。"))?;
    let mut result = remote_repositories(
        &kind,
        &config,
        &secret,
        token.is_some(),
        token
            .as_ref()
            .map(|c| c.organization.as_str())
            .unwrap_or(""),
        page.page,
    )
    .await?;
    result["credential_id"] = json!(Id::from(row.credential_id));
    result["account"] = json!(row.account);
    Ok(Json(result))
}

/// Called only when an execution needs a credential, not during metadata reads.
pub async fn ensure_fresh(s: &AppState, workspace: Id, credential: Id) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    let (row, version) = loop {
        let tx = crate::variables::transaction(s, workspace).await?;
        let Some(row) = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(workspace))
            .filter(connections::Column::CredentialId.eq(credential))
            .filter(connections::Column::TokenConfig.is_null())
            .one(&tx)
            .await?
        else {
            return Ok(());
        };
        if row.state == "reauthorize" {
            return Err(ApiError::bad(
                "仓库网页授权已失效，请在仓库授权页面重新授权。",
            ));
        }
        if row.state == "refreshing" {
            tx.commit().await?;
            if std::time::Instant::now() >= deadline {
                return Err(ApiError::bad("仓库授权正在续期，请稍后重试。"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        if row.expires_at == 0 || row.expires_at > db::now() + 30_000 {
            return Ok(());
        }
        if row.refresh_secret.is_empty() {
            if row.expires_at > db::now() {
                return Ok(());
            }
            return Err(ApiError::bad("仓库网页授权已过期，请重新授权。"));
        }
        let version = row.version + 1;
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("refreshing"))
            .col_expr(connections::Column::Version, Expr::value(version))
            .filter(connections::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        break (row, version);
    };
    let result: Result<()> = async {
        let config = OAuthSettings::load(s, &s.db)
            .await?
            .provider(&row.provider)?;
        require_same_config(&config, &row.config_fingerprint)?;
        let refresh = s.vault.open_secret(
            &row.refresh_secret,
            &connection_aad(workspace, row.id.into()),
        )?;
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
        ];
        let tokens = response_json(
            http()?
                .post(endpoint(
                    &config.base,
                    if matches!(row.provider.as_str(), "github" | "gitea") {
                        "login/oauth/access_token"
                    } else {
                        "oauth/token"
                    },
                )?)
                .header("Accept", "application/json")
                .form(&form)
                .send()
                .await
                .map_err(|_| ApiError::bad("授权续期失败，请重新授权。"))?,
        )
        .await?;
        let (access, expires, new_refresh) = token_data(&tokens)?;
        let tx = crate::variables::transaction(s, workspace).await?;
        if connections::Entity::find_by_id(row.id)
            .filter(connections::Column::Version.eq(version))
            .one(&tx)
            .await?
            .is_none()
        {
            return Ok(());
        }
        credentials::Entity::update_many()
            .col_expr(
                credentials::Column::Secret,
                Expr::value(s.vault.seal(
                    access,
                    &vault::aad(
                        &workspace.to_string(),
                        "credentials",
                        &credential.to_string(),
                    ),
                )?),
            )
            .col_expr(
                credentials::Column::Version,
                Expr::col(credentials::Column::Version).add(1),
            )
            .col_expr(credentials::Column::UpdatedAt, Expr::value(db::now()))
            .filter(credentials::Column::Id.eq(credential))
            .filter(credentials::Column::WorkspaceId.eq(workspace))
            .exec(&tx)
            .await?;
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("connected"))
            .col_expr(connections::Column::ExpiresAt, Expr::value(expires))
            .col_expr(
                connections::Column::RefreshSecret,
                Expr::value(match new_refresh {
                    Some(value) => s
                        .vault
                        .seal(value, &connection_aad(workspace, row.id.into()))?,
                    None => row.refresh_secret.clone(),
                }),
            )
            .col_expr(connections::Column::LastError, Expr::value(""))
            .col_expr(connections::Column::Version, Expr::value(version + 1))
            .col_expr(connections::Column::UpdatedAt, Expr::value(db::now()))
            .filter(connections::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        connections::Entity::update_many()
            .col_expr(connections::Column::State, Expr::value("reauthorize"))
            .col_expr(connections::Column::LastError, Expr::value(error.1.clone()))
            .filter(connections::Column::Id.eq(row.id))
            .filter(connections::Column::Version.eq(version))
            .exec(&s.db)
            .await?;
    }
    result
}
pub async fn recover(s: &AppState) -> Result<()> {
    connections::Entity::update_many()
        .col_expr(connections::Column::State, Expr::value("reauthorize"))
        .col_expr(
            connections::Column::LastError,
            Expr::value("服务中断，授权续期结果不确定，请重新授权。"),
        )
        .filter(connections::Column::State.eq("refreshing"))
        .exec(&s.db)
        .await?;
    flows::Entity::delete_many()
        .filter(flows::Column::ExpiresAt.lt(db::now()))
        .exec(&s.db)
        .await?;
    Ok(())
}

/// A web-authorized credential is usable only with its configured provider.
pub async fn validate_target(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    credential: Id,
    address: &str,
    api: bool,
) -> Result<()> {
    let Some(row) = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(workspace))
        .filter(connections::Column::CredentialId.eq(credential))
        .filter(connections::Column::Provider.ne("git"))
        .filter(
            sea_orm::Condition::any()
                .add(connections::Column::Name.eq(""))
                .add(connections::Column::TokenConfig.is_null()),
        )
        .one(conn)
        .await?
    else {
        return Ok(());
    };
    let config = connection_provider(s, conn, &row).await?;
    let expected = if api { config.api } else { config.base };
    let address =
        if let Some((host, path)) = address.strip_prefix("git@").and_then(|v| v.split_once(':')) {
            format!("ssh://git@{host}/{path}")
        } else {
            address.to_owned()
        };
    let actual = reqwest::Url::parse(&address).map_err(|_| ApiError::bad("仓库地址无效。"))?;
    if actual.host_str() != expected.host_str()
        || (actual.scheme() != "ssh"
            && actual.port_or_known_default() != expected.port_or_known_default())
        || (expected.scheme() == "https" && actual.scheme() == "http")
    {
        return Err(ApiError::bad(
            "网页授权凭据只能用于该授权平台的仓库和API接入点。",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_tokens_cannot_be_persisted_as_permanent_credentials() {
        assert_eq!(
            token_data(&json!({"access_token":"opaque-token"}))
                .unwrap()
                .1,
            0
        );
        assert!(
            token_data(&json!({"access_token":"opaque-token","expires_in":"3600"}))
                .unwrap()
                .1
                > db::now()
        );
        for value in [
            json!({"access_token":"a\npassword=another"}),
            json!({"access_token":"a","expires_in":-1}),
            json!({"access_token":"a","expires_in":"invalid"}),
            json!({"access_token":"a","expires_in":u64::MAX}),
            json!({"access_token":"a","refresh_token":"b\r\n"}),
        ] {
            assert!(token_data(&value).is_err());
        }
    }
}

const CONNECTION_SPEC: crate::resource::Spec = crate::resource::Spec {
    search: &["name", "account"],
    filters: &["provider", "state"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

fn connection_json(row: &connections::Model) -> Value {
    let config: Value = row
        .token_config
        .as_deref()
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or(json!({}));
    let mut value = json!({"id":Id::from(row.id),"name":row.name,"provider":row.provider,
        "description":row.description,"credential_id":Id::from(row.credential_id),
        "account":row.account,"state":row.state,"expires_at":row.expires_at,
        "last_error":row.last_error,"authorization":if row.token_config.is_none(){"oauth"}else{"credential"},
        "base_url":config["base_url"].as_str().unwrap_or(""),
        "api_url":config["api_url"].as_str().unwrap_or(""),
        "organization":config["organization"].as_str().unwrap_or(""),
        "version":row.version,"created_at":row.created_at,"updated_at":row.updated_at});
    if row.provider == "git" {
        value["url"] = json!(config["url"].as_str().unwrap_or(""));
    }
    value
}

async fn owned_connection(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<connections::Model> {
    connections::Entity::find_by_id(i64::from(id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

fn validate_connection_name(name: &str, description: &str) -> Result<()> {
    if name.trim().is_empty()
        || name.chars().count() > 60
        || name.chars().any(char::is_control)
        || description.chars().count() > 255
    {
        return Err(ApiError::bad(
            "连接名称必填且不超过60个字符，描述不超过255个字符。",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ConnectionBody {
    name: String,
    provider: String,
    #[serde(default)]
    credential_id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_url: String,
    #[serde(default)]
    organization: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    version: i64,
}

pub async fn list_connections(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let query = crate::resource::ListQuery::parse(&CONNECTION_SPEC, &raw)?;
    let (rows, total) = crate::resource::list(
        &s.db,
        &CONNECTION_SPEC,
        connections::Entity::find()
            .filter(connections::Column::UserId.eq(scope.user.id))
            .filter(connections::Column::Name.ne("")),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.iter().map(connection_json).collect::<Vec<_>>(),"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

pub async fn get_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    Ok(Json(connection_json(
        &owned_connection(&s.db, &scope, id.parse().map_err(|_| ApiError::missing())?).await?,
    )))
}

// Validate before opening a write transaction: provider requests never hold the workspace lock.
async fn check_connection(
    s: &AppState,
    scope: &Scope,
    body: &ConnectionBody,
) -> Result<(Id, String, String, String)> {
    validate_connection_name(&body.name, &body.description)?;
    scope.require("credentials.read")?;
    let credential = Id::reference(&body.credential_id)?;
    if body.provider == "git" && credential == Id::from(0) {
        if !body.url.is_empty() {
            parse_endpoint(&body.url)?;
        }
        return Ok((
            credential,
            String::new(),
            json!({"url":body.url}).to_string(),
            String::new(),
        ));
    }
    let (kind, username, secret) =
        crate::credentials::unseal(s, &scope.workspace_id, &credential).await?;
    // An OAuth credential belongs to its authorizing user and must retain its refresh owner.
    let managed = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::CredentialId.eq(credential))
        .filter(connections::Column::TokenConfig.is_null())
        .one(&s.db)
        .await?;
    if managed.is_some() {
        return Err(ApiError::bad(
            "网页授权凭据请通过网页授权创建或重新授权连接。",
        ));
    }
    if body.provider == "git" {
        if !crate::credentials::KINDS.contains(&kind.as_str()) {
            return Err(ApiError::bad("请选择Git授权凭据。"));
        }
        if !body.url.is_empty() {
            validate_git_credential(&body.url, &kind)?;
        }
        return Ok((
            credential,
            username,
            json!({"url":body.url}).to_string(),
            String::new(),
        ));
    }
    if kind != "token" {
        return Err(ApiError::bad("代码平台API连接需要个人访问令牌凭据。"));
    }
    let settings = token_config(
        &body.provider,
        &TokenConnect {
            token: secret.clone(),
            username: username.clone(),
            base_url: body.base_url.clone(),
            api_url: body.api_url.clone(),
            organization: body.organization.clone(),
        },
    )?;
    let provider = settings.provider()?;
    let account = if body.provider == "codeup" {
        username
    } else {
        let value = response_json(
            authorize_request(
                http()?.get(endpoint(&provider.api, "user")?),
                &body.provider,
                &secret,
                true,
            )
            .send()
            .await
            .map_err(|_| ApiError::bad("无法验证代码平台令牌。"))?,
        )
        .await?;
        value
            .get("login")
            .or_else(|| value.get("username"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() < 255)
            .ok_or_else(|| ApiError::bad("代码平台未返回有效账号。"))?
            .to_owned()
    };
    remote_repositories(
        &body.provider,
        &provider,
        &secret,
        true,
        &settings.organization,
        1,
    )
    .await?;
    Ok((
        credential,
        account,
        serde_json::to_string(&settings).map_err(|_| ApiError::bad("连接配置编码失败。"))?,
        config_fingerprint(&provider),
    ))
}

pub async fn create_connection(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<ConnectionBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("repositories.create")?;
    let (credential, account, config, fingerprint) = check_connection(&s, &scope, &body).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    // The credential could have been deleted while the remote API was responding.
    require_credential_in(&tx, &scope, credential).await?;
    let id = s.next_id()?;
    let now = db::now();
    connections::Entity::insert(connections::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        name: Set(body.name.trim().into()),
        description: Set(body.description.trim().into()),
        provider: Set(body.provider),
        credential_id: Set(credential.into()),
        account: Set(account),
        refresh_secret: Set(String::new()),
        expires_at: Set(0),
        state: Set("connected".into()),
        last_error: Set(String::new()),
        client_id: Set(String::new()),
        config_fingerprint: Set(fingerprint),
        token_config: Set(Some(config)),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "repositories",
        &id,
        json!({"connection":true,"name":body.name}),
    )
    .await?;
    let row = owned_connection(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(connection_json(&row))))
}

async fn require_credential_in(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    credential: Id,
) -> Result<()> {
    if credential == Id::from(0) {
        return Ok(());
    }
    credentials::Entity::find_by_id(i64::from(credential))
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(|| ApiError::bad("所选凭据不存在于当前工作空间。"))?;
    Ok(())
}

pub async fn update_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<ConnectionBody>,
) -> Result<Json<Value>> {
    scope.require("repositories.update")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let existing = owned_connection(&s.db, &scope, id).await?;
    validate_connection_name(&body.name, &body.description)?;
    let replacement = if existing.token_config.is_none() {
        if body.provider != existing.provider
            || Id::reference(&body.credential_id)? != Id::from(existing.credential_id)
        {
            return Err(ApiError::bad("网页授权连接请通过重新授权更换账号。"));
        }
        None
    } else {
        Some(check_connection(&s, &scope, &body).await?)
    };
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let mut update = connections::Entity::update_many()
        .col_expr(connections::Column::Name, Expr::value(body.name.trim()))
        .col_expr(
            connections::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(connections::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            connections::Column::Version,
            Expr::col(connections::Column::Version).add(1),
        )
        .filter(connections::Column::Id.eq(id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Version.eq(body.version));
    if let Some((credential, account, config, fingerprint)) = replacement {
        require_credential_in(&tx, &scope, credential).await?;
        update = update
            .col_expr(connections::Column::Provider, Expr::value(body.provider))
            .col_expr(
                connections::Column::CredentialId,
                Expr::value(i64::from(credential)),
            )
            .col_expr(connections::Column::Account, Expr::value(account))
            .col_expr(connections::Column::TokenConfig, Expr::value(config))
            .col_expr(
                connections::Column::ConfigFingerprint,
                Expr::value(fingerprint),
            );
    }
    if update.exec(&tx).await?.rows_affected == 0 {
        return Err(ApiError::conflict("连接已变化，请刷新后重试。"));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "repositories",
        &id,
        json!({"connection":true}),
    )
    .await?;
    let row = owned_connection(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok(Json(connection_json(&row)))
}

pub async fn delete_connection(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    scope.require("repositories.delete")?;
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = owned_connection(&tx, &scope, id).await?;
    let pipelines = crate::entities::pipelines::Entity::find()
        .filter(crate::entities::pipelines::Column::WorkspaceId.eq(scope.workspace_id))
        .all(&tx)
        .await?;
    for pipeline in pipelines {
        if crate::actions::document(&pipeline.definition).is_some_and(|document| {
            document
                .checkout_bindings
                .iter()
                .any(|binding| Id::reference(&binding.connection_id).is_ok_and(|value| value == id))
        }) {
            return Err(ApiError::conflict(
                "流水线源码节点仍在使用该连接，请先解除引用。",
            ));
        }
    }
    for run in crate::entities::pipeline_runs::Entity::find()
        .filter(crate::entities::pipeline_runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::pipeline_runs::Column::State.is_in([
            "queued",
            "running",
            "cancelling",
        ]))
        .all(&tx)
        .await?
    {
        if crate::actions::snapshot(&run.definition).is_some_and(|snapshot| {
            snapshot.checkouts.iter().any(|checkout| {
                Id::reference(&checkout.binding.connection_id).is_ok_and(|value| value == id)
            })
        }) {
            return Err(ApiError::conflict("代码连接仍有流水线任务尚未结束。"));
        }
    }
    // OAuth refresh ownership is retained while an existing source binding uses it.
    if row.token_config.is_none() {
        use sea_orm::PaginatorTrait;
        let used = crate::entities::repositories::Entity::find()
            .filter(crate::entities::repositories::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(crate::entities::repositories::Column::CredentialId.eq(row.credential_id))
            .count(&tx)
            .await?;
        if used > 0 {
            return Err(ApiError::conflict("已有仓库使用该网页授权，请先解除引用。"));
        }
    }
    connections::Entity::delete_by_id(row.id).exec(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "repositories",
        &id,
        json!({"connection":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn connection_repositories(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(page): Query<RepositoryPage>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if page.page == 0 || page.page > 10000 {
        return Err(ApiError::bad("页码无效。"));
    }
    let row = owned_connection(&s.db, &scope, id.parse().map_err(|_| ApiError::missing())?).await?;
    if row.provider == "git" {
        return Ok(Json(json!({"items":[],"page":page.page,"has_more":false})));
    }
    let config = connection_provider(&s, &s.db, &row).await?;
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
    let token = row
        .token_config
        .as_deref()
        .map(serde_json::from_str::<TokenConfig>)
        .transpose()
        .map_err(|_| ApiError::bad("连接配置损坏。"))?;
    let mut result = remote_repositories(
        &row.provider,
        &config,
        &secret,
        token.is_some(),
        token
            .as_ref()
            .map(|v| v.organization.as_str())
            .unwrap_or(""),
        page.page,
    )
    .await?;
    result["connection_id"] = json!(Id::from(row.id));
    Ok(Json(result))
}

fn validate_git_credential(address: &str, kind: &str) -> Result<()> {
    let ssh = address.starts_with("git@") || address.starts_with("ssh://");
    if (kind == "ssh-key") != ssh {
        return Err(ApiError::bad(
            "SSH地址需使用SSH私钥，HTTP(S)地址需使用账号密码或令牌。",
        ));
    }
    if ssh {
        let normalized = address
            .strip_prefix("git@")
            .and_then(|v| v.split_once(':'))
            .map(|(host, path)| format!("ssh://git@{host}/{path}"))
            .unwrap_or_else(|| address.to_owned());
        let parsed =
            reqwest::Url::parse(&normalized).map_err(|_| ApiError::bad("Git地址无效。"))?;
        if parsed.host_str().is_none()
            || parsed.password().is_some()
            || address.chars().any(char::is_control)
        {
            return Err(ApiError::bad("Git地址无效。"));
        }
    } else {
        parse_endpoint(address)?;
    }
    Ok(())
}

/// Resolve a selected connection without trusting client-supplied credential IDs.
pub async fn resolve_connection(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    connection_id: Id,
    address: &str,
) -> Result<(String, Id)> {
    let row = owned_connection(conn, scope, connection_id).await?;
    resolve_connection_row(s, conn, scope, row, address).await
}

/// Execution of a saved binding follows workspace permissions; editing still requires ownership.
pub async fn resolve_saved_connection(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    connection_id: Id,
    address: &str,
) -> Result<(String, Id)> {
    let row = connections::Entity::find_by_id(i64::from(connection_id))
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    resolve_connection_row(s, conn, scope, row, address).await
}

async fn resolve_connection_row(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    row: connections::Model,
    address: &str,
) -> Result<(String, Id)> {
    if row.state != "connected" {
        return Err(ApiError::bad("代码连接授权已失效，请重新授权。"));
    }
    let credential = Id::from(row.credential_id);
    if credential != Id::from(0) {
        let auth = credentials::Entity::find_by_id(row.credential_id)
            .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(|| ApiError::bad("所选凭据不存在于当前工作空间。"))?;
        validate_git_credential(address, &auth.kind)?;
    } else {
        parse_endpoint(address)?;
    }
    if row.provider != "git" {
        let config = connection_provider(s, conn, &row).await?;
        validate_origin(&config.base, address)?;
    }
    Ok((row.provider, credential))
}

fn validate_origin(expected: &reqwest::Url, address: &str) -> Result<()> {
    let actual = parse_endpoint(address)?;
    if actual.host_str() != expected.host_str()
        || actual.port_or_known_default() != expected.port_or_known_default()
        || (expected.scheme() == "https" && actual.scheme() != "https")
    {
        return Err(ApiError::bad("仓库地址与所选代码平台不匹配。"));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct BranchQuery {
    url: String,
    #[serde(default = "first_page")]
    page: u32,
}

pub async fn connection_branches(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Query(query): Query<BranchQuery>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if query.page == 0 || query.page > 10000 {
        return Err(ApiError::bad("页码无效。"));
    }
    let id: Id = id.parse().map_err(|_| ApiError::missing())?;
    resolve_connection(&s, &s.db, &scope, id, &query.url).await?;
    let row = owned_connection(&s.db, &scope, id).await?;
    if row.provider == "git" {
        return Err(ApiError::bad("普通Git请填写分支或引用。"));
    }
    let config = connection_provider(&s, &s.db, &row).await?;
    let source = parse_endpoint(&query.url)?;
    let path = source
        .path()
        .strip_prefix(config.base.path().trim_end_matches('/'))
        .unwrap_or(source.path())
        .trim_start_matches('/')
        .trim_end_matches(".git");
    if path.is_empty() {
        return Err(ApiError::bad("请选择有效仓库。"));
    }
    // Url::path is already escaped. Decode before pushing API path segments to
    // avoid turning %E4 into %25E4 for Unicode, spaces and literal percent signs.
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| ApiError::bad("仓库路径编码无效。"))?;
    let path = path.as_ref();
    let token = row
        .token_config
        .as_deref()
        .map(serde_json::from_str::<TokenConfig>)
        .transpose()
        .map_err(|_| ApiError::bad("连接配置损坏。"))?;
    let mut url = config.api.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| ApiError::bad("API地址无效。"))?;
        segments.pop_if_empty();
        match row.provider.as_str() {
            "gitlab" => {
                segments
                    .push("projects")
                    .push(path)
                    .push("repository")
                    .push("branches");
            }
            "codeup" => {
                segments.extend(["oapi", "v1", "codeup"]);
                let organization = token
                    .as_ref()
                    .map(|v| v.organization.as_str())
                    .unwrap_or("");
                if !organization.is_empty() {
                    segments.push("organizations").push(organization);
                }
                segments.push("repositories").push(path).push("branches");
            }
            _ => {
                segments
                    .push("repos")
                    .extend(path.split('/'))
                    .push("branches");
            }
        }
    }
    url.query_pairs_mut()
        .append_pair("page", &query.page.to_string())
        .append_pair(
            if row.provider == "codeup" {
                "perPage"
            } else if row.provider == "gitea" {
                "limit"
            } else {
                "per_page"
            },
            "50",
        );
    ensure_fresh(&s, scope.workspace_id, row.credential_id.into()).await?;
    let (_, _, secret) =
        crate::credentials::unseal(&s, &scope.workspace_id, &row.credential_id.into()).await?;
    let response = authorize_request(http()?.get(url), &row.provider, &secret, token.is_some())
        .send()
        .await
        .map_err(|_| ApiError::bad("读取仓库分支失败。"))?;
    let has_more = (row.provider == "codeup")
        .then(|| codeup_has_more(&response, query.page))
        .flatten();
    let value = repository_response(response, &row.provider).await?;
    let rows = value
        .as_array()
        .ok_or_else(|| ApiError::bad("代码平台返回了无效分支列表。"))?;
    let items = rows
        .iter()
        .filter_map(|row| {
            row.get("name")
                .or_else(|| row.get("branchName"))
                .and_then(Value::as_str)
        })
        .map(|name| json!({"name":name}))
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"items":items,"page":query.page,"has_more":has_more.unwrap_or(rows.len()==50)}),
    ))
}
