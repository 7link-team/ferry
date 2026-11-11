use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        alert_events as events, alert_rules as rules, cron_jobs, cron_runs, deployments, hosts,
        pipeline_runs, pipelines,
    },
    error::{ApiError, Result},
    i18n::{self, Message},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;
const RULES: Spec = Spec {
    search: &["name"],
    filters: &["target_kind", "target_ref", "metric"],
    sortable: &["name", "created_at"],
    default_sort: "-created_at",
};
const EVENTS: Spec = Spec {
    search: &["rule_name"],
    filters: &["rule_id", "state", "target_kind", "target_ref"],
    sortable: &["created_at", "triggered_at", "recovered_at"],
    default_sort: "-created_at",
};
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    description: String,
    target_kind: String,
    target_ref: String,
    metric: String,
    #[serde(default)]
    threshold: i64,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    webhook: Option<String>,
    #[serde(default)]
    clear_webhook: bool,
    #[serde(default)]
    notification_channel_id: Option<String>,
    #[serde(default)]
    version: i64,
}
fn public(r: rules::Model) -> Value {
    json!({"id":Id::from(r.id),"name":r.name,"description":r.description,"target_kind":r.target_kind,"target_ref":Id::from(r.target_ref),"metric":r.metric,"threshold":r.threshold,"enabled":r.enabled==1,"has_webhook":!r.webhook.is_empty(),"notification_channel_id":Id::from(r.notification_channel_id),"last_error":crate::i18n::display_stored(&r.last_error),"version":r.version,"created_at":r.created_at,"updated_at":r.updated_at})
}
fn event(r: events::Model) -> Value {
    json!({"id":Id::from(r.id),"rule_id":Id::from(r.rule_id),"rule_name":r.rule_name,"target_kind":r.target_kind,"target_ref":Id::from(r.target_ref),"state":r.state,"message":crate::i18n::display_stored(&r.message),"triggered_at":r.triggered_at,"recovered_at":r.recovered_at,"trigger_delivery":crate::i18n::display_stored(&r.trigger_delivery),"recovery_delivery":crate::i18n::display_stored(&r.recovery_delivery),"created_at":r.created_at,"version":r.version})
}
fn permission(kind: &str) -> Result<&'static str> {
    match kind {
        "host" => Ok("hosts.read"),
        "health-check" => Ok("alerts.read"),
        "application-health" => Ok("deployments.read"),
        "pipeline" => Ok("pipelines.read"),
        "deployment" => Ok("deployments.read"),
        "cron-job" => Ok("cron-jobs.read"),
        _ => Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("alerts.targetInvalid"),
        )),
    }
}
async fn target(conn: &impl ConnectionTrait, scope: &Scope, kind: &str, id: Id) -> Result<()> {
    scope.require(permission(kind)?)?;
    macro_rules! exists {
        ($e:ident) => {
            $e::Entity::find_by_id(i64::from(id))
                .filter($e::Column::WorkspaceId.eq(scope.workspace_id))
                .one(conn)
                .await?
                .is_some()
        };
    }
    if kind == "health-check" {
        crate::monitoring::scoped(conn, scope, id).await?;
        return Ok(());
    }
    let present = match kind {
        "host" => exists!(hosts),
        "pipeline" => exists!(pipelines),
        "deployment" | "application-health" => exists!(deployments),
        "cron-job" => exists!(cron_jobs),
        _ => false,
    };
    if present {
        Ok(())
    } else {
        Err(ApiError::missing())
    }
}
async fn validate(conn: &impl ConnectionTrait, scope: &Scope, b: &Body) -> Result<()> {
    if b.name.trim().is_empty()
        || b.name.chars().count() > 60
        || b.description.chars().count() > 2000
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("alerts.nameInvalid"),
        ));
    }
    let valid = if matches!(
        b.target_kind.as_str(),
        "health-check" | "application-health"
    ) {
        b.metric == "health-failed"
    } else if b.target_kind == "host" {
        match b.metric.as_str() {
            "offline" => true,
            "load1" => b.threshold >= 0,
            "cpu-percent" | "memory-percent" | "disk-percent" => (0..=100).contains(&b.threshold),
            _ => false,
        }
    } else {
        b.metric == "execution-failed"
    };
    if !valid {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("alerts.metricInvalid"),
        ));
    }
    if let Some(url) = b.webhook.as_ref().filter(|s| !s.is_empty()) {
        let parsed = reqwest::Url::parse(url).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("alerts.webhookInvalid"),
            )
        })?;
        if url.len() > 4096
            || !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("alerts.webhookAddressInvalid"),
            ));
        }
    }
    target(conn, scope, &b.target_kind, Id::reference(&b.target_ref)?).await
}
fn aad(workspace: Id, id: Id) -> String {
    format!("alert-webhook:{workspace}:{id}")
}
async fn notification_channel(
    _s: &AppState,
    conn: &impl ConnectionTrait,
    scope: &Scope,
    b: &Body,
    old: i64,
) -> Result<i64> {
    let Some(value) = &b.notification_channel_id else {
        return Ok(old);
    };
    let id = if value.is_empty() {
        Id::default()
    } else {
        Id::reference(value)?
    };
    if i64::from(id) == 0 || i64::from(id) == old {
        return Ok(id.into());
    }
    let channel = super::center::channel_in(conn, scope.workspace_id, id).await?;
    super::center::authorize_in(conn, scope, &channel).await?;
    Ok(id.into())
}

