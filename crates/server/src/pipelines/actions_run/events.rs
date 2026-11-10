use super::*;

pub(super) async fn stream_events(
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
    // The worker invocation's `ferry/preview@v1` channel, if it has one.
    preview: Option<&Preview<'_>>,
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
    let mut lines: std::collections::VecDeque<(String, Vec<u8>)> =
        std::collections::VecDeque::new();
    let mut processing = FuturesUnordered::new();
    let mut checks = FuturesUnordered::new();
    // Preview requests live as long as the worker invocation that made them.
    let mut previews: FuturesUnordered<
        std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>,
    > = FuturesUnordered::new();
    let mut stop_open = true;
    while exit.is_none() || !closed || !lines.is_empty() || !processing.is_empty() {
        if *stop.borrow() {
            cancel.send_replace(true);
        }
        if failure.is_some() {
            lines.clear();
        }
        if exit.is_some() && closed && lines.is_empty() && processing.is_empty() {
            break;
        }
        if processing.is_empty()
            && let Some((stream, line)) = lines.pop_front()
        {
            if worker
                && stream == "stdout"
                && let Some(preview) = preview
                && let Some(request) = preview_request(&line, preview)
            {
                // Not logged: the marker carries the nonce. The step prints
                // its own progress lines.
                previews.push(Box::pin(serve_preview(
                    s, run, job, seq, ch, preview, request, credential, variables,
                )));
                continue;
            }
            if worker
                && stream == "stdout"
                && let Some(preview) = preview
                && let Some(request) = super::notification::request(&line, preview)
            {
                previews.push(Box::pin(super::notification::serve(
                    s, run, job, ch, preview, request,
                )));
                continue;
            }
            let previous = outcome.take();
            processing.push(async move {
                let mut outcome = previous;
                let result = event(
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
                .await;
                (result, outcome)
            });
        }
        if watch_run && checks.is_empty() && !*cancel.borrow() && exit.is_none() {
            // Keep the query future alive across output chunks. Neither a busy
            // output stream nor a blocked pool may suppress cancellation.
            checks.push(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                history::active(s, run.id).await
            });
        }
        tokio::select! {
            value=&mut execution, if exit.is_none()=>match value {
                Ok(code)=>exit=Some(code),
                Err(error)=>{exit=Some(-1);failure=Some(error);},
            },
            // Preserve transport backpressure while one chunk's log writes are
            // pending; the command and cancellation stay polled independently.
            chunk=receiver.recv(), if !closed && processing.is_empty() && lines.is_empty()=>match chunk {
                None=>closed=true,
                Some((stream,data))=>{
                    if failure.is_some(){continue;}
                    let buffer=buffers.entry(stream.clone()).or_default();buffer.extend(data);
                    if buffer.len()>1024*1024 {
                        failure=Some(ApiError::localized(StatusCode::BAD_REQUEST, Message::new("execution.logLineTooLarge")));
                        cancel.send_replace(true);buffer.clear();continue;
                    }
                    while let Some(end)=buffer.iter().position(|byte|*byte==b'\n') {
                        let line:Vec<_>=buffer.drain(..=end).collect();
                        lines.push_back((stream.clone(), line));
                    }
                }
            },
            result=processing.next(), if !processing.is_empty()=>{
                let (result, updated) = result.expect("pending event");
                outcome = updated;
                if let Err(error) = result {
                    failure=Some(error);cancel.send_replace(true);
                }
            },
            status=checks.next(), if !checks.is_empty()=>{
                match status.expect("pending status") {
                    Ok(false)=>{cancel.send_replace(true);},
                    Ok(true)=>{},
                    Err(error)=>{failure=Some(error);cancel.send_replace(true);},
                }
            },
            _=previews.next(), if !previews.is_empty()=>{},
            changed=stop.changed(), if stop_open=>{
                if changed.is_err() { stop_open=false; }
                if !stop_open || *stop.borrow() { cancel.send_replace(true); }
            }
        }
    }
    // A status query may hold the only pooled connection while awaiting its
    // next poll. Release it before flushing a trailing partial output line.
    drop(checks);
    drop(processing);
    drop(previews);
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
        *outcome = Some(serde_json::from_value(value).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("execution.resultInvalid"),
            )
        })?);
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
            .filter(crate::executions::pipeline_fence(run.id))
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
            .filter(crate::executions::pipeline_fence(run.id))
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

