//! A shared scheduler for host, build and deployment tasks.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        cron_jobs as jobs, cron_runs as runs, deployment_revisions, deployments, host_groups,
        hosts, pipeline_runs, pipelines,
    },
    error::{ApiError, Result},
    i18n::{self, Message, StoredText},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, str::FromStr, time::Duration};
use tokio::sync::watch;

const SPEC: Spec = Spec {
    search: &["name", "description"],
    filters: &["scope"],
    sortable: &["name", "next_run_at", "created_at", "updated_at"],
    default_sort: "name",
};
const RUN_SPEC: Spec = Spec {
    search: &["job_name"],
    filters: &["job_id", "scope", "state"],
    sortable: &["created_at", "scheduled_at", "started_at"],
    default_sort: "-created_at",
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Body {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub scope: String,
    pub schedule: String,
    pub timezone: String,
    #[serde(default)]
    pub enabled: bool,
    pub action: String,
    pub target_kind: String,
    pub target_ref: String,
    #[serde(default)]
    pub environment_id: String,
    #[serde(default)]
    pub command: String,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default = "retry_delay")]
    pub retry_delay_seconds: u64,
    #[serde(default)]
    pub version: i64,
}
fn retry_delay() -> u64 {
    30
}
fn next(body: &Body, after: i64) -> Result<i64> {
    next_schedule(&body.schedule, &body.timezone, after)
}

pub(crate) fn next_schedule(schedule: &str, timezone: &str, after: i64) -> Result<i64> {
    if schedule.len() > 256 || ![5, 6].contains(&schedule.split_whitespace().count()) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.fieldsInvalid"),
        ));
    }
    let schedule = croner::parser::CronParser::builder()
        .seconds(croner::parser::Seconds::Optional)
        .build()
        .parse(schedule)
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("cron.expressionInvalid"),
            )
        })?;
    let timezone = chrono_tz::Tz::from_str(timezone).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.timezoneInvalid"),
        )
    })?;
    let mut cursor = DateTime::<Utc>::from_timestamp_millis(after)
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("cron.timeInvalid"),
            )
        })?
        .with_timezone(&timezone);
    // Do not make up a catch-up occurrence for a wall-clock time skipped by DST.
    for _ in 0..32 {
        let candidate = schedule.find_next_occurrence(&cursor, false).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("cron.noFutureTime"),
            )
        })?;
        if candidate.timestamp_millis() > after
            && schedule.is_time_matching(&candidate).unwrap_or(false)
        {
            return Ok(candidate.timestamp_millis());
        }
        cursor = candidate;
    }
    Err(ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("cron.noFutureLocalTime"),
    ))
}

