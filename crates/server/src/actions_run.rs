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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Outcome {
    #[serde(default)]
    steps: BTreeMap<String, String>,
    #[serde(default)]
    artifacts: Vec<String>,
    state: String,
    #[serde(default)]
    outputs: BTreeMap<String, String>,
}

pub async fn execute(
    s: &AppState,
    run: &runs::Model,
    snapshot: Snapshot,
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let variables = history::open_variables(s, run)?.values;
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
    let mut completed: BTreeMap<String, Outcome> = BTreeMap::new();
    let mut seq = 0;
    while completed.len() < snapshot.jobs.len() {
        if *stop.borrow() || !history::active(s, run.id).await? {
            break;
        }
        let job = snapshot
            .jobs
            .iter()
            .find(|job| {
                !completed.contains_key(&job.key)
                    && job.needs.iter().all(|key| completed.contains_key(key))
            })
            .ok_or_else(|| ApiError::bad("Actions 依赖无法调度。"))?;
        job_state(s, run.id, &job.key, "running", None).await?;
        let result = execute_job(
            s,
            run,
            &snapshot,
            job,
            &completed,
            &variables,
            credential.as_ref(),
            stop,
            &mut seq,
        )
        .await;
        let outcome = match result {
            Ok(result) => result,
            Err(error) => {
                append(
                    s,
                    run.id,
                    -1,
                    &mut seq,
                    "stderr",
                    &history::redact(
                        format!("任务 {} 执行或清理失败：{}\n", job.key, error.1),
                        credential.as_ref(),
                        &variables,
                    ),
                )
                .await?;
                Outcome {
                    state: "failure".into(),
                    outputs: BTreeMap::new(),
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
        completed.insert(job.key.clone(), outcome);
    }
    let final_state = if *stop.borrow() || !history::active(s, run.id).await? {
        "cancelled"
    } else if completed
        .values()
        .any(|outcome| !matches!(outcome.state.as_str(), "success" | "skipped"))
    {
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
async fn job_state(
    s: &AppState,
    run: i64,
    key: &str,
    state: &str,
    outputs: Option<String>,
) -> Result<()> {
    let mut update = jobs::Entity::update_many().col_expr(jobs::Column::State, Expr::value(state));
    update = if state == "running" {
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
    credential: Option<&(String, String, String)>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    seq: &mut i64,
) -> Result<Outcome> {
    let host = hosts::Entity::find_by_id(job.host_id)
        .filter(hosts::Column::WorkspaceId.eq(run.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
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
    let result = execute_private(
        s,
        run,
        snapshot,
        job,
        completed,
        variables,
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
    credential: Option<&(String, String, String)>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    seq: &mut i64,
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
        _ => {
            return Err(ApiError::bad(
                "当前 Actions worker 需要 Linux 或 macOS 执行器。",
            ));
        }
    };
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
    let binary = tokio::fs::read(directory.join(format!("actions-worker-{os}-{arch}")))
        .await
        .map_err(|_| ApiError::bad("缺少对应平台 Actions worker，请完成安装。"))?;
    let chunks: Vec<_> = binary
        .chunks(65536)
        .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    ch.write_file(
        &format!("{private}/worker"),
        0o700,
        Box::pin(futures::stream::iter(chunks)),
    )
    .await?;
    let environment = history::credential_environment(ch, private, credential).await?;
    let workdir = format!("{private}/source");
    let checkout = if run.repo_url.is_empty() {
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
            artifacts: Vec::new(),
            steps: BTreeMap::new(),
        });
    }
    if code != 0 {
        return Err(ApiError::bad("仓库拉取失败。"));
    }
    let (_, references) = crate::actions::parse(&snapshot.document)?;
    let mut secrets = BTreeMap::new();
    let mut vars = BTreeMap::new();
    for reference in references {
        let (namespace, name) = reference
            .name
            .split_once(':')
            .ok_or_else(|| ApiError::bad("变量引用无效。"))?;
        let value = variables
            .get(name)
            .ok_or_else(|| ApiError::bad("缺少变量快照。"))?
            .clone();
        if namespace == "secrets" {
            secrets.insert(name.to_owned(), value);
        } else {
            vars.insert(name.to_owned(), value);
        }
    }
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
    let request = json!({"workflow":snapshot.document.workflow,"job":job.key,"workdir":workdir,"event_name":"workflow_dispatch","needs":needs,"platforms":platforms,"secrets":secrets,"vars":vars,"timeout_seconds":86400,"run_id":run.id.to_string(),"artifact_dir":artifact_dir});
    upload(
        ch,
        &format!("{private}/request"),
        request.to_string(),
        0o600,
    )
    .await?;
    let command = format!(
        "exec {} --request {}",
        quote(&format!("{private}/worker")),
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

async fn stream_events(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    seq: &mut i64,
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
    seq: &mut i64,
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
        seq,
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
