//! Authenticated provider events and per-connection notification settings.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        repository_connections as connections, repository_events as events,
        repository_monitors as monitors, repository_poll_states as polls,
    },
    error::{ApiError, Result},
    repositories::auth::owned_connection,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::State,
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

mod detail;
mod git;
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
fn default_notification_language() -> String {
    crate::i18n::Locale::EnUs.tag().into()
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
    "tag.updated",
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
            if provider == "git" {
                mode == Mode::Poll
                    && matches!(
                        *kind,
                        "push"
                            | "branch.created"
                            | "branch.deleted"
                            | "tag.created"
                            | "tag.deleted"
                            | "tag.updated"
                    )
            } else {
                *kind != "tag.updated"
                    && (mode == Mode::Poll
                        || provider != "codeup"
                        || !kind.starts_with("repository."))
            }
        })
        .collect()
}

#[derive(Clone, Serialize, Deserialize)]
struct NotificationChannel {
    name: String,
    #[serde(default)]
    webhook: String,
    language: String,
    #[serde(default = "channel_enabled")]
    enabled: bool,
    #[serde(default, flatten)]
    options: crate::notifications::channel::Options,
}
fn channel_enabled() -> bool {
    true
}
fn channels(config: &Config) -> Vec<NotificationChannel> {
    config.notification_channels.clone().unwrap_or_else(|| {
        if config.webhook.is_empty() {
            Vec::new()
        } else {
            vec![NotificationChannel {
                name: "Webhook".into(),
                webhook: config.webhook.clone(),
                language: config.notification_language.clone(),
                enabled: true,
                options: Default::default(),
            }]
        }
    })
}

