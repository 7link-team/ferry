//! Only data crosses this boundary. QuickJS is never linked into the server.
use crate::{
    error::{ApiError, Result},
    i18n::Message,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::{Batch, ErrorCode, Evaluation, Reply, VERSION};
use serde_json::json;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// Whether a `=` template uses the current item (`$json`, `$binary`,
/// `$input.item`) inside its `{{ }}` code. Literal template text is ignored.
pub(super) fn current_item_reference(template: &str) -> bool {
    let Some(mut rest) = template.strip_prefix('=') else {
        return false;
    };
    while let Some(start) = rest.find("{{") {
        let code = &rest[start + 2..];
        let end = code.find("}}").unwrap_or(code.len());
        if code_reference(&code[..end]) {
            return true;
        }
        rest = &code[(end + 2).min(code.len())..];
    }
    false
}

/// Detect item-scoped JavaScript bindings without matching quoted text or comments.
fn code_reference(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'`' {
                        i += 1;
                        break;
                    } else if bytes.get(i..i + 2) == Some(b"${") {
                        let start = i + 2;
                        let mut end = start;
                        let mut depth = 1;
                        while end < bytes.len() && depth > 0 {
                            match bytes[end] {
                                b'{' => depth += 1,
                                b'}' => depth -= 1,
                                _ => {}
                            }
                            end += 1;
                        }
                        if code_reference(&source[start..end.saturating_sub(1)]) {
                            return true;
                        }
                        i = end;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
            b'$' if i == 0 || !identifier_byte(bytes[i - 1]) => {
                let previous = bytes[..i].iter().rev().find(|b| !b.is_ascii_whitespace());
                if matches!(previous, Some(b'.')) {
                    i += 1;
                    continue;
                }
                let tail = &bytes[i..];
                for name in [b"$json".as_slice(), b"$binary".as_slice()] {
                    if tail.starts_with(name)
                        && !tail.get(name.len()).is_some_and(|b| identifier_byte(*b))
                    {
                        return true;
                    }
                }
                if tail.starts_with(b"$input") && !tail.get(6).is_some_and(|b| identifier_byte(*b))
                {
                    let mut j = i + 6;
                    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'?') {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'.') {
                        j += 1;
                        while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                            j += 1;
                        }
                        if bytes.get(j..j + 4) == Some(b"item")
                            && !bytes.get(j + 4).is_some_and(|b| identifier_byte(*b))
                        {
                            return true;
                        }
                    } else if bytes.get(j) == Some(&b'[') {
                        j += 1;
                        while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                            j += 1;
                        }
                        if matches!(bytes.get(j), Some(b'\'' | b'"')) {
                            let quote = bytes[j];
                            if bytes.get(j + 1..j + 5) == Some(b"item")
                                && bytes.get(j + 5) == Some(&quote)
                            {
                                return true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod current_item_tests {
    use super::current_item_reference;

    #[test]
    fn identifies_item_bindings_only_in_executable_source() {
        for source in [
            "={{$json.value}}",
            "={{ $binary.file }}",
            "={{ $input.item.json.x }}",
            "={{ $input['item'].json.x }}",
            "={{ `value: ${$json.value}` }}",
        ] {
            assert!(current_item_reference(source), "{source}");
        }
        for source in [
            "={{ $input.all().map(i => i.json) }}",
            "={{ $input.first().json }}",
            "={{ $('Node').all() }}",
            "={{ '$json' }}",
            "={{ `literal $json` }}",
            "={{ obj.$json }}",
            "={{ /* $json */ $input.all() }}",
            "=Summarize $json fields {{ $input.all().length }}",
            "Summarize $json without template",
        ] {
            assert!(!current_item_reference(source), "{source}");
        }
    }
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

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
    clock_with_timezone(settings, None, at_ms)
}

/// A confirmed workspace zone is captured by the execution, never looked up on replay.
pub(crate) fn clock_with_timezone(
    settings: &serde_json::Value,
    workspace_timezone: Option<&str>,
    at_ms: i64,
) -> Result<(serde_json::Value, serde_json::Value)> {
    use chrono::{DateTime, Local, Utc};
    let override_settings = workspace_timezone.map(|zone| json!({"timezone":zone}));
    let settings = override_settings.as_ref().unwrap_or(settings);
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

pub(crate) async fn exchange(mut batch: Batch) -> Reply {
    batch.timeout_ms = batch.resource_policy.sandbox_timeout_ms;
    batch.memory_bytes = batch
        .resource_policy
        .sandbox_memory_bytes
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX));
    let packet_limit = batch.resource_policy.workflow_packet_bytes;
    let failure = |code| Reply::error(code, None);
    if !batch.valid() {
        return failure(ErrorCode::InvalidRequest);
    }
    let bytes = match serde_json::to_vec(&batch) {
        Ok(bytes) if ferry_protocol::resources::allows(packet_limit, bytes.len()) => bytes,
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
                    .take(
                        packet_limit
                            .map(|limit| limit.saturating_add(1))
                            .unwrap_or(u64::MAX),
                    )
                    .read_to_end(&mut bytes)
                    .await?;
                Ok::<_, std::io::Error>(bytes)
            },
            child.wait()
        )
    };
    let result = match batch.timeout_ms {
        Some(ms) => tokio::time::timeout(Duration::from_millis(ms.saturating_add(5_000)), io)
            .await
            .map_err(|_| ()),
        None => Ok(io.await),
    };
    match result {
        Ok(Ok(((), bytes, status))) if status.success() => {
            if !ferry_protocol::resources::allows(packet_limit, bytes.len()) {
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

pub(crate) async fn syntax(
    expressions: Vec<(String, String)>,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<()> {
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
        resource_policy: policy.clone(),
        version: VERSION,
        timeout_ms: None,
        memory_bytes: None,
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
    fn confirmed_workspace_clock_overrides_definition_and_legacy_keeps_it() {
        let settings = json!({"timezone":"UTC"});
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-01T16:30:12.345Z")
            .unwrap()
            .timestamp_millis();
        let (now, today) = clock_with_timezone(&settings, Some("Asia/Kuala_Lumpur"), at).unwrap();
        assert_eq!(now["iso"], "2026-11-02T00:30:12.345+08:00");
        assert_eq!(today["iso"], "2026-11-02T00:00:00.000+08:00");
        assert_eq!(now["zone"], "Asia/Kuala_Lumpur");
        assert_eq!(
            clock_with_timezone(&settings, None, at).unwrap(),
            clock(&settings, at).unwrap()
        );
        assert!(clock_with_timezone(&settings, Some("Mars/Unknown"), at).is_err());
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
