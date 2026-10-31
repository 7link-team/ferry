//! Authenticated provider events and per-connection notification settings.
use crate::{
    db,
    entities::{
        repository_connections as connections, repository_events as events,
        repository_monitors as monitors, repository_poll_states as polls,
    },
    error::{ApiError, Result},
    repository_auth::owned_connection,
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, Mac};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set,
    sea_query::{Expr, OnConflict},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use subtle::ConstantTimeEq;

mod polling;
pub use polling::scheduler;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Push,
    Poll,
}
fn default_poll_minutes() -> u32 {
    1
}

const KINDS: &[&str] = &[
    "repository.created",
    "repository.deleted",
    "branch.created",
    "branch.deleted",
    "push",
    "pull_request",
    "merge",
    "tag.created",
    "tag.deleted",
];
const HISTORY: Spec = Spec {
    search: &["repository", "reference", "actor"],
    filters: &["event"],
    sortable: &["created_at", "repository", "event"],
    default_sort: "-created_at",
};
fn supported_events(provider: &str, mode: Mode) -> Vec<&'static str> {
    KINDS
        .iter()
        .copied()
        .filter(|kind| {
            provider != "git"
                && (mode == Mode::Poll || provider != "codeup" || !kind.starts_with("repository."))
        })
        .collect()
}