fn validate_channels(channels: &[NotificationChannel]) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    if channels.len() > 20 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.webhookFormat"),
        ));
    }
    for channel in channels {
        if channel.name.trim().is_empty()
            || channel.name.len() > 120
            || !names.insert(channel.name.trim())
            || channel.webhook.len() > 4096
            || channel.options.resolve(&channel.webhook).is_err()
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.monitor.webhookFormat"),
            ));
        }
        if !crate::i18n::Locale::ALL
            .iter()
            .any(|locale| locale.tag() == channel.language)
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.monitor.languageInvalid"),
            ));
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ChannelDelivery {
    name: String,
    language: String,
    result: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "default_poll_minutes")]
    poll_interval_minutes: u32,
    #[serde(default)]
    runner_id: String,
    enabled: bool,
    provider: String,
    all_repositories: bool,
    repositories: Vec<String>,
    events: Vec<String>,
    secret: String,
    webhook: String,
    #[serde(default = "default_notification_language")]
    notification_language: String,
    #[serde(default)]
    notification_channels: Option<Vec<NotificationChannel>>,
}
#[derive(Deserialize)]
pub struct Body {
    #[serde(default)]
    mode: Mode,
    #[serde(default = "default_poll_minutes")]
    poll_interval_minutes: u32,
    #[serde(default)]
    runner_id: String,
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
    #[serde(default)]
    notification_language: Option<String>,
    #[serde(default)]
    notification_channels: Option<Vec<NotificationChannel>>,
}
fn aad(workspace: i64, id: i64) -> String {
    format!("repository-monitor:{workspace}:{id}")
}
fn decode(s: &AppState, row: &monitors::Model) -> Result<Config> {
    serde_json::from_str(
        &s.vault
            .open_secret(&row.config, &aad(row.workspace_id, row.id))?,
    )
    .map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.configRead"),
        )
    })
}
fn view(row: Option<&monitors::Model>, config: &Config, connection: &connections::Model) -> Value {
    json!({"id":Id::from(connection.id),"name":crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.notificationName")),"provider":connection.provider,"mode":config.mode,"poll_interval_minutes":config.poll_interval_minutes.max(1),"runner_id":config.runner_id,"enabled":config.enabled && config.provider==connection.provider,"all_repositories":config.all_repositories,"repositories":config.repositories.join("\n"),"events":config.events,"has_secret":!config.secret.is_empty(),"has_webhook":!config.webhook.is_empty(),"version":row.map_or(0,|r|r.version),"callback_path":format!("/api/repository-events/{}",connection.id),"provider_changed":row.is_some() && config.provider!=connection.provider,"capability_note":if connection.provider == "git" {crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.gitCapability"))} else if config.mode == Mode::Poll {crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.pollCapability"))} else if connection.provider=="codeup"{crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.codeupCapability"))}else{crate::i18n::render(crate::i18n::current_locale(), &crate::i18n::Message::new("repository.monitor.platformCapability"))}})
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
            notification_language: default_notification_language(),
            mode: if connection.provider == "git" {
                Mode::Poll
            } else {
                Mode::Push
            },
            all_repositories: connection.provider != "git",
            poll_interval_minutes: default_poll_minutes(),
            events: supported_events(
                &connection.provider,
                if connection.provider == "git" {
                    Mode::Poll
                } else {
                    Mode::Push
                },
            )
            .iter()
            .map(|v| v.to_string())
            .collect(),
            ..Default::default()
        },
    };
    let mut result = view(row.as_ref(), &config, &connection);
    result["notification_language"] = json!(config.notification_language);
    // Notification URLs can contain credentials; only editors receive the value.
    if scope.permissions.contains("repositories.update") {
        result["webhook"] = json!(config.webhook);
        result["notification_channels"] = json!(channels(&config));
    } else {
        result["notification_channels"] = json!(
            channels(&config)
                .into_iter()
                .map(|mut channel| {
                    if channel.options.provider.is_empty() || channel.options.provider == "auto" {
                        channel.options.provider = reqwest::Url::parse(&channel.webhook)
                            .ok()
                            .map(|url| crate::notifications::channel::provider(&url).to_owned())
                            .unwrap_or_default();
                    }
                    channel.webhook.clear();
                    channel.options.redact();
                    channel
                })
                .collect::<Vec<_>>()
        );
    }
    result["runner_name"] = json!(match config.runner_id.parse::<Id>() {
        Ok(id) => crate::entities::hosts::Entity::find_by_id(i64::from(id))
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(connection.workspace_id))
            .one(&s.db)
            .await?
            .map(|host| host.name)
            .unwrap_or_default(),
        Err(_) => String::new(),
    });
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
    result["last_poll_error"] = json!(crate::i18n::display_stored(
        poll.as_ref().map(|p| p.last_error.as_str()).unwrap_or("")
    ));
    let status_key = if !config.enabled {
        "repository.monitor.disabled"
    } else if poll.as_ref().is_some_and(|p| !p.last_error.is_empty()) {
        "repository.monitor.failed"
    } else if poll.as_ref().is_some_and(|p| !p.snapshot.is_empty()) {
        "repository.monitor.ready"
    } else {
        "repository.monitor.waiting"
    };
    result["poll_status"] = json!(crate::i18n::render(
        crate::i18n::current_locale(),
        &crate::i18n::Message::new(status_key)
    ));
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
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.intervalInvalid"),
        ));
    }
    if body.enabled && body.mode == Mode::Poll {
        scope.require("credentials.read")?;
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let connection = owned_connection(&tx, &scope, id).await?;
    if !matches!(
        connection.provider.as_str(),
        "github" | "gitlab" | "gitea" | "gitee" | "codeup" | "git"
    ) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.platformRequired"),
        ));
    }
    let mut runner_name = String::new();
    if connection.provider == "git" {
        if body.mode != Mode::Poll || body.all_repositories {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.monitor.gitPollOnly"),
            ));
        }
        if body.enabled {
            scope.require("hosts.terminal")?;
            let runner: Id = body.runner_id.parse().map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.monitor.runnerRequired"),
                )
            })?;
            let host = crate::entities::hosts::Entity::find_by_id(i64::from(runner))
                .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(crate::entities::hosts::Column::Connection.eq("runner"))
                .one(&tx)
                .await?
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.monitor.runnerMissing"),
                    )
                })?;
            runner_name = host.name;
        }
    }
    let old = monitors::Entity::find_by_id(i64::from(id)).one(&tx).await?;
    if old.as_ref().map_or(0, |r| r.version) != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.monitor.configChanged"),
        ));
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
        || repositories.iter().any(|v| {
            v.len() > 400
                || (connection.provider != "git" && !v.contains('/'))
                || v.chars().any(char::is_control)
        })
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.repositoriesInvalid"),
        ));
    }
    if connection.provider == "git" {
        for address in &repositories {
            crate::repositories::auth::resolve_connection(&s, &tx, &scope, id, address).await?;
        }
    }
    if body.events.len() > KINDS.len()
        || body
            .events
            .iter()
            .any(|v| !supported_events(&connection.provider, body.mode).contains(&v.as_str()))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.eventInvalid"),
        ));
    }
    // Delivery settings and scheduling do not change what was sampled. Keep that
    // baseline so editing a notification cannot silently discard unseen changes.
    let same_sampling = old.is_some()
        && previous.mode == Mode::Poll
        && body.mode == Mode::Poll
        && previous.provider == connection.provider
        && (connection.provider != "git" || previous.runner_id == body.runner_id)
        && previous.all_repositories == body.all_repositories
        && (body.all_repositories || previous.repositories == repositories)
        && previous
            .events
            .iter()
            .collect::<std::collections::HashSet<_>>()
            == body.events.iter().collect::<std::collections::HashSet<_>>();
    let reschedule = (!previous.enabled && body.enabled)
        || previous.poll_interval_minutes != body.poll_interval_minutes;
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
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.secretInvalid"),
        ));
    }
    let webhook = if body.clear_webhook {
        String::new()
    } else if body.webhook.is_empty() {
        previous.webhook
    } else {
        body.webhook
    };
    if !webhook.is_empty() {
        let url = reqwest::Url::parse(&webhook).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.monitor.webhookInvalid"),
            )
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || webhook.len() > 4096
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.monitor.webhookFormat"),
            ));
        }
    }
    if body.enabled
        && ((body.mode == Mode::Push && secret.is_empty())
            || body.events.is_empty()
            || (!body.all_repositories && repositories.is_empty()))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.configurationRequired"),
        ));
    }
    let notification_language = body.notification_language.unwrap_or_else(|| {
        if previous.notification_language.is_empty() {
            default_notification_language()
        } else {
            previous.notification_language
        }
    });
    if !crate::i18n::Locale::ALL
        .iter()
        .any(|locale| locale.tag() == notification_language)
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.languageInvalid"),
        ));
    }
    let notification_channels = body
        .notification_channels
        .or(previous.notification_channels);
    if let Some(channels) = &notification_channels {
        validate_channels(channels)?;
    }
    let config = Config {
        mode: body.mode,
        poll_interval_minutes: body.poll_interval_minutes,
        runner_id: if connection.provider == "git" {
            body.runner_id
        } else {
            String::new()
        },
        enabled: body.enabled,
        provider: connection.provider.clone(),
        all_repositories: body.all_repositories,
        repositories,
        events: body.events,
        secret,
        webhook,
        notification_language,
        notification_channels,
    };
    let encoded = serde_json::to_string(&config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.configInvalid"),
        )
    })?;
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
    let poll = polls::Entity::find_by_id(connection.id).one(&tx).await?;
    let preserve_poll = same_sampling
        && poll
            .as_ref()
            .is_some_and(|poll| poll.config_version == body.version);
    if preserve_poll {
        let mut update = polls::Entity::update_many()
            .col_expr(polls::Column::ConfigVersion, Expr::value(row.version))
            .filter(polls::Column::Id.eq(connection.id));
        if reschedule {
            update = update.col_expr(polls::Column::NextPollAt, Expr::value(db::now()));
        }
        update.exec(&tx).await?;
    } else {
        polls::Entity::delete_by_id(connection.id).exec(&tx).await?;
    }
    if !preserve_poll && config.enabled && config.mode == Mode::Poll {
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
    let mut result = view(Some(&row), &config, &connection);
    result["webhook"] = json!(config.webhook);
    result["notification_language"] = json!(config.notification_language);
    result["runner_name"] = json!(runner_name);
    result["notification_channels"] = json!(channels(&config));
    Ok(Json(result))
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
    /// Structured detail: commits, merge request, tag, repository, notes.
    detail: Value,
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
        actor: detail::webhook_actor(payload),
        action: crate::i18n::store_raw(&action),
        detail: detail::webhook(event, payload),
    })
}
fn event_json(row: events::Model) -> Value {
    let detail = row
        .detail
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or(Value::Null);
    let detail = detail::display(&detail);
    let mut delivery_details =
        serde_json::from_str::<Vec<ChannelDelivery>>(&row.delivery).unwrap_or_default();
    for delivery in &mut delivery_details {
        delivery.result = crate::i18n::display_stored(&delivery.result);
    }
    let delivery = if delivery_details.is_empty() {
        crate::i18n::display_stored(&row.delivery)
    } else {
        delivery_details
            .iter()
            .map(|item| format!("{}: {}", item.name, item.result))
            .collect::<Vec<_>>()
            .join("; ")
    };
    json!({"id":Id::from(row.id),"connection_id":Id::from(row.connection_id),"event":row.event,"repository":row.repository,"reference":row.reference,"actor":row.actor,"action":crate::i18n::display_stored(&row.action),"delivery":delivery,"delivery_details":delivery_details,"created_at":row.created_at,"summary":detail::summary(&detail),"detail":detail})
}