async fn validate(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    b: &Body,
    schedule: bool,
) -> Result<()> {
    if b.name.trim().is_empty()
        || b.name.chars().count() > 60
        || !(1..=86400).contains(&b.timeout_seconds)
        || b.max_retries > 10
        || !(1..=3600).contains(&b.retry_delay_seconds)
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.configurationInvalid"),
        ));
    }
    if schedule {
        next(b, db::now())?;
    }
    scope.require("cron-jobs.run")?;
    let id = Id::reference(&b.target_ref)?;
    let present = match (b.scope.as_str(), b.action.as_str(), b.target_kind.as_str()) {
        ("host", "command", "host") => {
            scope.require("hosts.terminal")?;
            hosts::Entity::find_by_id(i64::from(id))
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .one(conn)
                .await?
                .is_some()
        }
        ("host", "command", "host-group") => {
            scope.require("hosts.terminal")?;
            scope.require("host-groups.read")?;
            host_groups::Entity::find_by_id(i64::from(id))
                .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                .one(conn)
                .await?
                .is_some()
        }
        ("build", "pipeline", "pipeline") => {
            scope.require("pipelines.run")?;
            crate::pipelines::contexts::selected(
                conn,
                scope.workspace_id,
                id,
                Some(&b.environment_id),
            )
            .await?;
            pipelines::Entity::find_by_id(i64::from(id))
                .filter(pipelines::Column::WorkspaceId.eq(scope.workspace_id))
                .one(conn)
                .await?
                .is_some()
        }
        ("deploy", "sync-marketplace", "marketplace-source") => {
            scope.require("marketplace-sources.sync")?;
            crate::entities::marketplace_sources::Entity::find_by_id(i64::from(id))
                .filter(
                    crate::entities::marketplace_sources::Column::WorkspaceId
                        .eq(scope.workspace_id),
                )
                .one(conn)
                .await?
                .is_some()
        }
        ("deploy", "redeploy" | "restart", "deployment") => {
            scope.require("deployments.apply")?;
            deployments::Entity::find_by_id(i64::from(id))
                .filter(deployments::Column::WorkspaceId.eq(scope.workspace_id))
                .one(conn)
                .await?
                .is_some()
        }
        _ => {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("cron.targetMismatch"),
            ));
        }
    };
    if !present {
        return Err(ApiError::missing());
    }
    if b.scope == "host"
        && (b.command.trim().is_empty() || b.command.len() > 65536 || b.command.contains('\0'))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.commandRequired"),
        ));
    }
    if b.scope != "host" && !b.command.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.commandForbidden"),
        ));
    }
    Ok(())
}
fn job_json(row: jobs::Model) -> Result<Value> {
    let mut value: Value = serde_json::from_str(&row.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.configCorrupt"),
        )
    })?;
    value["id"] = json!(Id::from(row.id));
    value["version"] = json!(row.version);
    value["enabled"] = json!(row.enabled == 1);
    value["next_run_at"] = json!(row.next_run_at);
    value["run_as"] = json!(Id::from(row.run_as));
    value["run_as_name"] = json!(row.run_as_name);
    value["created_at"] = json!(row.created_at);
    value["updated_at"] = json!(row.updated_at);
    Ok(value)
}
fn run_json(row: runs::Model, detail: bool) -> Value {
    let mut value = json!({"id":Id::from(row.id),"job_id":Id::from(row.job_id),"job_name":row.job_name,"scope":row.scope,"state":row.state,"trigger_kind":row.trigger_kind,"triggered_by":Id::from(row.triggered_by),"scheduled_at":row.scheduled_at,"started_at":row.started_at,"finished_at":row.finished_at,"attempt":row.attempt,"downstream_id":Id::from(row.downstream_id),"downstream_kind":row.downstream_kind,"created_at":row.created_at,"version":row.version});
    if detail {
        value["output"] = json!(i18n::display_text_stored(&row.output));
        value["config"] = json!(row.config);
    }
    value
}
async fn scoped(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<jobs::Model> {
    jobs::Entity::find_by_id(i64::from(id))
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<jobs::Entity, jobs::Model>(
        &s.db,
        &SPEC,
        jobs::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(job_json).collect::<Result<Vec<_>>>()?,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.read")?;
    Ok(Json(job_json(scoped(&s.db, &scope, id).await?)?))
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(b): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_in(&s, &tx, &scope, b).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    b: Body,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("cron-jobs.create")?;
    let id = s.next_id()?;
    let now = db::now();
    validate(tx, &scope, &b, true).await?;
    jobs::Entity::insert(jobs::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(b.name.trim().into()),
        description: Set(b.description.clone()),
        scope: Set(b.scope.clone()),
        target_kind: Set(b.target_kind.clone()),
        target_ref: Set(Id::reference(&b.target_ref)?.into()),
        config: Set(serde_json::to_string(&b).unwrap()),
        enabled: Set(i64::from(b.enabled)),
        run_as: Set(scope.user.id.into()),
        run_as_name: Set(scope.user.name.clone()),
        next_run_at: Set(if b.enabled { next(&b, now)? } else { 0 }),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "cron-jobs",
        &id,
        json!({"name":b.name,"enabled":b.enabled}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(b): Json<Body>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    validate(&tx, &scope, &b, true).await?;
    let old = scoped(&tx, &scope, id).await?;
    if old.scope != b.scope {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.moduleImmutable"),
        ));
    }
    let count = jobs::Entity::update_many()
        .col_expr(jobs::Column::Name, Expr::value(b.name.trim()))
        .col_expr(
            jobs::Column::Description,
            Expr::value(b.description.clone()),
        )
        .col_expr(
            jobs::Column::Config,
            Expr::value(serde_json::to_string(&b).unwrap()),
        )
        .col_expr(jobs::Column::TargetKind, Expr::value(b.target_kind.clone()))
        .col_expr(
            jobs::Column::TargetRef,
            Expr::value(Id::reference(&b.target_ref)?),
        )
        .col_expr(jobs::Column::Enabled, Expr::value(i64::from(b.enabled)))
        .col_expr(jobs::Column::RunAs, Expr::value(scope.user.id))
        .col_expr(
            jobs::Column::RunAsName,
            Expr::value(scope.user.name.clone()),
        )
        .col_expr(
            jobs::Column::NextRunAt,
            Expr::value(if b.enabled { next(&b, db::now())? } else { 0 }),
        )
        .col_expr(
            jobs::Column::Version,
            Expr::col(jobs::Column::Version).add(1),
        )
        .col_expr(jobs::Column::UpdatedAt, Expr::value(db::now()))
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(jobs::Column::Id.eq(id))
        .filter(jobs::Column::Version.eq(b.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if count == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.configChanged"),
        ));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "cron-jobs",
        &id,
        json!({"name":b.name,"enabled":b.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":b.version+1})))
}
async fn busy(conn: &impl ConnectionTrait, workspace: Id, job: Id) -> Result<bool> {
    Ok(runs::Entity::find()
        .filter(runs::Column::WorkspaceId.eq(workspace))
        .filter(runs::Column::JobId.eq(job))
        .filter(runs::Column::State.is_in(["queued", "running", "needs-review"]))
        .one(conn)
        .await?
        .is_some())
}
pub async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("cron-jobs.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    scoped(&tx, &scope, id).await?;
    crate::notifications::alerts::ensure_unreferenced(&tx, scope.workspace_id, "cron-job", &[id])
        .await?;
    if busy(&tx, scope.workspace_id, id).await? {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.runsBusy"),
        ));
    }
    runs::Entity::delete_many()
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(runs::Column::JobId.eq(id))
        .exec(&tx)
        .await?;
    jobs::Entity::delete_many()
        .filter(jobs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(jobs::Column::Id.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "cron-jobs", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn insert_run(
    s: &AppState,
    conn: &impl ConnectionTrait,
    job: &jobs::Model,
    actor: Id,
    kind: &str,
    state: &str,
    message: Option<Message>,
    scheduled: i64,
) -> Result<Id> {
    let id = s.next_id()?;
    let now = db::now();
    runs::Entity::insert(runs::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(job.workspace_id),
        job_id: Set(job.id),
        job_name: Set(job.name.clone()),
        scope: Set(job.scope.clone()),
        config: Set(job.config.clone()),
        target_kind: Set(job.target_kind.clone()),
        target_ref: Set(job.target_ref),
        state: Set(state.into()),
        trigger_kind: Set(kind.into()),
        triggered_by: Set(actor.into()),
        scheduled_at: Set(scheduled),
        started_at: Set(0),
        finished_at: Set(if state == "skipped" { now } else { 0 }),
        attempt: Set(0),
        output: Set(message
            .map(|message| StoredText::from_message(message).stored())
            .unwrap_or_default()),
        downstream_id: Set(0),
        downstream_kind: Set(String::new()),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
    })
    .exec(conn)
    .await?;
    Ok(id)
}
pub async fn run_now(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("cron-jobs.run")?;
    let row = scoped(&s.db, &scope, id).await?;
    let b: Body = serde_json::from_str(&row.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.configCorrupt"),
        )
    })?;
    validate(&s.db, &scope, &b, false).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let current = scoped(&tx, &scope, id).await?;
    if current.version != row.version || busy(&tx, scope.workspace_id, id).await? {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.changedOrBusy"),
        ));
    }
    let run = insert_run(
        &s,
        &tx,
        &row,
        scope.user.id,
        "manual",
        "queued",
        None,
        db::now(),
    )
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "run-now",
        "cron-jobs",
        &id,
        json!({"run_id":run}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"id":run}))))
}
pub async fn list_runs(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.read")?;
    let q = ListQuery::parse(&RUN_SPEC, &raw)?;
    let (rows, total) = resource::list::<runs::Entity, runs::Model>(
        &s.db,
        &RUN_SPEC,
        runs::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(|r|run_json(r,false)).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.read")?;
    scoped(&s.db, &scope, id).await?;
    raw.insert("job_id".into(), id.to_string());
    list_runs(State(s), scope, Query(raw)).await
}
pub async fn get_run(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.read")?;
    let row = runs::Entity::find_by_id(i64::from(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(run_json(row, true)))
}
#[derive(Deserialize)]
pub struct Review {
    version: i64,
    note: String,
}
pub async fn acknowledge(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(b): Json<Review>,
) -> Result<Json<Value>> {
    scope.require("cron-jobs.run")?;
    if b.note.trim().is_empty() || b.note.chars().count() > 1000 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.noteRequired"),
        ));
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = runs::Entity::find_by_id(i64::from(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut output = StoredText::from_stored(&row.output);
    output.push_message(Message::new("cron.manualNote").with("note", b.note.clone()));
    let n = runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value("failed"))
        .col_expr(runs::Column::Output, Expr::value(output.stored()))
        .col_expr(runs::Column::FinishedAt, Expr::value(db::now()))
        .col_expr(
            runs::Column::Version,
            Expr::col(runs::Column::Version).add(1),
        )
        .filter(runs::Column::Id.eq(id))
        .filter(runs::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(runs::Column::Version.eq(b.version))
        .filter(runs::Column::State.eq("needs-review"))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.reviewChanged"),
        ));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "acknowledge",
        "cron-runs",
        &id,
        json!({"note":b.note}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

async fn due(s: &AppState, recovery: bool) -> Result<()> {
    let now = db::now();
    loop {
        let candidates = jobs::Entity::find()
            .filter(jobs::Column::Enabled.eq(1))
            .filter(jobs::Column::NextRunAt.lte(now))
            .order_by_asc(jobs::Column::NextRunAt)
            .limit(100)
            .all(&s.db)
            .await?;
        let count = candidates.len();
        for job in candidates {
            let b: Body = serde_json::from_str(&job.config).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("cron.configCorrupt"),
                )
            })?;
            let future = next(&b, now).unwrap_or(0);
            let tx = crate::variables::transaction(s, job.workspace_id.into()).await?;
            let n = jobs::Entity::update_many()
                .col_expr(jobs::Column::NextRunAt, Expr::value(future))
                .col_expr(jobs::Column::Enabled, Expr::value(i64::from(future > 0)))
                .filter(jobs::Column::Id.eq(job.id))
                .filter(jobs::Column::Enabled.eq(1))
                .filter(jobs::Column::Version.eq(job.version))
                .filter(jobs::Column::NextRunAt.eq(job.next_run_at))
                .exec(&tx)
                .await?
                .rows_affected;
            if n == 0 {
                continue;
            }
            let overlap = busy(&tx, job.workspace_id.into(), job.id.into()).await?;
            let run_id = insert_run(
                s,
                &tx,
                &job,
                job.run_as.into(),
                "scheduled",
                if recovery || overlap {
                    "skipped"
                } else {
                    "queued"
                },
                if recovery {
                    Some(Message::new("cron.skipDowntime"))
                } else if overlap {
                    Some(Message::new("cron.skipOverlap"))
                } else {
                    None
                },
                job.next_run_at,
            )
            .await?;
            crate::hosts::activity_record(
                s,
                &tx,
                job.workspace_id.into(),
                (job.run_as.into(), &job.run_as_name),
                if recovery || overlap {
                    "schedule-skip"
                } else {
                    "scheduled"
                },
                "cron-jobs",
                job.id.into(),
                json!({"run_id":run_id,"scheduled_at":job.next_run_at}),
            )
            .await?;
            tx.commit().await?;
        }
        if !recovery || count < 100 {
            break;
        }
    }
    Ok(())
}
async fn finish(s: &AppState, id: i64, state: &str, output: &StoredText) -> Result<()> {
    let mut text = output.clone();
    text.truncate_raw(1_048_576);
    runs::Entity::update_many()
        .col_expr(runs::Column::State, Expr::value(state))
        .col_expr(runs::Column::Output, Expr::value(text.stored()))
        .col_expr(
            runs::Column::FinishedAt,
            Expr::value(if state == "queued" { 0 } else { db::now() }),
        )
        .col_expr(runs::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            runs::Column::Version,
            Expr::col(runs::Column::Version).add(1),
        )
        .filter(runs::Column::Id.eq(id))
        .exec(&s.db)
        .await?;
    Ok(())
}