/// A worker invocation's side of `ferry/preview@v1`
/// (docs/preview-automation-design.md section 5).
pub(super) struct Preview<'a> {
    /// Known only to this invocation's rewritten preview steps.
    pub nonce: &'a str,
    /// `<job private dir>/ferry-preview`, the only place answers go.
    pub dir: &'a str,
    pub host_id: i64,
}

#[derive(Debug, Deserialize)]
struct PreviewRequest {
    id: String,
    port: i64,
    #[serde(default)]
    name: String,
    strip_prefix: bool,
    hold: String,
    ready_timeout_seconds: u64,
    timeout_minutes: i64,
    response: String,
}

/// The answer file for request `id`: exactly `<dir>/<id>.json`, with an id
/// that cannot name any other path.
pub(super) fn preview_path(dir: &str, id: &str) -> Option<String> {
    (!id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    .then(|| format!("{dir}/{id}.json"))
}

/// A preview request marker (`::ferry-preview::<nonce>::<json>`) in a worker
/// log record. Lines without the right nonce, with an unusable id, or naming
/// another answer path are not requests; they are logged like any output.
fn preview_request(line: &[u8], preview: &Preview<'_>) -> Option<PreviewRequest> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let text = value["raw"].as_str().or_else(|| value["msg"].as_str())?;
    let prefix = format!("::ferry-preview::{}::", preview.nonce);
    let start = text.find(&prefix)?;
    let request: PreviewRequest =
        serde_json::from_str(text[start + prefix.len()..].trim_end()).ok()?;
    (preview_path(preview.dir, &request.id)? == request.response).then_some(request)
}

/// Opens the requested preview as the run's initiator and waits until the
/// local service answers; returns the hole and its root-relative link.
async fn open_preview(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    host_id: i64,
    request: &PreviewRequest,
) -> Result<(i64, String)> {
    if !matches!(request.hold.as_str(), "approval" | "none")
        || !(1..=600).contains(&request.ready_timeout_seconds)
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("holes.invalid"),
        ));
    }
    let scope = crate::accounts::workspace::Scope::for_actor(
        s,
        Id::from(run.workspace_id),
        Id::from(run.triggered_by),
    )
    .await?;
    let row = crate::holes::open_owned(
        s,
        &scope,
        crate::holes::OwnedRequest {
            host_id: Id::from(host_id),
            port: request.port,
            name: request.name.clone(),
            strip_prefix: request.strip_prefix,
            source_kind: "pipeline",
            source_id: run.id,
            source_ref: job.key.clone(),
            review_minutes: (request.hold == "approval").then_some(request.timeout_minutes),
        },
    )
    .await?;
    crate::holes::wait_ready(s, &row, Duration::from_secs(request.ready_timeout_seconds)).await?;
    let id = row.id;
    let view = crate::holes::views(s, &scope, vec![row]).await?;
    let url = view
        .first()
        .and_then(|view| view["url"].as_str())
        .unwrap_or_default()
        .to_owned();
    Ok((id, url))
}

pub(super) async fn answer_preview(ch: &dyn HostChannel, path: &str, answer: Value) {
    if let Err(error) = upload(ch, path, answer.to_string(), 0o600).await {
        // The step gives up at its own deadline; the job's end revokes it.
        tracing::warn!(%error, "预览结果写回失败");
    }
}