/// Only the caller's already-authorized connections are considered; branches are unrestricted.
pub(crate) async fn latest_commit_events(
    s: &AppState,
    workspace: Id,
    ids: Vec<i64>,
) -> Result<std::collections::HashMap<i64, Value>> {
    use sea_orm::{QuerySelect, sea_query::ExprTrait};
    let mut result = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(result);
    }
    let latest: Vec<(i64, i64)> = events::Entity::find()
        .select_only()
        .column(events::Column::ConnectionId)
        .column_as(Expr::col(events::Column::Id).max(), "latest_id")
        .filter(events::Column::WorkspaceId.eq(workspace))
        .filter(events::Column::ConnectionId.is_in(ids))
        .filter(events::Column::Event.eq("push"))
        .group_by(events::Column::ConnectionId)
        .into_tuple()
        .all(&s.db)
        .await?;
    if latest.is_empty() {
        return Ok(result);
    }
    for event in events::Entity::find()
        .filter(events::Column::WorkspaceId.eq(workspace))
        .filter(events::Column::Id.is_in(latest.into_iter().map(|(_, id)| id)))
        .all(&s.db)
        .await?
    {
        result.insert(event.connection_id, event_json(event));
    }
    Ok(result)
}
/// Events of every connection accessible to the caller in this workspace, newest first.
pub async fn workspace_history(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let owned = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::repositories::auth::connection_access(scope.user.id))
        .all(&s.db)
        .await?;
    let names: HashMap<i64, String> = owned.iter().map(|c| (c.id, c.name.clone())).collect();
    let query = ListQuery::parse(&HISTORY, &raw)?;
    let (rows, total) = resource::list::<events::Entity, events::Model>(
        &s.db,
        &HISTORY,
        events::Entity::find().filter(events::Column::ConnectionId.is_in(names.keys().copied())),
        &scope.workspace_id,
        &query,
    )
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let name = names.get(&row.connection_id).cloned().unwrap_or_default();
            let mut value = event_json(row);
            value["connection_name"] = Value::from(name);
            value
        })
        .collect();
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