struct Outcome {
    state: &'static str,
    output: StoredText,
    retryable: bool,
}
impl Outcome {
    fn new(state: &'static str, output: StoredText, retryable: bool) -> Self {
        Self {
            state,
            output,
            retryable,
        }
    }

    fn message(state: &'static str, message: Message, retryable: bool) -> Self {
        Self::new(state, StoredText::from_message(message), retryable)
    }
}
async fn downstream(
    s: &AppState,
    run: &runs::Model,
    b: &Body,
    id: i64,
    kind: &str,
    stop: &mut watch::Receiver<bool>,
    drain: &mut watch::Receiver<bool>,
) -> Result<Outcome> {
    let started = std::time::Instant::now();
    loop {
        let (state, detail) = if kind == "pipeline-runs" {
            let r = pipeline_runs::Entity::find_by_id(id)
                .filter(pipeline_runs::Column::WorkspaceId.eq(run.workspace_id))
                .one(&s.db)
                .await?
                .ok_or_else(ApiError::missing)?;
            (
                r.state,
                Message::new("cron.pipelineRun").with("id", id.to_string()),
            )
        } else {
            let r = deployment_revisions::Entity::find_by_id(id)
                .filter(deployment_revisions::Column::WorkspaceId.eq(run.workspace_id))
                .one(&s.db)
                .await?
                .ok_or_else(ApiError::missing)?;
            (
                r.state,
                Message::new("cron.deploymentRun").with("id", id.to_string()),
            )
        };
        match state.as_str() {
            "succeeded" => {
                return Ok(Outcome::message(
                    "succeeded",
                    Message::new("cron.downstreamSucceeded").with_message("detail", detail),
                    false,
                ));
            }
            "failed" | "cancelled" => {
                return Ok(Outcome::message(
                    "failed",
                    Message::new("cron.downstreamFailed").with_message("detail", detail),
                    state == "failed",
                ));
            }
            "needs-review" => {
                return Ok(Outcome::message(
                    "needs-review",
                    Message::new("cron.downstreamUncertain").with_message("detail", detail),
                    false,
                ));
            }
            "queued" if *drain.borrow() => {
                return Ok(Outcome::message(
                    "queued",
                    Message::new("cron.downstreamQueued").with_message("detail", detail),
                    false,
                ));
            }
            _ => {}
        }
        if *stop.borrow() || started.elapsed() >= Duration::from_secs(b.timeout_seconds) {
            return Ok(Outcome::message(
                "needs-review",
                Message::new("cron.downstreamTimeout").with_message("detail", detail),
                false,
            ));
        }
        tokio::select! {_=stop.changed()=>{},_=drain.changed()=>{},_=tokio::time::sleep(Duration::from_millis(200))=>{}}
    }
}
async fn once(
    s: &AppState,
    run: &runs::Model,
    b: &Body,
    stop: &mut watch::Receiver<bool>,
    drain: &mut watch::Receiver<bool>,
) -> Result<Outcome> {
    let scope = match Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await {
        Ok(scope) => scope,
        Err(_) => {
            return Ok(Outcome::message(
                "failed",
                Message::new("cron.actorUnavailable"),
                false,
            ));
        }
    };
    if validate(&s.db, &scope, b, false).await.is_err() {
        return Ok(Outcome::message(
            "failed",
            Message::new("cron.targetUnavailable"),
            false,
        ));
    }
    if run.downstream_id != 0 {
        return downstream(
            s,
            run,
            b,
            run.downstream_id,
            &run.downstream_kind,
            stop,
            drain,
        )
        .await;
    }
    if b.scope == "host" {
        let target = Id::reference(&b.target_ref)?;
        let ids = if b.target_kind == "host" {
            vec![target]
        } else {
            hosts::Entity::find()
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(hosts::Column::GroupId.eq(target))
                .order_by_asc(hosts::Column::Id)
                .all(&s.db)
                .await?
                .into_iter()
                .map(|h| Id::from(h.id))
                .collect()
        };
        if ids.is_empty() {
            return Ok(Outcome::message(
                "failed",
                Message::new("cron.groupEmpty"),
                false,
            ));
        }
        let mut output = StoredText::new();
        for id in ids {
            let fresh =
                Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
            fresh.require("hosts.terminal")?;
            fresh.require("cron-jobs.run")?;
            let channel = match crate::hosts::remote::open(s, &scope.workspace_id, &id).await {
                Ok(channel) => channel,
                Err(_) => {
                    output.push_message(
                        Message::new("cron.hostConnectFailed").with("host", id.to_string()),
                    );
                    return Ok(Outcome::new("failed", output, true));
                }
            };
            let result = tokio::select! { result=channel.exec(&b.command,Duration::from_secs(b.timeout_seconds))=>result, _=stop.changed()=>return Ok(Outcome::message("needs-review",Message::new("cron.remoteInterrupted"),false)) };
            let out = match result {
                Ok(out) => out,
                Err(_) => {
                    output.push_message(
                        Message::new("cron.hostUncertain").with("host", id.to_string()),
                    );
                    return Ok(Outcome::new("needs-review", output, false));
                }
            };
            output.push_message(
                Message::new("cron.hostExit")
                    .with("host", id.to_string())
                    .with("code", out.code.to_string()),
            );
            output.push_raw(out.stdout);
            output.push_raw("\n");
            output.push_raw(out.stderr);
            output.push_raw("\n");
            if out.code != 0 {
                return Ok(Outcome::new("failed", output, true));
            }
        }
        return Ok(Outcome::new("succeeded", output, false));
    }
    if *drain.borrow() {
        return Ok(Outcome::message(
            "queued",
            Message::new("cron.shutdownQueued"),
            false,
        ));
    }
    if b.action == "sync-marketplace" {
        return match crate::marketplace::synchronize(
            s,
            scope,
            Id::reference(&b.target_ref)?,
            Duration::from_secs(b.timeout_seconds),
        )
        .await
        {
            Ok(count) => Ok(Outcome::message(
                "succeeded",
                Message::new("cron.marketplaceSynced").with("count", count.to_string()),
                false,
            )),
            Err(_) => Ok(Outcome::message(
                "failed",
                Message::new("cron.marketplaceFailed"),
                true,
            )),
        };
    }
    let (id, kind) = if b.scope == "build" {
        let (_, Json(value)) = crate::pipelines::runs::trigger_as(
            s.clone(),
            scope,
            b.target_ref.clone(),
            bytes::Bytes::from(json!({"environment_id":b.environment_id}).to_string()),
            "schedule",
        )
        .await?;
        (
            Id::reference(value["id"].as_str().ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("cron.runIdMissing"),
                )
            })?)?,
            "pipeline-runs",
        )
    } else {
        (
            match crate::deployments::scheduled(s, scope, &b.target_ref, b.action == "restart")
                .await
            {
                Ok(id) => id,
                Err(error) if error.0.is_client_error() => {
                    let mut output =
                        StoredText::from_message(Message::new("cron.deploymentRejected"));
                    if let Some(message) = error.2 {
                        output.push_message(message);
                    } else {
                        output.push_raw(error.1);
                    }
                    return Ok(Outcome::new("failed", output, false));
                }
                Err(error) => return Err(error),
            },
            "deployment-revisions",
        )
    };
    runs::Entity::update_many()
        .col_expr(runs::Column::DownstreamId, Expr::value(id))
        .col_expr(runs::Column::DownstreamKind, Expr::value(kind))
        .filter(runs::Column::Id.eq(run.id))
        .exec(&s.db)
        .await?;
    downstream(s, run, b, id.into(), kind, stop, drain).await
}
async fn execute(
    s: AppState,
    mut run: runs::Model,
    mut stop: watch::Receiver<bool>,
    mut drain: watch::Receiver<bool>,
) -> Result<()> {
    let b: Body = serde_json::from_str(&run.config).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("cron.configCorrupt"),
        )
    })?;
    let mut output = StoredText::from_stored(&run.output);
    for attempt in std::cmp::max(run.attempt, 1)..=i64::from(b.max_retries) + 1 {
        runs::Entity::update_many()
            .col_expr(runs::Column::Attempt, Expr::value(attempt))
            .filter(runs::Column::Id.eq(run.id))
            .exec(&s.db)
            .await?;
        let result = match once(&s, &run, &b, &mut stop, &mut drain).await {
            Ok(result) => result,
            Err(error) => Outcome::message(
                "needs-review",
                Message::new("cron.executionInterrupted")
                    .with("status", error.0.as_u16().to_string()),
                false,
            ),
        };
        output.push_message(Message::new("cron.attempt").with("attempt", attempt.to_string()));
        output.extend(result.output);
        output.push_raw("\n");
        if !result.retryable
            || attempt > i64::from(b.max_retries)
            || *stop.borrow()
            || *drain.borrow()
        {
            return finish(&s, run.id, result.state, &output).await;
        }
        tokio::select! { _=stop.changed()=>return finish(&s,run.id,"failed",&output).await,_=drain.changed()=>return finish(&s,run.id,"failed",&output).await,_=tokio::time::sleep(Duration::from_secs(b.retry_delay_seconds))=>{}}
        run.downstream_id = 0;
        run.downstream_kind.clear();
        runs::Entity::update_many()
            .col_expr(runs::Column::DownstreamId, Expr::value(0i64))
            .col_expr(runs::Column::DownstreamKind, Expr::value(""))
            .col_expr(runs::Column::Output, Expr::value(output.stored()))
            .filter(runs::Column::Id.eq(run.id))
            .exec(&s.db)
            .await?;
    }
    finish(&s, run.id, "failed", &output).await
}

