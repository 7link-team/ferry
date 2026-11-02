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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

impl Outcome {
    /// GitHub semantics: a failure the job tolerates counts as success for
    /// dependants and for the run.
    fn tolerated(&self) -> bool {
        self.continue_on_error && matches!(self.state.as_str(), "failure" | "timed_out")
    }
}

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
    snapshot: Snapshot,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let opened = history::open_variables(s, run)?;
    let delivered = Delivered::new(&opened, &snapshot)?;
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
        |job, completed, mut stop| {
            let snapshot = &snapshot;
            let variables = &variables;
            let delivered = &delivered;
            let approvals = &approvals;
            let credential = credential.as_ref();
            let seq = &seq;
            async move {
                if job.manual {
                    return wait_for_approval(s, run.id, &job.key, &mut stop).await;
                }
                if approvals.contains(&job.key) && !wait_for_environment(s, run.id, &job.key, &mut stop).await? {
                    return Ok(Outcome {
                        state: "cancelled".into(), steps: BTreeMap::new(),
                        artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false,
                    });
                }
                job_state(s, run.id, &job.key, "running", None).await?;
                let result = if has_approval {
                    let reserve = crate::runner_updates::reserve_with(s.cluster.as_ref(),vec![job.host_id.to_string()]);
                    tokio::pin!(reserve);
                    tokio::select! {
                        guards = &mut reserve => guards?.run(execute_job(
                            s, run, snapshot, &job, &completed, variables, delivered, credential, &mut stop, seq,
                        )).await,
                        _ = stop.changed() => Ok(Outcome {
                            state: "cancelled".into(), steps: BTreeMap::new(),
                            artifacts: vec![], outputs: BTreeMap::new(), continue_on_error: false,
                        }),
                    }
                } else {
                    execute_job(
                        s, run, snapshot, &job, &completed, variables, delivered, credential, &mut stop, seq,
                    )
                    .await
                };
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
                            continue_on_error: false,
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
        !outcome.tolerated() && !matches!(outcome.state.as_str(), "success" | "skipped")
    }) {
        "failed"
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
/// Run all ready jobs concurrently; retain every future until termination and
/// cleanup finish, including when scheduling or persistence encounters errors.
async fn schedule_jobs<F, Fut, A, Active>(
    jobs: &[Job],
    stop: &mut tokio::sync::watch::Receiver<bool>,
    mut active: A,
    mut execute: F,
) -> Result<BTreeMap<String, Outcome>>
where
    F: FnMut(Job, BTreeMap<String, Outcome>, tokio::sync::watch::Receiver<bool>) -> Fut,
    Fut: std::future::Future<Output = Result<Outcome>>,
    A: FnMut() -> Active,
    Active: std::future::Future<Output = Result<bool>>,
{
    let mut completed = BTreeMap::new();
    let mut started = BTreeSet::new();
    let mut running = FuturesUnordered::new();
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
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
        }
        if !*cancelled.borrow() {
            for job in jobs {
                if started.contains(&job.key)
                    || !job.needs.iter().all(|key| completed.contains_key(key))
                {
                    continue;
                }
                started.insert(job.key.clone());
                let future = execute(job.clone(), completed.clone(), cancelled.clone());
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
                        Err(error) => { failure.get_or_insert(error); cancel.send_replace(true); },
                    }
                }
            }
            _ = stop.changed(), if !*cancelled.borrow() => { cancel.send_replace(true); }
            // SQLite has one pooled connection by default. A pending job can
            // own that connection, so keep polling jobs while this read waits;
            // awaiting it outside select would deadlock the entire server.
            status = async { poll.tick().await; active().await }, if !*cancelled.borrow() => {
                match status {
                    Ok(true) => {},
                    Ok(false) => { cancel.send_replace(true); },
                    Err(error) => { failure.get_or_insert(error); cancel.send_replace(true); },
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
) -> Result<Outcome> {
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
    let checkout = if run.repo_url.is_empty()
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
        s, run, job, seq, credential, variables, stop, ch, &command, 1800, false,
    )
    .await?;
    if cancelled {
        return Ok(Outcome {
            state: "cancelled".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
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
    let existing = artifacts::Entity::find()
        .filter(artifacts::Column::RunId.eq(run.id))
        .filter(artifacts::Column::WorkspaceId.eq(run.workspace_id))
        .all(&s.db)
        .await?;
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
    let context = github_context(run);
    upload(
        ch,
        &format!("{private}/event.json"),
        context.payload.to_string(),
        0o600,
    )
    .await?;
    let request = json!({"workflow":workflow,"job":job.key,"workdir":workdir,
        "event_name":context.event_name,"event_path":format!("{private}/event.json"),
        "actor":context.actor,"run_number":run.number,"run_attempt":1,"ref":context.git_ref,
        "sha":run.commit,"repository":context.repository,"server_url":context.server_url,
        "inputs":context.inputs,"needs":needs,"platforms":platforms,"secrets":secrets,"vars":vars,
        "timeout_seconds":86400,"run_id":run.id.to_string(),"artifact_dir":artifact_dir});
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
        s, run, job, seq, credential, variables, stop, ch, &command, 86415, true,
    )
    .await?;
    if cancelled {
        return Ok(Outcome {
            state: "cancelled".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
            artifacts: Vec::new(),
            steps: BTreeMap::new(),
        });
    }
    let result = outcome.ok_or_else(|| ApiError::bad("Actions worker 没有返回执行结果。"))?;
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
    let fresh: Vec<_> = paths
        .into_iter()
        .filter(|path| !existing.iter().any(|artifact| artifact.path == *path))
        .collect();
    crate::artifacts::collect(s, run, ch, private, &fresh, stop).await?;
    Ok(result)
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
                match history::active(s,run.id).await {
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
            update
                .col_expr(steps::Column::State, Expr::value("running"))
                .col_expr(steps::Column::StartedAt, Expr::value(db::now()))
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

#[cfg(test)]
mod tests {
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
        }
    }
    fn success() -> Outcome {
        Outcome {
            state: "success".into(),
            outputs: BTreeMap::new(),
            continue_on_error: false,
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
            |job, completed, _| {
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
            |_, _, _| async {
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
            |job, _, mut cancelled| {
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
