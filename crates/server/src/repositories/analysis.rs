//! Contribution facts, durable monitor subscriptions and workspace-scoped queries.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        repository_analysis_commits as commits, repository_analysis_configs as configs,
        repository_analysis_jobs as jobs, repository_analysis_ref_commits as membership,
        repository_analysis_refs as refs, repository_analysis_repositories as repositories,
        repository_analysis_sources as sources, repository_connections as connections,
    },
    error::{ApiError, Result},
    repositories::auth::{MonitorAccess, owned_connection},
    snowflake::Id,
    state::AppState,
};
use axum::extract::State;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokio::sync::watch;

mod identity;
mod insight_identity;
pub(crate) mod insights;
mod query;
mod sync;
pub(crate) use identity::recover;
pub use query::insight_report;
pub use query::report;
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub email: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub contributor_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Connection-level collector demand is owned by standalone insights.
    pub insights_managed: bool,
    pub pipeline_bindings: BTreeMap<String, String>,
    pub coverage_starts: BTreeMap<String, i64>,
    /// Local identity projection migration version.
    pub identity_version: u32,
    /// Internal collector fence; independent of local identity edits.
    pub collection_version: i64,
    pub enabled: bool,
    pub all_repositories: bool,
    /// Exact paths/URLs, or platform group selectors ending in `/*`.
    pub repositories: Vec<String>,
    /// None preserves legacy event delivery for every selected repository.
    /// Some contains event-mode paths; an empty path selects every repository.
    pub event_repositories: Option<Vec<String>>,
    pub full_history: bool,
    pub initial_days: u32,
    pub reconcile_minutes: u32,
    pub pipeline_id: String,
    pub aliases: Vec<Alias>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            insights_managed: false,
            pipeline_bindings: BTreeMap::new(),
            coverage_starts: BTreeMap::new(),
            identity_version: 0,
            collection_version: 0,
            enabled: false,
            all_repositories: true,
            repositories: vec![],
            event_repositories: None,
            full_history: false,
            initial_days: 90,
            reconcile_minutes: 1440,
            pipeline_id: String::new(),
            aliases: vec![],
        }
    }
}
impl Config {
    fn required_coverage_start(&self, path: &str, fallback: i64) -> i64 {
        self.coverage_starts
            .iter()
            .filter(|(selected, _)| selected.is_empty() || repository_matches(selected, path))
            .map(|(_, start)| *start)
            .reduce(coverage_union)
            .unwrap_or(fallback)
    }

    fn same_collection(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.all_repositories == other.all_repositories
            && self.repositories == other.repositories
            && self.full_history == other.full_history
            && self.initial_days == other.initial_days
            && self.reconcile_minutes == other.reconcile_minutes
            && self.pipeline_id == other.pipeline_id
            && self.pipeline_bindings == other.pipeline_bindings
            && self.coverage_starts == other.coverage_starts
    }
    pub(crate) fn selects(&self, path: &str) -> bool {
        self.enabled
            && (self.all_repositories
                || self
                    .repositories
                    .iter()
                    .any(|p| repository_matches(p, path)))
    }
    pub(crate) fn event_selects(&self, path: &str) -> bool {
        self.selects(path)
            && self.event_repositories.as_ref().is_none_or(|paths| {
                paths
                    .iter()
                    .any(|selected| selected.is_empty() || repository_matches(selected, path))
            })
    }
}

/// Group selectors match descendants on a slash boundary, never similarly named groups.
fn repository_matches(selected: &str, path: &str) -> bool {
    selected == path
        || selected.strip_suffix("/*").is_some_and(|group| {
            !group.is_empty()
                && path
                    .strip_prefix(group)
                    .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
        })
}

// Zero means unlimited history, rather than the Unix epoch.
fn coverage_union(left: i64, right: i64) -> i64 {
    if left == 0 || right == 0 {
        0
    } else {
        std::cmp::min(left, right)
    }
}

fn coverage_intersection(left: i64, right: i64) -> i64 {
    match (left, right) {
        (0, _) => right,
        (_, 0) => left,
        _ => std::cmp::max(left, right),
    }
}

fn collection_version(row: &configs::Model) -> Result<i64> {
    let config = decode(row)?;
    Ok(if config.collection_version > 0 {
        config.collection_version
    } else {
        row.version
    })
}
fn decode(row: &configs::Model) -> Result<Config> {
    serde_json::from_str(&row.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.configRead"),
        )
    })
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.dataEncode"),
        )
    })
}

