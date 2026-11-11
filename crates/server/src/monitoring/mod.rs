//! Continuous observations are separate from deployment execution outcomes.
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{deployments, health_checks as checks, health_observations as observations},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{Router, extract::State, http::StatusCode, routing::get};
use futures::StreamExt;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
mod runtime;
#[cfg(test)]
mod tests;

const SPEC: Spec = Spec {
    search: &["name"],
    filters: &["kind", "role", "deployment_id", "host_id"],
    sortable: &["name", "created_at"],
    default_sort: "-created_at",
};
const HISTORY: Spec = Spec {
    search: &[],
    filters: &["check_id"],
    sortable: &["observed_at"],
    default_sort: "-observed_at",
};
const APPS: Spec = Spec {
    search: &["name"],
    filters: &["id"],
    sortable: &["name", "created_at"],
    default_sort: "-created_at",
};

fn bad(key: &str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new(key))
}
fn changed() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("monitoring.changed"))
}
fn optional_id(value: &str) -> Result<i64> {
    if value.is_empty() || value == "0" {
        Ok(0)
    } else {
        Ok(Id::reference(value)?.into())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub address: String,
    pub expected_status: u16,
    pub body_contains: String,
    pub interval_seconds: u32,
    pub timeout_seconds: u32,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub stale_seconds: u32,
    pub retention_days: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            address: String::new(),
            expected_status: 200,
            body_contains: String::new(),
            interval_seconds: 30,
            timeout_seconds: 5,
            failure_threshold: 3,
            recovery_threshold: 2,
            stale_seconds: 90,
            retention_days: 7,
        }
    }
}
impl Settings {
    fn validate(&self, kind: &str) -> Result<()> {
        if !(5..=86400).contains(&self.interval_seconds)
            || !(1..=300).contains(&self.timeout_seconds)
            || self.timeout_seconds >= self.interval_seconds
            || !(1..=100).contains(&self.failure_threshold)
            || !(1..=100).contains(&self.recovery_threshold)
            || self.stale_seconds < self.interval_seconds + self.timeout_seconds
            || self.stale_seconds > 604800
            || !(1..=365).contains(&self.retention_days)
        {
            return Err(bad("monitoring.parametersInvalid"));
        }
        if self.address.len() > 4096
            || self.body_contains.chars().count() > 1024
            || !(100..=599).contains(&self.expected_status)
        {
            return Err(bad("monitoring.addressInvalid"));
        }
        match kind {
            "http" => {
                let url = reqwest::Url::parse(&self.address)
                    .map_err(|_| bad("monitoring.addressInvalid"))?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    return Err(bad("monitoring.addressInvalid"));
                }
            }
            "tcp" => {
                let url = reqwest::Url::parse(&format!("tcp://{}", self.address))
                    .map_err(|_| bad("monitoring.addressInvalid"))?;
                if url.host_str().is_none()
                    || url.port().is_none_or(|port| port == 0)
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || !matches!(url.path(), "" | "/")
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(bad("monitoring.addressInvalid"));
                }
            }
            "runtime" if self.address.is_empty() && self.body_contains.is_empty() => {}
            _ => return Err(bad("monitoring.kindInvalid")),
        }
        Ok(())
    }
}
#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    description: String,
    kind: String,
    role: String,
    #[serde(default)]
    deployment_id: String,
    #[serde(default)]
    host_id: String,
    #[serde(default)]
    service_name: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    version: i64,
    #[serde(flatten)]
    settings: Settings,
}

