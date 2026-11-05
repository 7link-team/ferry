//! Platform-only observability. Never records SQL bind values, cookies or query strings.
use crate::{
    accounts::{auth::Account, platform::Admin},
    db,
    entities::platform_observations as events,
    error::{ApiError, Json, Query, Result},
    state::AppState,
};
use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
};
use chrono::{Datelike, Duration, TimeZone, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter, Set,
    Statement,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::Instant,
};

pub mod cache;
pub mod catalog;
pub mod database;
pub mod retention;

#[derive(Clone, Default)]
pub struct Store {
    sender: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Event>>>>,
    pub dropped: Arc<AtomicU64>,
    pub sql: Arc<Mutex<BTreeMap<String, SqlStat>>>,
    pub sql_reset_at: Arc<AtomicI64>,
    pub sql_skipped: Arc<AtomicU64>,
    pub sql_report: Arc<tokio::sync::Mutex<()>>,
    pub limits: Arc<Mutex<retention::Limits>>,
}
#[derive(Clone, Default, serde::Serialize)]
pub struct SqlStat {
    pub count: u64,
    pub failed: u64,
    pub slow: u64,
    pub total_ms: u64,
    pub max_ms: u64,
}
#[derive(Clone)]
pub struct RequestUser(pub Arc<AtomicI64>);
pub struct Event {
    at: i64,
    user: i64,
    method: String,
    route: String,
    status: i32,
    elapsed: i64,
    bytes: i64,
    file: String,
    kind: String,
}
impl Store {
    pub fn record(&self, info: &sea_orm::metric::Info<'_>) {
        if info.statement.sql.contains("platform_observations") {
            return;
        }
        let template = database::fingerprint(&info.statement.sql);
        let Ok(mut rows) = self.sql.try_lock() else {
            self.sql_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if rows.len() >= 200 && !rows.contains_key(&template) {
            self.sql_skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let row = rows.entry(template).or_default();
        let elapsed = info.elapsed.as_millis().min(u64::MAX as u128) as u64;
        row.count = row.count.saturating_add(1);
        row.failed = row.failed.saturating_add(u64::from(info.failed));
        row.slow = row.slow.saturating_add(u64::from(elapsed >= 1000));
        row.total_ms = row.total_ms.saturating_add(elapsed);
        row.max_ms = row.max_ms.max(elapsed);
    }
    pub fn enqueue(&self, event: Event) {
        if let Ok(sender) = self.sender.lock() {
            if let Some(sender) = sender.as_ref() {
                if sender.try_send(event).is_err() {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
    pub fn stop(&self) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
    }
    pub fn start(&self, s: &AppState) -> tokio::task::JoinHandle<()> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Event>(4096);
        *self.sender.lock().expect("monitor sender") = Some(sender);
        let state = Arc::downgrade(s);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut row_count = None;
            loop {
                let first = tokio::select! {value=receiver.recv()=>match value {Some(value)=>Some(value),None=>break},_=timer.tick()=>None};
                let Some(s) = state.upgrade() else {
                    break;
                };
                if first.is_some() {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                if first.is_none() || row_count.is_none() {
                    if let Ok(Some(config)) = crate::entities::platform_state::Entity::find_by_id(1)
                        .one(&s.db)
                        .await
                    {
                        if let Ok(limits) =
                            retention::Limits::read(config.monitor_config.as_deref())
                        {
                            if let Ok(mut current) = s.monitor.limits.lock() {
                                *current = limits;
                            }
                        }
                    }
                    let limits = s
                        .monitor
                        .limits
                        .lock()
                        .map(|v| v.clone())
                        .unwrap_or_default();
                    if let Err(error) = retention::trim(
                        &s.db,
                        None,
                        Some(db::now() - limits.days as i64 * 86_400_000),
                        1000,
                    )
                    .await
                    {
                        tracing::warn!(%error,"monitor history cleanup unavailable");
                    }
                    row_count = retention::count(&s.db, &s.instance_id).await.ok();
                    if let Some(stored) = row_count {
                        if stored > limits.rows {
                            if let Ok(removed) = retention::trim(
                                &s.db,
                                Some(&s.instance_id),
                                None,
                                stored - limits.rows,
                            )
                            .await
                            {
                                row_count = Some(stored.saturating_sub(removed));
                            }
                        }
                    }
                }
                let mut batch = first.into_iter().collect::<Vec<_>>();
                while batch.len() < 100 {
                    match receiver.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                if batch.is_empty() {
                    continue;
                }
                let count = batch.len() as u64;
                let limits = s
                    .monitor
                    .limits
                    .lock()
                    .map(|v| v.clone())
                    .unwrap_or_default();
                let Some(mut stored) = row_count else {
                    s.monitor.dropped.fetch_add(count, Ordering::Relaxed);
                    continue;
                };
                let excess = stored.saturating_add(count).saturating_sub(limits.rows);
                if excess > 0 {
                    match retention::trim(&s.db, Some(&s.instance_id), None, excess).await {
                        Ok(removed) => {
                            stored = stored.saturating_sub(removed);
                            row_count = Some(stored);
                        }
                        Err(error) => {
                            s.monitor.dropped.fetch_add(count, Ordering::Relaxed);
                            tracing::warn!(%error,"monitor row limit cleanup unavailable");
                            continue;
                        }
                    }
                }
                if stored.saturating_add(count) > limits.rows {
                    s.monitor.dropped.fetch_add(count, Ordering::Relaxed);
                    continue;
                }
                let models = batch
                    .into_iter()
                    .filter_map(|e| {
                        Some(events::ActiveModel {
                            id: Set(i64::from(s.next_id().ok()?)),
                            at: Set(e.at),
                            event_day: Set(e.at / 86_400_000),
                            user_id: Set(e.user),
                            instance_id: Set(s.instance_id.clone()),
                            kind: Set(e.kind),
                            method: Set(e.method),
                            route: Set(e.route),
                            file_path: Set(e.file),
                            status: Set(e.status),
                            elapsed_ms: Set(e.elapsed),
                            response_bytes: Set(e.bytes),
                        })
                    })
                    .collect::<Vec<_>>();
                let inserted = models.len() as u64;
                if inserted > 0 {
                    if events::Entity::insert_many(models)
                        .exec(&s.db)
                        .await
                        .is_err()
                    {
                        s.monitor.dropped.fetch_add(count, Ordering::Relaxed);
                        tracing::warn!("monitor observation batch unavailable");
                    } else {
                        row_count = Some(stored + inserted);
                    }
                }
            }
        })
    }
}

pub async fn middleware(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    if !path.starts_with("/api/")
        || path.starts_with("/api/platform/monitor")
        || path.starts_with("/api/runners/")
        || path == "/api/status"
        || path == "/api/monitor/pageview"
    {
        return next.run(req).await;
    }
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "/api/unmatched".into());
    let method = req.method().to_string();
    let user = RequestUser(Arc::new(AtomicI64::new(0)));
    req.extensions_mut().insert(user.clone());
    let started = Instant::now();
    let response = next.run(req).await;
    let status = i32::from(response.status().as_u16());
    let is_file = method == "GET"
        && (response.headers().contains_key("content-disposition")
            || (route.contains("assets/") || route.contains("artifacts/"))
                && route.ends_with("/content"));
    s.monitor.enqueue(Event {
        at: db::now(),
        user: user.0.load(Ordering::Relaxed),
        method,
        route,
        status,
        elapsed: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
        bytes: response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        file: if is_file {
            path.chars().take(1024).collect()
        } else {
            String::new()
        },
        kind: if is_file { "file" } else { "api" }.into(),
    });
    response
}

#[derive(Deserialize)]
pub struct PageView {
    pub path: String,
}
pub async fn pageview(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<PageView>,
) -> Result<Json<Value>> {
    let path = body.path.split(['?', '#']).next().unwrap_or_default();
    if !path.starts_with('/') || path.len() > 512 || path.contains(['\n', '\r', '\0']) {
        return Err(bad_query());
    }
    // Store a route template, never object IDs, search values or URL tokens.
    let route = path
        .split('/')
        .map(|part| {
            if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
                ":id"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    s.monitor.enqueue(Event {
        at: db::now(),
        user: i64::from(account.user.id),
        method: "NAVIGATE".into(),
        route,
        status: 200,
        elapsed: 0,
        bytes: 0,
        file: String::new(),
        kind: "page".into(),
    });
    Ok(Json(json!({"recorded":true})))
}
pub fn bad_query() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("request.query"),
    )
}
pub async fn query(
    db: &DatabaseConnection,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<Vec<Value>> {
    let mut sql = sql.to_owned();
    if db.get_database_backend() == DbBackend::MySql {
        for i in (1..=values.len()).rev() {
            sql = sql.replace(&format!("${i}"), "?");
        }
    }
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            values,
        ))
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for name in row.column_names() {
                let v = if let Ok(v) = row.try_get::<i64>("", &name) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<f64>("", &name) {
                    json!(v)
                } else if let Ok(v) = row.try_get::<String>("", &name) {
                    json!(v)
                } else {
                    Value::Null
                };
                object.insert(name, v);
            }
            Value::Object(object)
        })
        .collect())
}
fn since(period: &str) -> Result<i64> {
    let now = Utc::now();
    let date = now.date_naive();
    let start = match period {
        "day" => date,
        "week" => date - Duration::days(date.weekday().num_days_from_monday().into()),
        "month" => date.with_day(1).unwrap(),
        "quarter" => date
            .with_day(1)
            .unwrap()
            .with_month((date.month0() / 3) * 3 + 1)
            .unwrap(),
        "year" => date.with_day(1).unwrap().with_month(1).unwrap(),
        _ => return Err(bad_query()),
    };
    Ok(Utc
        .from_utc_datetime(&start.and_hms_opt(0, 0, 0).unwrap())
        .timestamp_millis())
}
pub async fn usage(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let period = raw.get("period").map(String::as_str).unwrap_or("month");
    let start = since(period)?;
    let summary=query(&s.db,"SELECT COUNT(*) AS requests, COUNT(DISTINCT CASE WHEN user_id > 0 THEN user_id ELSE NULL END) AS active_users, SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) AS errors, SUM(CASE WHEN kind = 'page' THEN 1 ELSE 0 END) AS page_views, SUM(elapsed_ms) AS elapsed_ms, MAX(elapsed_ms) AS max_ms FROM platform_observations WHERE at >= $1",vec![start.into()]).await?;
    let daily=query(&s.db,"SELECT event_day, COUNT(*) AS requests, COUNT(DISTINCT CASE WHEN user_id > 0 THEN user_id ELSE NULL END) AS active_users, SUM(CASE WHEN kind = 'page' THEN 1 ELSE 0 END) AS page_views, SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) AS errors FROM platform_observations WHERE at >= $1 GROUP BY event_day ORDER BY event_day",vec![start.into()]).await?;
    let mut active = serde_json::Map::new();
    for period in ["day", "week", "month", "quarter", "year"] {
        let result=query(&s.db,"SELECT COUNT(DISTINCT user_id) AS users FROM platform_observations WHERE user_id > 0 AND at >= $1",vec![since(period)?.into()]).await?;
        active.insert(
            period.into(),
            result
                .first()
                .and_then(|v| v.get("users"))
                .cloned()
                .unwrap_or(json!(0)),
        );
    }
    let users=query(&s.db,"SELECT COUNT(*) AS total, SUM(CASE WHEN created_at >= $1 THEN 1 ELSE 0 END) AS registered FROM users",vec![start.into()]).await?;
    let online=query(&s.db,"SELECT COUNT(DISTINCT user_id) AS users FROM sessions WHERE seen_at >= $1 AND expires_at >= $2",vec![(db::now()-300_000).into(),db::now().into()]).await?;
    let routes=query(&s.db,"SELECT route, kind, COUNT(*) AS requests, SUM(CASE WHEN status >= 400 THEN 1 ELSE 0 END) AS errors, MAX(elapsed_ms) AS max_ms FROM platform_observations WHERE at >= $1 GROUP BY route, kind ORDER BY requests DESC LIMIT 100",vec![start.into()]).await?;
    let coverage = query(
        &s.db,
        "SELECT MIN(at) AS oldest_at FROM platform_observations",
        vec![],
    )
    .await?;
    let limits = s
        .monitor
        .limits
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default();
    Ok(Json(
        json!({"period":period,"since":start,"summary":summary.first(),"active":active,"users":users.first(),"online":online.first(),"daily":daily,"routes":routes,"dropped_current_node":s.monitor.dropped.load(Ordering::Relaxed),"retention_days":limits.days,"max_rows_per_node":limits.rows,"oldest_at":coverage.first().and_then(|v|v.get("oldest_at")),"timezone":"UTC"}),
    ))
}
pub async fn files(
    State(s): State<AppState>,
    _: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let start = since(raw.get("period").map(String::as_str).unwrap_or("month"))?;
    let assets = query(
        &s.db,
        "SELECT COUNT(*) AS count, COALESCE(SUM(size),0) AS bytes FROM assets",
        vec![],
    )
    .await?;
    let artifacts = query(
        &s.db,
        "SELECT COUNT(*) AS count, COALESCE(SUM(size),0) AS bytes FROM pipeline_artifacts",
        vec![],
    )
    .await?;
    let types=query(&s.db,"SELECT mime, COUNT(*) AS count, COALESCE(SUM(size),0) AS bytes FROM assets GROUP BY mime ORDER BY bytes DESC LIMIT 100",vec![]).await?;
    let top=query(&s.db,"SELECT file_path, COUNT(*) AS requests, SUM(response_bytes) AS bytes, MAX(at) AS last_at FROM platform_observations WHERE kind = 'file' AND at >= $1 AND status >= 200 AND status < 400 GROUP BY file_path ORDER BY requests DESC LIMIT 100",vec![start.into()]).await?;
    let recent=query(&s.db,"SELECT at, user_id, instance_id, file_path, status, response_bytes, elapsed_ms FROM platform_observations WHERE kind = 'file' AND at >= $1 ORDER BY at DESC LIMIT 100",vec![start.into()]).await?;
    Ok(Json(
        json!({"assets":assets.first(),"artifacts":artifacts.first(),"types":types,"top":top,"recent":recent,"since":start}),
    ))
}