/// Connection visibility changes can change which local alias rules apply.
pub(crate) async fn connection_changed(
    s: &AppState,
    tx: &impl ConnectionTrait,
    workspace: i64,
    user: i64,
    connection: i64,
) -> Result<()> {
    let source_rows = sources::Entity::find()
        .filter(sources::Column::WorkspaceId.eq(workspace))
        .filter(sources::Column::ConnectionId.eq(connection))
        .all(tx)
        .await?;
    let repositories = source_rows
        .iter()
        .map(|source| source.repository_id)
        .collect::<Vec<_>>();
    if !source_rows.is_empty()
        && connections::Entity::find_by_id(connection)
            .one(tx)
            .await?
            .is_none()
    {
        let ids = source_rows
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>();
        sources::Entity::update_many()
            .col_expr(sources::Column::State, Expr::value("unavailable"))
            .filter(sources::Column::Id.is_in(ids.clone()))
            .exec(tx)
            .await?;
        // A removed connection cannot service queued work; retain facts and any
        // active lease for its worker's fenced cleanup, but stop retrying it.
        jobs::Entity::update_many()
            .col_expr(
                jobs::Column::CompletedGeneration,
                Expr::col(jobs::Column::Generation),
            )
            .col_expr(jobs::Column::State, Expr::value("complete"))
            .filter(jobs::Column::Id.is_in(ids))
            .exec(tx)
            .await?;
    }
    if !repositories.is_empty() {
        identity::reconcile_scope(s, tx, workspace, user, &repositories).await?;
    }
    Ok(())
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("repositories.read")?;
    owned_connection(&s.db, &scope, id).await?;
    let row = configs::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?;
    let mut config = row.as_ref().map(decode).transpose()?.unwrap_or_default();
    identity::hydrate_aliases(
        &s.db,
        i64::from(scope.workspace_id),
        i64::from(scope.user.id),
        &mut config.aliases,
    )
    .await?;
    let source_rows = sources::Entity::find()
        .filter(sources::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(sources::Column::ConnectionId.eq(id))
        .all(&s.db)
        .await?;
    let source_ids: Vec<i64> = source_rows.iter().map(|v| v.id).collect();
    let job_rows = jobs::Entity::find()
        .filter(jobs::Column::Id.is_in(source_ids))
        .all(&s.db)
        .await?;
    Ok(Json(
        json!({"id":id,"config":config,"version":row.as_ref().map(|v|v.version).unwrap_or(0),
        "coverage_start":row.as_ref().map(|v|v.coverage_start),
        "last_error":crate::i18n::display_stored(row.as_ref().map(|v|v.last_error.as_str()).unwrap_or("")),
        "sources":source_rows.iter().map(|v|json!({"id":Id::from(v.id),"repository":v.remote_path,
            "state":v.state,"last_error":crate::i18n::display_stored(&v.last_error),"synced_at":v.synced_at,
            "coverage_start":v.coverage_start,
            "job":job_rows.iter().find(|j|j.id==v.id).map(|j|json!({"state":j.state,
                "generation":j.generation,"completed_generation":j.completed_generation,"last_error":crate::i18n::display_stored(&j.last_error)}))
        })).collect::<Vec<_>>() }),
    ))
}

pub async fn flat_get(
    state: State<AppState>,
    scope: Scope,
    path: Path<(String, Id)>,
) -> Result<Json<Value>> {
    let Json(mut value) = get(state, scope, path).await?;
    if let Some(Value::Object(config)) = value.as_object_mut().and_then(|v| v.remove("config")) {
        value.as_object_mut().unwrap().extend(config);
    }
    Ok(Json(value))
}

#[derive(Deserialize)]
pub struct Body {
    pub config: Config,
    pub version: i64,
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("repositories.update")?;
    scope.require("credentials.read")?;
    if body.config.initial_days == 0 || body.config.reconcile_minutes == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.intervalInvalid"),
        ));
    }
    body.config.repositories = body
        .config
        .repositories
        .iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if body.config.aliases.iter().any(|a| {
        (a.name.trim().is_empty() && a.email.trim().is_empty()) || a.nickname.trim().is_empty()
    }) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.aliasRequired"),
        ));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let connection = owned_connection(&tx, &scope, id).await?;
    if connection.provider == "git"
        && body.config.enabled
        && (body.config.repositories.is_empty() || body.config.all_repositories)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.gitScopeRequired"),
        ));
    }
    if !body.config.pipeline_id.is_empty() {
        scope.require("pipelines.run")?;
        let pipeline: Id = body.config.pipeline_id.parse().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.pipelineRequired"),
            )
        })?;
        if crate::entities::pipelines::Entity::find_by_id(i64::from(pipeline))
            .filter(crate::entities::pipelines::Column::WorkspaceId.eq(scope.workspace_id))
            .one(&tx)
            .await?
            .is_none()
        {
            return Err(ApiError::missing());
        }
    }
    let old = configs::Entity::find_by_id(i64::from(id)).one(&tx).await?;
    if old
        .as_ref()
        .map(decode)
        .transpose()?
        .is_some_and(|c| c.insights_managed)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.insightManaged"),
        ));
    }
    if old.as_ref().map(|v| v.version).unwrap_or(0) != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.configChangedRetry"),
        ));
    }
    let mut previous = old.as_ref().map(decode).transpose()?.unwrap_or_default();
    identity::hydrate_aliases(
        &tx,
        i64::from(scope.workspace_id),
        i64::from(scope.user.id),
        &mut previous.aliases,
    )
    .await?;
    let collection_changed = old.is_none() || !previous.same_collection(&body.config);
    body.config.collection_version = if collection_changed {
        body.version + 1
    } else {
        collection_version(old.as_ref().unwrap())?
    };
    identity::prepare_aliases(
        &s,
        &tx,
        i64::from(scope.workspace_id),
        i64::from(scope.user.id),
        &previous.aliases,
        &mut body.config.aliases,
    )
    .await?;
    body.config.identity_version = 1;
    let now = db::now();
    let requested = if body.config.full_history {
        0
    } else {
        now - i64::from(body.config.initial_days) * 86_400_000
    };
    let start = old
        .as_ref()
        .map(|v| coverage_union(v.coverage_start, requested))
        .unwrap_or(requested);
    let row = configs::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        config: encode(&body.config)?,
        coverage_start: start,
        next_reconcile_at: if collection_changed {
            0
        } else {
            old.as_ref().unwrap().next_reconcile_at
        },
        last_error: if collection_changed {
            String::new()
        } else {
            old.as_ref().unwrap().last_error.clone()
        },
        version: body.version + 1,
        created_at: old.as_ref().map(|v| v.created_at).unwrap_or(now),
        updated_at: now,
    };
    if old.is_some() {
        configs::Entity::update(row.into_active_model().reset_all())
            .exec(&tx)
            .await?;
    } else {
        configs::Entity::insert(row.into_active_model())
            .exec(&tx)
            .await?;
    }
    let affected_sources = sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(id))
        .all(&tx)
        .await?;
    if collection_changed {
        for source in &affected_sources {
            sources::Entity::update_many()
                .col_expr(
                    sources::Column::ConfigVersion,
                    Expr::value(body.version + 1),
                )
                .col_expr(sources::Column::CoverageStart, Expr::value(start))
                .col_expr(
                    sources::Column::State,
                    Expr::value(if source.state == "unavailable" {
                        "unavailable"
                    } else {
                        "pending"
                    }),
                )
                .filter(sources::Column::Id.eq(source.id))
                .exec(&tx)
                .await?;
            dirty(&s, &tx, source.id, source.workspace_id).await?;
        }
    }
    let affected_repositories: Vec<i64> = affected_sources
        .iter()
        .map(|source| source.repository_id)
        .collect();
    if !affected_repositories.is_empty() {
        identity::rebuild(
            &s,
            &tx,
            i64::from(scope.workspace_id),
            i64::from(scope.user.id),
            &affected_repositories,
        )
        .await?;
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "repositories",
        &id,
        json!({"analysis":true,"enabled":body.config.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":body.version+1})))
}

pub async fn retry(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("repositories.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    owned_connection(&tx, &scope, id).await?;
    configs::Entity::update_many()
        .col_expr(configs::Column::NextReconcileAt, Expr::value(0_i64))
        .filter(configs::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    for row in sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(id))
        .all(&tx)
        .await?
    {
        let job = jobs::Entity::find_by_id(row.id).one(&tx).await?;
        if job.as_ref().is_some_and(|v| v.lease_until > db::now()) {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("repository.analysis.analysisRunning"),
            ));
        }
        jobs::Entity::update_many()
            .col_expr(jobs::Column::Cursor, Expr::value(""))
            .filter(jobs::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        dirty(&s, &tx, row.id, row.workspace_id).await?;
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "sync",
        "repositories",
        &id,
        json!({"analysis":true,"manual":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"queued":true})))
}

async fn dirty(s: &AppState, tx: &impl ConnectionTrait, source: i64, workspace: i64) -> Result<()> {
    if let Some(mut row) = jobs::Entity::find_by_id(source).one(tx).await? {
        // Repeated pushes coalesce without sending an already pending job to
        // the back of the queue. A completed job starts a fresh enqueue age.
        if row.generation <= row.completed_generation {
            row.updated_at = db::now();
        }
        row.generation += 1;
        row.retry_at = 0;
        if row.state != "running" {
            row.state = "queued".into();
        }
        jobs::Entity::update(row.into_active_model().reset_all())
            .exec(tx)
            .await?;
    } else {
        let _ = s;
        jobs::Entity::insert(
            jobs::Model {
                id: source,
                workspace_id: workspace,
                generation: 1,
                completed_generation: 0,
                state: "queued".into(),
                cursor: String::new(),
                retry_at: 0,
                lease_owner: String::new(),
                lease_until: 0,
                last_error: String::new(),
                updated_at: db::now(),
            }
            .into_active_model(),
        )
        .exec(tx)
        .await?;
    }
    Ok(())
}

/// Invoked inside the monitor's transaction, before notification filtering.
pub(crate) async fn event(
    s: &AppState,
    tx: &impl ConnectionTrait,
    connection: &connections::Model,
    path: &str,
) -> Result<()> {
    let Some(row) = configs::Entity::find_by_id(connection.id).one(tx).await? else {
        return Ok(());
    };
    if !decode(&row)?.event_selects(path) {
        return Ok(());
    }
    let source_rows = sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(connection.id))
        .filter(sources::Column::RemotePath.eq(path))
        .all(tx)
        .await?;
    if source_rows.is_empty() {
        configs::Entity::update_many()
            .col_expr(configs::Column::NextReconcileAt, Expr::value(0_i64))
            .filter(configs::Column::Id.eq(connection.id))
            .exec(tx)
            .await?;
    }
    for source in source_rows {
        dirty(s, tx, source.id, source.workspace_id).await?;
    }
    Ok(())
}