/// A raw result never contains response bodies, URLs, credentials or command output.
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Observation {
    pub outcome: String,
    pub reason: String,
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub instances: Vec<runtime::Instance>,
    pub services: Vec<runtime::Service>,
    pub revision_id: String,
    pub runtime_scope: String,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CheckResult {
    pub confirmed: String,
    pub successes: u32,
    pub failures: u32,
    pub observation: Observation,
}
fn transition(
    previous: &CheckResult,
    observation: Observation,
    settings: &Settings,
) -> CheckResult {
    let mut next = previous.clone();
    let mut observation = observation;
    if observation.outcome == "unknown" && observation.services.is_empty() {
        observation.services = previous.observation.services.clone();
        observation.instances = previous.observation.instances.clone();
        observation.revision_id = previous.observation.revision_id.clone();
        observation.runtime_scope = previous.observation.runtime_scope.clone();
    }
    next.observation = observation;
    match next.observation.outcome.as_str() {
        "up" => {
            next.failures = 0;
            next.successes = next
                .successes
                .saturating_add(1)
                .min(settings.recovery_threshold);
            if next.confirmed != "down" || next.successes >= settings.recovery_threshold {
                next.confirmed = "up".into();
            }
        }
        "down" => {
            next.successes = 0;
            next.failures = next
                .failures
                .saturating_add(1)
                .min(settings.failure_threshold);
            if next.failures >= settings.failure_threshold {
                next.confirmed = "down".into();
            }
        }
        _ => {
            next.successes = 0;
            next.failures = 0;
        }
    }
    next
}
pub(crate) fn state(row: &checks::Model, now: i64) -> String {
    if row.enabled == 0 {
        return "paused".into();
    }
    if row.checked_at == 0 {
        return "unknown".into();
    }
    let Ok(settings) = serde_json::from_str::<Settings>(&row.settings) else {
        return "unknown".into();
    };
    if now - row.checked_at > i64::from(settings.stale_seconds) * 1000 {
        return "stale".into();
    }
    let result: CheckResult = serde_json::from_str(&row.result).unwrap_or_default();
    if result.observation.outcome == "unknown" {
        "unknown".into()
    } else if result.confirmed.is_empty() {
        "pending".into()
    } else {
        result.confirmed
    }
}
fn public(row: checks::Model) -> Value {
    let status = state(&row, db::now());
    let settings: Settings = serde_json::from_str(&row.settings).unwrap_or_default();
    let mut value = serde_json::to_value(settings).unwrap();
    let result: CheckResult = serde_json::from_str(&row.result).unwrap_or_default();
    let mut instances = result.observation.instances.clone();
    if !matches!(status.as_str(), "up" | "down") {
        for instance in &mut instances {
            instance.state = "unknown".into();
        }
    }
    value.as_object_mut().unwrap().extend(json!({"id":Id::from(row.id),"name":row.name,"description":row.description,"kind":row.kind,"role":row.role,"deployment_id":if row.deployment_id==0 {String::new()} else {Id::from(row.deployment_id).to_string()},"host_id":if row.host_id==0 {String::new()} else {Id::from(row.host_id).to_string()},"service_name":row.service_name,"enabled":row.enabled==1,"version":row.version,"checked_at":row.checked_at,"state":status,"instances":instances,"result":result}).as_object().unwrap().clone());
    value
}

fn readable(scope: &Scope, row: &checks::Model) -> Result<()> {
    if row.deployment_id != 0 {
        scope.require("deployments.read")?;
    }
    if row.host_id != 0 {
        scope.require("hosts.read")?;
    }
    Ok(())
}
pub(crate) async fn scoped(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
) -> Result<checks::Model> {
    let row = checks::Entity::find_by_id(i64::from(id))
        .filter(checks::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    readable(scope, &row)?;
    Ok(row)
}
async fn validate(conn: &impl ConnectionTrait, scope: &Scope, body: &Body) -> Result<(i64, i64)> {
    if body.name.trim().is_empty()
        || body.name.chars().count() > 120
        || body.description.chars().count() > 2000
        || body.service_name.len() > 120
    {
        return Err(bad("monitoring.nameInvalid"));
    }
    body.settings.validate(&body.kind)?;
    if !matches!(body.role.as_str(), "entry" | "service") {
        return Err(bad("monitoring.bindingInvalid"));
    }
    let app_id = optional_id(&body.deployment_id)?;
    let host_id = optional_id(&body.host_id)?;
    if app_id != 0 {
        scope.require("deployments.read")?;
        let app = deployments::Entity::find_by_id(app_id)
            .filter(deployments::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        if !body.service_name.is_empty() {
            let manifest = crate::deployments::manifest::parse_for(&app.manifest, &app.mode)?;
            if !manifest
                .services
                .iter()
                .any(|service| service.name == body.service_name)
            {
                return Err(bad("monitoring.bindingInvalid"));
            }
        }
    } else if !body.service_name.is_empty() {
        return Err(bad("monitoring.bindingInvalid"));
    }
    if body.kind == "runtime" {
        scope.require("hosts.read")?;
        scope.require("hosts.terminal")?;
        if app_id == 0 || host_id == 0 || body.role != "service" || !body.service_name.is_empty() {
            return Err(bad("monitoring.bindingInvalid"));
        }
        crate::entities::hosts::Entity::find_by_id(host_id)
            .filter(crate::entities::hosts::Column::WorkspaceId.eq(scope.workspace_id))
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        runtime::validate_host(conn, scope.workspace_id, app_id, host_id).await?;
    } else if host_id != 0 || (body.role == "service" && body.service_name.is_empty()) {
        return Err(bad("monitoring.bindingInvalid"));
    }
    Ok((app_id, host_id))
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/health-checks", get(list).post(create))
        .route(
            "/{workspace}/health-checks/{id}",
            get(detail).patch(update).delete(delete),
        )
        .route("/{workspace}/health-observations", get(history))
        .route("/{workspace}/application-health", get(applications))
        .route("/{workspace}/application-health/{id}", get(application))
}
async fn display_rows(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    rows: Vec<checks::Model>,
) -> Result<Vec<Value>> {
    let mut values: Vec<_> = rows.into_iter().map(public).collect();
    use crate::resources::names::{self, Kind};
    let mut refs = vec![];
    if scope.permissions.contains("deployments.read") {
        refs.push(names::of(
            "deployment_id",
            "deployment_name",
            Kind::Deployment,
        ));
    }
    if scope.permissions.contains("hosts.read") {
        refs.push(names::of("host_id", "host_name", Kind::Host));
    }
    names::attach(conn, scope.workspace_id, &mut values, &refs).await?;
    Ok(values)
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let mut select = checks::Entity::find();
    if !scope.permissions.contains("deployments.read") {
        select = select.filter(checks::Column::DeploymentId.eq(0));
    }
    if !scope.permissions.contains("hosts.read") {
        select = select.filter(checks::Column::HostId.eq(0));
    }
    let (rows, total) = resource::list::<checks::Entity, checks::Model>(
        &s.db,
        &SPEC,
        select,
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":display_rows(&s.db,&scope,rows).await?,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
pub async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    let row = scoped(&s.db, &scope, id).await?;
    Ok(Json(
        display_rows(&s.db, &scope, vec![row]).await?.remove(0),
    ))
}
pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(mut body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("alerts.create")?;
    if body.kind == "runtime" {
        body.role = "service".into();
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let (app_id, host_id) = validate(&tx, &scope, &body).await?;
    let id = s.next_id()?;
    let now = db::now();
    checks::Entity::insert(checks::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(body.name.trim().into()),
        description: Set(body.description),
        kind: Set(body.kind),
        role: Set(body.role),
        deployment_id: Set(app_id),
        host_id: Set(host_id),
        service_name: Set(body.service_name),
        settings: Set(serde_json::to_string(&body.settings).unwrap()),
        enabled: Set(i64::from(body.enabled)),
        run_as: Set(scope.user.id.into()),
        result: Set("{}".into()),
        version: Set(1),
        created_at: Set(now),
        created_by: Set(scope.user.id.into()),
        updated_at: Set(now),
        updated_by: Set(scope.user.id.into()),
        checked_at: Set(0),
        checked_by: Set(0),
        due_at: Set(0),
        due_by: Set(0),
    })
    .exec(&tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "health-checks",
        &id,
        json!({"name":body.name}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(mut body): Json<Body>,
) -> Result<Json<Value>> {
    if body.kind == "runtime" {
        body.role = "service".into();
    }
    scope.require("alerts.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = scoped(&tx, &scope, id).await?;
    let (app_id, host_id) = validate(&tx, &scope, &body).await?;
    let old_result: CheckResult = serde_json::from_str(&old.result).unwrap_or_default();
    let reset = CheckResult {
        confirmed: old_result.confirmed,
        ..Default::default()
    };
    let affected = checks::Entity::update_many()
        .col_expr(checks::Column::Name, Expr::value(body.name.trim()))
        .col_expr(checks::Column::Description, Expr::value(body.description))
        .col_expr(checks::Column::Kind, Expr::value(body.kind))
        .col_expr(checks::Column::Role, Expr::value(body.role))
        .col_expr(checks::Column::DeploymentId, Expr::value(app_id))
        .col_expr(checks::Column::HostId, Expr::value(host_id))
        .col_expr(checks::Column::ServiceName, Expr::value(body.service_name))
        .col_expr(
            checks::Column::Settings,
            Expr::value(serde_json::to_string(&body.settings).unwrap()),
        )
        .col_expr(
            checks::Column::Enabled,
            Expr::value(i64::from(body.enabled)),
        )
        .col_expr(checks::Column::RunAs, Expr::value(scope.user.id))
        .col_expr(
            checks::Column::Result,
            Expr::value(serde_json::to_string(&reset).unwrap()),
        )
        .col_expr(checks::Column::CheckedAt, Expr::value(0_i64))
        .col_expr(checks::Column::DueAt, Expr::value(0_i64))
        .col_expr(checks::Column::Version, Expr::value(body.version + 1))
        .col_expr(checks::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(checks::Column::UpdatedBy, Expr::value(scope.user.id))
        .filter(checks::Column::Id.eq(id))
        .filter(checks::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(checks::Column::Version.eq(body.version))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(changed());
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "health-checks",
        &id,
        json!({"name":body.name,"enabled":body.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":body.version+1})))
}
pub async fn delete(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("alerts.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    scoped(&tx, &scope, id).await?;
    use crate::entities::alert_rules;
    if alert_rules::Entity::find()
        .filter(alert_rules::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(alert_rules::Column::TargetKind.eq("health-check"))
        .filter(alert_rules::Column::TargetRef.eq(id))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("monitoring.referenced"),
        ));
    }
    observations::Entity::delete_many()
        .filter(observations::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(observations::Column::CheckId.eq(id))
        .exec(&tx)
        .await?;
    checks::Entity::delete_by_id(i64::from(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "health-checks", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn history(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    if raw.get("check_id").is_none_or(|value| value.is_empty()) {
        let query = ListQuery::parse(&HISTORY, &raw)?;
        return Ok(Json(
            json!({"items":[],"total":0,"page":query.page,"page_size":query.page_size}),
        ));
    }
    let check_id = Id::reference(raw.get("check_id").map(String::as_str).unwrap_or(""))?;
    scoped(&s.db, &scope, check_id).await?;
    let query = ListQuery::parse(&HISTORY, &raw)?;
    let (rows, total) = resource::list::<observations::Entity, observations::Model>(
        &s.db,
        &HISTORY,
        observations::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(|row|json!({"id":Id::from(row.id),"check_id":Id::from(row.check_id),"observed_at":row.observed_at,"result":serde_json::from_str::<Value>(&row.result).unwrap_or(Value::Null)})).collect::<Vec<_>>(),"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}

async fn network_probe(client: &reqwest::Client, kind: &str, settings: &Settings) -> Observation {
    let start = std::time::Instant::now();
    let operation = async {
        if kind == "tcp" {
            let url = reqwest::Url::parse(&format!("tcp://{}", settings.address))
                .map_err(|_| "monitoring.addressInvalid")?;
            let host = url
                .host_str()
                .ok_or("monitoring.addressInvalid")?
                .trim_matches(['[', ']']);
            tokio::net::TcpStream::connect((host, url.port().ok_or("monitoring.addressInvalid")?))
                .await
                .map_err(|_| "monitoring.connectionFailed")?;
            return Ok(None);
        }
        let response = client
            .get(&settings.address)
            .send()
            .await
            .map_err(|_| "monitoring.connectionFailed")?;
        let status = response.status().as_u16();
        if status != settings.expected_status {
            return Err("monitoring.statusMismatch");
        }
        if !settings.body_contains.is_empty() {
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| "monitoring.connectionFailed")?;
                if bytes.len() + chunk.len() > 65536 {
                    return Err("monitoring.responseTooLarge");
                }
                bytes.extend_from_slice(&chunk);
            }
            if !String::from_utf8_lossy(&bytes).contains(&settings.body_contains) {
                return Err("monitoring.bodyMismatch");
            }
        }
        Ok(Some(status))
    };
    let result = tokio::time::timeout(
        Duration::from_secs(settings.timeout_seconds.into()),
        operation,
    )
    .await;
    let mut observation = Observation {
        latency_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        ..Default::default()
    };
    match result {
        Ok(Ok(status)) => {
            observation.outcome = "up".into();
            observation.status_code = status;
        }
        Ok(Err(reason)) => {
            observation.outcome = "down".into();
            observation.reason = reason.into();
        }
        Err(_) => {
            observation.outcome = "down".into();
            observation.reason = "monitoring.timeout".into();
        }
    }
    observation
}

/// A due-time CAS claims a bounded attempt; its timestamp fences late writers on all replicas.
async fn collect_one(s: &AppState, client: &reqwest::Client, row: checks::Model) -> Result<()> {
    let settings: Settings =
        serde_json::from_str(&row.settings).map_err(|_| bad("monitoring.parametersInvalid"))?;
    let now = db::now();
    let claim =
        now + (i64::from(settings.interval_seconds.max(settings.timeout_seconds + 10))) * 1000;
    let claimed = checks::Entity::update_many()
        .col_expr(checks::Column::DueAt, Expr::value(claim))
        .filter(checks::Column::Id.eq(row.id))
        .filter(checks::Column::Version.eq(row.version))
        .filter(checks::Column::Enabled.eq(1))
        .filter(checks::Column::DueAt.eq(row.due_at))
        .filter(checks::Column::DueAt.lte(now))
        .exec(&s.db)
        .await?
        .rows_affected;
    if claimed == 0 {
        return Ok(());
    }
    let observation = async {
        let scope = Scope::for_actor(s, row.workspace_id.into(), row.run_as.into()).await?;
        scope.require("alerts.read")?;
        readable(&scope, &row)?;
        if row.kind == "runtime" {
            scope.require("hosts.terminal")?;
            runtime::probe(s, &scope, &row, &settings).await
        } else {
            Ok(network_probe(client, &row.kind, &settings).await)
        }
    }
    .await
    .unwrap_or_else(|_| Observation {
        outcome: "unknown".into(),
        reason: "monitoring.observerUnavailable".into(),
        ..Default::default()
    });
    let finished = db::now();
    let mut previous: CheckResult = serde_json::from_str(&row.result).unwrap_or_default();
    if row.checked_at == 0 || finished - row.checked_at > i64::from(settings.stale_seconds) * 1000 {
        previous.successes = 0;
        previous.failures = 0;
    }
    let next = transition(&previous, observation.clone(), &settings);
    let tx = crate::variables::transaction(s, row.workspace_id.into()).await?;
    let updated = checks::Entity::update_many()
        .col_expr(
            checks::Column::Result,
            Expr::value(serde_json::to_string(&next).unwrap()),
        )
        .col_expr(checks::Column::CheckedAt, Expr::value(finished))
        .col_expr(
            checks::Column::DueAt,
            Expr::value(finished + i64::from(settings.interval_seconds) * 1000),
        )
        .filter(checks::Column::Id.eq(row.id))
        .filter(checks::Column::Version.eq(row.version))
        .filter(checks::Column::Enabled.eq(1))
        .filter(checks::Column::DueAt.eq(claim))
        .exec(&tx)
        .await?
        .rows_affected;
    if updated == 0 {
        return Ok(());
    }
    observations::Entity::insert(observations::ActiveModel {
        id: Set(s.next_id()?.into()),
        workspace_id: Set(row.workspace_id),
        check_id: Set(row.id),
        result: Set(serde_json::to_string(&observation).unwrap()),
        observed_at: Set(finished),
        observed_by: Set(0),
    })
    .exec(&tx)
    .await?;
    observations::Entity::delete_many()
        .filter(observations::Column::WorkspaceId.eq(row.workspace_id))
        .filter(observations::Column::CheckId.eq(row.id))
        .filter(
            observations::Column::ObservedAt
                .lt(finished - i64::from(settings.retention_days) * 86400000),
        )
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn scheduler(s: AppState, mut drain: tokio::sync::watch::Receiver<bool>) {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let mut last_prune = std::time::Instant::now();
    loop {
        if *drain.borrow() {
            break;
        }
        if last_prune.elapsed() >= Duration::from_secs(60) {
            if let Ok(rows) = checks::Entity::find().all(&s.db).await {
                for row in rows {
                    if let Ok(settings) = serde_json::from_str::<Settings>(&row.settings) {
                        if let Err(error) = observations::Entity::delete_many()
                            .filter(observations::Column::WorkspaceId.eq(row.workspace_id))
                            .filter(observations::Column::CheckId.eq(row.id))
                            .filter(
                                observations::Column::ObservedAt
                                    .lt(db::now() - i64::from(settings.retention_days) * 86400000),
                            )
                            .exec(&s.db)
                            .await
                        {
                            tracing::warn!(%error,"health history pruning failed");
                        }
                    }
                }
            }
            last_prune = std::time::Instant::now();
        }
        match checks::Entity::find()
            .filter(checks::Column::Enabled.eq(1))
            .filter(checks::Column::DueAt.lte(db::now()))
            .order_by_asc(checks::Column::DueAt)
            .limit(64)
            .all(&s.db)
            .await
        {
            Ok(rows) => {
                let work = futures::stream::iter(rows).for_each_concurrent(8, |row| {
                    let s = &s;
                    let client = &client;
                    async move {
                        if let Err(error) = collect_one(s, client, row).await {
                            tracing::warn!(error=%error.1,"health observation failed");
                        }
                    }
                });
                tokio::select! {_=work=>{},_=drain.changed()=>{break;}}
            }
            Err(error) => tracing::warn!(%error,"health checks query failed"),
        }
        tokio::select! {_=drain.changed()=>{break;},_=tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}

pub async fn application(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    let Json(value) = applications(
        State(s),
        scope,
        Query(HashMap::from([("id".into(), id.to_string())])),
    )
    .await?;
    value["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .map(Json)
        .ok_or_else(ApiError::missing)
}

pub async fn applications(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("alerts.read")?;
    scope.require("deployments.read")?;
    let query = ListQuery::parse(&APPS, &raw)?;
    let (apps, total) = resource::list::<deployments::Entity, deployments::Model>(
        &s.db,
        &APPS,
        deployments::Entity::find(),
        &scope.workspace_id,
        &query,
    )
    .await?;
    let app_ids: Vec<i64> = apps.iter().map(|app| app.id).collect();
    let rows = checks::Entity::find()
        .filter(checks::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(checks::Column::DeploymentId.is_in(app_ids))
        .all(&s.db)
        .await?;
    let mut items = Vec::new();
    for app in apps {
        let mut own: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.deployment_id == app.id
                    && (row.host_id == 0 || scope.permissions.contains("hosts.read"))
            })
            .cloned()
            .collect();
        let mut required_hosts = Vec::new();
        // Blueprint revisions store per-host applications, not one manifest: no derived
        // topology for them yet.
        if app.target_kind == "host" {
            required_hosts.push(app.target_ref);
        } else if app.target_kind == "host-group" && scope.permissions.contains("hosts.read") {
            use crate::entities::hosts;
            required_hosts = hosts::Entity::find()
                .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(hosts::Column::GroupId.eq(app.target_ref))
                .all(&s.db)
                .await?
                .into_iter()
                .map(|host| host.id)
                .collect();
        }
        for row in own.iter_mut().filter(|row| row.kind == "runtime") {
            let revision = runtime::running_revision(&s.db, &app, row.host_id).await?;
            let result: CheckResult = serde_json::from_str(&row.result).unwrap_or_default();
            if revision == 0 || result.observation.revision_id != Id::from(revision).to_string() {
                row.checked_at = 0;
            }
        }
        let references: Vec<_> = own.iter().collect();
        let mut summary = runtime::aggregate(&references, db::now());
        // Derive expected replicas from the deployed topology, including targets
        // that have never produced a sample. An empty observation is not zero demand.
        let mut topology = Vec::new();
        for host in &required_hosts {
            let revision_id = runtime::running_revision(&s.db, &app, *host).await?;
            if revision_id == 0 {
                continue;
            }
            use crate::entities::deployment_revisions as revisions;
            let Some(revision) = revisions::Entity::find_by_id(revision_id)
                .filter(revisions::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(revisions::Column::DeploymentId.eq(app.id))
                .one(&s.db)
                .await?
            else {
                continue;
            };
            let manifest = crate::deployments::manifest::parse_for(&revision.manifest, &app.mode)?;
            let runtime_scope = own
                .iter()
                .filter(|row| row.kind == "runtime" && row.host_id == *host && row.enabled == 1)
                .filter_map(|row| serde_json::from_str::<CheckResult>(&row.result).ok())
                .map(|result| result.observation.runtime_scope)
                .find(|scope| !scope.is_empty())
                .unwrap_or_else(|| host.to_string());
            for service in manifest.services {
                let expected = if app.mode == "stack" {
                    service.replicas
                } else {
                    1
                };
                let matching: Vec<_> = own
                    .iter()
                    .filter(|row| row.kind == "runtime" && row.host_id == *host && row.enabled == 1)
                    .collect();
                let samples: Vec<_> = matching
                    .iter()
                    .filter(|row| matches!(state(row, db::now()).as_str(), "up" | "down"))
                    .filter_map(|row| serde_json::from_str::<CheckResult>(&row.result).ok())
                    .filter_map(|result| {
                        result
                            .observation
                            .services
                            .into_iter()
                            .find(|observed| observed.name == service.name)
                    })
                    .collect();
                let healthy = samples
                    .iter()
                    .map(|sample| sample.healthy)
                    .min()
                    .unwrap_or(0)
                    .min(expected);
                let unknown = samples
                    .iter()
                    .map(|sample| sample.unknown)
                    .max()
                    .unwrap_or(expected)
                    .min(expected);
                let observed = samples
                    .iter()
                    .map(|sample| sample.observed)
                    .min()
                    .unwrap_or(0);
                topology.push(runtime::Service {
                    name: if required_hosts.len() > 1 {
                        format!(
                            "{} · {}",
                            service.name,
                            if app.mode == "stack" {
                                runtime_scope.clone()
                            } else {
                                host.to_string()
                            }
                        )
                    } else {
                        service.name
                    },
                    expected,
                    healthy,
                    observed,
                    unknown,
                });
            }
        }
        if app.mode == "stack" {
            topology = runtime::cluster_services(topology);
        }
        if !topology.is_empty() {
            summary.expected = topology.iter().map(|service| service.expected).sum();
            summary.healthy = topology.iter().map(|service| service.healthy).sum();
            summary.unknown = topology.iter().map(|service| service.unknown).sum();
            summary.services = topology;
            if summary.state == "up" && summary.unknown > 0 {
                summary.state = "unknown".into();
            }
        }
        let missing_hosts = required_hosts
            .iter()
            .filter(|host| {
                !own.iter()
                    .any(|row| row.kind == "runtime" && row.enabled == 1 && row.host_id == **host)
            })
            .count();
        if missing_hosts > 0 || required_hosts.is_empty() || app.current_revision == 0 {
            if summary.state == "up" {
                summary.state = "unknown".into();
            }
            // Missing collectors are an explicit coverage gap, never zero failed replicas.
            // Unknown host coverage is reported separately from instance counts.
        }
        let checks: Vec<_> = own.into_iter().map(public).collect();
        let instances: Vec<Value> = checks
            .iter()
            .flat_map(|row| row["instances"].as_array().cloned().unwrap_or_default())
            .collect();
        items.push(json!({"id":Id::from(app.id),"name":app.name,"deployment_state":app.state,"health":if matches!(app.state.as_str(),"running"|"queued"|"applying") {"updating"} else {summary.state.as_str()},"entry_state":summary.entry_state,"healthy_instances":summary.healthy,"expected_instances":summary.expected,"unknown_instances":summary.unknown,"unmonitored_hosts":missing_hosts,"services":summary.services,"instances":instances,"checks":checks}));
    }
    Ok(Json(
        json!({"items":items,"total":total,"page":query.page,"page_size":query.page_size}),
    ))
}
