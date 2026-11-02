use super::*;

pub(super) async fn job_cancelling(s: &AppState, run: i64, key: &str) -> Result<bool> {
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
pub(super) async fn acquire_job_group(
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
pub(super) async fn wait_for_environment(
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

pub(super) async fn wait_for_approval(
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
pub(super) async fn job_state(
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
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.windowsContainer"),
        ));
    }
    let host = crate::hosts::remote::Host {
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
        let shell = crate::hosts::windows_shell::GitShell::open(ch).await?;
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
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.createWorkdir"),
            ));
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
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.cleanupFailed"),
        ));
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
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.workerOs"),
            ));
        }
    };
    let exe = if os == "windows" { ".exe" } else { "" };
    let arch = match lines.get(1).copied() {
        Some("x86_64") => "amd64",
        Some("aarch64" | "arm64") => "arm64",
        _ => {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.workerArch"),
            ));
        }
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
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.workerMissing"),
            )
        })?;
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
        || crate::pipelines::actions::job_policies(&snapshot.document, &job.key)
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
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.checkoutFailed"),
        ));
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
            .ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.dependencyMissing"),
                )
            })?;
        let outcome = completed.get(&key).ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.dependencyIncomplete"),
            )
        })?;
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
        crate::pipelines::artifacts::validate_paths(&[artifact.path.clone()])?;
        if !artifact.path.starts_with(&format!("artifacts/{}/", run.id)) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.artifactPathInvalid"),
            ));
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
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.prepareArtifactDir"),
            ));
        }
        let file = tokio::fs::File::open(crate::pipelines::artifacts::file_path(s, artifact))
            .await
            .map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.artifactMissing"),
                )
            })?;
        let stream = futures::stream::try_unfold(file, |mut file| async move {
            let mut bytes = vec![0; 65536];
            let count = file.read(&mut bytes).await.map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("execution.readArtifact"),
                )
            })?;
            if count == 0 {
                return Ok(None);
            }
            bytes.truncate(count);
            Ok(Some((bytes::Bytes::from(bytes), file)))
        });
        ch.write_file(&path, 0o600, Box::pin(stream)).await?;
    }
    let workflow = checkout_workflow(snapshot, job, variables, &mut secrets)?;
    let mut workflow: Value = serde_yaml_ng::from_str(&workflow).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.yamlInvalid"),
        )
    })?;
    let container = job.image != "host" || workflow["jobs"][&job.key].get("container").is_some();
    let job_env = workflow["jobs"][&job.key]
        .as_object_mut()
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.jobMissing"),
            )
        })?
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
                (
                    "GIT_CONFIG_VALUE_1",
                    format!("!{}", quote(&format!("{private}/credential-helper"))),
                ),
                ("GIT_CONFIG_KEY_2", "core.askPass".into()),
                ("GIT_CONFIG_VALUE_2", "".into()),
                ("GIT_ASKPASS", "".into()),
                ("SSH_ASKPASS", "".into()),
            ] {
                job_env[key] = json!(value);
            }
        }
    }
    let workflow = serde_json::to_string(&workflow).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.prepareJobEnv"),
        )
    })?;
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
    let result = outcome.ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.workerNoResult"),
        )
    })?;
    if leg.plan {
        if code != 0 {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.matrixExpansion"),
            ));
        }
        return Ok(result);
    }
    if code != 0 && result.state == "success" {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.workerExit"),
        ));
    }
    let paths: Vec<_> = result
        .artifacts
        .iter()
        .map(|path| format!("artifacts/{path}"))
        .collect();
    crate::pipelines::artifacts::validate_paths(&paths)?;
    if paths
        .iter()
        .any(|path| !path.starts_with(&format!("artifacts/{}/", run.id)) || !path.ends_with(".zip"))
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.workerArtifactPath"),
        ));
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
    crate::pipelines::artifacts::remove(s, run, &replaced).await?;
    let fresh: Vec<_> = paths
        .into_iter()
        .filter(|path| {
            retention.contains_key(path) || !existing.iter().any(|artifact| artifact.path == *path)
        })
        .collect();
    let workflow: Value = serde_yaml_ng::from_str(&snapshot.document.workflow).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("execution.yamlInvalid"),
        )
    })?;
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
                && step["with"]["name"] == crate::pipelines::artifacts::legacy_artifact_name(path))
            .then(|| path.to_owned())
        })
        .collect();
    let modes =
        crate::pipelines::artifacts::legacy_modes(ch, &source_workdir, &fresh, &logical_paths)
            .await?;
    crate::pipelines::artifacts::collect_with_modes(
        s, run, ch, private, &fresh, &retention, stop, &modes,
    )
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
pub(super) async fn execute_legs(
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
    let strategy = crate::pipelines::actions::job_policies(&snapshot.document, &job.key)
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
            append_message(
                s,
                run.id,
                -1,
                &mut *seq.lock().await,
                "stdout",
                Message::new("execution.matrixAssigned")
                    .with("job", job.key.clone())
                    .with("name", instance.name.clone())
                    .with("host", host.to_string()),
            )
            .await?;
            running.push(async move {
                let reserve = crate::hosts::runner::reservations::reserve_with(
                    s.cluster.as_ref(),
                    vec![host.to_string()],
                );
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
                        append_failure(
                            s,
                            run.id,
                            &mut *seq.lock().await,
                            &job.key,
                            Some(&name),
                            error,
                            credential,
                            variables,
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