/// Polling reuses already-fetched complete branch snapshots, not notification rows.
pub(crate) async fn observes(
    s: &AppState,
    tx: &impl ConnectionTrait,
    connection: &connections::Model,
    path: &str,
    heads: &BTreeMap<String, String>,
) -> Result<()> {
    let Some(config_row) = configs::Entity::find_by_id(connection.id).one(tx).await? else {
        return Ok(());
    };
    if !decode(&config_row)?.event_selects(path) {
        return Ok(());
    }
    let Some(source) = sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(connection.id))
        .filter(sources::Column::RemotePath.eq(path))
        .one(tx)
        .await?
    else {
        return event(s, tx, connection, path).await;
    };
    let known: BTreeMap<String, String> = refs::Entity::find()
        .filter(refs::Column::SourceId.eq(source.id))
        .filter(refs::Column::Present.eq(1))
        .all(tx)
        .await?
        .into_iter()
        .map(|v| (v.name, v.head))
        .collect();
    if &known != heads {
        dirty(s, tx, source.id, source.workspace_id).await?;
    }
    Ok(())
}

pub(crate) async fn monitor_subscription(
    tx: &impl ConnectionTrait,
    connection: i64,
) -> Result<Option<Config>> {
    let Some(mut config) = configs::Entity::find_by_id(connection)
        .one(tx)
        .await?
        .map(|v| decode(&v))
        .transpose()?
        .filter(|v| v.enabled)
    else {
        return Ok(None);
    };
    if let Some(paths) = &config.event_repositories {
        if !paths.iter().any(String::is_empty) {
            let repositories = paths
                .iter()
                .filter(|path| config.selects(path))
                .cloned()
                .collect::<Vec<_>>();
            if repositories.is_empty() {
                return Ok(None);
            }
            config.all_repositories = false;
            config.repositories = repositories;
        }
    }
    Ok(Some(config))
}

