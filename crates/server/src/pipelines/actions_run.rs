//! Actions job orchestration. All target I/O uses the existing HostChannel.
use crate::i18n::Message;
use crate::{
    db,
    entities::{
        hosts, pipeline_run_jobs as jobs, pipeline_run_steps as steps, pipeline_runs as runs,
    },
    error::{ApiError, Result},
    hosts::channel::HostChannel,
    pipelines::actions::{Job, Snapshot},
    pipelines::runs::{self as history, append, append_message, quote, upload},
    snowflake::Id,
    state::AppState,
};
use axum::http::StatusCode;
use futures::{StreamExt, stream::FuturesUnordered};
#[cfg(test)]
use sea_orm::QueryOrder;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

mod checkout;
mod context;
mod events;
mod job;
mod scheduler;

use checkout::checkout_workflow;
use context::{Delivered, github_context, repository_url_without_credentials, workflow_timeout};
pub(crate) use context::{repository_parts, runtime_path};
use events::{append_failure, stream_events};
use job::{
    acquire_job_group, execute_legs, job_cancelling, job_state, wait_for_approval,
    wait_for_environment,
};
use scheduler::{cleanup_job, schedule_jobs};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Outcome {
    #[serde(default)]
    steps: BTreeMap<String, String>,
    #[serde(default)]
    artifacts: Vec<String>,
    state: String,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
    /// Job-level continue-on-error as evaluated by the worker.
    #[serde(default)]
    continue_on_error: bool,
    /// Archives this job created (incl. `overwrite: true` replacements), with
    /// upload-artifact retention-days; 0 means the history policy.
    #[serde(default)]
    artifact_retention: BTreeMap<String, i64>,
    /// Plan mode only.
    #[serde(flatten, default)]
    plan: Plan,
}

/// A worker's matrix plan: the instances in GitHub's expansion order and the
/// strategy that governs them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Plan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    instances: Vec<Instance>,
    #[serde(default, skip_serializing_if = "is_zero")]
    max_parallel: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fail_fast: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Instance {
    name: String,
    matrix: Value,
}
fn is_zero(value: &usize) -> bool {
    *value == 0
}

impl Outcome {
    /// GitHub semantics: a failure the job tolerates counts as success for
    /// dependants and for the run.
    fn tolerated(&self) -> bool {
        self.continue_on_error && matches!(self.state.as_str(), "failure" | "timed_out")
    }
}

/// GitHub's execution limit for a job on a self-hosted runner (5 days); the
/// job's own timeout-minutes (default 360) applies inside the worker.
const WORKER_SECONDS: u64 = 5 * 24 * 3600;

pub async fn execute(
    s: &AppState,
    run: &runs::Model,
    snapshot: Snapshot,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    // Jobs that wait for a person (manual gates, protected environments)
    // must not hold every host's update reservation meanwhile.
    let environment_approval = history::open_variables(s, run)?
        .environments
        .values()
        .any(|environment| environment.approval);
    if environment_approval || snapshot.jobs.iter().any(|job| job.manual) {
        return execute_reserved(s, run, snapshot, stop).await;
    }
    let mut hosts = Vec::new();
    for job in &snapshot.jobs {
        let pool = snapshot
            .document
            .executor_bindings
            .iter()
            .any(|binding| binding.job_id == job.key && binding.target_kind == "pool");
        if pool || job.host_group_id != 0 {
            hosts.extend(
                crate::hosts::execution::candidates(
                    &s.db,
                    run.workspace_id,
                    job.host_id,
                    job.host_group_id,
                    pool,
                    &job.labels,
                )
                .await?
                .into_iter()
                .map(|(host, _, _)| host.id.to_string()),
            );
        } else {
            hosts.push(job.host_id.to_string());
        }
    }
    // Only acquisition is cancellable here. Once execution starts, its existing
    // cancellation path owns remote process termination and credential cleanup.
    let reservations = scheduler::wait_for_reservation(
        crate::hosts::runner::reservations::reserve_with(s.cluster.as_ref(), hosts),
        stop,
        || history::active(s, run.id),
    )
    .await?;
    let Some(reservations) = reservations else {
        return history::finish(s, run.id, "cancelled").await;
    };
    reservations
        .run(execute_reserved(s, run, snapshot, stop))
        .await
}