/// Serves one request: the link (or the reason there is none), then, when
/// the step holds for review, the decision.
#[allow(clippy::too_many_arguments)]
async fn serve_preview(
    s: &AppState,
    run: &runs::Model,
    job: &Job,
    seq: &tokio::sync::Mutex<i64>,
    ch: &dyn HostChannel,
    preview: &Preview<'_>,
    request: PreviewRequest,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
) {
    let path = request.response.clone();
    match open_preview(s, run, job, preview.host_id, &request).await {
        Ok((hole, url)) => {
            answer_preview(ch, &path, json!({"url": url})).await;
            if request.hold == "approval" {
                match crate::holes::wait_review(s, hole).await {
                    Ok(decision) => {
                        answer_preview(ch, &path, json!({"url": url, "decision": decision})).await;
                    }
                    Err(error) => tracing::warn!(%error, "预览验收状态读取失败"),
                }
            }
        }
        Err(error) => {
            let text = error.render(crate::i18n::current_locale());
            let message = Message::new("execution.previewFailed").with("job", job.key.clone());
            let message = match error.2 {
                Some(cause) => message.with_message("error", cause),
                None => message.with("error", error.1),
            };
            if let Err(error) = append_message(
                s,
                run.id,
                -1,
                &mut *seq.lock().await,
                "stderr",
                redact_message(message, credential, variables),
            )
            .await
            {
                tracing::warn!(%error, "预览失败日志写入失败");
            }
            answer_preview(ch, &path, json!({"error": text})).await;
        }
    }
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

fn redact_message(
    mut message: Message,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
) -> Message {
    for value in message.params.values_mut() {
        *value = history::redact(std::mem::take(value), credential, variables);
    }
    for child in message.children.values_mut() {
        *child = redact_message(child.clone(), credential, variables);
    }
    message
}

pub(super) async fn append_failure(
    s: &AppState,
    run: i64,
    seq: &mut i64,
    job: &str,
    matrix: Option<&str>,
    error: ApiError,
    credential: Option<&(String, String, String)>,
    variables: &BTreeMap<String, String>,
) -> Result<()> {
    let key = match (matrix.is_some(), error.2.is_some()) {
        (false, true) => "execution.jobFailed",
        (true, true) => "execution.matrixFailed",
        (false, false) => "execution.jobFailedPrefix",
        (true, false) => "execution.matrixFailedPrefix",
    };
    let mut message = Message::new(key).with("job", job);
    if let Some(name) = matrix {
        message = message.with("name", name);
    }
    if let Some(cause) = error.2 {
        message = message.with_message("error", cause);
        append_message(
            s,
            run,
            -1,
            seq,
            "stderr",
            redact_message(message, credential, variables),
        )
        .await
    } else {
        append_message(
            s,
            run,
            -1,
            seq,
            "stderr",
            redact_message(message, credential, variables),
        )
        .await?;
        // Provider output may be arbitrarily large; keep it in raw UTF-8 chunks.
        append(
            s,
            run,
            -1,
            seq,
            "stderr",
            &history::redact(format!("{}\n", error.1), credential, variables),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn blocked_log_pool_cannot_prevent_remote_cancel_acknowledgement() {
        use crate::pipelines::runs::lease_tests::{BlockedCommand, fixture, run};
        let (_dir, s, _) = fixture(1).await;
        let channel = BlockedCommand::new(&s);
        let (cancel, mut stopped) = tokio::sync::watch::channel(false);
        let job = super::super::tests::test_job("build", &[]);
        let run = run(7);
        let seq = tokio::sync::Mutex::new(0);
        let variables = BTreeMap::new();
        let controller = async {
            channel.started.notified().await;
            // Let the emitted line reach its SQL insert before signalling stop.
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.send_replace(true);
        };
        let stream = stream_events(
            &s,
            &run,
            &job,
            &seq,
            None,
            &variables,
            &mut stopped,
            &channel,
            "command",
            60,
            false,
            true,
            None,
        );
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(stream, controller)
        })
        .await
        .expect("SQL persistence must not stop polling remote cancellation");
        let (code, _, cancelled) = result.unwrap();
        assert_eq!(code, -130);
        assert!(cancelled);
        assert!(
            channel
                .acknowledged
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert_eq!(*seq.lock().await, 2);
    }

    #[tokio::test]
    async fn failed_log_write_with_queued_lines_still_waits_for_remote_cleanup_and_returns() {
        use crate::pipelines::runs::lease_tests::{BlockedCommand, fixture, run};
        let (_dir, s, _) = fixture(1).await;
        use sea_orm::ConnectionTrait;
        s.db.execute_unprepared("DROP TABLE pipeline_run_logs")
            .await
            .unwrap();
        let channel = BlockedCommand::new(&s);
        let (cancel, mut stopped) = tokio::sync::watch::channel(false);
        let job = super::super::tests::test_job("build", &[]);
        let run = run(7);
        let seq = tokio::sync::Mutex::new(0);
        let variables = BTreeMap::new();
        let controller = async {
            channel.started.notified().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.send_replace(true);
        };
        let stream = stream_events(
            &s,
            &run,
            &job,
            &seq,
            None,
            &variables,
            &mut stopped,
            &channel,
            "command",
            60,
            false,
            true,
            None,
        );
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(stream, controller)
        })
        .await
        .expect("a failed queued line must not leave an idle select after remote exit");
        assert!(result.is_err());
        assert!(
            channel
                .acknowledged
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert_eq!(*seq.lock().await, 0);
    }

    #[tokio::test]
    async fn execution_logs_preserve_raw_collisions_large_causes_and_reader_locale() {
        use crate::{
            entities::pipeline_run_logs as logs,
            i18n::{self, Locale},
        };
        use sea_orm::{ConnectionTrait, Schema, Set};
        let dir = tempfile::tempdir().unwrap();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(logs::Entity))
            .await
            .unwrap();
        let s = std::sync::Arc::new(crate::state::State {
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "execution-log-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
        });
        let legacy = r#"{"$system_message":{"key":"execution.historicalRun","params":{}}}"#;
        logs::Entity::insert(logs::ActiveModel {
            id: Set(1),
            run_id: Set(7),
            seq: Set(1),
            step_index: Set(3),
            stream: Set("stdout".into()),
            chunk: Set(legacy.into()),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let mut seq = 1;
        let raw = format!(
            "{legacy}\n{}\n{}",
            i18n::StoredText::from_message(Message::new("execution.historicalRun")).stored(),
            "界".repeat(6000)
        );
        append(&s, 7, 3, &mut seq, "stdout", &raw).await.unwrap();
        let raw_end = seq;
        let nested = Message::new("execution.templateUnsupported")
            .with("token", "{{secret}}")
            .with_message(
                "context",
                Message::new("execution.stepContext")
                    .with("index", "1")
                    .with("name", "secret"),
            );
        let variables = BTreeMap::from([("TOKEN".into(), "secret".into())]);
        append_failure(
            &s,
            7,
            &mut seq,
            "build",
            None,
            ApiError::localized(StatusCode::BAD_REQUEST, nested),
            None,
            &variables,
        )
        .await
        .unwrap();
        let translated_seq = seq;
        let large = format!("{}secret", "原始 {error}\n".repeat(40000));
        append_failure(
            &s,
            7,
            &mut seq,
            "build",
            Some("linux"),
            ApiError::raw(StatusCode::BAD_REQUEST, large.clone()),
            None,
            &variables,
        )
        .await
        .unwrap();
        let rows = logs::Entity::find()
            .filter(logs::Column::RunId.eq(7))
            .order_by_asc(logs::Column::Seq)
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(rows.len() as i64, seq);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.seq, index as i64 + 1);
            assert_eq!(row.step_index, if row.seq <= raw_end { 3 } else { -1 });
            assert_eq!(
                row.stream,
                if row.seq <= raw_end {
                    "stdout"
                } else {
                    "stderr"
                }
            );
        }
        for locale in Locale::ALL {
            assert_eq!(i18n::render_text_stored(locale, &rows[0].chunk), legacy);
            let restored: String = rows
                .iter()
                .filter(|row| row.seq > 1 && row.seq <= raw_end)
                .map(|row| {
                    let raw = i18n::render_text_stored(locale, &row.chunk);
                    assert!(raw.len() <= 16384);
                    raw
                })
                .collect();
            assert_eq!(restored, raw);
            let localized = &rows[(translated_seq - 1) as usize].chunk;
            assert!(localized.contains("execution.templateUnsupported"));
            let displayed = i18n::render_text_stored(locale, localized);
            assert!(displayed.contains("[REDACTED]"));
            assert!(!displayed.contains("secret"));
            let cause: String = rows
                .iter()
                .filter(|row| row.seq > translated_seq + 1)
                .map(|row| {
                    let raw = i18n::render_text_stored(locale, &row.chunk);
                    assert!(raw.len() <= 16384);
                    raw
                })
                .collect();
            assert_eq!(
                cause,
                format!("{}\n", large.replace("secret", "[REDACTED]"))
            );
        }
        let localized = &rows[(translated_seq - 1) as usize].chunk;
        assert_ne!(
            i18n::render_text_stored(Locale::EnUs, localized),
            i18n::render_text_stored(Locale::JaJp, localized)
        );
    }

    /// Only a marker with this invocation's nonce, a plain id and exactly the
    /// expected answer path is a preview request.
    #[test]
    fn preview_markers_need_the_nonce_and_their_own_answer_path() {
        use super::{Preview, preview_path, preview_request};
        let preview = Preview {
            nonce: "n0nce",
            dir: "/tmp/actions-1/ferry-preview",
            host_id: 11,
        };
        let request = |id: &str, response: &str| {
            json!({"id": id, "port": 5173, "name": "Web", "strip_prefix": true, "hold": "approval",
                   "ready_timeout_seconds": 60, "timeout_minutes": 30, "response": response})
            .to_string()
        };
        // As act logs an unknown workflow command: the line is in `raw`, and
        // `msg` carries it behind a prefix.
        let record = |nonce: &str, body: &str| {
            let line = format!("::ferry-preview::{nonce}::{body}\n");
            json!({"level": "info", "stepID": ["preview"], "command": "ferry-preview",
                   "msg": format!("  \u{2753}  {line}"), "raw": line})
            .to_string()
            .into_bytes()
        };
        let good = request(
            "preview-0a1b",
            "/tmp/actions-1/ferry-preview/preview-0a1b.json",
        );
        let parsed = preview_request(&record("n0nce", &good), &preview).unwrap();
        assert_eq!(parsed.id, "preview-0a1b");
        assert_eq!(parsed.port, 5173);
        assert_eq!(parsed.name, "Web");
        assert!(parsed.strip_prefix);
        assert_eq!(parsed.hold, "approval");
        assert_eq!(parsed.ready_timeout_seconds, 60);
        assert_eq!(parsed.timeout_minutes, 30);
        // Without `raw`, the message still identifies the request.
        let mut only_msg: Value = serde_json::from_slice(&record("n0nce", &good)).unwrap();
        only_msg.as_object_mut().unwrap().remove("raw");
        assert!(preview_request(only_msg.to_string().as_bytes(), &preview).is_some());

        // Forgeries and anything that could name another file are not requests.
        assert!(preview_request(&record("guess", &good), &preview).is_none());
        assert!(preview_request(&record("", &good), &preview).is_none());
        assert!(
            preview_request(good.as_bytes(), &preview).is_none(),
            "not a log record"
        );
        assert!(preview_request(b"::ferry-preview::n0nce::{}", &preview).is_none());
        assert!(preview_request(&record("n0nce", "{not json"), &preview).is_none());
        for (id, response) in [
            ("../escape", "/tmp/actions-1/ferry-preview/../escape.json"),
            ("a/b", "/tmp/actions-1/ferry-preview/a/b.json"),
            ("Upper", "/tmp/actions-1/ferry-preview/Upper.json"),
            ("", "/tmp/actions-1/ferry-preview/.json"),
            ("dot.dot", "/tmp/actions-1/ferry-preview/dot.dot.json"),
            (
                "back\\slash",
                "/tmp/actions-1/ferry-preview/back\\slash.json",
            ),
            ("preview-0a1b", "/etc/passwd"),
            (
                "preview-0a1b",
                "/tmp/actions-2/ferry-preview/preview-0a1b.json",
            ),
            (
                "preview-0a1b",
                "/tmp/actions-1/ferry-preview/../preview-0a1b.json",
            ),
        ] {
            assert!(
                preview_request(&record("n0nce", &request(id, response)), &preview).is_none(),
                "{id} -> {response}"
            );
        }

        assert_eq!(
            preview_path("C:/Temp/ferry-actions-1/ferry-preview", "preview-9"),
            Some("C:/Temp/ferry-actions-1/ferry-preview/preview-9.json".into())
        );
        for id in [
            "",
            "..",
            "../x",
            "x/y",
            "x\\y",
            "X",
            "a.b",
            "a b",
            "\u{e9}",
            &"a".repeat(65),
        ] {
            assert_eq!(preview_path("/d", id), None, "{id}");
        }
        assert!(preview_path("/d", &"a".repeat(64)).is_some());
    }

    /// A worker that asks for a preview, then exits once it has an answer.
    struct PreviewWorker {
        lines: Vec<u8>,
        answered: tokio::sync::Notify,
        written: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl HostChannel for PreviewWorker {
        async fn exec_stream(
            &self,
            _: &str,
            _: Duration,
            output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
            _: tokio::sync::watch::Receiver<bool>,
        ) -> Result<i32> {
            output
                .send(("stdout".into(), self.lines.clone()))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(20), self.answered.notified())
                .await
                .expect("the service answers the request");
            Ok(0)
        }
        async fn write_file(
            &self,
            path: &str,
            _: u32,
            body: crate::hosts::channel::ByteStream,
        ) -> Result<()> {
            let body: Vec<_> = body.collect().await;
            let body = body
                .into_iter()
                .map(|chunk| String::from_utf8(chunk.unwrap().to_vec()).unwrap())
                .collect::<String>();
            self.written.lock().unwrap().push((path.into(), body));
            self.answered.notify_one();
            Ok(())
        }
        async fn read_file(
            &self,
            _: &str,
        ) -> Result<(
            crate::hosts::channel::FileMeta,
            crate::hosts::channel::ByteStream,
        )> {
            panic!("unexpected read")
        }
        async fn monitor(&self) -> Result<ferry_protocol::HostMonitor> {
            panic!("unexpected monitor")
        }
        async fn exec(&self, _: &str, _: Duration) -> Result<crate::hosts::channel::Output> {
            panic!("unexpected exec")
        }
        async fn open_pty(
            &self,
            _: crate::hosts::channel::PtySize,
        ) -> Result<Box<dyn crate::hosts::channel::Pty>> {
            panic!("unexpected pty")
        }
        async fn list_dir(&self, _: &str) -> Result<Vec<crate::hosts::channel::DirEntry>> {
            panic!("unexpected list")
        }
        async fn create_file(&self, _: &str) -> Result<()> {
            panic!("unexpected create file")
        }
        async fn create_dir(&self, _: &str) -> Result<()> {
            panic!("unexpected create directory")
        }
        async fn delete_file(&self, _: &str) -> Result<()> {
            panic!("unexpected delete")
        }
    }

    /// The marker opens a preview as the run's initiator; a service that never
    /// comes up gets the step a localized error, in exactly its answer file,
    /// and the run log says why. A forged marker is just output.
    #[tokio::test]
    async fn preview_requests_are_answered_in_their_own_file() {
        use crate::entities::{holes, hosts, pipeline_run_logs as logs};
        use sea_orm::{ActiveValue::Set, ConnectionTrait};
        let (_dir, s) = crate::agents::tests::setup().await;
        s.db.execute_unprepared("UPDATE users SET is_superadmin = 1 WHERE id = 1")
            .await
            .unwrap();
        hosts::Entity::insert(hosts::ActiveModel {
            id: Set(11),
            workspace_id: Set(1),
            name: Set("Runner".into()),
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
            seen_at: Set(0),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            os: Set(String::new()),
            arch: Set(String::new()),
            agent_version: Set(String::new()),
            metrics: Set(String::new()),
            credential_id: Set(0),
            ..Default::default()
        })
        .exec(&s.db)
        .await
        .unwrap();
        let mut run = crate::pipelines::runs::lease_tests::run(70);
        run.workspace_id = 1;
        let job = super::super::tests::test_job("build", &[]);
        let dir = "/tmp/actions-1/ferry-preview";
        let marker = |nonce: &str| {
            let line = format!(
                "::ferry-preview::{nonce}::{}\n",
                json!({"id": "preview-1", "port": 5173, "name": "Web", "strip_prefix": true,
                       "hold": "none", "ready_timeout_seconds": 1, "timeout_minutes": 60,
                       "response": format!("{dir}/preview-1.json")})
            );
            format!(
                "{}\n",
                json!({"level": "info", "msg": format!("  \u{2753}  {line}"), "raw": line})
            )
        };
        let channel = PreviewWorker {
            lines: format!("{}{}", marker("forged"), marker("n0nce")).into_bytes(),
            answered: Default::default(),
            written: Default::default(),
        };
        let seq = tokio::sync::Mutex::new(0);
        let variables = BTreeMap::new();
        let (_cancel, mut stopped) = tokio::sync::watch::channel(false);
        let (code, _, cancelled) = stream_events(
            &s,
            &run,
            &job,
            &seq,
            None,
            &variables,
            &mut stopped,
            &channel,
            "worker",
            60,
            true,
            false,
            Some(&Preview {
                nonce: "n0nce",
                dir,
                host_id: 11,
            }),
        )
        .await
        .unwrap();
        assert_eq!((code, cancelled), (0, false));

        let written = channel.written.lock().unwrap().clone();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].0, "/tmp/actions-1/ferry-preview/preview-1.json");
        let answer: Value = serde_json::from_str(&written[0].1).unwrap();
        assert!(
            answer["error"]
                .as_str()
                .unwrap()
                .contains("did not become reachable"),
            "{answer}"
        );
        assert!(answer.get("url").is_none());

        // Opened as the run's initiator for this job, then revoked.
        let rows = holes::Entity::find().all(&s.db).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (
                rows[0].source_kind.as_str(),
                rows[0].source_id,
                rows[0].source_ref.as_str()
            ),
            ("pipeline", 70, "build")
        );
        assert_eq!((rows[0].host_id, rows[0].target_port), (11, 5173));
        assert_eq!(rows[0].name, "Web");
        assert!(rows[0].revoked_at > 0);

        let chunks = logs::Entity::find()
            .filter(logs::Column::RunId.eq(70))
            .order_by_asc(logs::Column::Seq)
            .all(&s.db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.chunk)
            .collect::<Vec<_>>();
        assert_eq!(chunks.len(), 2, "{chunks:?}");
        assert!(
            chunks[0].contains("::ferry-preview::forged::"),
            "forgery is plain output"
        );
        assert!(
            chunks.iter().all(|chunk| !chunk.contains("n0nce")),
            "the nonce is never logged"
        );
        assert!(chunks[1].contains("execution.previewFailed"));
        let shown = crate::i18n::render_text_stored(crate::i18n::Locale::ZhCn, &chunks[1]);
        assert!(shown.contains("任务 build 的预览失败"), "{shown}");
    }

    #[test]
    fn step_exit_codes_come_from_engine_error_lines() {
        use super::step_exit_code;
        assert_eq!(step_exit_code("exit status 7"), Some(7));
        assert_eq!(step_exit_code("exitcode '2': failure"), Some(2));
        assert_eq!(step_exit_code("exit status seven"), None);
        assert_eq!(step_exit_code("Error: exit status 7 happened"), None);
    }
}