async fn discover(s: &AppState, row: &configs::Model) -> Result<()> {
    let config = decode(row)?;
    let connection = connections::Entity::find_by_id(row.id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let scope =
        Scope::for_actor(s, connection.workspace_id.into(), connection.user_id.into()).await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if connection.state != "connected" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.authorizationInvalid"),
        ));
    }
    let mut found = Vec::new();
    if connection.provider == "git" {
        for url in &config.repositories {
            found.push(json!({"full_name":url,"url":url}));
        }
    } else {
        let access = MonitorAccess::load(s, &connection).await?;
        let mut page = 1;
        loop {
            let remote = access.repositories(page).await?;
            let items = remote["items"].as_array().ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.repositoryListInvalid"),
                )
            })?;
            found.extend(
                items
                    .iter()
                    .filter(|v| v["full_name"].as_str().is_some_and(|p| config.selects(p)))
                    .cloned(),
            );
            if remote["has_more"].as_bool() == Some(false) {
                break;
            }
            if items.is_empty() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.repositoryPagesIncomplete"),
                ));
            }
            page += 1;
        }
    }
    let tx = crate::variables::transaction(s, connection.workspace_id.into()).await?;
    let current = configs::Entity::find_by_id(row.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if collection_version(&current)? != collection_version(row)? {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.configChanged"),
        ));
    }
    let mut visible = BTreeSet::new();
    let mut identity_changed = false;
    for remote in found {
        let path = remote["full_name"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.repositoryPathInvalid"),
            )
        })?;
        let url = remote["url"].as_str().ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.repositoryUrlInvalid"),
            )
        })?;
        let remote_id = remote["remote_id"].as_str().unwrap_or("");
        let key = if remote_id.is_empty() {
            url.trim_end_matches(".git").to_owned()
        } else {
            format!(
                "{}\n{}",
                remote["provider_instance"].as_str().unwrap_or(""),
                remote_id
            )
        };
        let identity = hex::encode(Sha256::digest(
            format!("{}\n{}", connection.provider, key).as_bytes(),
        ));
        let repo = if let Some(repo) = repositories::Entity::find()
            .filter(repositories::Column::WorkspaceId.eq(connection.workspace_id))
            .filter(repositories::Column::Identity.eq(&identity))
            .one(&tx)
            .await?
        {
            repo
        } else {
            repositories::Entity::insert(
                repositories::Model {
                    id: s.next_id()?.into(),
                    workspace_id: connection.workspace_id,
                    identity,
                    provider: connection.provider.clone(),
                    path: path.into(),
                    url: url.into(),
                    created_at: db::now(),
                }
                .into_active_model(),
            )
            .exec_with_returning(&tx)
            .await?
        };
        let source = if let Some(source) = sources::Entity::find()
            .filter(sources::Column::ConnectionId.eq(row.id))
            .filter(sources::Column::RepositoryId.eq(repo.id))
            .one(&tx)
            .await?
        {
            identity_changed |= source.state == "unavailable" || source.remote_path != path;
            source
        } else {
            identity_changed = true;
            sources::Entity::insert(
                sources::Model {
                    id: s.next_id()?.into(),
                    workspace_id: row.workspace_id,
                    connection_id: row.id,
                    repository_id: repo.id,
                    remote_path: path.into(),
                    coverage_start: config.required_coverage_start(path, row.coverage_start),
                    config_version: collection_version(row)?,
                    state: "pending".into(),
                    last_error: String::new(),
                    synced_at: 0,
                }
                .into_active_model(),
            )
            .exec_with_returning(&tx)
            .await?
        };
        if !identity_changed {
            identity_changed = identity::needs_projection(
                &tx,
                connection.workspace_id,
                connection.user_id,
                repo.id,
            )
            .await?;
        }
        // Provider IDs survive repository renames; keep the access path current.
        repositories::Entity::update_many()
            .col_expr(repositories::Column::Path, Expr::value(path))
            .col_expr(repositories::Column::Url, Expr::value(url))
            .filter(repositories::Column::Id.eq(repo.id))
            .exec(&tx)
            .await?;
        sources::Entity::update_many()
            .col_expr(sources::Column::RemotePath, Expr::value(path))
            .col_expr(
                sources::Column::State,
                Expr::value(if source.state == "unavailable" {
                    "pending"
                } else {
                    source.state.as_str()
                }),
            )
            .filter(sources::Column::Id.eq(source.id))
            .exec(&tx)
            .await?;
        visible.insert(source.id);
        if !config.insights_managed || source.synced_at == 0 || config.event_selects(path) {
            dirty(s, &tx, source.id, source.workspace_id).await?;
        }
    }
    // A complete authorized listing is evidence that an omitted repository can
    // no longer be read through this connection. Keep facts, revoke this source.
    for source in sources::Entity::find()
        .filter(sources::Column::ConnectionId.eq(row.id))
        .all(&tx)
        .await?
    {
        if config.selects(&source.remote_path) && !visible.contains(&source.id) {
            identity_changed |= source.state != "unavailable";
            sources::Entity::update_many()
                .col_expr(sources::Column::State, Expr::value("unavailable"))
                .col_expr(
                    sources::Column::LastError,
                    Expr::value(crate::i18n::store_message(&crate::i18n::Message::new(
                        "repository.analysis.unavailable",
                    ))),
                )
                .filter(sources::Column::Id.eq(source.id))
                .exec(&tx)
                .await?;
        }
    }
    configs::Entity::update_many()
        .col_expr(
            configs::Column::NextReconcileAt,
            Expr::value(db::now() + i64::from(config.reconcile_minutes) * 60_000),
        )
        .col_expr(configs::Column::LastError, Expr::value(""))
        .filter(configs::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    if identity_changed {
        let affected_repositories: Vec<i64> = sources::Entity::find()
            .filter(sources::Column::ConnectionId.eq(connection.id))
            .all(&tx)
            .await?
            .into_iter()
            .map(|source| source.repository_id)
            .collect();
        if !affected_repositories.is_empty() {
            identity::rebuild(
                s,
                &tx,
                connection.workspace_id,
                connection.user_id,
                &affected_repositories,
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn scheduler(
    s: AppState,
    concurrency: std::num::NonZeroUsize,
    mut stop: watch::Receiver<bool>,
) {
    let mut workers = tokio::task::JoinSet::new();
    loop {
        if *stop.borrow() {
            break;
        }
        let round = async {
            insights::schedule_due(&s).await?;
            for row in configs::Entity::find()
                .filter(configs::Column::NextReconcileAt.lte(db::now()))
                .all(&s.db)
                .await?
            {
                if !decode(&row)?.enabled {
                    continue;
                }
                if let Err(error) = discover(&s, &row).await {
                    configs::Entity::update_many()
                        .col_expr(configs::Column::LastError, Expr::value(error.stored()))
                        .col_expr(
                            configs::Column::NextReconcileAt,
                            Expr::value(db::now() + 60_000),
                        )
                        .filter(configs::Column::Id.eq(row.id))
                        .exec(&s.db)
                        .await?;
                }
            }
            while let Some(result) = workers.try_join_next() {
                if let Err(error) = result {
                    tracing::warn!(%error,"代码贡献任务中断");
                }
            }
            let pending = jobs::Entity::find()
                .filter(jobs::Column::RetryAt.lte(db::now()))
                .filter(jobs::Column::LeaseUntil.lte(db::now()))
                .filter(
                    Expr::col(jobs::Column::Generation)
                        .gt(Expr::col(jobs::Column::CompletedGeneration)),
                )
                .order_by_asc(jobs::Column::UpdatedAt)
                .order_by_asc(jobs::Column::Id)
                .all(&s.db)
                .await?;
            for job in pending {
                if workers.len() >= concurrency.get() {
                    break;
                }
                // Claim before spawning: replicas compete for durable global
                // slots, and no task waits indefinitely for local capacity.
                let claim = match sync::try_claim(&s, job, concurrency).await {
                    Ok(Some(claim)) => claim,
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::warn!(error=%error.1,"代码贡献任务领取失败");
                        continue;
                    }
                };
                let state = s.clone();
                workers.spawn(async move {
                    if let Err(error) = sync::run_claim(&state, claim).await {
                        tracing::warn!(error=%error.1,"代码贡献同步未完成");
                    }
                });
            }
            Ok::<(), ApiError>(())
        };
        tokio::select! { _=stop.changed()=>break, result=round=>{
            if let Err(error)=result {tracing::warn!(error=%error.1,"读取代码分析任务失败");}
        }}
        tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(std::time::Duration::from_secs(2))=>{}}
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
}
