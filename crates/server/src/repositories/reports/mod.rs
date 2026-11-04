//! Persisted insight report definitions, revision snapshots and leased generation.
mod entities;
mod exports;
mod generation;
pub(crate) mod migration;
mod period;
#[cfg(test)]
mod tests;

use super::analysis::{self, insights};
use crate::{
    accounts::workspace::Scope,
    db,
    error::{ApiError, Json, Path, Query, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use entities::{bindings, definitions as defs, runs};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
    TransactionTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Repository {
    pub connection_id: String,
    pub repository: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Application {
    pub application_id: String,
    #[serde(default)]
    pub repositories: Vec<Repository>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub name: String,
    pub insight_id: String,
    pub dimension: String,
    pub periods: Vec<String>,
    #[serde(default)]
    pub author_keys: Vec<String>,
    #[serde(default)]
    pub applications: Vec<Application>,
    pub timezone: String,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub version: i64,
}
fn enabled_default() -> bool {
    true
}
fn bad(key: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new(format!("reports.error.{key}")),
    )
}
fn conflict(key: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::CONFLICT,
        crate::i18n::Message::new(format!("reports.error.{key}")),
    )
}
fn decode(row: &defs::Model) -> Result<Config> {
    serde_json::from_str(&row.config).map_err(|_| bad("persisted"))
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| bad("data"))
}
fn key(value: &str) -> Result<i64> {
    let value = i64::from(Id::reference(value)?);
    if value <= 0 {
        return Err(bad("reference"));
    }
    Ok(value)
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/insight-reports", get(list).post(create))
        .route("/{workspace}/insight-reports/options", get(options))
        .route(
            "/{workspace}/insight-reports/{id}",
            get(detail).patch(update).delete(delete),
        )
        .route("/{workspace}/insight-reports/{id}/generate", post(generate))
        .route("/{workspace}/insight-reports/{id}/runs", get(history))
        .route(
            "/{workspace}/insight-reports/{id}/runs/{run_id}",
            get(run_detail),
        )
        .route(
            "/{workspace}/insight-reports/{id}/runs/{run_id}/export",
            get(export),
        )
}
async fn owned(tx: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<defs::Model> {
    defs::Entity::find_by_id(i64::from(id))
        .filter(defs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(defs::Column::UserId.eq(scope.user.id))
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn source(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    config: &Config,
) -> Result<crate::entities::code_insights::Model> {
    scope.require("code-insights.read")?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    insights::owned(tx, scope, key(&config.insight_id)?.into()).await
}
async fn authorized_sources(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    insight: &crate::entities::code_insights::Model,
) -> Result<Vec<crate::entities::repository_analysis_sources::Model>> {
    scope.require("credentials.read")?;
    let mut sources = insights::selected_sources(tx, scope, insight).await?;
    use crate::entities::{credentials, repository_connections as connections};
    let connections = connections::Entity::find()
        .filter(connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(connections::Column::UserId.eq(scope.user.id))
        .filter(connections::Column::State.eq("connected"))
        .filter(connections::Column::Id.is_in(sources.iter().map(|source| source.connection_id)))
        .all(tx)
        .await?;
    let credentials: BTreeSet<_> = credentials::Entity::find()
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(
            credentials::Column::Id.is_in(
                connections
                    .iter()
                    .filter(|connection| connection.credential_id != 0)
                    .map(|connection| connection.credential_id),
            ),
        )
        .all(tx)
        .await?
        .into_iter()
        .map(|credential| credential.id)
        .collect();
    let allowed: BTreeSet<_> = connections
        .iter()
        .filter(|connection| {
            connection.credential_id == 0 || credentials.contains(&connection.credential_id)
        })
        .map(|connection| connection.id)
        .collect();
    for source in &mut sources {
        if source.id != 0 && !allowed.contains(&source.connection_id) {
            source.id = 0;
            source.repository_id = 0;
            source.state = "unavailable".into();
            source.last_error = ApiError::forbidden().stored();
        }
    }
    Ok(sources)
}
async fn validate(tx: &impl ConnectionTrait, scope: &Scope, config: &mut Config) -> Result<()> {
    config.name = config.name.trim().to_owned();
    if config.name.is_empty() || config.name.len() > 200 {
        return Err(bad("name"));
    }
    if !["person", "application"].contains(&config.dimension.as_str()) {
        return Err(bad("dimension"));
    }
    config.periods.sort();
    config.periods.dedup();
    config.author_keys.sort();
    config.author_keys.dedup();
    if config.periods.is_empty()
        || config
            .periods
            .iter()
            .any(|p| !period::PERIODS.contains(&p.as_str()))
    {
        return Err(bad("periods"));
    }
    period::timezone(&config.timezone)?;
    let insight = source(tx, scope, config).await?;
    if config.dimension == "person" {
        config.applications.clear();
        if config.author_keys.is_empty() {
            return Err(bad("people"));
        }
        let people: BTreeSet<String> = crate::entities::code_insight_people::Entity::find()
            .filter(
                crate::entities::code_insight_people::Column::WorkspaceId.eq(scope.workspace_id),
            )
            .filter(crate::entities::code_insight_people::Column::InsightId.eq(insight.id))
            .all(tx)
            .await?
            .into_iter()
            .map(|p| p.id.to_string())
            .collect();
        if config.author_keys.iter().any(|p| !people.contains(p)) {
            return Err(bad("personScope"));
        }
    } else {
        config.author_keys.clear();
        scope.require("deployments.read")?;
        if config.applications.is_empty() {
            return Err(bad("applications"));
        }
        let sources = authorized_sources(tx, scope, &insight).await?;
        let mut seen = BTreeSet::new();
        for app in &config.applications {
            let id = key(&app.application_id)?;
            if !seen.insert(id) {
                return Err(bad("duplicateApp"));
            }
            crate::entities::deployments::Entity::find_by_id(id)
                .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
                .one(tx)
                .await?
                .ok_or_else(ApiError::missing)?;
            if app.repositories.is_empty() {
                return Err(bad("bindRepository"));
            }
            let mut seen = BTreeSet::new();
            for repo in &app.repositories {
                let connection = key(&repo.connection_id)?;
                if !seen.insert((connection, repo.repository.clone())) {
                    return Err(bad("duplicateRepo"));
                }
                if !sources.iter().any(|s| {
                    s.id != 0 && s.connection_id == connection && s.remote_path == repo.repository
                }) {
                    return Err(bad("repositoryScope"));
                }
            }
        }
    }
    Ok(())
}
async fn save_bindings(
    s: &AppState,
    tx: &impl ConnectionTrait,
    scope: &Scope,
    config: &Config,
) -> Result<()> {
    for app in &config.applications {
        let app_id = key(&app.application_id)?;
        bindings::Entity::delete_many()
            .filter(bindings::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(bindings::Column::UserId.eq(scope.user.id))
            .filter(bindings::Column::ApplicationId.eq(app_id))
            .exec(tx)
            .await?;
        for repo in &app.repositories {
            bindings::ActiveModel {
                id: Set(s.next_id()?.into()),
                workspace_id: Set(scope.workspace_id.into()),
                user_id: Set(scope.user.id.into()),
                application_id: Set(app_id),
                connection_id: Set(key(&repo.connection_id)?),
                repository: Set(repo.repository.clone()),
                version: Set(config.version),
            }
            .insert(tx)
            .await?;
        }
    }
    Ok(())
}
fn summary(row: &defs::Model) -> Result<Value> {
    let mut value = serde_json::to_value(decode(row)?).map_err(|_| bad("data"))?;
    value["id"] = json!(Id::from(row.id));
    value["version"] = json!(row.version);
    value["next_run_at"] = json!(row.next_run_at);
    value["last_generated_at"] = json!(row.last_generated_at);
    value["created_at"] = json!(row.created_at);
    value["updated_at"] = json!(row.updated_at);
    value["last_run_state"] = json!("");
    value["last_error"] = json!("");
    value["object_count"] = json!(if value["dimension"] == "person" {
        value["author_keys"].as_array().map_or(0, Vec::len)
    } else {
        value["applications"].as_array().map_or(0, Vec::len)
    });
    Ok(value)
}
async fn summaries(tx: &impl ConnectionTrait, rows: &[defs::Model]) -> Result<Vec<Value>> {
    use sea_orm::{
        FromQueryResult, QuerySelect,
        sea_query::{ExprTrait, Query as Q},
    };
    #[derive(FromQueryResult)]
    struct Status {
        definition_id: i64,
        state: String,
        error: String,
    }
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let latest = Q::select()
        .expr(Expr::col(runs::Column::Id).max())
        .from(runs::Entity)
        .and_where(Expr::col(runs::Column::WorkspaceId).eq(rows[0].workspace_id))
        .and_where(Expr::col(runs::Column::UserId).eq(rows[0].user_id))
        .and_where(Expr::col(runs::Column::DefinitionId).is_in(rows.iter().map(|r| r.id)))
        .group_by_col(runs::Column::DefinitionId)
        .to_owned();
    let statuses: HashMap<_, _> = runs::Entity::find()
        .select_only()
        .columns([
            runs::Column::DefinitionId,
            runs::Column::State,
            runs::Column::Error,
        ])
        .filter(runs::Column::Id.in_subquery(latest))
        .into_model::<Status>()
        .all(tx)
        .await?
        .into_iter()
        .map(|r| (r.definition_id, r))
        .collect();
    rows.iter()
        .map(|row| {
            let mut value = summary(row)?;
            if let Some(status) = statuses.get(&row.id) {
                value["last_run_state"] = json!(status.state);
                value["last_error"] = json!(crate::i18n::display_stored(&status.error));
            }
            Ok(value)
        })
        .collect()
}
const SPEC: Spec = Spec {
    search: &["name"],
    filters: &[],
    sortable: &[
        "name",
        "created_at",
        "updated_at",
        "next_run_at",
        "last_generated_at",
    ],
    default_sort: "-created_at",
};
async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<defs::Entity, defs::Model>(
        &s.db,
        &SPEC,
        defs::Entity::find().filter(defs::Column::UserId.eq(scope.user.id)),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":summaries(&s.db,&rows).await?,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(mut config): Json<Config>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.create")?;
    let tx = s.db.begin().await?;
    validate(&tx, &scope, &mut config).await?;
    config.version = 1;
    let now = db::now();
    let row = defs::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(scope.workspace_id.into()),
        user_id: Set(scope.user.id.into()),
        name: Set(config.name.clone()),
        insight_id: Set(key(&config.insight_id)?),
        config: Set(encode(&config)?),
        enabled: Set(i64::from(config.enabled)),
        version: Set(1),
        next_run_at: Set(if config.enabled {
            period::next(&config.periods, period::timezone(&config.timezone)?, now)?
        } else {
            0
        }),
        last_generated_at: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await?;
    save_bindings(&s, &tx, &scope, &config).await?;
    tx.commit().await?;
    Ok(Json(summary(&row)?))
}
async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.read")?;
    let row = owned(&s.db, &scope, id).await?;
    source(&s.db, &scope, &decode(&row)?).await?;
    Ok(Json(summaries(&s.db, &[row]).await?.remove(0)))
}
async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut config): Json<Config>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.update")?;
    let tx = s.db.begin().await?;
    let row = owned(&tx, &scope, id).await?;
    if config.version != row.version {
        return Err(conflict("changed"));
    }
    validate(&tx, &scope, &mut config).await?;
    config.version = row.version + 1;
    let now = db::now();
    let result = defs::Entity::update_many()
        .col_expr(defs::Column::Name, Expr::value(config.name.clone()))
        .col_expr(
            defs::Column::InsightId,
            Expr::value(key(&config.insight_id)?),
        )
        .col_expr(defs::Column::Config, Expr::value(encode(&config)?))
        .col_expr(
            defs::Column::Enabled,
            Expr::value(i64::from(config.enabled)),
        )
        .col_expr(defs::Column::Version, Expr::value(config.version))
        .col_expr(defs::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            defs::Column::NextRunAt,
            Expr::value(if config.enabled {
                period::next(&config.periods, period::timezone(&config.timezone)?, now)?
            } else {
                0
            }),
        )
        .filter(defs::Column::Id.eq(row.id))
        .filter(defs::Column::Version.eq(row.version))
        .exec(&tx)
        .await?;
    if result.rows_affected != 1 {
        return Err(conflict("changed"));
    }
    save_bindings(&s, &tx, &scope, &config).await?;
    let row = owned(&tx, &scope, id).await?;
    tx.commit().await?;
    Ok(Json(summary(&row)?))
}
async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.delete")?;
    let tx = s.db.begin().await?;
    let row = owned(&tx, &scope, id).await?;
    runs::Entity::delete_many()
        .filter(runs::Column::DefinitionId.eq(row.id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    defs::Entity::delete_by_id(row.id).exec(&tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct Generate {
    period: String,
    period_start: Option<String>,
}
async fn generate(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Generate>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.generate")?;
    let tx = s.db.begin().await?;
    let row = owned(&tx, &scope, id).await?;
    let config = decode(&row)?;
    source(&tx, &scope, &config).await?;
    if !config.periods.contains(&body.period) {
        return Err(bad("notConfigured"));
    }
    let tz = period::timezone(&config.timezone)?;
    let today = period::local_date(db::now(), tz)?;
    let date = match body.period_start {
        Some(value) => {
            chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|_| bad("date"))?
        }
        None => period::floor(today, &body.period)? - chrono::Duration::days(1),
    };
    let (start, end) = period::bounds(&body.period, date, tz)?;
    let run = enqueue(&s, &tx, &row, &body.period, start, end, "manual", None).await?;
    tx.commit().await?;
    Ok(Json(run_json(&run)?))
}
async fn enqueue(
    s: &AppState,
    tx: &impl ConnectionTrait,
    row: &defs::Model,
    period: &str,
    start: i64,
    end: i64,
    trigger: &str,
    idempotency: Option<String>,
) -> Result<runs::Model> {
    let id = s.next_id()?;
    let idempotency = idempotency.unwrap_or_else(|| format!("manual:{id}"));
    if let Some(run) = runs::Entity::find()
        .filter(runs::Column::IdempotencyKey.eq(&idempotency))
        .one(tx)
        .await?
    {
        return Ok(run);
    }
    Ok(runs::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(row.workspace_id),
        user_id: Set(row.user_id),
        definition_id: Set(row.id),
        period: Set(period.into()),
        period_start: Set(start),
        period_end: Set(end),
        definition_version: Set(row.version),
        state: Set("queued".into()),
        trigger_kind: Set(trigger.into()),
        idempotency_key: Set(idempotency),
        lease_token: Set(String::new()),
        lease_until: Set(0),
        config_snapshot: Set(row.config.clone()),
        snapshot: Set("{}".into()),
        error: Set(String::new()),
        created_at: Set(db::now()),
        generated_at: Set(0),
    }
    .insert(tx)
    .await?)
}
fn run_json(row: &runs::Model) -> Result<Value> {
    let mut snapshot: Value = serde_json::from_str(&row.snapshot).map_err(|_| bad("snapshot"))?;
    localize_snapshot(&mut snapshot);
    Ok(run_fields(row, snapshot))
}
fn localize_snapshot(snapshot: &mut Value) {
    // Preserve immutable stored history, but do not expose retired review data.
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("review_evidence");
    }
    if let Some(groups) = snapshot.get_mut("groups").and_then(Value::as_array_mut) {
        for group in groups {
            if let Some(object) = group.as_object_mut() {
                object.remove("review_evidence");
            }
        }
    }

    if let Some(sources) = snapshot["coverage"]["sources"].as_array_mut() {
        for source in sources {
            if let Some(error) = source["last_error"].as_str() {
                source["last_error"] = json!(crate::i18n::display_stored(error));
            }
        }
    }
    if let Some(error) = snapshot["coverage"]["identity_error"].as_str() {
        snapshot["coverage"]["identity_error"] = json!(crate::i18n::display_stored(error));
    }
}
fn run_fields(row: &runs::Model, snapshot: Value) -> Value {
    json!({"id":Id::from(row.id),"definition_id":Id::from(row.definition_id),"period":row.period,"period_start":row.period_start,"period_end":row.period_end,"definition_version":row.definition_version,"state":row.state,"trigger_kind":row.trigger_kind,"created_at":row.created_at,"generated_at":row.generated_at,"error":crate::i18n::display_stored(&row.error),"snapshot":snapshot})
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct HistorySnapshot {
    summary: Value,
    coverage: Value,
    previous: Value,
    preview: bool,
}
fn history_json(row: &runs::Model) -> Result<Value> {
    // Deserialize only history fields; evidence is available on the detail and
    // export endpoints and is never copied into every row of a history page.
    let value: HistorySnapshot =
        serde_json::from_str(&row.snapshot).map_err(|_| bad("snapshot"))?;
    let mut snapshot = serde_json::to_value(value).map_err(|_| bad("snapshot"))?;
    localize_snapshot(&mut snapshot);
    Ok(run_fields(row, snapshot))
}
async fn authorize_history(
    tx: &impl ConnectionTrait,
    scope: &Scope,
    row: &defs::Model,
    run: &runs::Model,
) -> Result<()> {
    let config: Config = serde_json::from_str(&run.config_snapshot).map_err(|_| bad("snapshot"))?;
    let insight = source(tx, scope, &config).await?;
    if config.dimension == "application" {
        scope.require("deployments.read")?;
        for app in &config.applications {
            crate::entities::deployments::Entity::find_by_id(key(&app.application_id)?)
                .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
                .one(tx)
                .await?
                .ok_or_else(ApiError::missing)?;
        }
    }
    let current = authorized_sources(tx, scope, &insight).await?;
    let snapshot: HistorySnapshot =
        serde_json::from_str(&run.snapshot).map_err(|_| bad("snapshot"))?;
    if row.user_id != run.user_id || row.workspace_id != run.workspace_id {
        return Err(ApiError::missing());
    }
    if let Some(sources) = snapshot.coverage["sources"].as_array() {
        for old in sources {
            if old["state"] == "unavailable" {
                continue;
            }
            let connection = old["connection_id"].as_str().unwrap_or_default();
            let path = old["repository"].as_str().unwrap_or_default();
            if !current.iter().any(|s| {
                s.id != 0 && s.connection_id.to_string() == connection && s.remote_path == path
            }) {
                return Err(ApiError::forbidden());
            }
        }
    }
    Ok(())
}
const RUN_SPEC: Spec = Spec {
    search: &[],
    filters: &["period", "state"],
    sortable: &["created_at", "generated_at", "period_start"],
    default_sort: "-created_at",
};
async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.read")?;
    let row = owned(&s.db, &scope, id).await?;
    source(&s.db, &scope, &decode(&row)?).await?;
    let mut select = runs::Entity::find()
        .filter(runs::Column::DefinitionId.eq(row.id))
        .filter(runs::Column::UserId.eq(scope.user.id));
    if let Some(value) = raw.remove("period_start") {
        let date =
            chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|_| bad("date"))?;
        let tz = period::timezone(&decode(&row)?.timezone)?;
        select = select.filter(runs::Column::PeriodStart.eq(period::boundary(date, tz)?));
    }
    let query = ListQuery::parse(&RUN_SPEC, &raw)?;
    let (rows, total) = resource::list::<runs::Entity, runs::Model>(
        &s.db,
        &RUN_SPEC,
        select,
        &scope.workspace_id,
        &query,
    )
    .await?;
    let mut items = vec![];
    for run in rows {
        authorize_history(&s.db, &scope, &row, &run).await?;
        items.push(history_json(&run)?);
    }
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
async fn load_run(s: &AppState, scope: &Scope, id: Id, run_id: Id) -> Result<Value> {
    let row = owned(&s.db, scope, id).await?;
    let run = runs::Entity::find_by_id(i64::from(run_id))
        .filter(runs::Column::DefinitionId.eq(row.id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(runs::Column::UserId.eq(scope.user.id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    authorize_history(&s.db, scope, &row, &run).await?;
    run_json(&run)
}
async fn run_detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, run_id)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.read")?;
    Ok(Json(load_run(&s, &scope, id, run_id).await?))
}
async fn export(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, run_id)): Path<(String, Id, Id)>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<impl axum::response::IntoResponse> {
    scope.require("insight-reports.read")?;
    let value = load_run(&s, &scope, id, run_id).await?;
    if value["state"] != "complete" {
        return Err(conflict("unfinished"));
    }
    let (content_type, filename, content) = exports::render(
        &value,
        raw.get("format").map(String::as_str).unwrap_or("json"),
    )?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::CONTENT_DISPOSITION, filename),
        ],
        content,
    ))
}