/// One event with its detail flattened for the shared detail page.
pub async fn event(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    let row = events::Entity::find_by_id(i64::from(id))
        .filter(events::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    owned_connection(&s.db, &scope, Id::from(row.connection_id)).await?;
    let mut value = event_json(row);
    let view = detail::view(&value["detail"]);
    if let Value::Object(map) = &mut value {
        map.extend(view);
    }
    Ok(Json(value))
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
    let payload: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.eventJsonInvalid"),
        )
    })?;
    let Some(mut event) = normalize(&connection.provider, &headers, &payload) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let selected = config.events.contains(&event.event)
        || (event.event == "merge" && config.events.iter().any(|kind| kind == "pull_request"))
        || (matches!(
            event.event.as_str(),
            "branch.created" | "branch.deleted" | "tag.created" | "tag.deleted"
        ) && payload.get("before").is_some()
            && config.events.iter().any(|kind| kind == "push"));
    let selected =
        selected && (config.all_repositories || config.repositories.contains(&event.repository));
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
            .is_none_or(|v| v.version != connection.version)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.monitor.eventConfigChanged"),
        ));
    }
    // Admit the independent analysis subscription and selected notification in
    // one version-fenced transaction, even when push notifications are disabled.
    if matches!(
        event.event.as_str(),
        "push"
            | "branch.created"
            | "branch.deleted"
            | "repository.created"
            | "repository.deleted"
            | "merge"
    ) {
        crate::repositories::analysis::event(&s, &tx, &connection, &event.repository).await?;
    }
    if !selected {
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let event_id = record(&s, &tx, &connection, &config, &event, &delivery_key).await?;
    tx.commit().await?;
    if let Some(event_id) = event_id {
        if event.event == "push" && event.detail.get("line_stats").is_none() {
            // Enrichment never holds the workspace transaction or fails delivery.
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                if let Ok(access) =
                    crate::repositories::auth::MonitorAccess::load(&s, &connection).await
                {
                    polling::enrich_stats(&access, &mut event).await;
                }
            })
            .await;
            events::Entity::update_many()
                .col_expr(
                    events::Column::Detail,
                    Expr::value(event.detail.to_string()),
                )
                .filter(events::Column::WorkspaceId.eq(connection.workspace_id))
                .filter(events::Column::Id.eq(event_id))
                .exec(&s.db)
                .await?;
        }
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
    let delivery = if !channels(config).iter().any(|channel| channel.enabled) {
        crate::i18n::store_message(&crate::i18n::Message::new(
            "repository.monitor.unconfiguredDelivery",
        ))
    } else {
        crate::i18n::store_message(&crate::i18n::Message::new(
            "repository.monitor.pendingDelivery",
        ))
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
        detail: Set((!event.detail.is_null()).then(|| event.detail.to_string())),
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
    let mut deliveries = Vec::new();
    for channel in channels(config)
        .into_iter()
        .filter(|channel| channel.enabled)
    {
        let locale = crate::i18n::Locale::from_tag(&channel.language).unwrap_or_default();
        let result = crate::i18n::with_locale(
            locale,
            deliver_notice(
                connection,
                &channel.webhook,
                &channel.options,
                event_id,
                event,
            ),
        )
        .await?;
        deliveries.push(ChannelDelivery {
            name: channel.name,
            language: channel.language,
            result,
        });
    }
    if !deliveries.is_empty() {
        // Keep the legacy single-channel representation for old consumers.
        let result = if config.notification_channels.is_none() {
            deliveries.remove(0).result
        } else {
            serde_json::to_string(&deliveries).map_err(|_| ApiError::missing())?
        };
        events::Entity::update_many()
            .col_expr(events::Column::Delivery, Expr::value(result))
            .filter(events::Column::WorkspaceId.eq(connection.workspace_id))
            .filter(events::Column::Id.eq(event_id))
            .exec(&s.db)
            .await?;
    }
    Ok(())
}

