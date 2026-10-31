//! The collector owns sampling; HTTP queries only read the bounded in-memory history.
use crate::{
    db,
    entities::hosts,
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use ferry_protocol::HostMonitor;
use futures::StreamExt;
use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::RwLock,
    time::Duration,
};

pub const COLLECTION_SECONDS: u64 = 5;
const RETENTION_MS: i64 = 3_600_000;
const MAX_SAMPLES: usize = 721;

#[derive(Clone, Serialize)]
pub struct Sample {
    pub sampled_at: i64,
    pub monitor: HostMonitor,
}

#[derive(Default)]
struct History {
    samples: VecDeque<Sample>,
    error: Option<String>,
}

#[derive(Default)]
pub struct Store(RwLock<HashMap<(i64, i64), History>>);

impl Store {
    pub fn record(&self, workspace: Id, host: Id, sample: Sample) {
        let mut rows = self.0.write().unwrap_or_else(|error| error.into_inner());
        let history = rows.entry((workspace.into(), host.into())).or_default();
        if history
            .samples
            .back()
            .is_some_and(|last| last.sampled_at >= sample.sampled_at)
        {
            return;
        }
        let cutoff = sample.sampled_at - RETENTION_MS;
        while history
            .samples
            .front()
            .is_some_and(|old| old.sampled_at < cutoff)
        {
            history.samples.pop_front();
        }
        history.samples.push_back(sample);
        while history.samples.len() > MAX_SAMPLES {
            history.samples.pop_front();
        }
        history.error = None;
    }

    fn failed(&self, workspace: Id, host: Id, error: String) {
        self.0
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .entry((workspace.into(), host.into()))
            .or_default()
            .error = Some(error);
    }

    fn retain(&self, hosts: &HashSet<(i64, i64)>, now: i64) {
        let mut rows = self.0.write().unwrap_or_else(|error| error.into_inner());
        rows.retain(|key, history| {
            while history
                .samples
                .front()
                .is_some_and(|sample| sample.sampled_at < now - RETENTION_MS)
            {
                history.samples.pop_front();
            }
            hosts.contains(key)
        });
    }