pub async fn scheduler(
    s: AppState,
    mut stop: watch::Receiver<bool>,
    mut drain: watch::Receiver<bool>,
) {
    let recovery = async {
        for row in runs::Entity::find()
            .filter(runs::Column::State.eq("running"))
            .all(&s.db)
            .await?
        {
            let child = if row.downstream_id == 0 {
                None
            } else if row.downstream_kind == "pipeline-runs" {
                pipeline_runs::Entity::find_by_id(row.downstream_id)
                    .one(&s.db)
                    .await?
                    .map(|r| r.state)
            } else {
                deployment_revisions::Entity::find_by_id(row.downstream_id)
                    .one(&s.db)
                    .await?
                    .map(|r| r.state)
            };
            let state = match child.as_deref() {
                Some("queued") => "queued",
                Some("succeeded") => "succeeded",
                _ => "needs-review",
            };
            finish(
                &s,
                row.id,
                state,
                &StoredText::from_message(Message::new(if state == "needs-review" {
                    "cron.recoveryUncertain"
                } else {
                    "cron.recoveryTracking"
                })),
            )
            .await?;
        }
        due(&s, true).await
    }
    .await;
    if let Err(error) = recovery {
        tracing::error!(%error,"定时任务恢复失败，暂停调度");
        return;
    }
    let mut active = tokio::task::JoinSet::new();
    let mut next_cleanup = std::time::Instant::now();
    loop {
        if *stop.borrow() || *drain.borrow() {
            break;
        }
        while let Some(result) = active.try_join_next() {
            if let Err(error) = result {
                tracing::error!(%error,"定时任务执行器异常");
            }
        }
        if std::time::Instant::now() >= next_cleanup {
            if let Err(error) = prune(&s).await {
                tracing::error!(%error,"定时任务历史清理失败");
            }
            next_cleanup = std::time::Instant::now() + Duration::from_secs(60);
        }
        if let Err(error) = due(&s, false).await {
            tracing::error!(%error,"定时任务领取失败");
        }
        if active.len() < 32 {
            match runs::Entity::find()
                .filter(runs::Column::State.eq("queued"))
                .order_by_asc(runs::Column::CreatedAt)
                .limit((32 - active.len()) as u64)
                .all(&s.db)
                .await
            {
                Ok(rows) => {
                    for row in rows {
                        let claimed = runs::Entity::update_many()
                            .col_expr(runs::Column::State, Expr::value("running"))
                            .col_expr(runs::Column::StartedAt, Expr::value(db::now()))
                            .filter(runs::Column::Id.eq(row.id))
                            .filter(runs::Column::State.eq("queued"))
                            .exec(&s.db)
                            .await;
                        if claimed.is_ok_and(|r| r.rows_affected == 1) {
                            let state = s.clone();
                            let stopped = stop.clone();
                            let draining = drain.clone();
                            active.spawn(async move {
                                let id = row.id;
                                if let Err(error) =
                                    execute(state.clone(), row, stopped, draining).await
                                {
                                    tracing::error!(%error,"定时任务执行异常");
                                    let _ = finish(
                                        &state,
                                        id,
                                        "needs-review",
                                        &StoredText::from_message(Message::new(
                                            "cron.executorFailed",
                                        )),
                                    )
                                    .await;
                                }
                            });
                        }
                    }
                }
                Err(error) => tracing::error!(%error,"定时任务队列读取失败"),
            }
        }
        tokio::select! {_=stop.changed()=>{},_=drain.changed()=>{},_=tokio::time::sleep(Duration::from_millis(200))=>{}}
    }
    while let Some(result) = active.join_next().await {
        if let Err(error) = result {
            tracing::error!(%error,"等待定时任务退出失败");
        }
    }
}