fn reference_label(event: &Event) -> &'static str {
    if event.event.starts_with("tag.")
        || event.reference.starts_with("refs/tags/")
        || event.detail["reference_type"] == "tag"
    {
        "repository.monitor.tagLabel"
    } else {
        "repository.monitor.branchLabel"
    }
}

fn notice_accent(event: &str) -> crate::notifications::webhook::Accent {
    use crate::notifications::webhook::Accent;
    match event {
        "branch.deleted" | "tag.deleted" | "repository.deleted" => Accent::Red,
        "branch.created" | "tag.created" | "repository.created" | "merge" => Accent::Green,
        "pull_request" => Accent::Purple,
        _ => Accent::Blue,
    }
}

fn notice_action(event: &Event) -> String {
    // Keep sampling provenance in the event record, not the outbound message.
    if crate::i18n::decode_stored(&event.action)
        .is_some_and(|message| message.key.starts_with("repository.monitor.detected"))
    {
        detail::label(&event.event)
    } else {
        crate::i18n::display_stored(&event.action)
    }
}

async fn deliver_notice(
    connection: &connections::Model,
    webhook: &str,
    options: &crate::notifications::channel::Options,
    event_id: Id,
    event: &Event,
) -> Result<String> {
    if !webhook.is_empty() || !options.bot_token.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.monitor.notificationClient"),
                )
            })?;
        let title = detail::label(&event.event);
        let action = notice_action(event);
        let detail = detail::display(&event.detail);
        let mut fields: Vec<(String, String)> = [
            (
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.repositoryLabel"),
                ),
                &event.repository,
            ),
            (
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new(reference_label(event)),
                ),
                &event.reference,
            ),
            (
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.actorLabel"),
                ),
                &event.actor,
            ),
            (
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.actionLabel"),
                ),
                &action,
            ),
            (
                &crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.providerLabel"),
                ),
                &connection.provider,
            ),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.clone()))
        .collect();
        fields.extend(detail::lines(&detail));
        let mut data = json!(event);
        data["action"] = json!(action);
        data["detail"] = detail;
        let notice = crate::notifications::webhook::Notice {
            title: &title,
            accent: Some(notice_accent(&event.event)),
            field_accents: vec![
                (
                    crate::i18n::render(
                        crate::i18n::current_locale(),
                        &crate::i18n::Message::new("repository.monitor.commitMessageLabel"),
                    ),
                    crate::notifications::webhook::Accent::Blue,
                ),
                (
                    crate::i18n::render(
                        crate::i18n::current_locale(),
                        &crate::i18n::Message::new("repository.monitor.addedLinesLabel"),
                    ),
                    crate::notifications::webhook::Accent::Green,
                ),
                (
                    crate::i18n::render(
                        crate::i18n::current_locale(),
                        &crate::i18n::Message::new("repository.monitor.deletedLinesLabel"),
                    ),
                    crate::notifications::webhook::Accent::Red,
                ),
            ],
            field_links: vec![(
                crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new("repository.monitor.repositoryLabel"),
                ),
                detail::field(&event.detail, &["/repository_url", "/repository/url"], 1000),
            )],
            fields: fields.clone(),
            generic: json!({"id":event_id,"workspace_id":Id::from(connection.workspace_id),"connection_id":Id::from(connection.id),"provider":connection.provider,"event":event.event,"data":data,"notification":{"title":title,"fields":fields}}),
        };
        let delivery =
            crate::notifications::webhook::deliver_configured(&client, webhook, options, &notice)
                .await;
        return Ok(delivery);
    }
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn notice_reference_and_line_statistics_are_explicit_in_all_locales() {
        use super::*;
        for locale in crate::i18n::Locale::ALL {
            crate::i18n::with_locale(locale, async {
                for (kind, reference, detail, expected) in [
                    ("push", "main", json!({}), "repository.monitor.branchLabel"),
                    (
                        "tag.created",
                        "v1",
                        json!({}),
                        "repository.monitor.tagLabel",
                    ),
                    (
                        "push",
                        "refs/tags/v2",
                        json!({}),
                        "repository.monitor.tagLabel",
                    ),
                    (
                        "push",
                        "v3",
                        json!({"reference_type":"tag"}),
                        "repository.monitor.tagLabel",
                    ),
                ] {
                    let event = Event {
                        event: kind.into(),
                        repository: "team/app".into(),
                        reference: reference.into(),
                        actor: String::new(),
                        action: String::new(),
                        detail,
                    };
                    assert_eq!(reference_label(&event), expected);
                    assert!(
                        !crate::i18n::render(
                            locale,
                            &crate::i18n::Message::new(reference_label(&event))
                        )
                        .contains(" / ")
                    );
                    assert!(!detail::label(kind).contains("team/app"));
                }
                let unknown = detail::lines(&json!({}));
                let polled = Event {
                    event: "push".into(),
                    repository: "team/app".into(),
                    reference: "main".into(),
                    actor: String::new(),
                    action: crate::i18n::store_message(&crate::i18n::Message::new(
                        "repository.monitor.detectedPush",
                    )),
                    detail: json!({"source":"polling"}),
                };
                assert_eq!(notice_action(&polled), detail::label("push"));
                assert_ne!(
                    notice_action(&polled),
                    crate::i18n::display_stored(&polled.action)
                );
                let added = crate::i18n::render(
                    locale,
                    &crate::i18n::Message::new("repository.monitor.addedLinesLabel"),
                );
                assert!(!unknown.iter().any(|(key, _)| key == &added));
                let known = detail::lines(&json!({"line_stats":{"additions":12,"deletions":0}}));
                assert!(
                    known
                        .iter()
                        .any(|(key, value)| key == &added && value.contains("12"))
                );
                assert_eq!(known.len(), 2);
            })
            .await;
        }
    }

    #[test]
    fn notification_channels_preserve_legacy_and_validate_independent_configuration() {
        use super::*;
        let config = Config {
            webhook: "https://example.invalid/hook".into(),
            notification_language: "ja-JP".into(),
            ..Default::default()
        };
        let legacy = channels(&config);
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].language, "ja-JP");
        assert!(validate_channels(&legacy).is_ok());
        let config = Config {
            notification_channels: Some(Vec::new()),
            ..config
        };
        assert!(
            channels(&config).is_empty(),
            "explicit empty list removes legacy delivery"
        );
        let mut invalid = legacy.clone();
        invalid[0].webhook = "file:///secret".into();
        assert!(validate_channels(&invalid).is_err());
        invalid[0] = legacy[0].clone();
        invalid[0].language = "invalid".into();
        assert!(validate_channels(&invalid).is_err());
        assert!(validate_channels(&[legacy[0].clone(), legacy[0].clone()]).is_err());
    }

    #[test]
    fn legacy_notification_language_remains_english() {
        let mut legacy = serde_json::to_value(super::Config::default()).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("notification_language");
        let decoded: super::Config = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.notification_language, "en-US");
    }

    use super::*;

    #[tokio::test]
    async fn event_readers_localize_owned_fields_once_and_preserve_provider_data() {
        use crate::i18n::{self, Locale, Message};
        let raw = i18n::store_message(&Message::new("error.unauthorized"));
        let action = Message::new("repository.monitor.detectedPush");
        let note = Message::new("repository.monitor.commitsFailed")
            .with_message("error", Message::new("error.unauthorized"));
        let delivery = Message::new("webhook.failed")
            .with("provider", "Provider 原文")
            .with_message(
                "reason",
                Message::new("webhook.httpDetail")
                    .with("status", "403")
                    .with("detail", &raw),
            );
        let row = events::Model {
            id: 1, workspace_id: 1, connection_id: 1, delivery_key: "test".into(),
            event: "push".into(), repository: "原文/repository".into(), reference: "main".into(),
            actor: "用户原文".into(), action: i18n::store_message(&action),
            delivery: i18n::store_message(&delivery), created_at: 0,
            detail: Some(json!({"note":i18n::store_message(&note),"commits":[{"message":raw,"author":"用户原文"}]}).to_string()),
        };
        for locale in Locale::ALL {
            i18n::with_locale(locale, async {
                let value = event_json(row.clone());
                assert_eq!(value["action"], i18n::render(locale, &action));
                assert_eq!(value["delivery"], i18n::render(locale, &delivery));
                assert_eq!(value["detail"]["note"], i18n::render(locale, &note));
                assert_eq!(value["detail"]["commits"][0]["message"], raw);
                assert_eq!(value["actor"], "用户原文");
                assert!(value["delivery"].as_str().unwrap().contains(&raw));
            })
            .await;
        }
        let mut provider = row;
        provider.action = i18n::store_raw(&raw);
        provider.detail = Some(json!({"note":i18n::store_raw(&raw)}).to_string());
        let value = event_json(provider);
        assert_eq!(value["action"], raw);
        assert_eq!(value["detail"]["note"], raw);
        assert_eq!(detail::view(&value["detail"])["note"], raw);
    }

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