pub(crate) async fn prepare_channels(s: &AppState) -> Result<()> {
    for rule in rules::Entity::find()
        .filter(rules::Column::NotificationChannelId.eq(0))
        .filter(rules::Column::Webhook.ne(""))
        .all(&s.db)
        .await?
    {
        let tx = crate::variables::transaction(s, rule.workspace_id.into()).await?;
        let Some(rule) = rules::Entity::find_by_id(rule.id).one(&tx).await? else {
            continue;
        };
        if rule.notification_channel_id != 0 || rule.webhook.is_empty() {
            continue;
        }
        let url = s.vault.open_secret(
            &rule.webhook,
            &aad(rule.workspace_id.into(), rule.id.into()),
        )?;
        let parsed = reqwest::Url::parse(&url).map_err(|_| ApiError::missing())?;
        let provider = super::channel::provider(&parsed).to_owned();
        let config = super::center::Transport {
            webhook: url,
            options: super::channel::Options {
                provider: provider.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let id = super::center::import_channel(
            s,
            &tx,
            rule.workspace_id.into(),
            rule.name.clone(),
            provider,
            true,
            config,
        )
        .await?;
        rules::Entity::update_many()
            .col_expr(rules::Column::NotificationChannelId, Expr::value(id))
            .filter(rules::Column::Id.eq(rule.id))
            .exec(&tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}
fn webhook(s: &AppState, scope: &Scope, id: Id, b: &Body, old: &str) -> Result<String> {
    if b.clear_webhook {
        return Ok(String::new());
    }
    match b.webhook.as_ref().filter(|v| !v.is_empty()) {
        Some(url) => s.vault.seal(url, &aad(scope.workspace_id, id)),
        None => Ok(old.into()),
    }
}
async fn scoped(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<rules::Model> {
    rules::Entity::find_by_id(i64::from(id))
        .filter(rules::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    let q = ListQuery::parse(&RULES, &raw)?;
    let (rows, total) = resource::list::<rules::Entity, rules::Model>(
        &s.db,
        &RULES,
        rules::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(public).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    Ok(Json(public(scoped(&s.db, &scope, id).await?)))
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
    scope.require("alerts.create")?;
    let id = s.next_id()?;
    validate(tx, &scope, &b).await?;
    let channel_id = notification_channel(s, tx, scope, &b, 0).await?;
    let now = db::now();
    rules::Entity::insert(rules::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(b.name.trim().into()),
        description: Set(b.description.clone()),
        target_kind: Set(b.target_kind.clone()),
        target_ref: Set(Id::reference(&b.target_ref)?.into()),
        metric: Set(b.metric.clone()),
        threshold: Set(b.threshold),
        enabled: Set(i64::from(b.enabled)),
        run_as: Set(scope.user.id.into()),
        webhook: Set(webhook(&s, &scope, id, &b, "")?),
        notification_channel_id: Set(channel_id),
        last_error: Set(String::new()),
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
        "alert-rules",
        &id,
        json!({"name":b.name}),
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
    scope.require("alerts.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = scoped(&tx, &scope, id).await?;
    validate(&tx, &scope, &b).await?;
    if old.target_kind != b.target_kind
        || old.target_ref != i64::from(Id::reference(&b.target_ref)?)
        || old.metric != b.metric
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("alerts.targetImmutable"),
        ));
    }
    let channel_id = notification_channel(&s, &tx, &scope, &b, old.notification_channel_id).await?;
    let n = rules::Entity::update_many()
        .col_expr(rules::Column::Name, Expr::value(b.name.trim()))
        .col_expr(
            rules::Column::Description,
            Expr::value(b.description.clone()),
        )
        .col_expr(rules::Column::Threshold, Expr::value(b.threshold))
        .col_expr(rules::Column::Enabled, Expr::value(i64::from(b.enabled)))
        .col_expr(rules::Column::RunAs, Expr::value(scope.user.id))
        .col_expr(
            rules::Column::NotificationChannelId,
            Expr::value(channel_id),
        )
        .col_expr(
            rules::Column::Webhook,
            Expr::value(webhook(&s, &scope, id, &b, &old.webhook)?),
        )
        .col_expr(rules::Column::Version, Expr::value(b.version + 1))
        .col_expr(rules::Column::UpdatedAt, Expr::value(db::now()))
        .filter(rules::Column::Id.eq(id))
        .filter(rules::Column::Version.eq(b.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("alerts.ruleChanged"),
        ));
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "alert-rules",
        &id,
        json!({"name":b.name,"enabled":b.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":b.version+1})))
}
pub async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("alerts.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    scoped(&tx, &scope, id).await?;
    rules::Entity::delete_by_id(i64::from(id)).exec(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "alert-rules",
        &id,
        json!({"events_preserved":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    let q = ListQuery::parse(&EVENTS, &raw)?;
    let (rows, total) = resource::list::<events::Entity, events::Model>(
        &s.db,
        &EVENTS,
        events::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(event).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    let row = events::Entity::find_by_id(i64::from(id))
        .filter(events::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(event(row)))
}
async fn abnormal(s: &AppState, r: &rules::Model) -> Result<Option<(bool, Message)>> {
    let scope = Scope::for_actor(s, r.workspace_id.into(), r.run_as.into()).await?;
    scope.require("alerts.read")?;
    target(&s.db, &scope, &r.target_kind, r.target_ref.into()).await?;
    if r.target_kind == "application-health" {
        let Json(value) = crate::monitoring::application(
            State(s.clone()),
            scope,
            Path((r.workspace_id.to_string(), r.target_ref.into())),
        )
        .await?;
        return Ok(match value["health"].as_str() {
            Some("up") => Some((false, Message::new("monitoring.recovered"))),
            Some("down" | "degraded") => Some((true, Message::new("monitoring.failed"))),
            _ => None,
        });
    }
    if r.target_kind == "health-check" {
        let row = crate::monitoring::scoped(&s.db, &scope, r.target_ref.into()).await?;
        return Ok(match crate::monitoring::state(&row, db::now()).as_str() {
            "up" => Some((false, Message::new("monitoring.recovered"))),
            "down" => Some((true, Message::new("monitoring.failed"))),
            _ => None,
        });
    }
    if r.target_kind == "host" {
        let host = hosts::Entity::find_by_id(r.target_ref)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if r.metric == "offline" {
            return Ok(Some((
                host.state != "online",
                Message::new("alerts.hostState").with("state", host.state),
            )));
        }
        if host.state != "online" {
            return Ok(None);
        }
        let metrics: Value = serde_json::from_str(&host.metrics).unwrap_or(Value::Null);
        let value = match r.metric.as_str() {
            "load1" => metrics["load1"].as_f64(),
            "cpu-percent" => metrics["cpu_percent"].as_f64(),
            "memory-percent" | "disk-percent" => {
                let prefix = if r.metric == "memory-percent" {
                    "mem"
                } else {
                    "disk"
                };
                metrics[format!("{prefix}_used")]
                    .as_f64()
                    .zip(metrics[format!("{prefix}_total")].as_f64())
                    .filter(|(_, total)| *total > 0.)
                    .map(|(used, total)| used / total * 100.)
            }
            _ => None,
        };
        return Ok(value.filter(|v| v.is_finite()).map(|v| {
            (
                v > r.threshold as f64,
                Message::new("alerts.metricValue")
                    .with("metric", r.metric.clone())
                    .with("value", format!("{v:.2}"))
                    .with("threshold", r.threshold.to_string()),
            )
        }));
    }
    let state = match r.target_kind.as_str() {
        "pipeline" => pipeline_runs::Entity::find()
            .filter(pipeline_runs::Column::WorkspaceId.eq(r.workspace_id))
            .filter(pipeline_runs::Column::PipelineId.eq(r.target_ref))
            .order_by_desc(pipeline_runs::Column::CreatedAt)
            .order_by_desc(pipeline_runs::Column::Id)
            .one(&s.db)
            .await?
            .map(|r| r.state),
        "deployment" => deployments::Entity::find_by_id(r.target_ref)
            .one(&s.db)
            .await?
            .map(|r| r.state),
        "cron-job" => cron_runs::Entity::find()
            .filter(cron_runs::Column::WorkspaceId.eq(r.workspace_id))
            .filter(cron_runs::Column::JobId.eq(r.target_ref))
            .filter(cron_runs::Column::State.ne("skipped"))
            .order_by_desc(cron_runs::Column::CreatedAt)
            .order_by_desc(cron_runs::Column::Id)
            .one(&s.db)
            .await?
            .map(|r| r.state),
        _ => None,
    };
    Ok(state.and_then(|v| match v.as_str() {
        "failed" | "needs-review" => {
            Some((true, Message::new("alerts.executionState").with("state", v)))
        }
        "succeeded" => Some((false, Message::new("alerts.executionSucceeded"))),
        _ => None,
    }))
}
async fn deliver(
    s: &AppState,
    client: &reqwest::Client,
    r: &rules::Model,
    id: Id,
    recovered: bool,
) -> Result<()> {
    let column = if recovered {
        events::Column::RecoveryDelivery
    } else {
        events::Column::TriggerDelivery
    };
    let row = events::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let title = i18n::render(
        i18n::current_locale(),
        &Message::new(if recovered {
            "alerts.noticeRecovered"
        } else {
            "alerts.noticeTriggered"
        })
        .with("name", row.rule_name.clone()),
    );
    let (state, kind, message) = (
        row.state.clone(),
        row.target_kind.clone(),
        i18n::display_stored(&row.message),
    );
    let notice = crate::notifications::webhook::Notice {
        title: &title,
        accent: None,
        field_links: vec![],
        field_accents: vec![],
        fields: vec![
            (i18n::message("alerts.stateLabel"), state),
            (i18n::message("alerts.kindLabel"), kind),
            (i18n::message("alerts.messageLabel"), message),
        ],
        generic: json!({"event":if recovered{"recovered"}else{"triggered"},"data":event(row)}),
    };
    let result = if r.notification_channel_id != 0 {
        match crate::notifications::center::send_system_notice(
            s,
            r.workspace_id.into(),
            r.notification_channel_id.into(),
            format!(
                "alert:{id}:{}",
                if recovered { "recovered" } else { "triggered" }
            ),
            "alert",
            &id.to_string(),
            &notice,
        )
        .await
        {
            Ok(result) => result,
            Err(error) => error.2.as_ref().map(i18n::store_message).unwrap_or(error.1),
        }
    } else {
        match s
            .vault
            .open_secret(&r.webhook, &aad(r.workspace_id.into(), r.id.into()))
        {
            Ok(url) => crate::notifications::webhook::deliver(client, &url, &notice).await,
            Err(_) => i18n::store_message(&Message::new("alerts.webhookDecryptFailed")),
        }
    };
    events::Entity::update_many()
        .col_expr(column, Expr::value(result))
        .filter(events::Column::Id.eq(id))
        .exec(&s.db)
        .await?;
    Ok(())
}
async fn evaluate(s: &AppState, client: &reqwest::Client, r: rules::Model) -> Result<()> {
    let observed = abnormal(s, &r).await;
    let error = if observed.is_err() {
        i18n::store_message(&Message::new("alerts.targetUnavailable"))
    } else {
        String::new()
    };
    rules::Entity::update_many()
        .col_expr(rules::Column::LastError, Expr::value(error))
        .filter(rules::Column::Id.eq(r.id))
        .exec(&s.db)
        .await?;
    let Ok(Some((firing, message))) = observed else {
        return Ok(());
    };
    let tx = crate::variables::transaction(s, r.workspace_id.into()).await?;
    let Some(current) = rules::Entity::find_by_id(r.id)
        .filter(rules::Column::Version.eq(r.version))
        .filter(rules::Column::Enabled.eq(1))
        .one(&tx)
        .await?
    else {
        return Ok(());
    };
    let active = events::Entity::find()
        .filter(events::Column::RuleId.eq(r.id))
        .filter(events::Column::WorkspaceId.eq(r.workspace_id))
        .filter(events::Column::State.eq("firing"))
        .one(&tx)
        .await?;
    let now = db::now();
    let delivery = if current.webhook.is_empty() && current.notification_channel_id == 0 {
        i18n::store_message(&Message::new("alerts.webhookMissing"))
    } else {
        i18n::store_message(&Message::new("alerts.delivering"))
    };
    let transition = match (firing, active) {
        (true, None) => {
            let id = s.next_id()?;
            events::Entity::insert(events::ActiveModel {
                id: Set(id.into()),
                workspace_id: Set(r.workspace_id),
                rule_id: Set(r.id),
                rule_name: Set(r.name.clone()),
                target_kind: Set(r.target_kind.clone()),
                target_ref: Set(r.target_ref),
                state: Set("firing".into()),
                message: Set(i18n::store_message(&message)),
                triggered_at: Set(now),
                recovered_at: Set(0),
                trigger_delivery: Set(delivery.into()),
                recovery_delivery: Set(String::new()),
                version: Set(1),
                created_at: Set(now),
                updated_at: Set(now),
            })
            .exec(&tx)
            .await?;
            Some((id, false))
        }
        (false, Some(event)) => {
            events::Entity::update_many()
                .col_expr(events::Column::State, Expr::value("recovered"))
                .col_expr(events::Column::RecoveredAt, Expr::value(now))
                .col_expr(events::Column::RecoveryDelivery, Expr::value(delivery))
                .col_expr(events::Column::UpdatedAt, Expr::value(now))
                .col_expr(
                    events::Column::Version,
                    Expr::col(events::Column::Version).add(1),
                )
                .filter(events::Column::Id.eq(event.id))
                .exec(&tx)
                .await?;
            Some((Id::from(event.id), true))
        }
        _ => None,
    };
    if let Some((id, recovered)) = transition {
        crate::hosts::activity_record(
            s,
            &tx,
            r.workspace_id.into(),
            (Id::default(), "系统"),
            if recovered {
                "alert-recovered"
            } else {
                "alert-triggered"
            },
            "alert-events",
            id,
            json!({"rule_id":Id::from(r.id)}),
        )
        .await?;
    }
    tx.commit().await?;
    if let Some((id, recovered)) = transition {
        if !current.webhook.is_empty() || current.notification_channel_id != 0 {
            deliver(s, client, &current, id, recovered).await?;
        }
    }
    Ok(())
}
async fn prune(s: &AppState) -> Result<()> {
    use crate::entities::settings;
    for setting in settings::Entity::find()
        .filter(settings::Column::Key.eq("history_retention_days"))
        .filter(settings::Column::Value.gt(0))
        .all(&s.db)
        .await?
    {
        let workspace = Id::from(setting.workspace_id);
        let tx = crate::variables::transaction(s, workspace).await?;
        let days = crate::maintenance::setting(&tx, workspace, "history_retention_days").await?;
        if days == 0 {
            continue;
        }
        let rows = events::Entity::find()
            .filter(events::Column::WorkspaceId.eq(workspace))
            .filter(events::Column::State.eq("recovered"))
            .filter(events::Column::RecoveredAt.lt(db::now() - days * 86_400_000))
            .limit(200)
            .all(&tx)
            .await?;
        if !rows.is_empty() {
            let count = events::Entity::delete_many()
                .filter(events::Column::WorkspaceId.eq(workspace))
                .filter(events::Column::Id.is_in(rows.iter().map(|r| r.id)))
                .exec(&tx)
                .await?
                .rows_affected;
            crate::hosts::activity_record(
                s,
                &tx,
                workspace,
                (Id::default(), "系统"),
                "retention-cleanup",
                "alert-events",
                Id::default(),
                json!({"removed":count,"retention_days":days}),
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}
async fn recover_deliveries(conn: &impl ConnectionTrait) -> Result<()> {
    for column in [
        events::Column::TriggerDelivery,
        events::Column::RecoveryDelivery,
    ] {
        events::Entity::update_many()
            .col_expr(
                column,
                Expr::value(i18n::store_message(&Message::new(
                    "alerts.deliveryInterrupted",
                ))),
            )
            .filter(
                sea_orm::Condition::any()
                    .add(column.eq("投递中"))
                    .add(column.eq(i18n::store_message(&Message::new("alerts.delivering")))),
            )
            .exec(conn)
            .await?;
    }
    Ok(())
}

pub async fn scheduler(s: AppState, mut drain: watch::Receiver<bool>) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(%e,"告警HTTP客户端初始化失败");
            return;
        }
    };
    if let Err(error) = recover_deliveries(&s.db).await {
        tracing::error!(%error,"告警投递恢复失败");
        return;
    }
    let mut cleanup = std::time::Instant::now();
    loop {
        if std::time::Instant::now() >= cleanup {
            if let Err(error) = prune(&s).await {
                tracing::error!(%error,"告警历史清理失败");
            }
            cleanup = std::time::Instant::now() + Duration::from_secs(60);
        }
        if *drain.borrow() {
            break;
        }
        match rules::Entity::find()
            .filter(rules::Column::Enabled.eq(1))
            .all(&s.db)
            .await
        {
            Ok(rows) => {
                for row in rows {
                    if *drain.borrow() {
                        break;
                    }
                    if let Err(error) = evaluate(&s, &client, row).await {
                        tracing::error!(%error,"告警检查失败");
                    }
                }
            }
            Err(error) => tracing::error!(%error,"告警规则读取失败"),
        }
        tokio::select! {_=drain.changed()=>{},_=tokio::time::sleep(Duration::from_secs(2))=>{}}
    }
}
pub async fn ensure_unreferenced(
    conn: &impl ConnectionTrait,
    workspace: Id,
    kind: &str,
    ids: &[Id],
) -> Result<()> {
    if rules::Entity::find()
        .filter(rules::Column::WorkspaceId.eq(workspace))
        .filter(rules::Column::TargetKind.eq(kind))
        .filter(rules::Column::TargetRef.is_in(ids.iter().copied()))
        .one(conn)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("alerts.targetReferenced"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod locale_tests {
    use super::*;
    use crate::i18n::{Locale, render_stored, with_locale};
    use sea_orm::{Database, IntoActiveModel};
    use sea_orm_migration::MigratorTrait;

    #[tokio::test]
    async fn persisted_event_projection_and_restart_keep_identity_and_legacy_values() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migration::Migrator::up(&db, None).await.unwrap();
        let name = r#"User {name} {"$system_message":{"key":"error.forbidden"}}"#;
        let message = Message::new("alerts.metricValue")
            .with("metric", "load1")
            .with("value", "4.20")
            .with("threshold", "3");
        let pending = i18n::store_message(&Message::new("alerts.delivering"));
        for (id, delivery) in [
            (1, pending.clone()),
            (2, "投递中".into()),
            (3, "HTTP 204".into()),
        ] {
            events::Entity::insert(
                events::Model {
                    id,
                    workspace_id: 7,
                    rule_id: 8,
                    rule_name: name.into(),
                    target_kind: "host".into(),
                    target_ref: 9,
                    state: "firing".into(),
                    message: i18n::store_message(&message),
                    triggered_at: 1,
                    recovered_at: 0,
                    trigger_delivery: delivery.clone(),
                    recovery_delivery: delivery,
                    version: 1,
                    created_at: 1,
                    updated_at: 1,
                }
                .into_active_model(),
            )
            .exec(&db)
            .await
            .unwrap();
        }
        recover_deliveries(&db).await.unwrap();
        // A second startup neither changes final statuses nor initiates a retry.
        recover_deliveries(&db).await.unwrap();
        for id in [1, 2, 3] {
            let row = events::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.rule_name, name);
            if id == 3 {
                assert_eq!(row.trigger_delivery, "HTTP 204");
                assert_eq!(row.recovery_delivery, "HTTP 204");
            } else {
                assert_eq!(
                    row.trigger_delivery,
                    i18n::store_message(&Message::new("alerts.deliveryInterrupted"))
                );
                assert_eq!(row.recovery_delivery, row.trigger_delivery);
            }
            for locale in Locale::ALL {
                let view = with_locale(locale, async { event(row.clone()) }).await;
                assert_eq!(view["rule_name"], name);
                assert_eq!(view["message"], i18n::render(locale, &message));
                assert_eq!(
                    view["trigger_delivery"],
                    render_stored(locale, &row.trigger_delivery)
                );
                assert_eq!(view["state"], "firing");
            }
        }
        let rule = rules::Model {
            id: 8,
            workspace_id: 7,
            name: name.into(),
            description: name.into(),
            target_kind: "host".into(),
            target_ref: 9,
            metric: "load1".into(),
            threshold: 3,
            enabled: 1,
            run_as: 1,
            webhook: String::new(),
            notification_channel_id: 0,
            last_error: i18n::store_message(&Message::new("alerts.targetUnavailable")),
            version: 1,
            created_at: 1,
            updated_at: 1,
        };
        for locale in Locale::ALL {
            let view = with_locale(locale, async { public(rule.clone()) }).await;
            assert_eq!(view["last_error"], render_stored(locale, &rule.last_error));
            assert_eq!(view["name"], name);
            assert_eq!(view["description"], name);
        }
    }
}