async fn prune(s: &AppState) -> Result<()> {
    use crate::entities::settings;
    for row in settings::Entity::find()
        .filter(settings::Column::Key.eq("history_retention_days"))
        .filter(settings::Column::Value.gt(0))
        .all(&s.db)
        .await?
    {
        let workspace = Id::from(row.workspace_id);
        let tx = crate::variables::transaction(s, workspace).await?;
        let days = crate::maintenance::setting(&tx, workspace, "history_retention_days").await?;
        if days == 0 {
            continue;
        }
        let rows = runs::Entity::find()
            .filter(runs::Column::WorkspaceId.eq(workspace))
            .filter(runs::Column::State.is_in(["succeeded", "failed", "skipped"]))
            .filter(runs::Column::FinishedAt.gt(0))
            .filter(runs::Column::FinishedAt.lt(db::now() - days * 86_400_000))
            .order_by_asc(runs::Column::FinishedAt)
            .limit(200)
            .all(&tx)
            .await?;
        if !rows.is_empty() {
            let n = runs::Entity::delete_many()
                .filter(runs::Column::WorkspaceId.eq(workspace))
                .filter(runs::Column::Id.is_in(rows.iter().map(|r| r.id)))
                .exec(&tx)
                .await?
                .rows_affected;
            crate::hosts::activity_record(
                s,
                &tx,
                workspace,
                (Id::default(), "系统"),
                "retention-cleanup",
                "cron-runs",
                Id::default(),
                json!({"removed":n,"retention_days":days}),
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// Keep saved and pending schedules valid when pipeline environment bindings change.
pub async fn check_pipeline_environments(
    conn: &impl ConnectionTrait,
    workspace: Id,
    pipeline: Id,
    environments: &std::collections::BTreeSet<Id>,
) -> Result<()> {
    let mut configs = jobs::Entity::find()
        .filter(jobs::Column::WorkspaceId.eq(workspace))
        .filter(jobs::Column::TargetKind.eq("pipeline"))
        .filter(jobs::Column::TargetRef.eq(pipeline))
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.config)
        .collect::<Vec<_>>();
    configs.extend(
        runs::Entity::find()
            .filter(runs::Column::WorkspaceId.eq(workspace))
            .filter(runs::Column::TargetKind.eq("pipeline"))
            .filter(runs::Column::TargetRef.eq(pipeline))
            .filter(runs::Column::State.is_in(["queued", "running", "needs-review"]))
            .all(conn)
            .await?
            .into_iter()
            .map(|row| row.config),
    );
    for config in configs {
        let body: Body = serde_json::from_str(&config).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("cron.configCorrupt"),
            )
        })?;
        let environment = Id::reference(&body.environment_id)?;
        if !(environments.contains(&environment)
            || (environments.is_empty() && i64::from(environment) == 0))
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("cron.environmentReferenced"),
            ));
        }
    }
    Ok(())
}