async fn options(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("insight-reports.read")?;
    let insight = insights::owned(
        &s.db,
        &scope,
        key(raw
            .get("insight_id")
            .map(String::as_str)
            .unwrap_or_default())?
        .into(),
    )
    .await?;
    scope.require("code-insights.read")?;
    let sources = authorized_sources(&s.db, &scope, &insight).await?;
    let identities = analysis::report_author_identities(&s.db, &scope, &insight, &sources).await?;
    let mut by_person: std::collections::BTreeMap<i64, Vec<_>> = std::collections::BTreeMap::new();
    for identity in identities {
        by_person
            .entry(identity.contributor_id)
            .or_default()
            .push(identity);
    }
    let people:Vec<_>=by_person.into_iter().map(|(id,identities)|{
        let first=&identities[0];
        let aliases:Vec<_>=identities.iter().map(|identity|json!({"name":identity.author_name,"email":identity.author_email})).collect();
        json!({"key":Id::from(id),"nickname":first.nickname,"name":first.author_name,"email":first.author_email,"aliases":aliases})
    }).collect();
    let mut apps = vec![];
    if scope.require("deployments.read").is_ok() {
        for app in crate::entities::deployments::Entity::find()
            .filter(crate::entities::deployments::Column::WorkspaceId.eq(scope.workspace_id))
            .all(&s.db)
            .await?
        {
            let bindings = bindings::Entity::find()
                .filter(bindings::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(bindings::Column::UserId.eq(scope.user.id))
                .filter(bindings::Column::ApplicationId.eq(app.id))
                .all(&s.db)
                .await?;
            apps.push(json!({"id":Id::from(app.id),"name":app.name,"repositories":bindings.iter().filter(|b|sources.iter().any(|s|s.id!=0&&s.connection_id==b.connection_id&&s.remote_path==b.repository)).map(|b|json!({"connection_id":Id::from(b.connection_id),"repository":b.repository})).collect::<Vec<_>>()}));
        }
    }
    Ok(Json(
        json!({"people":people,"applications":apps,"repositories":sources.iter().filter(|s|s.id!=0).map(|s|json!({"connection_id":Id::from(s.connection_id),"repository":s.remote_path,"state":s.state})).collect::<Vec<_>>()}),
    ))
}

/// Recover missed fixed calendar schedules, then execute a bounded leased batch.
pub(crate) async fn tick(s: &AppState) -> Result<()> {
    let now = db::now();
    let due = defs::Entity::find()
        .filter(defs::Column::Enabled.eq(1))
        .filter(defs::Column::NextRunAt.gt(0))
        .filter(defs::Column::NextRunAt.lte(now))
        .all(&s.db)
        .await?;
    for row in due {
        if let Err(error) = schedule(s, &row, now).await {
            tracing::warn!(%error, report=row.id,"report schedule deferred");
        }
    }
    generation::work(s).await
}
async fn schedule(s: &AppState, row: &defs::Model, now: i64) -> Result<()> {
    let tx = s.db.begin().await?;
    let current = defs::Entity::find_by_id(row.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.version != row.version
        || current.next_run_at != row.next_run_at
        || current.enabled == 0
    {
        return Ok(());
    }
    let config = decode(row)?;
    let tz = period::timezone(&config.timezone)?;
    let mut cursor = row.next_run_at;
    let mut count = 0;
    while cursor <= now && count < 100 {
        let date = period::local_date(cursor - 1_800_000, tz)?;
        for period in &config.periods {
            if period::floor(date, period)? == date {
                let previous = date - chrono::Duration::days(1);
                let (start, end) = period::bounds(period, previous, tz)?;
                enqueue(
                    s,
                    &tx,
                    row,
                    period,
                    start,
                    end,
                    "scheduled",
                    Some(format!(
                        "schedule:{}:{}:{}:{}",
                        row.id, row.version, period, start
                    )),
                )
                .await?;
            }
        }
        cursor = period::next(&config.periods, tz, cursor)?;
        count += 1;
    }
    defs::Entity::update_many()
        .col_expr(defs::Column::NextRunAt, Expr::value(cursor))
        .filter(defs::Column::Id.eq(row.id))
        .filter(defs::Column::Version.eq(row.version))
        .filter(defs::Column::NextRunAt.eq(row.next_run_at))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
