use crate::{
    db,
    entities::{
        alert_events as events, alert_rules as rules, cron_jobs, cron_runs, deployments, hosts,
        pipeline_runs, pipelines,
    },
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
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
    default_sort: "name",
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
    version: i64,
}
fn public(r: rules::Model) -> Value {
    json!({"id":Id::from(r.id),"name":r.name,"description":r.description,"target_kind":r.target_kind,"target_ref":Id::from(r.target_ref),"metric":r.metric,"threshold":r.threshold,"enabled":r.enabled==1,"has_webhook":!r.webhook.is_empty(),"last_error":r.last_error,"version":r.version,"created_at":r.created_at,"updated_at":r.updated_at})
}
fn event(r: events::Model) -> Value {
    json!({"id":Id::from(r.id),"rule_id":Id::from(r.rule_id),"rule_name":r.rule_name,"target_kind":r.target_kind,"target_ref":Id::from(r.target_ref),"state":r.state,"message":r.message,"triggered_at":r.triggered_at,"recovered_at":r.recovered_at,"trigger_delivery":r.trigger_delivery,"recovery_delivery":r.recovery_delivery,"created_at":r.created_at,"version":r.version})
}
fn permission(kind: &str) -> Result<&'static str> {
    match kind {
        "host" => Ok("hosts.read"),
        "pipeline" => Ok("pipelines.read"),
        "deployment" => Ok("deployments.read"),
        "cron-job" => Ok("cron-jobs.read"),
        _ => Err(ApiError::bad("监控目标类型无效。")),
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
    let present = match kind {
        "host" => exists!(hosts),
        "pipeline" => exists!(pipelines),
        "deployment" => exists!(deployments),
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
        return Err(ApiError::bad("规则名称需1–60字，描述不超过2000字。"));
    }
    let valid = if b.target_kind == "host" {
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
        return Err(ApiError::bad(
            "指标或阈值无效；主机支持离线、负载、内存/磁盘百分比，执行目标支持执行失败。",
        ));
    }
    if let Some(url) = b.webhook.as_ref().filter(|s| !s.is_empty()) {
        let parsed = reqwest::Url::parse(url).map_err(|_| ApiError::bad("Webhook URL无效。"))?;
        if url.len() > 4096
            || !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ApiError::bad(
                "Webhook需为HTTP(S)地址，不支持用户信息或片段。",
            ));
        }
    }
    target(conn, scope, &b.target_kind, Id::reference(&b.target_ref)?).await
}
fn aad(workspace: Id, id: Id) -> String {
    format!("alert-webhook:{workspace}:{id}")
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
        return Err(ApiError::bad("监控目标和指标不可更改；请创建新规则。"));
    }
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
        return Err(ApiError::conflict("规则已变化，请刷新后重试。"));
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
async fn abnormal(s: &AppState, r: &rules::Model) -> Result<Option<(bool, String)>> {
    let scope = Scope::for_actor(s, r.workspace_id.into(), r.run_as.into()).await?;
    scope.require("alerts.read")?;
    target(&s.db, &scope, &r.target_kind, r.target_ref.into()).await?;
    if r.target_kind == "host" {
        let host = hosts::Entity::find_by_id(r.target_ref)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if r.metric == "offline" {
            return Ok(Some((
                host.state != "online",
                format!("主机状态：{}", host.state),
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
                format!("{} = {:.2}；阈值 > {}", r.metric, v, r.threshold),
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
        "failed" | "needs-review" => Some((true, format!("最近执行状态：{v}"))),
        "succeeded" => Some((false, "最近执行成功。".into())),
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
    let result = match s
        .vault
        .open_secret(&r.webhook, &aad(r.workspace_id.into(), r.id.into()))
    {
        Ok(url) => match client
            .post(url)
            .json(&json!({"event":if recovered{"recovered"}else{"triggered"},"data":event(row)}))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    format!("已投递 HTTP {}", response.status().as_u16())
                } else {
                    format!("投递失败 HTTP {}；不自动重试", response.status().as_u16())
                }
            }
            Err(_) => "投递失败或超时；不自动重试".into(),
        },
        Err(_) => "Webhook无法解密；未投递".into(),
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
        "当前执行身份权限或监控目标不可用。"
    } else {
        ""
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
    let delivery = if current.webhook.is_empty() {
        "未配置Webhook"
    } else {
        "投递中"
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
                message: Set(message),
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
        if !current.webhook.is_empty() {
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
    for column in [
        events::Column::TriggerDelivery,
        events::Column::RecoveryDelivery,
    ] {
        if let Err(error) = events::Entity::update_many()
            .col_expr(column, Expr::value("服务中断，投递结果不确定；不自动重试"))
            .filter(column.eq("投递中"))
            .exec(&s.db)
            .await
        {
            tracing::error!(%error,"告警投递恢复失败");
            return;
        }
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
        return Err(ApiError::conflict("目标仍被告警规则引用，请先删除规则。"));
    }
    Ok(())
}
