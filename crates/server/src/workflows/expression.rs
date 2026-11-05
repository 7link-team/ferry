//! Only data crosses this boundary. QuickJS is never linked into the server.
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::{
    Batch, ErrorCode, Evaluation, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Reply, VERSION,
};
use serde_json::json;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub(crate) fn timezone(settings: &serde_json::Value, path: &str) -> Result<Option<chrono_tz::Tz>> {
    settings
        .get("timezone")
        .map(|value| {
            value
                .as_str()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        Message::new("workflow.timezoneInvalid").with("path", path.to_owned()),
                    )
                })
        })
        .transpose()
}

/// The caller freezes this instant for its attempt and persists it for recovery.
/// Preview freezes one instant per request. The sandbox never reads a clock.
pub(crate) fn clock(
    settings: &serde_json::Value,
    at_ms: i64,
) -> Result<(serde_json::Value, serde_json::Value)> {
    use chrono::{DateTime, Local, Utc};
    let invalid = || error(ErrorCode::Evaluation, "$now");
    let instant = DateTime::<Utc>::from_timestamp_millis(at_ms).ok_or_else(invalid)?;
    match timezone(settings, "/settings/timezone")? {
        Some(zone) => clock_in(instant, zone, zone.name()).ok_or_else(invalid),
        None => clock_in(instant, Local, "local").ok_or_else(invalid),
    }
}

fn clock_in<T: chrono::TimeZone>(
    instant: chrono::DateTime<chrono::Utc>,
    zone: T,
    name: &str,
) -> Option<(serde_json::Value, serde_json::Value)> {
    use chrono::{Duration, SecondsFormat};
    let now = instant.with_timezone(&zone);
    let date = now.date_naive();
    let midnight = date.and_hms_opt(0, 0, 0)?;
    // Some IANA zones skip or repeat midnight. Earliest picks the first instant
    // of a repeated midnight; scanning a gap also handles historical seconds.
    let today = (0..86400).find_map(|second| {
        zone.from_local_datetime(&midnight.checked_add_signed(Duration::seconds(second))?)
            .earliest()
    })?;
    let encode = |time: chrono::DateTime<T>| {
        json!({
            "iso":time.fixed_offset().to_rfc3339_opts(SecondsFormat::Millis, true),"zone":name
        })
    };
    Some((encode(now), encode(today)))
}

fn executable() -> Option<PathBuf> {
    match std::env::var_os("FERRY_SANDBOX_PATH") {
        Some(path) => std::path::absolute(PathBuf::from(path)).ok(),
        None => std::env::current_exe().ok().map(|path| {
            path.with_file_name(if cfg!(windows) {
                "ferry-sandbox.exe"
            } else {
                "ferry-sandbox"
            })
        }),
    }
}