/// Resource deletion uses this within its own transaction.
pub async fn ensure_unreferenced(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    ids: &[Id],
) -> Result<()> {
    if jobs::Entity::find()
        .filter(jobs::Column::WorkspaceId.eq(workspace))
        .filter(jobs::Column::TargetKind.eq(kind))
        .filter(jobs::Column::TargetRef.is_in(ids.iter().copied()))
        .one(conn)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.targetReferenced"),
        ));
    }
    if runs::Entity::find()
        .filter(runs::Column::WorkspaceId.eq(workspace))
        .filter(runs::Column::TargetKind.eq(kind))
        .filter(runs::Column::TargetRef.is_in(ids.iter().copied()))
        .filter(runs::Column::State.is_in(["queued", "running", "needs-review"]))
        .one(conn)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("cron.targetRunsBusy"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stored_run_output_keeps_large_logs_and_localizes_skips_and_manual_review_per_reader() {
        use crate::entities::workspaces;
        use crate::i18n::{Locale, with_locale};
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std())
                    .unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "cron-locale-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            cluster: None,
        });
        workspaces::Entity::insert(workspaces::ActiveModel {
            id: Set(1),
            slug: Set("cron-locale-test".into()),
            name: Set("Workspace".into()),
            owner_id: Set(1),
            join_code: Set("cron-locale-test".into()),
            pipeline_run_number: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let user_text = r#"User {name} {"$system_message":{"key":"error.forbidden"}}"#;
        let job = jobs::Model {
            id: 2,
            workspace_id: 1,
            name: user_text.into(),
            description: user_text.into(),
            scope: "host".into(),
            config: serde_json::to_string(&body("* * * * *", "UTC")).unwrap(),
            target_kind: "host".into(),
            target_ref: 3,
            enabled: 1,
            run_as: 1,
            run_as_name: user_text.into(),
            next_run_at: 0,
            version: 1,
            created_at: 0,
            updated_at: 0,
        };
        let id = insert_run(
            &s,
            &s.db,
            &job,
            1.into(),
            "scheduled",
            "skipped",
            Some(Message::new("cron.skipDowntime")),
            10,
        )
        .await
        .unwrap();
        let row = runs::Entity::find_by_id(i64::from(id))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        for locale in Locale::ALL {
            let view = with_locale(locale, async { run_json(row.clone(), true) }).await;
            assert_eq!(
                view["output"],
                i18n::render(locale, &Message::new("cron.skipDowntime"))
            );
            assert_eq!(view["job_name"], user_text);
        }
        let header = Message::new("cron.hostExit")
            .with("host", "3")
            .with("code", "0");
        let raw = "x".repeat(1_048_575);
        let mut output = StoredText::from_message(header.clone());
        output.push_raw(raw.clone());
        output.push_raw("界");
        finish(&s, i64::from(id), "needs-review", &output)
            .await
            .unwrap();
        let before = runs::Entity::find_by_id(i64::from(id))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            before.output.len() > 1_048_576,
            "encoded metadata is outside the raw log budget"
        );
        for locale in Locale::ALL {
            let view = with_locale(locale, async { run_json(before.clone(), true) }).await;
            assert_eq!(
                view["output"],
                format!(
                    "{}{}{}",
                    i18n::render(locale, &header),
                    raw,
                    i18n::render(locale, &Message::new("output.truncated"))
                )
            );
        }
        assert_eq!(
            runs::Entity::find_by_id(i64::from(id))
                .one(&s.db)
                .await
                .unwrap()
                .unwrap(),
            before
        );
        let scope = Scope {
            workspace_id: 1.into(),
            user: crate::accounts::auth::User {
                id: 1.into(),
                email: "test@example.invalid".into(),
                name: user_text.into(),
                is_superadmin: false,
            },
            permissions: ["cron-jobs.run".into()].into_iter().collect(),
        };
        let Json(reviewed) = with_locale(
            Locale::JaJp,
            acknowledge(
                State(s.clone()),
                scope,
                Path(("1".into(), id)),
                Json(Review {
                    version: before.version,
                    note: user_text.into(),
                }),
            ),
        )
        .await
        .unwrap();
        assert_eq!(reviewed["ok"], true);
        let after = runs::Entity::find_by_id(i64::from(id))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.state, "failed");
        assert_eq!(after.version, before.version + 1);
        for locale in Locale::ALL {
            let view = with_locale(locale, async { run_json(after.clone(), true) }).await;
            let output = view["output"].as_str().unwrap();
            assert!(output.contains(&raw));
            assert!(output.contains(user_text));
            assert!(output.contains(&i18n::render(
                locale,
                &Message::new("cron.manualNote").with("note", user_text)
            )));
        }
    }
    fn body(schedule: &str, timezone: &str) -> Body {
        Body {
            name: "test".into(),
            description: String::new(),
            scope: "host".into(),
            schedule: schedule.into(),
            timezone: timezone.into(),
            enabled: true,
            action: "command".into(),
            target_kind: "host".into(),
            target_ref: "1".into(),
            environment_id: String::new(),
            command: "true".into(),
            timeout_seconds: 10,
            max_retries: 0,
            retry_delay_seconds: 1,
            version: 0,
        }
    }
    fn millis(time: &str) -> i64 {
        DateTime::parse_from_rfc3339(time)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn timezone_and_dst_boundaries() {
        assert_eq!(
            next(
                &body("0 9 * * *", "Asia/Shanghai"),
                millis("2026-01-01T00:59:59Z")
            )
            .unwrap(),
            millis("2026-01-01T01:00:00Z")
        );
        assert_eq!(
            next(
                &body("0 30 2 * * *", "America/New_York"),
                millis("2026-03-08T06:59:59Z")
            )
            .unwrap(),
            millis("2026-03-09T06:30:00Z")
        );
        let after = millis("2026-11-01T06:00:00Z");
        assert!(next(&body("0 30 1 * * *", "America/New_York"), after).unwrap() > after);
        assert!(next(&body("* * * * *", "Invalid/Timezone"), db::now()).is_err());
        assert!(next(&body("bad", "UTC"), db::now()).is_err());
    }
}