    pub fn snapshot(&self, workspace: Id, host: Id, query: &MonitorQuery, now: i64) -> Value {
        let rows = self.0.read().unwrap_or_else(|error| error.into_inner());
        let history = rows.get(&(workspace.into(), host.into()));
        let latest = history.and_then(|row| row.samples.back());
        let error = history.and_then(|row| row.error.as_deref());
        let fresh = latest.is_some_and(|sample| now - sample.sampled_at <= 15_000);
        let mut result = latest
            .map(|sample| json!(sample.monitor))
            .unwrap_or_else(|| json!({}));
        result["sampled_at"] = latest
            .map(|sample| json!(sample.sampled_at))
            .unwrap_or(Value::Null);
        result["server_time"] = json!(now);
        result["collection_interval_seconds"] = json!(COLLECTION_SECONDS);
        result["status"] = json!(if error.is_some() || (latest.is_some() && !fresh) {
            "offline"
        } else if latest.is_some() {
            "online"
        } else {
            "collecting"
        });
        if let Some(error) = error {
            result["error"] = json!(error);
        }
        if let Some(window) = query.window_seconds {
            let cutoff = now - i64::from(window) * 1000;
            let step = i64::from(query.step_seconds.unwrap_or(30)) * 1000;
            let mut samples: Vec<&Sample> = Vec::new();
            if let Some(history) = history {
                for sample in history
                    .samples
                    .iter()
                    .filter(|sample| sample.sampled_at >= cutoff && sample.sampled_at <= now)
                {
                    // Pick the last actual observation in each bucket, retaining its real timestamp.
                    if samples
                        .last()
                        .is_some_and(|last| last.sampled_at / step == sample.sampled_at / step)
                    {
                        samples.pop();
                    }
                    samples.push(sample);
                }
            }
            result["samples"] = json!(samples);
        }
        result
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorQuery {
    pub window_seconds: Option<u32>,
    pub step_seconds: Option<u32>,
}

impl MonitorQuery {
    pub(crate) fn validate(&self) -> Result<()> {
        if self
            .window_seconds
            .is_some_and(|value| ![300, 900, 3600].contains(&value))
            || self
                .step_seconds
                .is_some_and(|value| ![5, 15, 30, 60].contains(&value))
        {
            return Err(ApiError::bad(
                "监控时段仅支持 5 分钟、15 分钟、1 小时；采样刻度仅支持 5、15、30、60 秒。",
            ));
        }
        Ok(())
    }
}

pub async fn read(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Query(query): Query<MonitorQuery>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    query.validate()?;
    // Authorization and existence still use the business DB; metric values never do.
    hosts::Entity::find_by_id(i64::from(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if let Some(cluster) = &s.cluster {
        match cluster.owner(&id.to_string()).await? {
            Some(owner) if owner.workspace != scope.workspace_id => return Err(ApiError::missing()),
            Some(owner) if owner.node != cluster.node => {
                return Ok(Json(
                    cluster
                        .rpc(
                            &owner.node,
                            &crate::cluster::Request::Monitor {
                                workspace: scope.workspace_id,
                                host: id,
                                query,
                            },
                        )
                        .await?,
                ));
            }
            None => {
                let mut value = s
                    .host_monitor
                    .snapshot(scope.workspace_id, id, &query, db::now());
                value["status"] = json!("offline");
                value["error"] = json!("主机暂无有效连接或采集服务，等待连接恢复。 ");
                return Ok(Json(value));
            }
            _ => {}
        }
    }
    Ok(Json(s.host_monitor.snapshot(
        scope.workspace_id,
        id,
        &query,
        db::now(),
    )))
}

pub async fn collect(s: AppState, mut drain: tokio::sync::watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(COLLECTION_SECONDS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_summary = tokio::time::Instant::now() - Duration::from_secs(30);
    loop {
        tokio::select! { _ = drain.changed() => break, _ = tick.tick() => {} }
        let rows = match hosts::Entity::find().all(&s.db).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "读取监控采集节点失败");
                continue;
            }
        };
        s.host_monitor.retain(
            &rows.iter().map(|row| (row.workspace_id, row.id)).collect(),
            db::now(),
        );
        let summary = last_summary.elapsed() >= Duration::from_secs(30);
        if summary {
            last_summary = tokio::time::Instant::now();
        }
        let work = futures::stream::iter(rows).for_each_concurrent(8, |row| {
            let s = s.clone();
            async move {
                let workspace = Id::from(row.workspace_id);
                let id = Id::from(row.id);
                let owner = if let Some(cluster) = &s.cluster {
                    match cluster.collector_owner(&s, &row).await {
                        Ok(Some(owner)) => Some(owner),
                        Ok(None) => {
                            // Crash expiry must not leave a permanently online DB summary.
                            if row.connection == "runner" && row.state == "online" {
                                use crate::entities::locks;
                                use sea_orm::sea_query::Query;
                                let active = Expr::exists(
                                    Query::select()
                                        .expr(Expr::val(1))
                                        .from(locks::Entity)
                                        .and_where(
                                            locks::Column::Name.eq(format!("runner-owner:{id}")),
                                        )
                                        .and_where(locks::Column::ExpiresAt.gt(db::now()))
                                        .to_owned(),
                                );
                                let _ = hosts::Entity::update_many()
                                    .col_expr(hosts::Column::State, Expr::value("offline"))
                                    .filter(hosts::Column::Id.eq(id))
                                    .filter(active.not())
                                    .exec(&s.db)
                                    .await;
                            }
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(host=%id,error=%error.1,"无法确认监控采集归属");
                            return;
                        }
                    }
                } else {
                    None
                };
                let result = tokio::time::timeout(Duration::from_secs(15), async {
                    crate::remote::open(&s, &workspace, &id)
                        .await?
                        .monitor()
                        .await
                })
                .await
                .unwrap_or_else(|_| {
                    Err(ApiError(
                        StatusCode::GATEWAY_TIMEOUT,
                        "主机监控采集超时。".into(),
                    ))
                });
                if let (Some(cluster), Some(owner)) = (&s.cluster, &owner) {
                    if !cluster
                        .owner(&id.to_string())
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|current| {
                            current.fence == owner.fence && current.node == owner.node
                        })
                    {
                        return;
                    }
                }
                match result {
                    Ok(monitor) => {
                        let sampled_at = db::now();
                        // Existing alert rules/list views consume one summary, at their prior cadence.
                        if summary || (row.connection == "ssh" && row.state != "online") {
                            let mut update = hosts::Entity::update_many()
                                .col_expr(hosts::Column::SeenAt, Expr::value(sampled_at))
                                .col_expr(
                                    hosts::Column::Metrics,
                                    Expr::value(crate::hosts::monitor_summary(&monitor)),
                                );
                            if row.connection == "ssh" {
                                update =
                                    update.col_expr(hosts::Column::State, Expr::value("online"));
                            }
                            if let Some(owner) = &owner {
                                update = update.filter(crate::cluster::fence_condition(
                                    &id.to_string(),
                                    &owner.fence,
                                ));
                            }
                            let _ = update
                                .filter(hosts::Column::WorkspaceId.eq(workspace))
                                .filter(hosts::Column::Id.eq(id))
                                .exec(&s.db)
                                .await;
                        }
                        s.host_monitor.record(
                            workspace,
                            id,
                            Sample {
                                sampled_at,
                                monitor,
                            },
                        );
                    }
                    Err(error) => {
                        s.host_monitor.failed(workspace, id, error.1);
                        if row.connection == "ssh" && row.state != "offline" {
                            let mut update = hosts::Entity::update_many()
                                .col_expr(hosts::Column::State, Expr::value("offline"))
                                .filter(hosts::Column::WorkspaceId.eq(workspace))
                                .filter(hosts::Column::Id.eq(id));
                            if let Some(owner) = &owner {
                                update = update.filter(crate::cluster::fence_condition(
                                    &id.to_string(),
                                    &owner.fence,
                                ));
                            }
                            let _ = update.exec(&s.db).await;
                        }
                    }
                }
            }
        });
        tokio::select! { _ = drain.changed() => break, _ = work => {} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::ActiveValue::Set;

    #[tokio::test]
    async fn queries_use_collected_history_without_contacting_host_or_writing_summary() {
        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, tokio::time::Instant::now().into_std())
                    .unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "monitor-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            cluster: None,
        });
        crate::entities::workspaces::Entity::insert(crate::entities::workspaces::ActiveModel {
            id: Set(1),
            slug: Set("monitor-test".into()),
            name: Set("Monitor".into()),
            owner_id: Set(1),
            join_code: Set("monitor-test".into()),
            pipeline_run_number: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        // No Runner is connected: a query cannot accidentally rely on remote::open.
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(2),
            workspace_id: Set(1),
            name: Set("disconnected".into()),
            address: Set(String::new()),
            port: Set(0),
            username: Set(String::new()),
            connection: Set("runner".into()),
            state: Set("offline".into()),
            group_id: Set(0),
            description: Set(String::new()),
            host_key: Set(String::new()),
            runner_key_hash: Set(String::new()),
            enroll_token: Set(String::new()),
            enroll_token_hash: Set(String::new()),
            jump_host_id: Set(0),
            seen_at: Set(123),
            created_at: Set(0),
            updated_at: Set(456),
            version: Set(7),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            agent_update: Set(None),
            metrics: Set("unchanged".into()),
            credential_id: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let scope = |workspace: i64, allowed: bool| Scope {
            workspace_id: workspace.into(),
            user: crate::auth::User {
                id: 1.into(),
                email: "monitor@example.invalid".into(),
                name: "Monitor".into(),
                is_superadmin: false,
            },
            permissions: if allowed {
                ["hosts.read".into()].into()
            } else {
                Default::default()
            },
        };
        let before = hosts::Entity::find_by_id(2)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let at = db::now();
        s.host_monitor.record(1.into(), 2.into(), sample(at, 42.0));
        for _ in 0..2 {
            let Json(value) = read(
                State(s.clone()),
                scope(1, true),
                Path(("1".into(), 2.into())),
                Query(query(5)),
            )
            .await
            .unwrap();
            assert_eq!(value["cpu_percent"], 42.0);
            assert_eq!(value["sampled_at"], at);
            assert_eq!(value["samples"].as_array().unwrap().len(), 1);
        }
        let after = hosts::Entity::find_by_id(2)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            before, after,
            "monitor GET must not write status, summary or edit version"
        );
        assert_eq!(
            read(
                State(s.clone()),
                scope(1, false),
                Path(("1".into(), 2.into())),
                Query(query(5))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            read(
                State(s.clone()),
                scope(9, true),
                Path(("9".into(), 2.into())),
                Query(query(5))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            read(
                State(s),
                scope(1, true),
                Path(("1".into(), 99.into())),
                Query(query(5))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::NOT_FOUND
        );
    }
    fn sample(at: i64, cpu: f64) -> Sample {
        Sample {
            sampled_at: at,
            monitor: HostMonitor {
                cpu_percent: cpu,
                load1: 0.0,
                memory_used: 0,
                memory_total: 100,
                disks: vec![],
                networks: vec![],
            },
        }
    }
    fn query(step: u32) -> MonitorQuery {
        MonitorQuery {
            window_seconds: Some(3600),
            step_seconds: Some(step),
        }
    }

    #[test]
    fn empty_and_one_point_history_are_not_backfilled_and_tenants_are_isolated() {
        let store = Store::default();
        let (workspace, host) = (Id::from(1), Id::from(2));
        assert_eq!(
            store.snapshot(workspace, host, &query(5), 10_000)["samples"],
            json!([])
        );
        store.record(workspace, host, sample(10_000, 42.0));
        let value = store.snapshot(workspace, host, &query(5), 10_000);
        assert_eq!(value["samples"].as_array().unwrap().len(), 1);
        assert_eq!(value["samples"][0]["sampled_at"], 10_000);
        assert_eq!(value["cpu_percent"], 42.0);
        assert_eq!(
            store.snapshot(Id::from(9), host, &query(5), 10_000)["samples"],
            json!([])
        );
    }

    #[test]
    fn retention_is_bounded_and_downsampling_keeps_real_times_and_values() {
        let store = Store::default();
        for index in 0..1000 {
            store.record(1.into(), 2.into(), sample(index * 5000 + 123, index as f64));
        }
        let full = store.snapshot(1.into(), 2.into(), &query(5), 4_995_123);
        assert_eq!(full["samples"].as_array().unwrap().len(), MAX_SAMPLES);
        let reduced = store.snapshot(1.into(), 2.into(), &query(30), 4_995_123);
        assert!(reduced["samples"].as_array().unwrap().len() <= 121);
        assert_eq!(
            reduced["samples"].as_array().unwrap().last().unwrap()["sampled_at"],
            4_995_123
        );
        assert_eq!(
            reduced["samples"].as_array().unwrap().last().unwrap()["monitor"]["cpu_percent"],
            999.0
        );
    }

    #[test]
    fn failure_keeps_last_real_data_and_recovery_does_not_fill_the_gap() {
        let store = Store::default();
        store.record(1.into(), 2.into(), sample(5000, 42.0));
        store.failed(1.into(), 2.into(), "offline".into());
        let value = store.snapshot(1.into(), 2.into(), &query(5), 30_000);
        assert_eq!(value["status"], "offline");
        assert_eq!(value["samples"].as_array().unwrap().len(), 1);
        store.record(1.into(), 2.into(), sample(30_000, 10.0));
        let recovered = store.snapshot(1.into(), 2.into(), &query(5), 30_000);
        assert_eq!(recovered["status"], "online");
        assert!(recovered.get("error").is_none());
        assert_eq!(recovered["samples"].as_array().unwrap().len(), 2);
        assert!(
            MonitorQuery {
                window_seconds: Some(0),
                step_seconds: Some(0)
            }
            .validate()
            .is_err()
        );
    }
}