#[derive(Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "default_poll_minutes")]
    poll_interval_minutes: u32,
    enabled: bool,
    provider: String,
    all_repositories: bool,
    repositories: Vec<String>,
    events: Vec<String>,
    secret: String,
    webhook: String,
}
#[derive(Deserialize)]
pub struct Body {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "default_poll_minutes")]
    poll_interval_minutes: u32,
    version: i64,
    enabled: bool,
    all_repositories: bool,
    repositories: String,
    events: Vec<String>,
    #[serde(default)]
    secret: String,
    #[serde(default)]
    webhook: String,
    #[serde(default)]
    clear_webhook: bool,
}
fn aad(workspace: i64, id: i64) -> String {
    format!("repository-monitor:{workspace}:{id}")
}
fn decode(s: &AppState, row: &monitors::Model) -> Result<Config> {
    serde_json::from_str(
        &s.vault
            .open_secret(&row.config, &aad(row.workspace_id, row.id))?,
    )
    .map_err(|_| ApiError::bad("监控配置无法读取。"))
}
fn view(row: Option<&monitors::Model>, config: &Config, connection: &connections::Model) -> Value {
    json!({"id":Id::from(connection.id),"name":"监控通知","provider":connection.provider,"mode":config.mode,"poll_interval_minutes":config.poll_interval_minutes.max(1),"enabled":config.enabled && config.provider==connection.provider,"all_repositories":config.all_repositories,"repositories":config.repositories.join("\n"),"events":config.events,"has_secret":!config.secret.is_empty(),"has_webhook":!config.webhook.is_empty(),"version":row.map_or(0,|r|r.version),"callback_path":format!("/api/repository-events/{}",connection.id),"provider_changed":row.is_some() && config.provider!=connection.provider,"capability_note":if config.mode == Mode::Poll {"使用当前连接凭据比较完整快照。首次检查只建立基线；间隔内发生后又撤销的操作可能无法捕获。仓库移除也可能由权限变化引起；操作人无法可靠确定时不显示。"} else if connection.provider=="codeup"{"Codeup 仓库 Webhook 支持推送、标签和合并请求，暂不提供仓库新增/删除事件。全部仓库表示接收范围，仍需在各仓库配置 Webhook。"}else{"仓库新增/删除需要平台提供组织级或系统级事件订阅；普通 Git 不提供平台事件。"}})
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let connection = owned_connection(&s.db, &scope, id).await?;
    let row = monitors::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?;
    let config = match &row {
        Some(row) => decode(&s, row)?,
        None => Config {
            all_repositories: true,
            poll_interval_minutes: default_poll_minutes(),
            events: supported_events(&connection.provider, Mode::Push)
                .iter()
                .map(|v| v.to_string())
                .collect(),
            ..Default::default()
        },
    };
    let mut result = view(row.as_ref(), &config, &connection);
    let poll = polls::Entity::find_by_id(connection.id).one(&s.db).await?;
    result["last_polled_at"] = json!(
        poll.as_ref()
            .filter(|p| p.last_polled_at > 0)
            .map(|p| p.last_polled_at)
    );
    result["next_poll_at"] = json!(
        poll.as_ref()
            .filter(|_| config.enabled && config.mode == Mode::Poll)
            .map(|p| p.next_poll_at)
    );
    result["last_poll_error"] = json!(poll.as_ref().map(|p| p.last_error.as_str()).unwrap_or(""));
    result["poll_status"] = json!(if !config.enabled {
        "未启用"
    } else if poll.as_ref().is_some_and(|p| !p.last_error.is_empty()) {
        "检查失败，保留上次基线"
    } else if poll.as_ref().is_some_and(|p| !p.snapshot.is_empty()) {
        "已建立基线"
    } else {
        "等待首次检查"
    });
    Ok(Json(result))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("repositories.update")?;
    if body.poll_interval_minutes < 1 {
        return Err(ApiError::bad("主动轮询间隔不得小于 1 分钟。"));
    }
    if body.enabled && body.mode == Mode::Poll {
        scope.require("credentials.read")?;
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let connection = owned_connection(&tx, &scope, id).await?;
    if !matches!(
        connection.provider.as_str(),
        "github" | "gitlab" | "gitea" | "gitee" | "codeup"
    ) {
        return Err(ApiError::bad(
            "普通 Git 连接不提供平台事件；请使用代码平台连接。",
        ));
    }
    let old = monitors::Entity::find_by_id(i64::from(id)).one(&tx).await?;
    if old.as_ref().map_or(0, |r| r.version) != body.version {
        return Err(ApiError::conflict("监控配置已变更，请刷新后重试。"));
    }
    let previous = match &old {
        Some(row) => decode(&s, row)?,
        None => Config::default(),
    };
    let mut repositories = body
        .repositories
        .lines()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    repositories.sort();
    repositories.dedup();
    if repositories.len() > 1000
        || repositories
            .iter()
            .any(|v| v.len() > 400 || !v.contains('/') || v.chars().any(char::is_control))
    {
        return Err(ApiError::bad(
            "指定仓库须为完整命名空间/仓库路径，每行一个，最多 1000 个。",
        ));
    }
    if body.events.len() > KINDS.len()
        || body
            .events
            .iter()
            .any(|v| !supported_events(&connection.provider, body.mode).contains(&v.as_str()))
    {
        return Err(ApiError::bad("监控事件类型无效。"));
    }
    let secret = if body.secret.is_empty() {
        previous.secret
    } else {
        body.secret
    };
    if !secret.is_empty()
        && (secret.len() < 16
            || secret.len() > 256
            || !secret.is_ascii()
            || secret.chars().any(char::is_control))
    {
        return Err(ApiError::bad("事件密钥须为 16–256 位 ASCII 字符。"));
    }
    let webhook = if body.clear_webhook {
        String::new()
    } else if body.webhook.is_empty() {
        previous.webhook
    } else {
        body.webhook
    };
    if !webhook.is_empty() {
        let url = reqwest::Url::parse(&webhook).map_err(|_| ApiError::bad("Webhook 地址无效。"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || webhook.len() > 4096
        {
            return Err(ApiError::bad(
                "Webhook 须为不带用户凭据或片段的 HTTP(S) 地址。",
            ));
        }
    }
    if body.enabled
        && ((body.mode == Mode::Push && secret.is_empty())
            || body.events.is_empty()
            || (!body.all_repositories && repositories.is_empty()))
    {
        return Err(ApiError::bad(
            "启用监控前请设置事件类型和仓库范围；事件推送还需设置验证密钥。",
        ));
    }
    let config = Config {
        mode: body.mode,
        poll_interval_minutes: body.poll_interval_minutes,
        enabled: body.enabled,
        provider: connection.provider.clone(),
        all_repositories: body.all_repositories,
        repositories,
        events: body.events,
        secret,
        webhook,
    };
    let encoded = serde_json::to_string(&config).map_err(|_| ApiError::bad("监控配置无效。"))?;
    let row = monitors::Model {
        id: i64::from(id),
        workspace_id: i64::from(scope.workspace_id),
        config: s
            .vault
            .seal(&encoded, &aad(i64::from(scope.workspace_id), i64::from(id)))?,
        version: body.version + 1,
        created_at: old.as_ref().map_or(db::now(), |r| r.created_at),
        updated_at: db::now(),
    };
    let active: monitors::ActiveModel = row.clone().into();
    monitors::Entity::insert(active)
        .on_conflict(
            OnConflict::column(monitors::Column::Id)
                .update_columns([
                    monitors::Column::Config,
                    monitors::Column::Version,
                    monitors::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&tx)
        .await?;
    polls::Entity::delete_by_id(connection.id).exec(&tx).await?;
    if config.enabled && config.mode == Mode::Poll {
        polls::Entity::insert(polls::ActiveModel {
            id: Set(connection.id),
            workspace_id: Set(connection.workspace_id),
            config_version: Set(row.version),
            snapshot: Set(String::new()),
            last_polled_at: Set(0),
            next_poll_at: Set(db::now()),
            last_error: Set(String::new()),
        })
        .exec_without_returning(&tx)
        .await?;
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "repositories",
        &id,
        json!({"monitoring":true,"enabled":config.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(view(Some(&row), &config, &connection)))
}

fn header<'a>(headers: &'a HeaderMap, key: &str) -> &'a str {
    headers.get(key).and_then(|v| v.to_str().ok()).unwrap_or("")
}
fn authenticate(provider: &str, secret: &str, headers: &HeaderMap, bytes: &[u8]) -> bool {
    if secret.is_empty() {
        return false;
    }
    if matches!(provider, "github" | "gitea") {
        let signature = if provider == "gitea" {
            header(headers, "x-gitea-signature")
        } else {
            header(headers, "x-hub-signature-256")
                .strip_prefix("sha256=")
                .unwrap_or("")
        };
        let Ok(signature) = hex::decode(signature) else {
            return false;
        };
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(bytes);
        mac.verify_slice(&signature).is_ok()
    } else {
        let supplied = match provider {
            "gitlab" => header(headers, "x-gitlab-token"),
            "gitee" => header(headers, "x-gitee-token"),
            "codeup" => header(headers, "x-codeup-token"),
            _ => return false,
        };
        Sha256::digest(secret.as_bytes())
            .ct_eq(&Sha256::digest(supplied.as_bytes()))
            .into()
    }
}
#[derive(Debug, Serialize)]
struct Event {
    event: String,
    repository: String,
    reference: String,
    actor: String,
    action: String,
}
fn text(value: &Value, paths: &[&str]) -> String {
    paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .find(|value| !value.is_empty())
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(400)
        .collect()
}
fn normalize(provider: &str, headers: &HeaderMap, payload: &Value) -> Option<Event> {
    let kind = match provider {
        "github" => header(headers, "x-github-event"),
        "gitea" => header(headers, "x-gitea-event"),
        "gitlab" => header(headers, "x-gitlab-event"),
        "gitee" => header(headers, "x-gitee-event"),
        "codeup" => header(headers, "codeup-event"),
        _ => return None,
    };
    let kind = if kind.is_empty() {
        text(payload, &["/object_kind", "/objectKind", "/event_name"])
    } else {
        kind.to_owned()
    }
    .to_ascii_lowercase();
    let action = text(payload, &["/object_attributes/action", "/action"]);
    let reference = text(
        payload,
        &[
            "/ref",
            "/object_attributes/source_branch",
            "/pull_request/head/ref",
        ],
    );
    let zero = |path: &str| {
        payload
            .pointer(path)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty() && v.chars().all(|c| c == '0'))
    };
    let is_tag = reference.starts_with("refs/tags/") || text(payload, &["/ref_type"]) == "tag";
    let event = match kind.as_str() {
        "push" | "push hook" | "tag_push" | "tag push hook" => {
            if payload["deleted"] == true || zero("/after") {
                if is_tag {
                    "tag.deleted"
                } else {
                    "branch.deleted"
                }
            } else if payload["created"] == true || zero("/before") {
                if is_tag {
                    "tag.created"
                } else {
                    "branch.created"
                }
            } else {
                "push"
            }
        }
        "create" => {
            if is_tag {
                "tag.created"
            } else {
                "branch.created"
            }
        }
        "delete" => {
            if is_tag {
                "tag.deleted"
            } else {
                "branch.deleted"
            }
        }
        "repository" => match action.as_str() {
            "created" => "repository.created",
            "deleted" => "repository.deleted",
            _ => return None,
        },
        "project_create" | "repository_create" => "repository.created",
        "project_destroy" | "repository_delete" => "repository.deleted",
        "pull_request" | "pull request hook" | "merge_request" | "merge request hook" => {
            if payload.pointer("/pull_request/merged") == Some(&Value::Bool(true))
                || action == "merge"
                || text(payload, &["/object_attributes/state"]) == "merged"
            {
                "merge"
            } else {
                "pull_request"
            }
        }
        _ => return None,
    };
    let mut repository = text(
        payload,
        &[
            "/repository/full_name",
            "/project/path_with_namespace",
            "/repository/path_with_namespace",
            "/path_with_namespace",
            "/repository/pathWithNamespace",
            "/project/pathWithNamespace",
        ],
    );
    if repository.is_empty() {
        let url = text(
            payload,
            &[
                "/repository/homepage",
                "/project/web_url",
                "/repository/html_url",
                "/repository/git_http_url",
                "/object_attributes/target/web_url",
                "/object_attributes/target/http_url",
            ],
        );
        if let Ok(url) = reqwest::Url::parse(&url) {
            repository = percent_encoding::percent_decode_str(url.path())
                .decode_utf8()
                .ok()?
                .trim_matches('/')
                .trim_end_matches(".git")
                .chars()
                .take(400)
                .collect();
        }
    }
    if repository.is_empty() {
        return None;
    }
    Some(Event {
        event: event.into(),
        repository,
        reference,
        actor: text(
            payload,
            &[
                "/sender/login",
                "/user/username",
                "/user_name",
                "/pusher/name",
            ],
        ),
        action,
    })
}
fn event_json(row: events::Model) -> Value {
    json!({"id":Id::from(row.id),"event":row.event,"repository":row.repository,"reference":row.reference,"actor":row.actor,"action":row.action,"delivery":row.delivery,"created_at":row.created_at})
}
pub async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    owned_connection(&s.db, &scope, id).await?;
    let query = ListQuery::parse(&HISTORY, &raw)?;
    let (rows, total) = resource::list::<events::Entity, events::Model>(
        &s.db,
        &HISTORY,
        events::Entity::find().filter(events::Column::ConnectionId.eq(id)),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(event_json).collect::<Vec<_>>(),"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
pub async fn receive(
    State(s): State<AppState>,
    Path(id): Path<Id>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<StatusCode> {
    let connection = connections::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let monitor = monitors::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let config = decode(&s, &monitor)?;
    if !config.enabled
        || config.mode != Mode::Push
        || config.provider != connection.provider
        || connection.workspace_id != monitor.workspace_id
    {
        return Err(ApiError::missing());
    }
    if !authenticate(&connection.provider, &config.secret, &headers, &bytes) {
        return Err(ApiError::forbidden());
    }
    Scope::for_actor(
        &s,
        connection.workspace_id.into(),
        connection.user_id.into(),
    )
    .await?
    .require("repositories.read")?;
    let payload: Value =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("事件须为有效 JSON。"))?;
    let Some(event) = normalize(&connection.provider, &headers, &payload) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let selected = config.events.contains(&event.event)
        || (event.event == "merge" && config.events.iter().any(|kind| kind == "pull_request"))
        || (matches!(
            event.event.as_str(),
            "branch.created" | "branch.deleted" | "tag.created" | "tag.deleted"
        ) && payload.get("before").is_some()
            && config.events.iter().any(|kind| kind == "push"));
    if !selected || (!config.all_repositories && !config.repositories.contains(&event.repository)) {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Delivery headers are not covered by provider payload signatures. Hash the
    // authenticated body so changing a delivery ID cannot replay a notification.
    let mut digest = Sha256::new();
    digest.update(connection.provider.as_bytes());
    digest.update(&bytes);
    let delivery_key = hex::encode(digest.finalize());
    let tx = crate::variables::transaction(&s, connection.workspace_id.into()).await?;
    let current = monitors::Entity::find_by_id(connection.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.version != monitor.version
        || connections::Entity::find_by_id(connection.id)
            .one(&tx)
            .await?
            .is_none()
    {
        return Err(ApiError::conflict("监控配置已变更，请重新发送事件。"));
    }
    let event_id = record(&s, &tx, &connection, &config, &event, &delivery_key).await?;
    tx.commit().await?;
    if let Some(event_id) = event_id {
        deliver(&s, &connection, &config, event_id, &event).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn record(
    s: &AppState,
    tx: &impl sea_orm::ConnectionTrait,
    connection: &connections::Model,
    config: &Config,
    event: &Event,
    delivery_key: &str,
) -> Result<Option<Id>> {
    // The shared workspace transaction serializes admission across replicas.
    // Check under that lock instead of relying on dialect-specific conflict
    // affected-row counts (MySQL's no-op update has different semantics).
    if events::Entity::find()
        .filter(events::Column::ConnectionId.eq(connection.id))
        .filter(events::Column::DeliveryKey.eq(delivery_key))
        .one(tx)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let event_id = s.next_id()?;
    let delivery = if config.webhook.is_empty() {
        "未配置通知地址"
    } else {
        "投递中；服务中断不会自动重发"
    };
    events::Entity::insert(events::ActiveModel {
        id: Set(event_id.into()),
        workspace_id: Set(connection.workspace_id),
        connection_id: Set(connection.id),
        delivery_key: Set(delivery_key.into()),
        event: Set(event.event.clone()),
        repository: Set(event.repository.clone()),
        reference: Set(event.reference.clone()),
        actor: Set(event.actor.clone()),
        action: Set(event.action.clone()),
        delivery: Set(delivery.into()),
        created_at: Set(db::now()),
    })
    .exec_without_returning(tx)
    .await?;
    Ok(Some(event_id))
}

async fn deliver(
    s: &AppState,
    connection: &connections::Model,
    config: &Config,
    event_id: Id,
    event: &Event,
) -> Result<()> {
    if !config.webhook.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ApiError::bad("通知客户端不可用。"))?;
        let delivery=match client.post(&config.webhook).json(&json!({"id":event_id,"workspace_id":Id::from(connection.workspace_id),"connection_id":Id::from(connection.id),"provider":connection.provider,"event":event.event,"data":event})).send().await {Ok(response) if response.status().is_success()=>format!("已投递 HTTP {}",response.status().as_u16()),Ok(response)=>format!("投递失败 HTTP {}；不自动重试",response.status().as_u16()),Err(_)=>"投递失败或超时；不自动重试".into()};
        events::Entity::update_many()
            .col_expr(events::Column::Delivery, Expr::value(delivery))
            .filter(events::Column::Id.eq(event_id))
            .exec(&s.db)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(name: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        headers
    }
    #[test]
    fn signed_payloads_reject_mutation_wrong_key_and_missing_signature() {
        let body = br#"{"ref":"refs/heads/main"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"private-fixture-secret").unwrap();
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());
        for provider in ["github", "gitea"] {
            let h = if provider == "github" {
                headers("x-hub-signature-256", &format!("sha256={signature}"))
            } else {
                headers("x-gitea-signature", &signature)
            };
            assert!(authenticate(provider, "private-fixture-secret", &h, body));
            assert!(!authenticate(provider, "wrong-private-secret", &h, body));
            assert!(!authenticate(
                provider,
                "private-fixture-secret",
                &h,
                b"tampered"
            ));
            assert!(!authenticate(
                provider,
                "private-fixture-secret",
                &HeaderMap::new(),
                body
            ));
        }
        for (provider, header) in [
            ("gitlab", "x-gitlab-token"),
            ("gitee", "x-gitee-token"),
            ("codeup", "x-codeup-token"),
        ] {
            assert!(authenticate(
                provider,
                "private-fixture-secret",
                &headers(header, "private-fixture-secret"),
                body
            ));
            assert!(!authenticate(
                provider,
                "private-fixture-secret",
                &headers(header, "wrong-token"),
                body
            ));
            assert!(!authenticate(provider, "", &HeaderMap::new(), body));
        }
    }
    #[test]
    fn codeup_official_old_and_new_webhook_shapes() {
        let h = headers("codeup-event", "merge_request");
        for repository in [
            json!({"repository":{"homepage":"https://codeup.example/org/team/repo"}}),
            json!({"object_attributes":{"target":{"web_url":"https://codeup.example/org/team/repo"}}}),
        ] {
            let mut payload = repository;
            let attrs = payload
                .as_object_mut()
                .unwrap()
                .entry("object_attributes")
                .or_insert(json!({}));
            attrs["action"] = json!("merge");
            attrs["state"] = json!("merged");
            attrs["source_branch"] = json!("feature/test");
            let event = normalize("codeup", &h, &payload).unwrap();
            assert_eq!(event.repository, "org/team/repo");
            assert_eq!(event.event, "merge");
            assert_eq!(event.reference, "feature/test");
        }
        let payload = json!({"object_kind":"push","repository":{"git_http_url":"https://codeup.example/org/team/repo.git"},"ref":"refs/heads/main","before":"abc","after":"def"});
        let event = normalize("codeup", &HeaderMap::new(), &payload).unwrap();
        assert_eq!(event.repository, "org/team/repo");
        assert_eq!(event.event, "push");
        assert!(!authenticate(
            "codeup",
            "private-fixture-secret",
            &headers("x-gitlab-token", "private-fixture-secret"),
            b"{}"
        ));
        assert!(
            !supported_events("codeup", Mode::Push)
                .iter()
                .any(|event| event.starts_with("repository."))
        );
    }
    #[test]
    fn webhook_url_paths_match_decoded_repository_names() {
        for (path, expected) in [
            (
                "org/team/%E4%B8%AD%E6%96%87%20+%20repo.git",
                "org/team/中文 + repo",
            ),
            ("org/team/100%2520.git", "org/team/100%20"),
        ] {
            let payload = json!({"object_kind":"push","repository":{"homepage":format!("https://codeup.example/{path}")},"ref":"refs/heads/main","before":"abc","after":"def"});
            let event = normalize("codeup", &HeaderMap::new(), &payload).unwrap();
            assert_eq!(event.repository, expected);
        }
        let invalid = json!({"object_kind":"push","repository":{"homepage":"https://codeup.example/org/%FF"}});
        assert!(normalize("codeup", &HeaderMap::new(), &invalid).is_none());
    }
    #[test]
    fn platform_event_shapes_preserve_real_repository_reference_and_actions() {
        let repo = json!({"full_name":"org/team/repo"});
        for (provider, key, event) in [
            ("github", "x-github-event", "push"),
            ("gitea", "x-gitea-event", "push"),
            ("gitee", "x-gitee-event", "Push Hook"),
        ] {
            let p = json!({"repository":repo,"ref":"refs/heads/feature/test","before":"a1","after":"b2","sender":{"login":"developer"}});
            let e = normalize(provider, &headers(key, event), &p).unwrap();
            assert_eq!(e.event, "push");
            assert_eq!(e.repository, "org/team/repo");
            assert_eq!(e.reference, "refs/heads/feature/test");
            assert_eq!(e.actor, "developer");
        }
        for provider in ["gitlab", "codeup"] {
            let p = json!({"object_kind":"push","project":{"path_with_namespace":"org/team/repo"},"ref":"refs/heads/feature/test","before":"abc","after":"0000"});
            assert_eq!(
                normalize(provider, &HeaderMap::new(), &p).unwrap().event,
                "branch.deleted"
            );
            let p = json!({"object_kind":"merge_request","project":{"path_with_namespace":"org/team/repo"},"object_attributes":{"action":"merge","source_branch":"feature/test","state":"merged"}});
            let event = normalize(provider, &HeaderMap::new(), &p).unwrap();
            assert_eq!(event.event, "merge");
            assert_eq!(event.reference, "feature/test");
        }
        let p = json!({"repository":repo,"ref":"release/v1","ref_type":"tag"});
        assert_eq!(
            normalize("github", &headers("x-github-event", "create"), &p)
                .unwrap()
                .event,
            "tag.created"
        );
        assert!(normalize("github", &headers("x-github-event", "ping"), &json!({})).is_none());
        assert!(
            normalize(
                "github",
                &headers("x-github-event", "push"),
                &json!({"ref":"refs/heads/main"})
            )
            .is_none()
        );
    }
}
