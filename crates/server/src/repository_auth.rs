//! Browser authorization for external repositories. App secrets stay server-side.
use crate::{
    db,
    entities::{
        credentials, repository_connections as connections, repository_oauth_flows as flows,
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
    response::Redirect,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
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
fn provider(kind: &str) -> Result<Provider> {
    let default = match kind {
        "github" => "https://github.com",
        "gitlab" => "https://gitlab.com",
        "gitee" => "https://gitee.com",
        _ => return Err(ApiError::bad("该平台请使用API令牌授权。")),
    };
    let prefix = format!("REPOSITORY_OAUTH_{}", kind.to_ascii_uppercase());
    let client_id = std::env::var(format!("{prefix}_CLIENT_ID")).unwrap_or_default();
    let client_secret = std::env::var(format!("{prefix}_CLIENT_SECRET")).unwrap_or_default();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(ApiError::bad(
            "部署端尚未配置该平台的网页授权应用，可先使用API令牌。",
        ));
    }
    let parse = |value: &str| -> Result<reqwest::Url> {
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
    };
    let base = parse(&std::env::var(format!("{prefix}_BASE_URL")).unwrap_or(default.into()))?;
    let api_default = match kind {
        "github" if base.host_str() == Some("github.com") => "https://api.github.com".to_owned(),
        "github" => endpoint(&base, "api/v3")?.to_string(),
        "gitlab" => endpoint(&base, "api/v4")?.to_string(),
        _ => endpoint(&base, "api/v5")?.to_string(),
    };
    let api = parse(&std::env::var(format!("{prefix}_API_URL")).unwrap_or(api_default))?;
    Ok(Provider {
        client_id,
        client_secret,
        base,
        api,
    })
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
fn callback(s: &AppState) -> Result<String> {
    let base = std::env::var("REPOSITORY_OAUTH_PUBLIC_URL")
        .unwrap_or_else(|_| format!("http://{}", s.bind));
    let url =
        reqwest::Url::parse(&base).map_err(|_| ApiError::bad("请配置网页授权的公开访问地址。"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url
            .host_str()
            .is_none_or(|h| matches!(h, "0.0.0.0" | "[::]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad(
            "请使用REPOSITORY_OAUTH_PUBLIC_URL配置有效公开访问地址。",
        ));
    }
    Ok(endpoint(&url, "api/repository-auth/callback")?.to_string())
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
    for kind in ["github", "gitlab", "gitee", "codeup"] {
        let row = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(connections::Column::UserId.eq(scope.user.id))
            .filter(connections::Column::Provider.eq(kind))
            .one(&s.db)
            .await?;
        let configured = provider(kind).is_ok() && callback(&s).is_ok();
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
        json!({"items":rows,"total":4,"page":1,"page_size":20}),
    ))
}
pub async fn start(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, kind)): Path<(String, String)>,
) -> Result<Json<Value>> {
    scope.require("credentials.create")?;
    let config = provider(&kind)?;
    let redirect = callback(&s)?;
    let state = random_token();
    let verifier = random_token();
    let id = s.next_id()?;
    let now = db::now();
    let proof = json!({"verifier":verifier,"redirect":redirect,"client_id":config.client_id,"base":config.base.as_str(),"config":config_fingerprint(&config)});
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
    let workspace = crate::entities::workspaces::Entity::find_by_id(pending.workspace_id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?
        .slug;
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
    if !b.error.is_empty() || b.code.is_empty() {
        return Ok(Redirect::to(&format!(
            "/{workspace}/build/repository-auth?authorization=cancelled"
        )));
    }
    let config = provider(&flow.provider)?;
    let proof: Value = serde_json::from_str(
        &s.vault
            .open_secret(&flow.proof, &flow_aad(scope.workspace_id, flow.id.into()))?,
    )
    .map_err(|_| ApiError::bad("授权状态损坏，请重新授权。"))?;
    if proof["client_id"].as_str() != Some(&config.client_id)
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
                if flow.provider == "github" {
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
    let fresh = Scope::for_actor(&s, scope.workspace_id, scope.user.id).await?;
    fresh.require("credentials.create")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let now = db::now();
    let existing = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::Provider.eq(&flow.provider))
        .one(&tx)
        .await?;
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
            name: Set(format!("{} · {}", flow.provider, account)
                .chars()
                .take(60)
                .collect()),
            kind: Set("token".into()),
            username: Set(account.into()),
            secret: Set(sealed),
            description: Set("网页授权管理；请通过仓库授权页面重新授权。".into()),
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
        provider: Set(flow.provider),
        credential_id: Set(credential.into()),
        account: Set(account.into()),
        refresh_secret: Set(match refresh {
            Some(v) => s.vault.seal(v, &connection_aad(scope.workspace_id, id))?,
            None => String::new(),
        }),
        expires_at: Set(expires),
        state: Set("connected".into()),
        last_error: Set(String::new()),
        config_fingerprint: Set(config_fingerprint(&config)),
        client_id: Set(config.client_id),
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
    Ok(Redirect::to(&format!(
        "/{workspace}/build/repository-auth?authorization=connected"
    )))
}

/// Called only when an execution needs a credential, not during metadata reads.
pub async fn ensure_fresh(s: &AppState, workspace: Id, credential: Id) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    let (row, version) = loop {
        let tx = crate::variables::transaction(s, workspace).await?;
        let Some(row) = connections::Entity::find()
            .filter(connections::Column::WorkspaceId.eq(workspace))
            .filter(connections::Column::CredentialId.eq(credential))
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
        let config = provider(&row.provider)?;
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
                    if row.provider == "github" {
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
    workspace: Id,
    credential: Id,
    address: &str,
    api: bool,
) -> Result<()> {
    let Some(row) = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(workspace))
        .filter(connections::Column::CredentialId.eq(credential))
        .one(&s.db)
        .await?
    else {
        return Ok(());
    };
    let config = provider(&row.provider)?;
    require_same_config(&config, &row.config_fingerprint)?;
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
