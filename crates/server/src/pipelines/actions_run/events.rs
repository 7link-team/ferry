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
                        failure=Some(ApiError::localized(StatusCode::BAD_REQUEST, Message::new("execution.logLineTooLarge")));
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

    #[test]
    fn step_exit_codes_come_from_engine_error_lines() {
        use super::step_exit_code;
        assert_eq!(step_exit_code("exit status 7"), Some(7));
        assert_eq!(step_exit_code("exitcode '2': failure"), Some(2));
        assert_eq!(step_exit_code("exit status seven"), None);
        assert_eq!(step_exit_code("Error: exit status 7 happened"), None);
    }
}
