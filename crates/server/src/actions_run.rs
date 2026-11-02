//! Actions job orchestration. All target I/O uses the existing HostChannel.
use crate::{
    actions::{Job, Snapshot},
    channel::HostChannel,
    db,
    entities::{
        hosts, pipeline_run_jobs as jobs, pipeline_run_steps as steps, pipeline_runs as runs,
    },
    error::{ApiError, Result},
    pipeline_runs::{self as history, append, quote, upload},
    snowflake::Id,
    state::AppState,
};
use futures::{StreamExt, stream::FuturesUnordered};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

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
    let hosts = snapshot
        .jobs
        .iter()
        .map(|job| job.host_id.to_string())
        .collect();
    // Only acquisition is cancellable here. Once execution starts, its existing
    // cancellation path owns remote process termination and credential cleanup.
    let reservations = {
        let pending = crate::runner_updates::reserve_with(s.cluster.as_ref(), hosts);
        tokio::pin!(pending);
        let mut poll = tokio::time::interval(Duration::from_millis(200));
        loop {
            if *stop.borrow() || !history::active(s, run.id).await? {
                return history::finish(s, run.id, "cancelled").await;
            }
            tokio::select! {
                biased;
                _ = stop.changed() => {
                    return history::finish(s, run.id, "cancelled").await;
                }
                guards = &mut pending => break guards?,
                _ = poll.tick() => {}
            }
        }
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
                return Err(ApiError::bad("拉取代码步骤需要选择仓库。"));
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
            crate::repository_auth::validate_target(
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
        crate::repository_auth::validate_target(
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
                                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                        if job_cancelling(&s, id, &key).await.unwrap_or(false) {
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
                    let reserve = crate::runner_updates::reserve_with(s.cluster.as_ref(),vec![job.host_id.to_string()]);
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
                        append(
                            s,
                            run.id,
                            -1,
                            &mut *seq.lock().await,
                            "stderr",
                            &history::redact(
                                format!("任务 {} 执行或清理失败：{}\n", job.key, error.1),
                                credential,
                                &variables,
                            ),
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
                        .map_err(|_| ApiError::bad("无法保存任务输出。"))?,
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
        .filter(jobs::Column::State.eq("queued"))
        .exec(&s.db)
        .await?;
    history::finish(s, run.id, final_state).await
}
/// Whether a job may still run once its run is cancelled: GitHub evaluates a
/// pending job's `if` with cancelled() true, so only a condition calling
/// always() or cancelled() can pass. The worker evaluates the condition.
fn cleanup_job(document: &crate::actions::Document, key: &str) -> bool {
    crate::actions::job_policies(document, key)
        .and_then(|policies| policies.condition)
        .is_some_and(|condition| {
            let condition = condition
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            condition.contains("always()") || condition.contains("cancelled()")
        })
}

/// Run all ready jobs concurrently; retain every future until termination and
/// cleanup finish, including when scheduling or persistence encounters errors.
/// After the run is cancelled, ready jobs that `cleanup` admits still start
/// (with `run_cancelled`), as GitHub runs `if: always()` jobs of a cancelled
/// run; the others are skipped so their dependants can be evaluated.
async fn schedule_jobs<F, Fut, A, Active, C>(
    jobs: &[Job],
    stop: &mut tokio::sync::watch::Receiver<bool>,
    mut active: A,
    cleanup: C,
    mut execute: F,
) -> Result<BTreeMap<String, Outcome>>
where
    F: FnMut(Job, BTreeMap<String, Outcome>, tokio::sync::watch::Receiver<bool>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<Outcome>>,
    A: FnMut() -> Active,
    Active: std::future::Future<Output = Result<bool>>,
    C: Fn(&Job) -> bool,
{
    let mut completed = BTreeMap::new();
    let mut started = BTreeSet::new();
    let mut running = FuturesUnordered::new();
    // `cancel` stops ordinary jobs; `halt` (service stop or an internal
    // failure) also stops cleanup jobs.
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let (halt, halted) = tokio::sync::watch::channel(false);
    let mut run_cancelled = false;
    let mut failure = None;
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    // No jobs exist yet, so this initial check cannot wait for a resource
    // held by an unpolled job future.
    if !active().await? || *stop.borrow() {
        return Ok(completed);
    }
    loop {
        if *stop.borrow() {
            cancel.send_replace(true);
            halt.send_replace(true);
        }
        let mut progressed = true;
        while progressed && !*halted.borrow() {
            progressed = false;
            for job in jobs {
                if started.contains(&job.key)
                    || !job.needs.iter().all(|key| completed.contains_key(key))
                {
                    continue;
                }
                let receiver = if !*cancelled.borrow() {
                    cancelled.clone()
                } else if run_cancelled && cleanup(job) {
                    halted.clone()
                } else if run_cancelled {
                    started.insert(job.key.clone());
                    completed.insert(
                        job.key.clone(),
                        Outcome {
                            state: "skipped".into(),
                            steps: BTreeMap::new(),
                            artifacts: vec![],
                            outputs: BTreeMap::new(),
                            continue_on_error: false,
                            artifact_retention: BTreeMap::new(),
                            plan: Plan::default(),
                        },
                    );
                    progressed = true;
                    continue;
                } else {
                    continue;
                };
                started.insert(job.key.clone());
                let future = execute(
                    job.clone(),
                    completed.clone(),
                    receiver,
                    *cancelled.borrow(),
                );
                let key = job.key.clone();
                running.push(async move { (key, future.await) });
            }
        }
        if running.is_empty() {
            break;
        }
        tokio::select! {
            result = running.next() => {
                if let Some((key, result)) = result {
                    match result {
                        Ok(outcome) => { completed.insert(key, outcome); },
                        Err(error) => {
                            failure.get_or_insert(error);
                            cancel.send_replace(true);
                            halt.send_replace(true);
                        },
                    }
                }
            }
            _ = stop.changed(), if !*halted.borrow() => {
                cancel.send_replace(true);
                halt.send_replace(true);
            }
            // SQLite has one pooled connection by default. A pending job can
            // own that connection, so keep polling jobs while this read waits;
            // awaiting it outside select would deadlock the entire server.
            status = async { poll.tick().await; active().await }, if !*cancelled.borrow() => {
                match status {
                    Ok(true) => {},
                    Ok(false) => { run_cancelled = true; cancel.send_replace(true); },
                    Err(error) => {
                        failure.get_or_insert(error);
                        cancel.send_replace(true);
                        halt.send_replace(true);
                    },
                }
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if !*cancelled.borrow() && completed.len() != jobs.len() {
        return Err(ApiError::bad("Actions 依赖无法调度。"));
    }
    Ok(completed)
}

/// The secrets and vars contexts the jobs of a run receive; jobs whose
/// `environment` names a Ferry environment get that environment's values.
struct Delivered {
    secrets: BTreeMap<String, String>,
    vars: BTreeMap<String, String>,
    jobs: BTreeMap<String, (BTreeMap<String, String>, BTreeMap<String, String>)>,
}

impl Delivered {
    fn for_job(&self, key: &str) -> (&BTreeMap<String, String>, &BTreeMap<String, String>) {
        self.jobs
            .get(key)
            .map(|(secrets, vars)| (secrets, vars))
            .unwrap_or((&self.secrets, &self.vars))
    }

    fn new(opened: &history::VariableSnapshot, snapshot: &Snapshot) -> Result<Self> {
        if let (Some(secrets), Some(vars)) = (&opened.secrets, &opened.vars) {
            let jobs = opened
                .environments
                .iter()
                .filter_map(
                    |(key, environment)| match (&environment.secrets, &environment.vars) {
                        (Some(secrets), Some(vars)) => {
                            Some((key.clone(), (secrets.clone(), vars.clone())))
                        }
                        _ => None,
                    },
                )
                .collect();
            return Ok(Self {
                secrets: secrets.clone(),
                vars: vars.clone(),
                jobs,
            });
        }
        // Runs queued before contexts were snapshotted: static references only.
        let (_, references) = crate::actions::parse(&snapshot.document)?;
        let mut delivered = Self {
            secrets: BTreeMap::new(),
            vars: BTreeMap::new(),
            jobs: BTreeMap::new(),
        };
        for reference in references {
            let (namespace, name) = reference
                .name
                .split_once(':')
                .ok_or_else(|| ApiError::bad("变量引用无效。"))?;
            let value = opened
                .values
                .get(name)
                .ok_or_else(|| ApiError::bad("缺少变量快照。"))?
                .clone();
            if namespace == "secrets" {
                delivered.secrets.insert(name.to_owned(), value);
            } else {
                delivered.vars.insert(name.to_owned(), value);
            }
        }
        Ok(delivered)
    }
}

struct GithubContext {
    event_name: &'static str,
    payload: Value,
    actor: String,
    git_ref: String,
    repository: String,
    server_url: String,
    inputs: BTreeMap<String, String>,
}

/// The github context GitHub would give this run: its trigger as the event,
/// the person who started it, and the repository and ref it builds.
fn github_context(run: &runs::Model) -> GithubContext {
    let inputs: BTreeMap<String, Value> = run
        .inputs
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    // The inputs context is typed; github.event.inputs carries strings.
    let inputs: BTreeMap<String, String> = inputs
        .into_iter()
        .map(|(name, value)| {
            let text = match value {
                Value::String(text) => text,
                other => other.to_string(),
            };
            (name, text)
        })
        .collect();
    let (server_url, repository) = repository_parts(&run.repo_url);
    let git_ref = if run.branch.is_empty() {
        String::new()
    } else {
        format!("refs/heads/{}", run.branch)
    };
    let event_name = if run.trigger_kind == "schedule" {
        "schedule"
    } else {
        "workflow_dispatch"
    };
    let mut payload = json!({
        "repository": {"full_name": repository, "html_url": if repository.is_empty() { String::new() } else { format!("{server_url}/{repository}") }},
        "sender": {"login": run.triggered_by_name},
    });
    if event_name == "workflow_dispatch" {
        payload["inputs"] = json!(inputs);
        payload["ref"] = json!(git_ref);
    }
    GithubContext {
        event_name,
        payload,
        actor: run.triggered_by_name.clone(),
        git_ref,
        repository,
        server_url,
        inputs,
    }
}

/// (`https://host`, `owner/repo`) from an HTTP(S), ssh:// or scp-style Git URL,
/// without any credentials it may contain.
pub(crate) fn repository_parts(url: &str) -> (String, String) {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let scheme = if scheme == "http" { "http" } else { "https" };
        return (format!("{scheme}://{host}"), path.to_owned());
    }
    if let Some((authority, path)) = url.split_once(':')
        && !authority.contains('/')
    {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        return (format!("https://{host}"), path.to_owned());
    }
    (String::new(), String::new())
}

fn repository_url_without_credentials(input: &str) -> String {
    if let Some((scheme, rest)) = input.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if scheme == "ssh" {
            let authority = authority
                .rsplit_once('@')
                .map(|(user, _)| format!("{}@{host}", user.split(':').next().unwrap_or(user)))
                .unwrap_or_else(|| host.to_owned());
            return format!("ssh://{authority}/{path}");
        }
        format!("{scheme}://{host}/{path}")
    } else if let Some((authority, path)) = input.split_once(':') {
        if !authority.contains('/') {
            return format!("{authority}:{path}");
        }
        input.into()
    } else {
        input.into()
    }
}

pub(crate) fn runtime_path(input: &str, run: &runs::Model) -> String {
    let repository = repository_url_without_credentials(&run.repo_url);
    let number = run.number.to_string();
    let tokens = [
        ("${{ github.run_number }}", number.as_str()),
        ("${{ github.ref_name }}", run.branch.as_str()),
        ("${{ github.sha }}", run.commit.as_str()),
        ("${{ env.FERRY_REPOSITORY_URL }}", repository.as_str()),
    ];
    let mut rest = input;
    let mut output = String::new();
    while let Some(start) = rest.find("${{") {
        output.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some((token, value)) = tokens.iter().find(|(token, _)| tail.starts_with(token)) {
            output.push_str(value);
            rest = &tail[token.len()..];
        } else {
            output.push_str("${{");
            rest = &tail[3..];
        }
    }
    output.push_str(rest);
    output
}

fn workflow_timeout(workflow: &str, job: &str) -> u64 {
    serde_json::from_str::<Value>(workflow)
        .ok()
        .and_then(|workflow| workflow["jobs"][job]["timeout-minutes"].as_u64())
        .and_then(|minutes| minutes.checked_mul(60))
        .map(|seconds| seconds.saturating_add(60).max(WORKER_SECONDS))
        .unwrap_or(WORKER_SECONDS)
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
async fn job_cancelling(s: &AppState, run: i64, key: &str) -> Result<bool> {
    Ok(jobs::Entity::find()
        .filter(jobs::Column::RunId.eq(run))
        .filter(jobs::Column::JobKey.eq(key))
        .one(&s.db)
        .await?
        .is_some_and(|row| row.state == "cancelling"))
}

/// GitHub job concurrency: one running job per group. A newer job replaces an
/// older pending one; with cancel-in-progress it also cancels the running
/// one. Returns false when this job was replaced or the run was cancelled.
async fn acquire_job_group(
    s: &AppState,
    run: i64,
    key: &str,
    cancel_in_progress: bool,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool> {
    let own = jobs::Entity::find()
        .filter(jobs::Column::RunId.eq(run))
        .filter(jobs::Column::JobKey.eq(key))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if own.concurrency_group.is_empty() {
        return Ok(true);
    }
    let replace = |state: &'static str, from: &'static str| {
        jobs::Entity::update_many()
            .col_expr(jobs::Column::State, Expr::value(state))
            .filter(jobs::Column::ConcurrencyGroup.eq(own.concurrency_group.clone()))
            .filter(jobs::Column::State.eq(from))
            .filter(jobs::Column::Id.ne(own.id))
            .exec(&s.db)
    };
    replace("cancelled", "pending").await?;
    if cancel_in_progress {
        replace("cancelling", "running").await?;
    }
    job_state(s, run, key, "pending", None).await?;
    loop {
        if *stop.borrow() || !history::active(s, run).await? {
            return Ok(false);
        }
        let current = jobs::Entity::find_by_id(own.id)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if current.state == "cancelled" {
            return Ok(false);
        }
        let held = jobs::Entity::find()
            .filter(jobs::Column::ConcurrencyGroup.eq(own.concurrency_group.clone()))
            .filter(jobs::Column::State.is_in(["running", "cancelling"]))
            .filter(jobs::Column::Id.ne(own.id))
            .one(&s.db)
            .await?
            .is_some();
        if !held {
            return Ok(true);
        }
        tokio::select! {
            _ = stop.changed() => {},
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
        }
    }
}

/// A job whose environment requires approval waits until someone approves it
/// (true) or the run is cancelled (false). Unlike a manual gate it then runs.
async fn wait_for_environment(
    s: &AppState,
    run: i64,
    key: &str,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool> {
    job_state(s, run, key, "waiting", None).await?;
    loop {
        if *stop.borrow() || !history::active(s, run).await? {
            job_state(s, run, key, "cancelled", None).await?;
            return Ok(false);
        }
        let row = jobs::Entity::find()
            .filter(jobs::Column::RunId.eq(run))
            .filter(jobs::Column::JobKey.eq(key))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if row.state == "approved" {
            return Ok(true);
        }
        tokio::select! {
            _ = stop.changed() => {},
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
        }
    }
}

async fn wait_for_approval(
    s: &AppState,
    run: i64,
    key: &str,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Outcome> {
    job_state(s, run, key, "waiting", None).await?;
    loop {
        if *stop.borrow() || !history::active(s, run).await? {
            job_state(s, run, key, "cancelled", None).await?;
            return Ok(Outcome {
                state: "cancelled".into(),
                steps: BTreeMap::new(),
                artifacts: vec![],
                outputs: BTreeMap::new(),
                continue_on_error: false,
                artifact_retention: BTreeMap::new(),
                plan: Plan::default(),
            });
        }
        let row = jobs::Entity::find()
            .filter(jobs::Column::RunId.eq(run))
            .filter(jobs::Column::JobKey.eq(key))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if row.state == "approved" {
            job_state(s, run, key, "succeeded", None).await?;
            return Ok(Outcome {
                state: "success".into(),
                steps: BTreeMap::new(),
                artifacts: vec![],
                outputs: BTreeMap::new(),
                continue_on_error: false,
                artifact_retention: BTreeMap::new(),
                plan: Plan::default(),
            });
        }
        tokio::select! {
            _ = stop.changed() => {},
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
        }
    }
}
async fn job_state(
    s: &AppState,
    run: i64,
    key: &str,
    state: &str,
    outputs: Option<String>,
) -> Result<()> {
    let mut update = jobs::Entity::update_many().col_expr(jobs::Column::State, Expr::value(state));
    update = if state == "running" || state == "waiting" {
        update.col_expr(jobs::Column::StartedAt, Expr::value(db::now()))
    } else {
        update
            .col_expr(jobs::Column::FinishedAt, Expr::value(db::now()))
            .col_expr(jobs::Column::Outputs, Expr::value(outputs))
    };
    update
        .filter(jobs::Column::RunId.eq(run))
        .filter(jobs::Column::JobKey.eq(key))
        .exec(&s.db)
        .await?;
    Ok(())
}

async fn execute_job(
    s: &AppState,
    run: &runs::Model,
    snapshot: &Snapshot,
    job: &Job,
    completed: &BTreeMap<String, Outcome>,
    variables: &BTreeMap<String, String>,
    delivered: &Delivered,
    credential: Option<&(String, String, String)>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    seq: &tokio::sync::Mutex<i64>,
    leg: &Leg,
) -> Result<Outcome> {
    let host = hosts::Entity::find_by_id(job.host_id)
        .filter(hosts::Column::WorkspaceId.eq(run.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let windows = host.os == "windows";
    if windows && job.image != "host" {
        return Err(ApiError::bad(
            "Windows 主机只能直接在主机上执行任务（runs-on: self-hosted），不支持容器镜像。",
        ));
    }
    let host = crate::remote::Host {
        id: host.id.into(),
        workspace_id: host.workspace_id.into(),
        credential_id: host.credential_id.into(),
        name: host.name,
        address: host.address,
        port: host.port as u16,
        username: host.username,
        connection: host.connection,
        host_key: host.host_key,
    };
    let ch = crate::channel_open(s, &host).await?;
    // Windows hosts run the same POSIX job scripts through Git for Windows' sh.
    let (ch, private): (Box<dyn HostChannel>, String) = if windows {
        let shell = crate::windows_shell::GitShell::open(ch).await?;
        let private = shell.private_dir("ferry-actions").await?;
        (Box::new(shell), private)
    } else {
        let private = format!("/tmp/actions-{}", uuid::Uuid::new_v4());
        let made = ch
            .exec(
                &format!("umask 077; mkdir {}", quote(&private)),
                Duration::from_secs(15),
            )
            .await?;
        if made.code != 0 {
            return Err(ApiError::bad("无法创建 Actions 工作目录。"));
        }
        (ch, private)
    };
    let result = execute_private(
        s,
        run,
        snapshot,
        job,
        completed,
        variables,
        delivered,
        credential,
        stop,
        seq,
        ch.as_ref(),
        &private,
        leg,
    )
    .await;
    let cleanup = ch
        .exec(
            &format!("rm -rf -- {}", quote(&private)),
            Duration::from_secs(30),
        )
        .await?;
    if cleanup.code != 0 {
        return Err(ApiError::bad("Actions 临时目录清理失败。"));
    }
    result
}
async fn execute_private(
    s: &AppState,
    run: &runs::Model,
    snapshot: &Snapshot,
    job: &Job,
    completed: &BTreeMap<String, Outcome>,
    variables: &BTreeMap<String, String>,
    delivered: &Delivered,
    credential: Option<&(String, String, String)>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    seq: &tokio::sync::Mutex<i64>,
    ch: &dyn HostChannel,
    private: &str,
    leg: &Leg,
) -> Result<Outcome> {
    let run_cancelled = leg.run_cancelled;
    let platform = ch
        .exec("uname -s; uname -m", Duration::from_secs(15))
        .await?;
    let lines: Vec<_> = platform.stdout.lines().collect();
    let os = match lines.first().copied() {
        Some("Linux") => "linux",
        Some("Darwin") => "darwin",
        // Git for Windows' sh reports MINGW64_NT-10.0-… / MSYS_NT-….
        Some(name) if name.starts_with("MINGW") || name.starts_with("MSYS") => "windows",
        _ => {
            return Err(ApiError::bad(
                "Actions worker 需要 Linux、macOS 或 Windows 执行器。",
            ));
        }
    };
    let exe = if os == "windows" { ".exe" } else { "" };
    let arch = match lines.get(1).copied() {
        Some("x86_64") => "amd64",
        Some("aarch64" | "arm64") => "arm64",
        _ => return Err(ApiError::bad("执行器架构尚未支持。")),
    };
    let directory = std::env::var_os("ACTIONS_WORKER_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("actions-workers")
        });
    let binary = tokio::fs::read(directory.join(format!("actions-worker-{os}-{arch}{exe}")))
        .await
        .map_err(|_| ApiError::bad("缺少对应平台 Actions worker，请完成安装。"))?;
    let chunks: Vec<_> = binary
        .chunks(65536)
        .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    ch.write_file(
        &format!("{private}/worker{exe}"),
        0o700,
        Box::pin(futures::stream::iter(chunks)),
    )
    .await?;
    let environment = history::credential_environment(ch, private, credential).await?;
    let workdir = format!("{private}/source");
    // Planning only evaluates the strategy; it needs no sources. As on GitHub,
    // a job gets the repository only through a checkout step (or a reusable
    // workflow, whose file lives in the repository).
    let fetches = job.reusable()
        || crate::actions::job_policies(&snapshot.document, &job.key)
            .is_some_and(|policies| policies.checkout);
    let checkout = if leg.plan
        || !fetches
        || run.repo_url.is_empty()
        || snapshot
            .checkouts
            .iter()
            .any(|checkout| checkout.binding.job_id == job.key)
    {
        "git init -q\n".into()
    } else {
        history::checkout_script(run)?
    };
    upload(
        ch,
        &format!("{private}/checkout"),
        format!(
            "set -e\n{environment}mkdir {}\ncd {}\n{checkout}",
            quote(&workdir),
            quote(&workdir)
        ),
        0o600,
    )
    .await?;
    let command = format!("sh {}", quote(&format!("{private}/checkout")));
    let (code, _, cancelled) = stream_events(
        s,
        run,
        job,
        seq,
        credential,
        variables,
        stop,
        ch,
        &command,
        1800,
        false,
        !run_cancelled,
    )
    .await?;
    if cancelled {
        return Ok(Outcome {
            state: "cancelled".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
            artifact_retention: BTreeMap::new(),
            plan: Plan::default(),
            artifacts: Vec::new(),
            steps: BTreeMap::new(),
        });
    }
    if code != 0 {
        return Err(ApiError::bad("仓库拉取失败。"));
    }
    let (secrets, vars) = delivered.for_job(&job.key);
    let mut secrets = secrets.clone();
    let vars = vars.clone();
    let image = if job.image == "host" {
        "-self-hosted"
    } else {
        &job.image
    };
    let platforms: BTreeMap<_, _> = job.labels.iter().map(|label| (label, image)).collect();
    // act's success()/failure() checks traverse ancestors, while `needs`
    // expressions remain restricted by each job's declared direct dependencies.
    let mut ancestors = job.needs.clone();
    let mut needs = BTreeMap::new();
    while let Some(key) = ancestors.pop() {
        if needs.contains_key(&key) {
            continue;
        }
        let ancestor = snapshot
            .jobs
            .iter()
            .find(|job| job.key == key)
            .ok_or_else(|| ApiError::bad("Actions 依赖不存在。"))?;
        let outcome = completed
            .get(&key)
            .ok_or_else(|| ApiError::bad("Actions 依赖尚未完成。"))?;
        ancestors.extend(ancestor.needs.iter().cloned());
        let mut outcome = outcome.clone();
        if outcome.tolerated() {
            outcome.state = "success".into();
        }
        needs.insert(key, outcome);
    }
    // Restore prior job archives through the existing host transport, so
    // download-artifact can consume them on a different execution host.
    use crate::entities::pipeline_artifacts as artifacts;
    use tokio::io::AsyncReadExt;
    let existing = if leg.plan {
        Vec::new()
    } else {
        artifacts::Entity::find()
            .filter(artifacts::Column::RunId.eq(run.id))
            .filter(artifacts::Column::WorkspaceId.eq(run.workspace_id))
            .all(&s.db)
            .await?
    };
    let artifact_dir = format!("{private}/artifacts");
    for artifact in &existing {
        crate::artifacts::validate_paths(&[artifact.path.clone()])?;
        if !artifact.path.starts_with(&format!("artifacts/{}/", run.id)) {
            return Err(ApiError::bad("Actions 产物路径无效。"));
        }
        let path = format!("{private}/{}", artifact.path);
        let parent = path.rsplit_once('/').unwrap().0;
        let made = ch
            .exec(
                &format!("mkdir -p -- {}", quote(parent)),
                Duration::from_secs(15),
            )
            .await?;
        if made.code != 0 {
            return Err(ApiError::bad("无法准备产物下载目录。"));
        }
        let file = tokio::fs::File::open(crate::artifacts::file_path(s, artifact))
            .await
            .map_err(|_| ApiError::bad("构建产物已丢失。"))?;
        let stream = futures::stream::try_unfold(file, |mut file| async move {
            let mut bytes = vec![0; 65536];
            let count = file
                .read(&mut bytes)
                .await
                .map_err(|_| ApiError::bad("无法读取构建产物。"))?;
            if count == 0 {
                return Ok(None);
            }
            bytes.truncate(count);
            Ok(Some((bytes::Bytes::from(bytes), file)))
        });
        ch.write_file(&path, 0o600, Box::pin(stream)).await?;
    }
    let workflow = checkout_workflow(snapshot, job, variables, &mut secrets)?;
    let mut workflow: Value =
        serde_yaml_ng::from_str(&workflow).map_err(|_| ApiError::bad("Actions YAML 无效。"))?;
    let container = job.image != "host" || workflow["jobs"][&job.key].get("container").is_some();
    let job_env = workflow["jobs"][&job.key]
        .as_object_mut()
        .ok_or_else(|| ApiError::bad("Actions 任务不存在。"))?
        .entry("env")
        .or_insert_with(|| json!({}));
    if !run.repo_url.is_empty() {
        job_env["FERRY_REPOSITORY_URL"] = json!(repository_url_without_credentials(&run.repo_url));
    }
    // The same private helper files used by checkout remain alive for the job.
    // Values contain paths only; credentials never enter command arguments.
    job_env["GIT_TERMINAL_PROMPT"] = json!("0");
    if let Some((kind, user, secret)) = credential {
        if container {
            // A container cannot read the host's private helper files. Use the
            // same Git helper protocol with values delivered as masked secrets.
            for (key, value) in [
                (
                    "__FERRY_GIT_USER",
                    if user.is_empty() { "git" } else { user },
                ),
                ("__FERRY_GIT_SECRET", secret.as_str()),
            ] {
                secrets.insert(key.into(), value.into());
                job_env[key] = json!(format!("${{{{ secrets.{key} }}}}"));
            }
            if kind == "ssh-key" {
                job_env["GIT_SSH_COMMAND"] = json!(format!(
                    "sh -c {} sh",
                    quote(
                        "set -eu; umask 077; key=$(mktemp); trap 'rm -f -- \"$key\"' EXIT HUP INT TERM; printf '%s' \"$__FERRY_GIT_SECRET\" > \"$key\"; ssh -o BatchMode=yes -o IdentitiesOnly=yes -i \"$key\" \"$@\""
                    )
                ));
            } else {
                job_env["GIT_CONFIG_COUNT"] = json!("3");
                job_env["GIT_CONFIG_KEY_0"] = json!("credential.helper");
                job_env["GIT_CONFIG_VALUE_0"] = json!("");
                job_env["GIT_CONFIG_KEY_1"] = json!("credential.helper");
                job_env["GIT_CONFIG_VALUE_1"] = json!(
                    "!f() { cat >/dev/null; [ \"$1\" = get ] || exit 0; printf 'username=%s\\npassword=%s\\n\\n' \"$__FERRY_GIT_USER\" \"$__FERRY_GIT_SECRET\"; }; f"
                );
                job_env["GIT_CONFIG_KEY_2"] = json!("core.askPass");
                job_env["GIT_CONFIG_VALUE_2"] = json!("");
            }
        } else if kind == "ssh-key" {
            job_env["GIT_SSH_COMMAND"] = json!(format!(
                "ssh -o BatchMode=yes -o IdentitiesOnly=yes -i {}",
                quote(&format!("{private}/key"))
            ));
        } else {
            for (key, value) in [
                ("GIT_CONFIG_COUNT", "3".to_owned()),
                ("GIT_CONFIG_KEY_0", "credential.helper".into()),
                ("GIT_CONFIG_VALUE_0", "".into()),
                ("GIT_CONFIG_KEY_1", "credential.helper".into()),
                ("GIT_CONFIG_VALUE_1", format!("{private}/credential-helper")),
                ("GIT_CONFIG_KEY_2", "core.askPass".into()),
                ("GIT_CONFIG_VALUE_2", "".into()),
                ("GIT_ASKPASS", "".into()),
                ("SSH_ASKPASS", "".into()),
            ] {
                job_env[key] = json!(value);
            }
        }
    }
    let workflow =
        serde_json::to_string(&workflow).map_err(|_| ApiError::bad("无法准备任务环境。"))?;
    let context = github_context(run);
    upload(
        ch,
        &format!("{private}/event.json"),
        context.payload.to_string(),
        0o600,
    )
    .await?;
    let timeout_seconds = workflow_timeout(&workflow, &job.key);
    let request = json!({"workflow":workflow,"job":job.key,"workdir":workdir,
        "event_name":context.event_name,"event_path":format!("{private}/event.json"),
        "actor":context.actor,"run_number":run.number,"run_attempt":1,"ref":context.git_ref,
        "sha":run.commit,"repository":context.repository,"server_url":context.server_url,
        "inputs":context.inputs,"needs":needs,"platforms":platforms,"secrets":secrets,"vars":vars,
        "run_cancelled":run_cancelled,"timeout_seconds":timeout_seconds,
        "run_id":run.id.to_string(),"artifact_dir":artifact_dir,
        "mode":if leg.plan { "plan" } else { "" },"matrix":leg.matrix});
    upload(
        ch,
        &format!("{private}/request"),
        request.to_string(),
        0o600,
    )
    .await?;
    let command = format!(
        "exec {} --request {}",
        quote(&format!("{private}/worker{exe}")),
        quote(&format!("{private}/request"))
    );
    let (code, outcome, cancelled) = stream_events(
        s,
        run,
        job,
        seq,
        credential,
        variables,
        stop,
        ch,
        &command,
        // The worker's cancellation cleanup grace (35 s) plus the runner's.
        timeout_seconds + 75,
        true,
        !run_cancelled,
    )
    .await?;
    if cancelled {
        return Ok(Outcome {
            state: "cancelled".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
            artifact_retention: BTreeMap::new(),
            plan: Plan::default(),
            artifacts: Vec::new(),
            steps: BTreeMap::new(),
        });
    }
    let result = outcome.ok_or_else(|| ApiError::bad("Actions worker 没有返回执行结果。"))?;
    if leg.plan {
        if code != 0 {
            return Err(ApiError::bad("Actions 矩阵展开失败。"));
        }
        return Ok(result);
    }
    if code != 0 && result.state == "success" {
        return Err(ApiError::bad("Actions worker 异常退出。"));
    }
    let paths: Vec<_> = result
        .artifacts
        .iter()
        .map(|path| format!("artifacts/{path}"))
        .collect();
    crate::artifacts::validate_paths(&paths)?;
    if paths
        .iter()
        .any(|path| !path.starts_with(&format!("artifacts/{}/", run.id)) || !path.ends_with(".zip"))
    {
        return Err(ApiError::bad("Actions worker 返回了无效产物路径。"));
    }
    let retention: BTreeMap<_, _> = result
        .artifact_retention
        .iter()
        .map(|(path, days)| (format!("artifacts/{path}"), *days))
        .collect();
    // A restored archive missing from the final manifest was deleted by the job;
    // one the job created again was overwritten. Both drop the stored copy.
    let replaced: Vec<_> = existing
        .iter()
        .filter(|artifact| {
            !paths.contains(&artifact.path) || retention.contains_key(&artifact.path)
        })
        .map(|artifact| artifact.path.clone())
        .collect();
    crate::artifacts::remove(s, run, &replaced).await?;
    let fresh: Vec<_> = paths
        .into_iter()
        .filter(|path| {
            retention.contains_key(path) || !existing.iter().any(|artifact| artifact.path == *path)
        })
        .collect();
    let workflow: Value = serde_yaml_ng::from_str(&snapshot.document.workflow)
        .map_err(|_| ApiError::bad("Actions YAML 无效。"))?;
    let source_workdir = workflow["jobs"][&job.key]["defaults"]["run"]["working-directory"]
        .as_str()
        .map(|path| runtime_path(path, run))
        .unwrap_or_default();
    let logical_paths: Vec<String> = workflow["jobs"][&job.key]["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|step| {
            let path = step["name"].as_str()?.strip_prefix("Archive ")?;
            (step["uses"] == "actions/upload-artifact@v4"
                && step["with"]["name"] == crate::artifacts::legacy_artifact_name(path))
            .then(|| path.to_owned())
        })
        .collect();
    let modes = crate::artifacts::legacy_modes(ch, &source_workdir, &fresh, &logical_paths).await?;
    crate::artifacts::collect_with_modes(s, run, ch, private, &fresh, &retention, stop, &modes)
        .await?;
    Ok(result)
}

/// One worker invocation of a job: the whole job, its matrix plan, or one
/// matrix instance placed on a host-group member.
#[derive(Clone, Debug, Default)]
struct Leg {
    run_cancelled: bool,
    plan: bool,
    matrix: Option<Value>,
}

/// Run a job. A matrix job bound to a host group with several online members
/// is planned once, then its instances are spread over the members (fewest
/// running instances first) under the strategy's max-parallel and fail-fast,
/// as GitHub spreads them over runners. Otherwise one worker runs the job and
/// expands its matrix itself.
#[allow(clippy::too_many_arguments)]
async fn execute_legs(
    s: &AppState,
    run: &runs::Model,
    snapshot: &Snapshot,
    job: &Job,
    completed: &BTreeMap<String, Outcome>,
    variables: &BTreeMap<String, String>,
    delivered: &Delivered,
    credential: Option<&(String, String, String)>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    seq: &tokio::sync::Mutex<i64>,
    run_cancelled: bool,
) -> Result<Outcome> {
    let whole = Leg {
        run_cancelled,
        ..Leg::default()
    };
    let strategy = crate::actions::job_policies(&snapshot.document, &job.key)
        .is_some_and(|policies| policies.strategy);
    let members: Vec<i64> = if job.host_group_id != 0 && strategy {
        hosts::Entity::find()
            .filter(hosts::Column::WorkspaceId.eq(run.workspace_id))
            .filter(hosts::Column::GroupId.eq(job.host_group_id))
            .filter(hosts::Column::State.eq("online"))
            .order_by_asc(hosts::Column::Id)
            .all(&s.db)
            .await?
            .into_iter()
            .map(|host| host.id)
            .collect()
    } else {
        Vec::new()
    };
    if members.len() < 2 {
        return execute_job(
            s, run, snapshot, job, completed, variables, delivered, credential, stop, seq, &whole,
        )
        .await;
    }
    let planned = execute_job(
        s,
        run,
        snapshot,
        job,
        completed,
        variables,
        delivered,
        credential,
        stop,
        seq,
        &Leg {
            plan: true,
            ..whole.clone()
        },
    )
    .await?;
    if planned.state != "success" {
        return Ok(planned);
    }
    let Plan {
        instances,
        max_parallel,
        fail_fast,
    } = planned.plan;
    if instances.len() < 2 {
        return execute_job(
            s, run, snapshot, job, completed, variables, delivered, credential, stop, seq, &whole,
        )
        .await;
    }
    let limit = if max_parallel == 0 {
        instances.len()
    } else {
        max_parallel.min(instances.len())
    };
    let fail_fast = fail_fast.unwrap_or(true);
    // Legs stop with the job, and the others stop when fail-fast trips.
    let (abort, aborted) = tokio::sync::watch::channel(*stop.borrow());
    let mut load: BTreeMap<i64, usize> = members.iter().map(|id| (*id, 0)).collect();
    let mut pending = instances.into_iter();
    let mut running = FuturesUnordered::new();
    let mut merged = Outcome {
        state: "success".into(),
        ..Outcome::default()
    };
    let (mut failed, mut tolerated, mut cancelled, mut ran) = (false, true, false, false);
    loop {
        while running.len() < limit && !*aborted.borrow() {
            let Some(instance) = pending.next() else {
                break;
            };
            let host = *load
                .iter()
                .min_by_key(|(id, count)| (**count, **id))
                .map(|(id, _)| id)
                .expect("members");
            *load.get_mut(&host).unwrap() += 1;
            let mut leg_job = job.clone();
            leg_job.host_id = host;
            let leg = Leg {
                matrix: Some(instance.matrix.clone()),
                ..whole.clone()
            };
            let mut leg_stop = aborted.clone();
            append(
                s,
                run.id,
                -1,
                &mut *seq.lock().await,
                "stdout",
                &format!(
                    "任务 {} 的矩阵实例 {} 分配到主机 {host}\n",
                    job.key, instance.name
                ),
            )
            .await?;
            running.push(async move {
                let reserve =
                    crate::runner_updates::reserve_with(s.cluster.as_ref(), vec![host.to_string()]);
                let result = match reserve.await {
                    Ok(guards) => {
                        guards
                            .run(execute_job(
                                s,
                                run,
                                snapshot,
                                &leg_job,
                                completed,
                                variables,
                                delivered,
                                credential,
                                &mut leg_stop,
                                seq,
                                &leg,
                            ))
                            .await
                    }
                    Err(error) => Err(error),
                };
                (host, instance.name, result)
            });
        }
        if running.is_empty() {
            break;
        }
        tokio::select! {
            next = running.next() => {
                let Some((host, name, result)) = next else { break };
                *load.get_mut(&host).unwrap() -= 1;
                let outcome = match result {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        append(
                            s,
                            run.id,
                            -1,
                            &mut *seq.lock().await,
                            "stderr",
                            &history::redact(
                                format!("任务 {} 的矩阵实例 {name} 执行失败：{}\n", job.key, error.1),
                                credential,
                                variables,
                            ),
                        )
                        .await?;
                        Outcome {
                            state: "failure".into(),
                            ..Outcome::default()
                        }
                    }
                };
                ran |= outcome.state != "skipped";
                // Later-completed instances win for the same output, as on GitHub.
                merged.outputs.extend(outcome.outputs);
                for (key, state) in outcome.steps {
                    let rank = |state: &str| match state {
                        "skipped" => 1,
                        "success" => 2,
                        "failure" => 3,
                        "cancelled" => 4,
                        "timed_out" => 5,
                        _ => 0,
                    };
                    if rank(&state) > rank(merged.steps.get(&key).map_or("", String::as_str)) {
                        merged.steps.insert(key, state);
                    }
                }
                match outcome.state.as_str() {
                    "failure" | "timed_out" => {
                        failed = true;
                        tolerated &= outcome.continue_on_error;
                        if fail_fast && !outcome.continue_on_error {
                            abort.send_replace(true);
                        }
                    }
                    "cancelled" => cancelled = true,
                    _ => {}
                }
            }
            changed = stop.changed(), if !*aborted.borrow() => {
                if changed.is_err() || *stop.borrow() {
                    abort.send_replace(true);
                }
            }
        }
    }
    merged.state = if failed {
        merged.continue_on_error = tolerated;
        "failure".into()
    } else if cancelled || *stop.borrow() {
        "cancelled".into()
    } else if !ran {
        "skipped".into()
    } else {
        "success".into()
    };
    Ok(merged)
}

/// Replace bound checkout nodes only; act still evaluates their conditions,
/// reports their original step IDs, and runs them in the selected container.
fn checkout_workflow(
    snapshot: &Snapshot,
    job: &Job,
    variables: &BTreeMap<String, String>,
    secrets: &mut BTreeMap<String, String>,
) -> Result<String> {
    if snapshot.checkouts.is_empty() && !snapshot.jobs.iter().any(|item| item.manual) {
        return Ok(snapshot.document.workflow.clone());
    }
    let mut workflow: Value = serde_yaml_ng::from_str(&snapshot.document.workflow)
        .map_err(|_| ApiError::bad("Actions YAML 无效。"))?;
    for gate in snapshot.jobs.iter().filter(|item| item.manual) {
        workflow["jobs"][&gate.key] = json!({
            "name": gate.name,
            "needs": gate.needs,
            "runs-on": "self-hosted",
            "steps": [{"run": "echo approved"}],
        });
    }
    for (index, checkout) in snapshot.checkouts.iter().enumerate() {
        let binding = &checkout.binding;
        if binding.job_id != job.key {
            continue;
        }
        let step = workflow["jobs"][&job.key]["steps"]
            .as_array_mut()
            .and_then(|steps| {
                steps
                    .iter_mut()
                    .find(|step| step["id"].as_str() == Some(binding.step_id.as_str()))
            })
            .and_then(Value::as_object_mut)
            .ok_or_else(|| ApiError::bad("代码源步骤不存在。"))?;
        let mut env = step
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for field in ["KIND", "USER", "SECRET"] {
            let key = format!("__SOURCE_{index}_{field}");
            let value = variables.get(&key).cloned().unwrap_or_default();
            secrets.insert(key.clone(), value);
            env.insert(
                format!("SOURCE_{field}"),
                json!(format!("${{{{ secrets.{key} }}}}")),
            );
        }
        step.remove("uses");
        step.remove("with");
        step.insert("shell".into(), json!("sh"));
        step.insert("working-directory".into(), json!("${{ github.workspace }}"));
        step.insert("env".into(), json!(env));
        step.insert("run".into(), json!(checkout_node_script(binding)));
    }
    serde_json::to_string(&workflow).map_err(|_| ApiError::bad("无法准备代码源步骤。"))
}

fn checkout_node_script(binding: &crate::actions::CheckoutBinding) -> String {
    let path = if binding.path.is_empty() {
        "."
    } else {
        &binding.path
    };
    let reference = if binding.git_ref.is_empty() {
        "HEAD"
    } else {
        &binding.git_ref
    };
    format!(
        r#"set -eu
workspace=$(pwd -P)
relative={path}
case "$relative" in
    /*|[A-Za-z]:/*) [ -d "$relative" ] || mkdir -p -- "$relative"; cd -- "$relative"; relative='' ;;
esac
original_ifs=$IFS
IFS=/
set -f
for component in $relative; do
    [ -n "$component" ] || continue
    mkdir -p -- "$component"
    cd -- "$component"
    target=$(pwd -P)
    case "$target/" in "$workspace/"*) ;; *) echo 'Checkout path escapes workspace' >&2; exit 1;; esac
done
set +f
IFS=$original_ifs
# Never mix a checkout into unrelated files: an existing directory must be
# empty or already a Git repository (a persistent workdir from earlier runs).
if [ ! -d .git ] && [ -n "$(ls -A)" ]; then
    echo 'Checkout directory is not empty and is not a Git repository; refusing to check out into it' >&2
    exit 1
fi
umask 077
credential_dir=$(mktemp -d "$workspace/.checkout-credentials.XXXXXX")
trap 'rm -rf -- "$credential_dir"' EXIT HUP INT TERM
chmod 700 "$credential_dir"
export GIT_TERMINAL_PROMPT=0
if [ "$SOURCE_KIND" = ssh-key ]; then
    printf '%s' "$SOURCE_SECRET" > "$credential_dir/key"
    chmod 600 "$credential_dir/key"
    export GIT_SSH_COMMAND="ssh -o BatchMode=yes -o IdentitiesOnly=yes -i '$credential_dir/key'"
elif [ -n "$SOURCE_SECRET" ]; then
    cat > "$credential_dir/helper" <<'HELPER'
#!/bin/sh
cat >/dev/null
[ "$1" = get ] || exit 0
printf 'username=%s\npassword=%s\n\n' "${{SOURCE_USER:-git}}" "$SOURCE_SECRET"
HELPER
    chmod 700 "$credential_dir/helper"
    export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=credential.helper GIT_CONFIG_VALUE_0=''
    export GIT_CONFIG_KEY_1=credential.helper GIT_CONFIG_VALUE_1="$credential_dir/helper"
fi
git init -q
git remote remove origin 2>/dev/null || true
git remote add origin {url}
git fetch --no-tags -- origin {reference}
revision=$(git rev-parse --verify --end-of-options 'FETCH_HEAD^{{commit}}')
git checkout --detach "$revision" --
if [ -n "${{GITHUB_OUTPUT:-}}" ]; then
    printf 'commit=%s\nref=%s\n' "$revision" {reference} >> "$GITHUB_OUTPUT"
fi
"#,
        path = quote(path),
        url = quote(&binding.repository_url),
        reference = quote(reference)
    )
}

async fn stream_events(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    seq: &tokio::sync::Mutex<i64>,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    ch: &dyn HostChannel,
    command: &str,
    seconds: u64,
    worker: bool,
    // Cleanup jobs of a cancelled run stop only with the service.
    watch_run: bool,
) -> Result<(i32, Option<Outcome>, bool)> {
    let (output, mut receiver) = tokio::sync::mpsc::channel(64);
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let execution = ch.exec_stream(command, Duration::from_secs(seconds), output, cancelled);
    tokio::pin!(execution);
    let mut buffers: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut outcome = None;
    let mut exit = None;
    let mut closed = false;
    let mut failure = None;
    // An interval remains due under continuous output; recreating sleep each
    // loop would indefinitely postpone cancellation for a noisy command.
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    while exit.is_none() || !closed {
        tokio::select! {
            value=&mut execution, if exit.is_none()=>match value {
                Ok(code)=>exit=Some(code),
                Err(error)=>{exit=Some(-1);failure=Some(error);},
            },
            chunk=receiver.recv(), if !closed=>match chunk {
                None=>closed=true,
                Some((stream,data))=>{
                    if failure.is_some(){continue;}
                    let buffer=buffers.entry(stream.clone()).or_default();buffer.extend(data);
                    if buffer.len()>1024*1024 {
                        failure=Some(ApiError::bad("Actions 日志单行超过 1MB。"));
                        cancel.send_replace(true);buffer.clear();continue;
                    }
                    while let Some(end)=buffer.iter().position(|byte|*byte==b'\n') {
                        let line:Vec<_>=buffer.drain(..=end).collect();
                        if let Err(error)=event(s,run,job,seq,&stream,&line,credential,variables,&mut outcome,worker).await {
                            failure=Some(error);cancel.send_replace(true);break;
                        }
                    }
                }
            },
            _=poll.tick()=>{
                match if watch_run { history::active(s,run.id).await } else { Ok(true) } {
                    Ok(active)=>{if !active || *stop.borrow(){cancel.send_replace(true);}},
                    Err(error)=>{failure=Some(error);cancel.send_replace(true);},
                }
            }
        }
    }
    // Process completion/termination is acknowledged before errors propagate
    // and the caller removes credentials and the job workspace.
    if let Some(error) = failure {
        return Err(error);
    }
    for (stream, line) in buffers {
        if !line.is_empty() {
            event(
                s,
                run,
                job,
                seq,
                &stream,
                &line,
                credential,
                variables,
                &mut outcome,
                worker,
            )
            .await?;
        }
    }
    let cancelled = *cancel.borrow();
    Ok((exit.unwrap_or(-1), outcome, cancelled))
}

async fn event(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    seq: &tokio::sync::Mutex<i64>,
    stream: &str,
    line: &[u8],
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
    outcome: &mut Option<Outcome>,
    worker: bool,
) -> Result<()> {
    let raw = String::from_utf8_lossy(line);
    let value: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
    if worker && stream == "stdout" && value["kind"] == "result" {
        *outcome = Some(
            serde_json::from_value(value).map_err(|_| ApiError::bad("Actions 结果格式无效。"))?,
        );
        return Ok(());
    }
    let step_ids = value
        .get("stepID")
        .or_else(|| value.get("stepid"))
        .and_then(Value::as_array);
    let step_id = step_ids.and_then(|ids| ids.first()).and_then(Value::as_str);
    let step = step_id.and_then(|id| job.steps.iter().find(|step| step.key == id));
    let message = value["msg"].as_str().unwrap_or(&raw);
    let text = history::redact(
        format!("[{}] {}\n", job.key, message.trim_end()),
        credential,
        variables,
    );
    append(
        s,
        run.id,
        step.map(|step| step.index).unwrap_or(-1),
        &mut *seq.lock().await,
        stream,
        &text,
    )
    .await?;
    if let Some(step) = step
        && value["level"] == "error"
        && let Some(code) = step_exit_code(message)
    {
        steps::Entity::update_many()
            .col_expr(steps::Column::ExitCode, Expr::value(code))
            .filter(steps::Column::RunId.eq(run.id))
            .filter(steps::Column::StepIndex.eq(step.index))
            .exec(&s.db)
            .await?;
    }
    if let Some(step) = step {
        let matrix = value["matrix"]
            .as_object()
            .is_some_and(|matrix| !matrix.is_empty());
        let terminal = if !matrix && step_ids.is_some_and(|ids| ids.len() == 1) {
            value["stepResult"].as_str()
        } else {
            None
        };
        let mut update = steps::Entity::update_many();
        update = if let Some(result) = terminal {
            update
                .col_expr(steps::Column::State, Expr::value(state(result)))
                .col_expr(steps::Column::FinishedAt, Expr::value(db::now()))
        } else {
            let update = update
                .col_expr(steps::Column::State, Expr::value("running"))
                .col_expr(steps::Column::StartedAt, Expr::value(db::now()));
            // Show the step name as the engine evaluated it (`${{ ... }}`
            // expanded), as GitHub does. Matrix legs share one row.
            match value["step"].as_str() {
                Some(evaluated) if !matrix && !evaluated.is_empty() => update.col_expr(
                    steps::Column::Name,
                    Expr::value(
                        format!("{} / {}", job.name, evaluated)
                            .chars()
                            .take(255)
                            .collect::<String>(),
                    ),
                ),
                _ => update,
            }
        };
        update
            .filter(steps::Column::RunId.eq(run.id))
            .filter(steps::Column::StepIndex.eq(step.index))
            .filter(steps::Column::State.is_in(if terminal.is_some() {
                vec!["queued", "running"]
            } else {
                vec!["queued"]
            }))
            .exec(&s.db)
            .await?;
    }
    Ok(())
}

/// A failed run step's exit status as the engine reports it: `exit status N`
/// on hosts, `exitcode 'N': failure` in containers.
fn step_exit_code(message: &str) -> Option<i32> {
    let message = message.trim();
    message
        .strip_prefix("exit status ")
        .or_else(|| {
            message
                .strip_prefix("exitcode '")
                .and_then(|rest| rest.strip_suffix("': failure"))
        })
        .and_then(|code| code.parse().ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn step_exit_codes_come_from_engine_error_lines() {
        use super::step_exit_code;
        assert_eq!(step_exit_code("exit status 7"), Some(7));
        assert_eq!(step_exit_code("exitcode '2': failure"), Some(2));
        assert_eq!(step_exit_code("exit status seven"), None);
        assert_eq!(step_exit_code("Error: exit status 7 happened"), None);
    }

    #[test]
    fn github_repository_comes_from_the_url_without_credentials() {
        use super::repository_parts;
        for (url, expected) in [
            (
                "https://github.com/acme/app.git",
                ("https://github.com", "acme/app"),
            ),
            (
                "https://user:token@codeup.aliyun.com/org/group/app",
                ("https://codeup.aliyun.com", "org/group/app"),
            ),
            (
                "git@github.com:acme/app.git",
                ("https://github.com", "acme/app"),
            ),
            (
                "ssh://git@git.example.com:2222/acme/app.git",
                ("https://git.example.com:2222", "acme/app"),
            ),
            (
                "http://git.local/acme/app/",
                ("http://git.local", "acme/app"),
            ),
            ("", ("", "")),
        ] {
            let (server, repository) = repository_parts(url);
            assert_eq!((server.as_str(), repository.as_str()), expected, "{url}");
        }
    }
    use super::*;

    #[test]
    fn repository_placeholder_keeps_ssh_login_and_removes_passwords() {
        for (input, expected) in [
            (
                "https://user:password@example.test/team/repo.git",
                "https://example.test/team/repo.git",
            ),
            (
                "git@example.test:team/repo.git",
                "git@example.test:team/repo.git",
            ),
            (
                "ssh://git:password@example.test/team/repo.git",
                "ssh://git@example.test/team/repo.git",
            ),
        ] {
            assert_eq!(repository_url_without_credentials(input), expected);
        }
    }

    use sea_orm::{ActiveModelTrait, ConnectionTrait, IntoActiveModel, Schema};

    fn test_job(key: &str, needs: &[&str]) -> Job {
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
    fn success() -> Outcome {
        Outcome {
            state: "success".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
            artifact_retention: BTreeMap::new(),
            plan: Plan::default(),
            artifacts: vec![],
            steps: BTreeMap::new(),
        }
    }

    #[test]
    fn worker_receives_standard_dependency_for_manual_gate() {
        let workflow = "jobs:\n  gate:\n    ferry-approval: true\n  deploy:\n    needs: gate\n    runs-on: self-hosted\n    steps:\n      - run: echo deploy\n";
        let mut gate = test_job("gate", &[]);
        gate.manual = true;
        gate.host_id = 0;
        let deploy = test_job("deploy", &["gate"]);
        let snapshot = Snapshot {
            engine: crate::actions::ENGINE.into(),
            document: crate::actions::Document {
                engine: crate::actions::ENGINE.into(),
                workflow: workflow.into(),
                executor_bindings: vec![],
                checkout_bindings: vec![],
                graph_layout: Value::Null,
            },
            jobs: vec![gate, deploy.clone()],
            checkouts: vec![],
        };
        let transformed =
            checkout_workflow(&snapshot, &deploy, &BTreeMap::new(), &mut BTreeMap::new()).unwrap();
        let value: Value = serde_json::from_str(&transformed).unwrap();
        assert_eq!(value["jobs"]["gate"]["runs-on"], "self-hosted");
        assert_eq!(value["jobs"]["gate"]["steps"][0]["run"], "echo approved");
        assert!(value["jobs"]["gate"].get("ferry-approval").is_none());
        assert_eq!(value["jobs"]["deploy"]["needs"], "gate");
        assert_eq!(snapshot.document.workflow, workflow);
    }

    #[tokio::test]
    async fn dag_starts_parallel_branches_and_waits_for_both_before_join() {
        let jobs = vec![
            test_job("left", &[]),
            test_job("right", &[]),
            test_job("join", &["left", "right"]),
        ];
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async { Ok(true) },
            |_| false,
            |job, completed, _, _| {
                let barrier = barrier.clone();
                let finished = finished.clone();
                async move {
                    if job.key == "join" {
                        assert_eq!(finished.load(std::sync::atomic::Ordering::SeqCst), 2);
                        assert_eq!(completed.len(), 2);
                    } else {
                        // A serial implementation deadlocks here and fails the timeout.
                        barrier.wait().await;
                        if job.key == "right" {
                            tokio::time::sleep(Duration::from_millis(30)).await;
                        }
                        finished.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    Ok(success())
                }
            },
        );
        let result = tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[tokio::test]
    async fn scheduler_status_checks_do_not_starve_jobs_holding_the_only_db_connection() {
        use sea_orm::TransactionTrait;
        let mut options = sea_orm::ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = sea_orm::Database::connect(options).await.unwrap();
        let jobs = vec![test_job("database-user", &[])];
        let initial_status = std::sync::atomic::AtomicBool::new(true);
        let connection_owned = tokio::sync::Notify::new();
        let status_started = tokio::sync::Notify::new();
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async {
                if !initial_status.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    // The status future can start before begin() completes.
                    // Wait for ownership instead of inspecting a racy flag.
                    connection_owned.notified().await;
                    status_started.notify_one();
                }
                db.execute_unprepared("SELECT 1").await?;
                Ok(true)
            },
            |_| false,
            |_, _, _, _| async {
                let tx = db.begin().await?;
                connection_owned.notify_one();
                // Release only after the scheduler starts a competing status
                // read. It must still poll this job while that read is pending.
                status_started.notified().await;
                tx.rollback().await?;
                Ok(success())
            },
        );
        let completed = tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .expect("scheduler stopped polling the job holding its database connection")
            .unwrap();
        assert_eq!(completed.len(), 1);
        db.execute_unprepared("SELECT 1").await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_run_still_runs_cleanup_jobs_and_skips_the_rest() {
        let jobs = vec![
            test_job("build", &[]),
            test_job("notify", &["build"]),
            test_job("deploy", &["build"]),
            test_job("report", &["deploy"]),
        ];
        let cancelled_run = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || {
                let cancelled_run = cancelled_run.clone();
                async move { Ok(!cancelled_run.load(std::sync::atomic::Ordering::SeqCst)) }
            },
            |job| ["notify", "report"].contains(&job.key.as_str()),
            |job, completed, mut cancel, run_cancelled| {
                let cancelled_run = cancelled_run.clone();
                let started = started.clone();
                async move {
                    started
                        .lock()
                        .unwrap()
                        .push((job.key.clone(), run_cancelled));
                    if job.key == "build" {
                        // The user cancels while the first job runs.
                        cancelled_run.store(true, std::sync::atomic::Ordering::SeqCst);
                        while !*cancel.borrow() {
                            cancel.changed().await.unwrap();
                        }
                        return Ok(Outcome {
                            state: "cancelled".into(),
                            ..success()
                        });
                    }
                    // Cleanup jobs see their needs, including the skipped ones.
                    if job.key == "report" {
                        assert_eq!(completed["deploy"].state, "skipped");
                    }
                    assert!(
                        !*cancel.borrow(),
                        "cleanup jobs are not cancelled by the run"
                    );
                    Ok(success())
                }
            },
        );
        let completed = tokio::time::timeout(Duration::from_secs(5), execution)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed["build"].state, "cancelled");
        assert_eq!(completed["notify"].state, "success");
        assert_eq!(completed["deploy"].state, "skipped");
        assert_eq!(completed["report"].state, "success");
        let mut started = started.lock().unwrap().clone();
        started.sort();
        assert_eq!(
            started,
            vec![
                ("build".to_owned(), false),
                ("notify".to_owned(), true),
                ("report".to_owned(), true)
            ]
        );
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

    #[test]
    fn cleanup_jobs_are_those_whose_condition_calls_always_or_cancelled() {
        let document = |condition: &str| crate::actions::Document {
            engine: "actions".into(),
            workflow: format!(
                "on: workflow_dispatch\njobs:\n  a:\n    runs-on: self-hosted\n    if: {condition}\n    steps:\n      - run: echo\n"
            ),
            ..serde_json::from_value(json!({"engine":"actions","workflow":""})).unwrap()
        };
        assert!(cleanup_job(&document("${{ always() }}"), "a"));
        assert!(cleanup_job(&document("cancelled ( )"), "a"));
        assert!(cleanup_job(&document("${{ !cancelled() }}"), "a"));
        assert!(!cleanup_job(&document("success()"), "a"));
        assert!(!cleanup_job(
            &document("github.ref == 'refs/heads/main'"),
            "a"
        ));
    }

    #[tokio::test]
    async fn scheduler_failure_cancels_and_drains_other_running_jobs() {
        let jobs = vec![
            test_job("broken", &[]),
            test_job("running", &[]),
            test_job("dependent", &["broken"]),
        ];
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_stop, mut stop) = tokio::sync::watch::channel(false);
        let execution = schedule_jobs(
            &jobs,
            &mut stop,
            || async { Ok(true) },
            |_| false,
            |job, _, mut cancelled, _| {
                let barrier = barrier.clone();
                let cleaned = cleaned.clone();
                async move {
                    assert_ne!(job.key, "dependent");
                    barrier.wait().await;
                    if job.key == "broken" {
                        return Err(ApiError::bad("persistence failure"));
                    }
                    while !*cancelled.borrow() {
                        cancelled.changed().await.unwrap();
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(success())
                }
            },
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), execution)
                .await
                .unwrap()
                .is_err()
        );
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn checkout_nodes_preserve_order_conditions_and_keep_credentials_out_of_scripts() {
        let binding = crate::actions::CheckoutBinding {
            job_id: "build".into(),
            step_id: "source".into(),
            connection_id: "1".into(),
            repository_url: "https://example.invalid/project.git".into(),
            git_ref: "release".into(),
            path: "source".into(),
        };
        let snapshot = Snapshot {
            engine: crate::actions::ENGINE.into(),
            document: crate::actions::Document {
                engine: crate::actions::ENGINE.into(),
                workflow: "jobs: {build: {runs-on: self-hosted, steps: [{id: before, run: echo before}, {id: source, if: 'success()', uses: 'actions/checkout@v4'}, {id: after, run: echo after}]}}".into(),
                executor_bindings: vec![], checkout_bindings: vec![binding.clone()], graph_layout: Value::Null,
            },
            jobs: vec![], checkouts: vec![crate::actions::ResolvedCheckout { binding, credential_id: 7 }],
        };
        let variables = BTreeMap::from([("__SOURCE_0_SECRET".into(), "private-token".into())]);
        let mut secrets = BTreeMap::new();
        let workflow =
            checkout_workflow(&snapshot, &test_job("build", &[]), &variables, &mut secrets)
                .unwrap();
        assert!(!workflow.contains("private-token"));
        let workflow: Value = serde_json::from_str(&workflow).unwrap();
        let steps = workflow["jobs"]["build"]["steps"].as_array().unwrap();
        assert_eq!(steps[0]["id"], "before");
        assert_eq!(steps[1]["id"], "source");
        assert_eq!(steps[1]["if"], "success()");
        assert!(steps[1].get("uses").is_none());
        assert_eq!(steps[2]["id"], "after");
        assert_eq!(secrets["__SOURCE_0_SECRET"], "private-token");
    }

    #[test]
    fn checkout_scripts_fetch_independent_sources_and_reject_symlink_escape() {
        use std::process::Command;
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("before-checkout"), "preserved").unwrap();
        for name in ["first", "second"] {
            let repo = root.path().join(name);
            std::fs::create_dir(&repo).unwrap();
            std::fs::write(repo.join("identity"), name).unwrap();
            for args in [
                vec!["init", "-q"],
                vec!["add", "identity"],
                vec![
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "fixture",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .current_dir(&repo)
                        .args(args)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            let binding = crate::actions::CheckoutBinding {
                repository_url: repo.to_string_lossy().into(),
                path: name.into(),
                ..Default::default()
            };
            let output = Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(&binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join(name).join("identity")).unwrap(),
                name
            );
        }
        assert_eq!(
            std::fs::read_to_string(workspace.join("before-checkout")).unwrap(),
            "preserved"
        );
        // An absolute (persistent) directory holding unrelated files is refused
        // and left untouched; once it is a checkout, later runs refresh it.
        let persistent = root.path().join("persistent");
        std::fs::create_dir(&persistent).unwrap();
        std::fs::write(persistent.join("keep"), "preserved").unwrap();
        let absolute = |path: &std::path::Path| crate::actions::CheckoutBinding {
            repository_url: root.path().join("first").to_string_lossy().into(),
            path: path.to_string_lossy().replace('\\', "/"),
            ..Default::default()
        };
        let run = |binding: &crate::actions::CheckoutBinding| {
            Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap()
        };
        let refused = run(&absolute(&persistent));
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("not empty"));
        assert_eq!(
            std::fs::read_to_string(persistent.join("keep")).unwrap(),
            "preserved"
        );
        assert!(!persistent.join(".git").exists());
        let reused = root.path().join("reused");
        for _ in 0..2 {
            let output = run(&absolute(&reused));
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(
            std::fs::read_to_string(reused.join("identity")).unwrap(),
            "first"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path(), workspace.join("outside")).unwrap();
            let binding = crate::actions::CheckoutBinding {
                repository_url: root.path().join("first").to_string_lossy().into(),
                path: "outside".into(),
                ..Default::default()
            };
            let output = Command::new("sh")
                .arg("-c")
                .arg(checkout_node_script(&binding))
                .current_dir(&workspace)
                .env("SOURCE_KIND", "")
                .env("SOURCE_SECRET", "")
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("escapes workspace"));
        }
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
        ] {
            db.execute(&statement).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
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
            environment_id: 0,
            environment_name: String::new(),
            inputs: None,
            concurrency_group: String::new(),
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
        let update = crate::runner_updates::exclusive(&second).await;
        let snapshot = Snapshot {
            checkouts: vec![],
            engine: crate::actions::ENGINE.into(),
            document: crate::actions::Document {
                engine: crate::actions::ENGINE.into(),
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
                    crate::runner_updates::exclusive(&first),
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
            crate::runner_updates::exclusive(&first),
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
        ] {
            db.execute(&statement).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
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
            environment_id: 0,
            environment_name: String::new(),
            inputs: None,
            concurrency_group: String::new(),
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