pub(crate) async fn exchange(batch: Batch) -> Reply {
    let failure = |code| Reply::error(code, None);
    if !batch.valid() {
        return failure(ErrorCode::InvalidRequest);
    }
    let bytes = match serde_json::to_vec(&batch) {
        Ok(bytes) if bytes.len() <= MAX_REQUEST_BYTES => bytes,
        _ => return failure(ErrorCode::InvalidRequest),
    };
    let Some(path) = executable() else {
        return failure(ErrorCode::Isolation);
    };
    let mut command = Command::new(&path);
    command
        .env_clear()
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    {
        // Windows process creation needs these system locations. Never forward
        // PATH, database URLs, vault keys or arbitrary business environment.
        for key in [
            "SystemRoot",
            "windir",
            "SystemDrive",
            "ProgramData",
            "ALLUSERSPROFILE",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "CommonProgramFiles",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return failure(ErrorCode::Isolation),
    };
    let mut input = child.stdin.take().expect("piped sandbox input");
    let output = child.stdout.take().expect("piped sandbox output");
    let expected = if batch.syntax_only {
        0
    } else {
        batch.evaluations.len()
    };
    let count = batch.evaluations.len();
    let io = async {
        tokio::try_join!(
            async move {
                input.write_all(&bytes).await?;
                input.shutdown().await?;
                drop(input);
                Ok::<_, std::io::Error>(())
            },
            async move {
                let mut bytes = Vec::new();
                output
                    .take((MAX_RESPONSE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .await?;
                Ok::<_, std::io::Error>(bytes)
            },
            child.wait()
        )
    };
    let result = tokio::time::timeout(Duration::from_millis(batch.timeout_ms + 5_000), io).await;
    match result {
        Ok(Ok(((), bytes, status))) if status.success() => {
            if bytes.len() > MAX_RESPONSE_BYTES {
                return failure(ErrorCode::ResultLimit);
            }
            let reply = match serde_json::from_slice::<Reply>(&bytes) {
                Ok(reply) => reply,
                Err(_) => return failure(ErrorCode::Internal),
            };
            if valid_reply(&reply, expected, count) {
                reply
            } else {
                failure(ErrorCode::Internal)
            }
        }
        result => {
            // Await termination before reporting timeout or a broken pipe. A
            // dropped request also kills the launcher (job/PDEATHSIG kills JS).
            let _ = child.kill().await;
            let _ = child.wait().await;
            failure(if result.is_err() {
                ErrorCode::Timeout
            } else {
                ErrorCode::Isolation
            })
        }
    }
}

fn valid_reply(reply: &Reply, expected: usize, count: usize) -> bool {
    match reply {
        Reply::Ok { version, values } => *version == VERSION && values.len() == expected,
        Reply::Error { version, index, .. } => {
            *version == VERSION && index.is_none_or(|index| index < count)
        }
    }
}

pub(crate) fn error(code: ErrorCode, path: &str) -> ApiError {
    let (status, key) = match code {
        ErrorCode::Syntax => (StatusCode::BAD_REQUEST, "workflow.expressionSyntax"),
        ErrorCode::Evaluation => (StatusCode::BAD_REQUEST, "workflow.expressionFailed"),
        ErrorCode::AmbiguousItem => (StatusCode::BAD_REQUEST, "workflow.expressionAmbiguousItem"),
        ErrorCode::MissingItem => (StatusCode::BAD_REQUEST, "workflow.expressionMissingItem"),
        ErrorCode::InvalidRequest | ErrorCode::ResultLimit => {
            (StatusCode::BAD_REQUEST, "workflow.expressionLimit")
        }
        ErrorCode::Timeout | ErrorCode::Memory => {
            (StatusCode::TOO_MANY_REQUESTS, "workflow.expressionLimit")
        }
        ErrorCode::Isolation | ErrorCode::Internal => (
            StatusCode::SERVICE_UNAVAILABLE,
            "workflow.sandboxUnavailable",
        ),
    };
    ApiError::localized(status, Message::new(key).with("path", path.to_owned()))
}

pub(crate) async fn syntax(expressions: Vec<(String, String)>) -> Result<()> {
    if expressions.is_empty() {
        return Ok(());
    }
    let evaluations = expressions
        .iter()
        .map(|(_, expression)| Evaluation {
            expression: expression.clone(),
            template: true,
            capture_presence: false,
            item_index: None,
            context: json!({}),
        })
        .collect();
    let reply = exchange(Batch {
        program: None,
        version: VERSION,
        timeout_ms: 1000,
        memory_bytes: 64 * 1024 * 1024,
        deterministic: true,
        syntax_only: true,
        data: None,
        context: None,
        evaluations,
    })
    .await;
    match reply {
        Reply::Ok { .. } => Ok(()),
        Reply::Error { code, index, .. } => Err(error(
            code,
            index
                .and_then(|i| expressions.get(i))
                .map(|(path, _)| path.as_str())
                .unwrap_or("/"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_clock_uses_workflow_timezone_and_first_valid_day_instant() {
        for (zone, at, now, today) in [
            (
                "Asia/Kuala_Lumpur",
                "2026-11-01T16:30:12.345Z",
                "2026-11-02T00:30:12.345+08:00",
                "2026-11-02T00:00:00.000+08:00",
            ),
            (
                "UTC",
                "2026-11-01T16:30:12.345Z",
                "2026-11-01T16:30:12.345Z",
                "2026-11-01T00:00:00.000Z",
            ),
            (
                "America/New_York",
                "2026-03-08T07:30:00Z",
                "2026-03-08T03:30:00.000-04:00",
                "2026-03-08T00:00:00.000-05:00",
            ),
            (
                "America/New_York",
                "2026-11-01T06:30:00Z",
                "2026-11-01T01:30:00.000-05:00",
                "2026-11-01T00:00:00.000-04:00",
            ),
            (
                "America/Sao_Paulo",
                "2018-11-04T04:30:00Z",
                "2018-11-04T02:30:00.000-02:00",
                "2018-11-04T01:00:00.000-02:00",
            ),
            (
                "America/Havana",
                "2020-11-01T06:30:00Z",
                "2020-11-01T01:30:00.000-05:00",
                "2020-11-01T00:00:00.000-04:00",
            ),
        ] {
            let at = chrono::DateTime::parse_from_rfc3339(at)
                .unwrap()
                .timestamp_millis();
            let (actual_now, actual_today) = clock(&json!({"timezone":zone}), at).unwrap();
            assert_eq!(actual_now, json!({"iso":now,"zone":zone}));
            assert_eq!(actual_today, json!({"iso":today,"zone":zone}));
        }
        assert!(
            timezone(&json!({}), "/settings/timezone")
                .unwrap()
                .is_none()
        );
        for value in [json!(null), json!(42), json!("private-invalid-zone")] {
            assert!(timezone(&json!({"timezone":value}), "/settings/timezone").is_err());
        }
        assert!(clock(&json!({"timezone":"UTC"}), i64::MAX).is_err());
        let (now, today) = clock(&json!({}), 0).unwrap();
        assert_eq!(now["zone"], "local");
        assert_eq!(today["zone"], "local");
    }
    #[test]
    fn corrupt_worker_replies_cannot_supply_another_protocol_or_partial_batch() {
        assert!(valid_reply(
            &Reply::Ok {
                version: VERSION,
                values: vec![json!(null)]
            },
            1,
            1
        ));
        assert!(!valid_reply(
            &Reply::Ok {
                version: 2,
                values: vec![json!(null)]
            },
            1,
            1
        ));
        assert!(!valid_reply(
            &Reply::Ok {
                version: VERSION,
                values: vec![]
            },
            1,
            1
        ));
        assert!(!valid_reply(
            &Reply::error(ErrorCode::Syntax, Some(1)),
            0,
            1
        ));
        assert!(valid_reply(&Reply::error(ErrorCode::Syntax, Some(0)), 0, 1));
    }
}