async fn execute_reserved(
    s: &AppState,
    run: &runs::Model,
    mut snapshot: Snapshot,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    for checkout in &mut snapshot.checkouts {
        if checkout.binding.pipeline_repository() {
            if run.repo_url.is_empty() {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.repositoryRequired"),
                ));
            }
            checkout.binding.repository_url = run.repo_url.clone();
            checkout.binding.git_ref = if run.commit.is_empty() {
                run.branch.clone()
            } else {
                run.commit.clone()
            };
            checkout.credential_id = run.credential_id;
        }
        checkout.binding.path = runtime_path(&checkout.binding.path, run);
    }
    let opened = history::open_variables(s, run)?;
    let delivered = Delivered::new(&opened, &snapshot)?;
    let job_cancel_in_progress = opened.job_cancel_in_progress.clone();
    let approvals: BTreeSet<String> = opened
        .environments
        .iter()
        .filter(|(_, environment)| environment.approval)
        .map(|(key, _)| key.clone())
        .collect();
    let mut variables = opened.values;
    // Include every source credential in server-side redaction, even if a later
    // job fails before its worker can install its own log masks.
    for (index, checkout) in snapshot.checkouts.iter().enumerate() {
        if checkout.credential_id != 0 {
            crate::repositories::auth::validate_target(
                s,
                &s.db,
                run.workspace_id.into(),
                checkout.credential_id.into(),
                &checkout.binding.repository_url,
                false,
            )
            .await?;
            let (kind, user, secret) = crate::credentials::unseal(
                s,
                &Id::from(run.workspace_id),
                &Id::from(checkout.credential_id),
            )
            .await?;
            variables.insert(format!("__SOURCE_{index}_KIND"), kind);
            variables.insert(format!("__SOURCE_{index}_USER"), user);
            variables.insert(format!("__SOURCE_{index}_SECRET"), secret);
        }
    }
    let credential = if run.credential_id == 0 {
        None
    } else {
        crate::repositories::auth::validate_target(
            s,
            &s.db,
            run.workspace_id.into(),
            run.credential_id.into(),
            &run.repo_url,
            false,
        )
        .await?;
        Some(
            crate::credentials::unseal(
                s,
                &Id::from(run.workspace_id),
                &Id::from(run.credential_id),
            )
            .await?,
        )
    };
    let seq = tokio::sync::Mutex::new(0i64);
    let has_approval = !approvals.is_empty() || snapshot.jobs.iter().any(|job| job.manual);
    let completed = schedule_jobs(
        &snapshot.jobs,
        stop,
        || history::active(s, run.id),
        |job| cleanup_job(&snapshot.document, &job.key),
        |job, completed, mut stop, run_cancelled| {
            let snapshot = &snapshot;
            let variables = &variables;
            let delivered = &delivered;
            let approvals = &approvals;
            let job_cancel_in_progress = &job_cancel_in_progress;
            let credential = credential.as_ref();
            let seq = &seq;
            async move {
                // A person cannot approve a gate of a run being cancelled.
                if run_cancelled && (job.manual || approvals.contains(&job.key)) {
                    job_state(s, run.id, &job.key, "cancelled", None).await?;
                    return Ok(Outcome {
                        state: "cancelled".into(), steps: BTreeMap::new(),
                        artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false, artifact_retention: BTreeMap::new(),
 plan: Plan::default(),
                    });
                }
                if job.manual {
                    return wait_for_approval(s, run.id, &job.key, &mut stop).await;
                }
                if approvals.contains(&job.key) && !wait_for_environment(s, run.id, &job.key, &mut stop).await? {
                    return Ok(Outcome {
                        state: "cancelled".into(), steps: BTreeMap::new(),
                        artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false, artifact_retention: BTreeMap::new(),
 plan: Plan::default(),
                    });
                }
                let grouped = job_cancel_in_progress
                    .get(&job.key)
                    .copied()
                    .filter(|_| !run_cancelled);
                if let Some(cancel) = grouped
                    && !acquire_job_group(s, run.id, &job.key, cancel, &mut stop).await?
                {
                    job_state(s, run.id, &job.key, "cancelled", None).await?;
                    return Ok(Outcome {
                        state: "cancelled".into(), steps: BTreeMap::new(),
                        artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false, artifact_retention: BTreeMap::new(),
 plan: Plan::default(),
                    });
                }
                // A grouped job can be cancelled alone by a newer job of its
                // group (cancel-in-progress), so it also watches its own row.
                let (mut stop, poller) = match grouped {
                    Some(_) => {
                        let (cancel, job_stop) = tokio::sync::watch::channel(*stop.borrow());
                        let (s, id, key, mut run_stop) = (s.clone(), run.id, job.key.clone(), stop.clone());
                        let poller = tokio::spawn(async move {
                            loop {
                                tokio::select! {
                                    changed = run_stop.changed() => {
                                        if changed.is_err() { break; }
                                        if *run_stop.borrow() { cancel.send_replace(true); }
                                    }
                                    status = async {
                                        tokio::time::sleep(Duration::from_secs(1)).await;
                                        job_cancelling(&s, id, &key).await
                                    } => {
                                        if status.unwrap_or(false) {
                                            cancel.send_replace(true);
                                        }
                                    }
                                }
                            }
                        });
                        (job_stop, Some(poller))
                    }
                    None => (stop, None),
                };
                job_state(s, run.id, &job.key, "running", None).await?;
                let result = if has_approval {
                    let reserve = crate::hosts::runner::reservations::reserve_with(s.cluster.as_ref(),vec![job.host_id.to_string()]);
                    tokio::pin!(reserve);
                    tokio::select! {
                        guards = &mut reserve => guards?.run(execute_legs(
                            s, run, snapshot, &job, &completed, variables, delivered, credential, &mut stop, seq, run_cancelled,
                        )).await,
                        _ = stop.changed() => Ok(Outcome {
                            state: "cancelled".into(), steps: BTreeMap::new(),
                            artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false, artifact_retention: BTreeMap::new(),
 plan: Plan::default(),
                        }),
                    }
                } else {
                    execute_legs(
                        s, run, snapshot, &job, &completed, variables, delivered, credential, &mut stop, seq, run_cancelled,
                    )
                    .await
                };
                if let Some(poller) = poller {
                    poller.abort();
                }
                let outcome = match result {
                    Ok(result) => result,
                    Err(error) => {
                        append_failure(
                            s,
                            run.id,
                            &mut *seq.lock().await,
                            &job.key,
                            None,
                            error,
                            credential,
                            &variables,
                        )
                        .await?;
                        Outcome {
                            state: "failure".into(),
                            outputs: BTreeMap::new(),
                            continue_on_error: false, artifact_retention: BTreeMap::new(),
 plan: Plan::default(),
                            artifacts: Vec::new(),
                            steps: BTreeMap::new(),
                        }
                    }
                };
                let sealed = s.vault.seal(
                    &serde_json::to_string(&outcome.outputs)
                        .map_err(|_| ApiError::localized(StatusCode::BAD_REQUEST, Message::new("execution.saveJobOutput")))?,
                    &crate::vault::aad(
                        &run.workspace_id.to_string(),
                        "actions-job-outputs",
                        &format!("{}:{}", run.id, job.key),
                    ),
                )?;
                job_state(s, run.id, &job.key, state(&outcome.state), Some(sealed)).await?;
                // Matrix instances share a UI step row. Final worker summaries fold
                // all instances; a fast success must not hide a later failure.
                for step in &job.steps {
                    let terminal = outcome.steps.get(&step.key).map(|value| state(value));
                    let row = steps::Entity::find()
                        .filter(steps::Column::RunId.eq(run.id))
                        .filter(steps::Column::StepIndex.eq(step.index))
                        .one(&s.db)
                        .await?;
                    if let Some(row) = row
                        && ["queued", "running"].contains(&row.state.as_str())
                    {
                        let terminal = terminal.unwrap_or_else(|| {
                            if row.state == "queued" {
                                "skipped"
                            } else {
                                state(&outcome.state)
                            }
                        });
                        steps::Entity::update_many()
                            .col_expr(steps::Column::State, Expr::value(terminal))
                            .col_expr(steps::Column::FinishedAt, Expr::value(db::now()))
                            .filter(steps::Column::Id.eq(row.id))
                            .filter(crate::executions::pipeline_fence(run.id))
                            .exec(&s.db)
                            .await?;
                    }
                }
                Ok(outcome)
            }
        },
    )
    .await?;
    let final_state = if *stop.borrow() || !history::active(s, run.id).await? {
        "cancelled"
    } else if completed.values().any(|outcome| {
        !outcome.tolerated()
            && !matches!(outcome.state.as_str(), "success" | "skipped" | "cancelled")
    }) {
        "failed"
    } else if completed
        .values()
        .any(|outcome| outcome.state == "cancelled")
    {
        // e.g. a job cancelled by a newer job of its concurrency group.
        "cancelled"
    } else {
        "succeeded"
    };
    jobs::Entity::update_many()
        .col_expr(jobs::Column::State, Expr::value("skipped"))
        .col_expr(jobs::Column::FinishedAt, Expr::value(db::now()))
        .filter(jobs::Column::RunId.eq(run.id))
        .filter(crate::executions::pipeline_fence(run.id))
        .filter(jobs::Column::State.eq("queued"))
        .exec(&s.db)
        .await?;
    history::finish(s, run.id, final_state).await
}
fn state(value: &str) -> &str {
    match value {
        "success" => "succeeded",
        "failure" => "failed",
        "cancelled" => "cancelled",
        "timed_out" => "timed_out",
        "skipped" => "skipped",
        _ => "failed",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectionTrait, IntoActiveModel, Schema};
    pub(super) fn test_job(key: &str, needs: &[&str]) -> Job {
        Job {
            key: key.into(),
            name: key.into(),
            needs: needs.iter().map(|key| (*key).into()).collect(),
            labels: vec![],
            steps: vec![],
            host_id: 1,
            image: "host".into(),
            manual: false,
            host_group_id: 0,
        }
    }
    #[test]
    fn worker_plan_results_carry_instances_and_strategy() {
        let plan: Outcome = serde_json::from_value(json!({"kind":"result","state":"success","continue_on_error":false,"outputs":{},"instances":[{"name":"build (a)","matrix":{"os":"a"}},{"name":"build (b)","matrix":{"os":"b"}}],"max_parallel":1,"fail_fast":false})).unwrap();
        assert_eq!(plan.plan.instances.len(), 2);
        assert_eq!(plan.plan.instances[1].matrix, json!({"os":"b"}));
        assert_eq!(
            (plan.plan.max_parallel, plan.plan.fail_fast),
            (1, Some(false))
        );
        let run: Outcome = serde_json::from_value(json!({"kind":"result","state":"failure","continue_on_error":true,"outputs":{},"artifact_retention":{"1/a/a.zip":3}})).unwrap();
        assert!(run.tolerated());
        assert_eq!(run.artifact_retention["1/a/a.zip"], 3);
        // Dependants receive needs without plan fields.
        let encoded = serde_json::to_value(&run).unwrap();
        assert!(encoded.get("instances").is_none() && encoded.get("max_parallel").is_none());
    }

    async fn cancel_while_waiting(stop_service: bool) {
        let dir = tempfile::tempdir().unwrap();
        let mut options = sea_orm::ConnectOptions::new("sqlite://ferry");
        let filename = dir.path().join("runs.sqlite");
        options.map_sqlx_sqlite_opts(move |opts| opts.filename(&filename).create_if_missing(true));
        let db = sea_orm::Database::connect(options).await.unwrap();
        let backend = db.get_database_backend();
        let schema = Schema::new(backend);
        for statement in [
            schema.create_table_from_entity(runs::Entity),
            schema.create_table_from_entity(jobs::Entity),
            schema.create_table_from_entity(steps::Entity),
            schema.create_table_from_entity(crate::entities::holes::Entity),
        ] {
            db.execute(&statement).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "cancel-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let run = runs::Model {
            id: 1,
            workspace_id: 1,
            pipeline_id: 1,
            revision_id: 0,
            environment_id: 0,
            environment_name: String::new(),
            inputs: None,
            concurrency_group: String::new(),
            lease_owner: String::new(),
            lease_epoch: 0,
            not_before: None,
            pipeline_name: String::new(),
            host_name: String::new(),
            triggered_by_name: String::new(),
            host_id: 0,
            number: 1,
            state: "running".into(),
            trigger_kind: "manual".into(),
            triggered_by: 1,
            definition: String::new(),
            variable_snapshot: None,
            repo_url: String::new(),
            branch: String::new(),
            commit: String::new(),
            credential_id: 0,
            created_at: 0,
            started_at: 0,
            finished_at: 0,
        };
        run.clone().into_active_model().insert(&s.db).await.unwrap();
        let base = if stop_service { 8100 } else { 8200 };
        let first = (base + 1).to_string();
        let second = (base + 2).to_string();
        let update = crate::hosts::runner::reservations::exclusive(&second).await;
        let snapshot = Snapshot {
            checkouts: vec![],
            engine: crate::pipelines::actions::ENGINE.into(),
            document: crate::pipelines::actions::Document {
                engine: crate::pipelines::actions::ENGINE.into(),
                workflow: String::new(),
                executor_bindings: vec![],
                checkout_bindings: vec![],
                graph_layout: Value::Null,
            },
            jobs: [base + 1, base + 2]
                .into_iter()
                .map(|host_id| Job {
                    key: host_id.to_string(),
                    name: "must not execute".into(),
                    needs: vec![],
                    labels: vec![],
                    steps: vec![],
                    host_id,
                    image: "host".into(),
                    manual: false,
                    host_group_id: 0,
                })
                .collect(),
        };
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let state = s.clone();
        let task = tokio::spawn(async move { execute(&state, &run, snapshot, &mut stopped).await });
        // Prove the task already holds A while waiting for the upgrade on B.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if tokio::time::timeout(
                    Duration::from_millis(30),
                    crate::hosts::runner::reservations::exclusive(&first),
                )
                .await
                .is_err()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if stop_service {
            stop.send_replace(true);
        } else {
            runs::Entity::update_many()
                .col_expr(runs::Column::State, Expr::value("cancelling"))
                .filter(runs::Column::Id.eq(1))
                .exec(&s.db)
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            runs::Entity::find_by_id(1)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .state,
            "cancelled"
        );
        let _released = tokio::time::timeout(
            Duration::from_secs(1),
            crate::hosts::runner::reservations::exclusive(&first),
        )
        .await
        .unwrap();
        drop(update);
    }

    #[tokio::test]
    async fn database_cancel_releases_partial_reservations() {
        cancel_while_waiting(false).await;
    }

    #[tokio::test]
    async fn service_stop_releases_partial_reservations() {
        cancel_while_waiting(true).await;
    }

    #[tokio::test]
    async fn manual_gate_waits_for_approval_without_a_host() {
        let dir = tempfile::tempdir().unwrap();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let schema = Schema::new(db.get_database_backend());
        for statement in [
            schema.create_table_from_entity(runs::Entity),
            schema.create_table_from_entity(jobs::Entity),
            schema.create_table_from_entity(crate::entities::holes::Entity),
        ] {
            db.execute(&statement).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "approval-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        runs::Model {
            id: 1,
            workspace_id: 1,
            pipeline_id: 1,
            revision_id: 0,
            environment_id: 0,
            environment_name: String::new(),
            inputs: None,
            concurrency_group: String::new(),
            lease_owner: String::new(),
            lease_epoch: 0,
            not_before: None,
            pipeline_name: String::new(),
            host_name: String::new(),
            triggered_by_name: String::new(),
            host_id: 0,
            number: 1,
            state: "running".into(),
            trigger_kind: "manual".into(),
            triggered_by: 1,
            definition: String::new(),
            variable_snapshot: None,
            repo_url: String::new(),
            branch: String::new(),
            commit: String::new(),
            credential_id: 0,
            created_at: 0,
            started_at: 0,
            finished_at: 0,
        }
        .into_active_model()
        .insert(&s.db)
        .await
        .unwrap();
        jobs::Entity::insert(jobs::ActiveModel {
            id: sea_orm::Set(2),
            run_id: sea_orm::Set(1),
            job_key: sea_orm::Set("gate".into()),
            host_id: sea_orm::Set(0),
            state: sea_orm::Set("queued".into()),
            outputs: sea_orm::Set(None),
            concurrency_group: sea_orm::Set(String::new()),
            started_at: sea_orm::Set(0),
            finished_at: sea_orm::Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let (_sender, mut stop) = tokio::sync::watch::channel(false);
        let state = s.clone();
        let waiting =
            tokio::spawn(async move { wait_for_approval(&state, 1, "gate", &mut stop).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let row = jobs::Entity::find_by_id(2)
                    .one(&s.db)
                    .await
                    .unwrap()
                    .unwrap();
                if row.state == "waiting" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!waiting.is_finished());
        jobs::Entity::update_many()
            .col_expr(jobs::Column::State, Expr::value("approved"))
            .filter(jobs::Column::Id.eq(2))
            .exec(&s.db)
            .await
            .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(outcome.state, "success");
        assert_eq!(
            jobs::Entity::find_by_id(2)
                .one(&s.db)
                .await
                .unwrap()
                .unwrap()
                .state,
            "succeeded"
        );
    }
}
