//! Structured native CLI execution. The caller owns authorization, persistence,
//! directory exclusion and host/user/native-session binding; no shell is used.
mod probe;
mod protocol;
pub(super) use probe::probe;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
use ferry_protocol::agent::{Event, Input, Outcome, Permission, Request, Runtime};
use serde_json::Value;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, watch},
};

const LINE_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

/// A stable, isolated directory under this Runner instance's private state.
/// Keys are server-issued conversation/Agent IDs, never filesystem paths.
pub(super) async fn workspace(
    state_dir: &std::path::Path,
    key: &str,
) -> Result<std::path::PathBuf> {
    let valid = key.split_once('-').is_some_and(|(conversation, agent)| {
        [conversation, agent].into_iter().all(|value| {
            !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<u64>().is_ok_and(|id| id > 0)
        })
    });
    ensure!(key.len() <= 64 && valid, "invalid managed workspace key");
    let base = tokio::fs::canonicalize(state_dir).await?;
    let mut path = base.clone();
    for segment in ["workspaces", "conversations", key] {
        path.push(segment);
        match tokio::fs::create_dir(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "managed workspace is not a plain directory"
        );
        path = tokio::fs::canonicalize(&path).await?;
        ensure!(
            path.starts_with(&base),
            "managed workspace escaped Runner state"
        );
    }
    Ok(path)
}

fn validate(request: &Request) -> Result<()> {
    ensure!(
        std::path::Path::new(&request.cwd).is_absolute(),
        "Agent workdir must be absolute"
    );
    ensure!(
        !request.prompt.trim().is_empty() && request.prompt.len() <= 64 * 1024,
        "invalid Agent prompt"
    );
    ensure!(
        (1..=3600).contains(&request.timeout_seconds),
        "invalid Agent timeout"
    );
    for value in [&request.model, &request.native_session_id]
        .into_iter()
        .flatten()
    {
        ensure!(
            !value.is_empty()
                && value.len() <= 256
                && !value.starts_with('-')
                && !value.chars().any(char::is_control),
            "invalid native model or session id"
        );
    }
    Ok(())
}

fn command(request: &Request) -> Command {
    let executable = probe::executable(request.runtime);
    let command = match request.runtime {
        Runtime::Codex => {
            let mut c = native_command(
                executable
                    .as_deref()
                    .unwrap_or(std::path::Path::new("codex")),
            );
            c.arg("app-server");
            c
        }
        Runtime::Claude => {
            let mut c = native_command(
                executable
                    .as_deref()
                    .unwrap_or(std::path::Path::new("claude")),
            );
            c.args([
                "--print",
                "--verbose",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--permission-prompt-tool",
                "stdio",
                "--permission-mode",
            ]);
            c.arg(if request.permission == Permission::ReadOnly {
                "dontAsk"
            } else {
                "default"
            });
            // Claude tool filtering is not an OS sandbox. Read-only disables all
            // tools; on-request has no pre-approved tools or session-wide grant.
            c.args([
                "--tools",
                if request.permission == Permission::ReadOnly {
                    ""
                } else {
                    "Read,Glob,Grep,Edit,Write,Bash"
                },
                "--allowedTools",
                "",
            ]);
            if let Some(model) = &request.model {
                c.arg("--model").arg(model);
            }
            if let Some(id) = &request.native_session_id {
                c.arg("--resume").arg(id);
            }
            c
        }
    };
    command
}

fn native_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    // Keep native user login/config discovery, exclude the platform service's
    // enrollment tokens, database credentials and arbitrary inherited secrets.
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        if matches!(
            key.to_string_lossy().to_ascii_uppercase().as_str(),
            "PATH"
                | "HOME"
                | "USERPROFILE"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "SYSTEMROOT"
                | "WINDIR"
                | "TEMP"
                | "TMP"
                | "LANG"
                | "LC_ALL"
                | "PATHEXT"
        ) {
            command.env(key, value);
        }
    }
    command
}

pub(super) async fn run(
    request: Request,
    events: mpsc::Sender<Event>,
    inputs: mpsc::Receiver<Input>,
    cancel: watch::Receiver<bool>,
) -> Result<Outcome> {
    validate(&request)?;
    // A kernel lock stays held until native process cleanup completes. Canonical
    // paths also exclude symlink/drive aliases and separate Runner processes.
    let directory = tokio::fs::canonicalize(&request.cwd).await?;
    let _lock = directory_lock(&directory)?;
    if let Some(binding) = &request.binding {
        let inspected = probe(request.runtime, &request.cwd).await?;
        ensure!(
            inspected.canonical_directory.as_deref() == Some(&binding.canonical_directory)
                && inspected.execution_user == binding.execution_user
                && inspected.execution_identity == binding.execution_identity
                && inspected.executable.as_deref() == Some(&binding.executable),
            "Agent execution identity changed; create a new conversation"
        );
    }
    let command = command(&request);
    #[cfg(unix)]
    let mut command = command;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let descriptor = _lock.0.as_raw_fd();
        // Let the native process retain the lock if its Runner is killed.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    run_command(request, command, events, inputs, cancel).await
}

struct DirectoryLock(std::fs::File);

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        // A concurrent fork can briefly inherit this file before CLOEXEC closes
        // it. Unlock explicitly so its duplicate cannot extend the execution.
        let _ = self.0.unlock();
    }
}

fn directory_lock(directory: &std::path::Path) -> Result<DirectoryLock> {
    let directory = std::fs::canonicalize(directory)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".agent-execution.lock"))?;
    file.try_lock()
        .context("another Agent is using this directory")?;
    Ok(DirectoryLock(file))
}

async fn write(stdin: &mut tokio::process::ChildStdin, value: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .context("native runtime input closed")
}

// read_until is cancellation safe only if its buffer survives select! retries.
// Take caps the read before allocation, including a malicious unterminated line.
async fn read(
    reader: &mut (impl AsyncBufRead + Unpin),
    buffer: &mut Vec<u8>,
) -> Result<Option<Value>> {
    let remaining = LINE_LIMIT + 1 - buffer.len();
    let size = reader
        .take(remaining as u64)
        .read_until(b'\n', buffer)
        .await?;
    ensure!(
        buffer.len() <= LINE_LIMIT,
        "native event exceeds size limit"
    );
    if size == 0 {
        ensure!(buffer.is_empty(), "truncated native event");
        return Ok(None);
    }
    ensure!(buffer.last() == Some(&b'\n'), "unterminated native event");
    let value = serde_json::from_slice(buffer).context("invalid native JSON event")?;
    Ok(Some(value))
}

async fn drive(
    driver: &mut protocol::Protocol,
    stdin: &mut tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    events: &mpsc::Sender<Event>,
    mut inputs: mpsc::Receiver<Input>,
) -> Result<Outcome> {
    write(stdin, driver.initial()).await?;
    let mut reader = BufReader::new(stdout);
    let mut buffer = Vec::new();
    let mut total = 0;
    loop {
        let step = tokio::select! {
            value=read(&mut reader,&mut buffer)=>{
                let value=value?.context("native runtime ended without a completion event")?;
                total+=buffer.len();buffer.clear();
                ensure!(total<=OUTPUT_LIMIT,"native output exceeds size limit");
                driver.receive(value)?
            }
            input=inputs.recv()=>match input {
                Some(Input::Cancel)|None=>return Ok(Outcome::Cancelled),
                Some(input)=>driver.input(input)?,
            },
        };
        for value in step.writes {
            write(stdin, value).await?;
        }
        for event in step.events {
            events
                .send(event)
                .await
                .context("Agent event receiver closed")?;
        }
        if let Some(outcome) = step.done {
            return Ok(outcome);
        }
    }
}

async fn run_command(
    request: Request,
    mut command: Command,
    events: mpsc::Sender<Event>,
    inputs: mpsc::Receiver<Input>,
    mut cancel: watch::Receiver<bool>,
) -> Result<Outcome> {
    validate(&request)?;
    ensure!(
        tokio::fs::metadata(&request.cwd).await?.is_dir(),
        "Agent workdir must be a directory"
    );
    if *cancel.borrow() || events.is_closed() {
        return Ok(Outcome::Cancelled);
    }
    command
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("cannot start native Agent runtime")?;
    #[cfg(windows)]
    let job = crate::process_job::ProcessJob::attach(&child)?;
    #[cfg(unix)]
    let group = crate::exec_stream::ProcessGroup(child.id().context("missing native process id")?);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    // Drain diagnostics, but never publish inherited-runtime diagnostics as chat
    // messages or logs: they can contain local paths and authentication material.
    let diagnostics =
        tokio::spawn(async move { tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await });
    let scope = uuid::Uuid::new_v4().to_string();
    let mut driver = protocol::Protocol::new(request.clone(), scope);
    let result = {
        let execution = drive(&mut driver, &mut stdin, stdout, &events, inputs);
        tokio::pin!(execution);
        tokio::select! {
            result=&mut execution=>result,
            _=cancel.changed()=>Ok(Outcome::Cancelled),
            _=events.closed()=>Ok(Outcome::Cancelled),
            _=tokio::time::sleep(Duration::from_secs(request.timeout_seconds))=>Ok(Outcome::TimedOut),
        }
    };
    // Give a completed CLI a short EOF shutdown window to persist its history.
    // All exits, including parse errors and consumer disconnects, reclaim children.
    drop(stdin);
    let mut outcome = result.as_ref().copied().unwrap_or(Outcome::Failed);
    let graceful = if outcome == Outcome::Completed {
        tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .ok()
    } else {
        None
    };
    if let Some(Ok(status)) = graceful {
        if !status.success() {
            outcome = Outcome::Failed;
        }
    }
    #[cfg(windows)]
    let cleanup = job.terminate();
    #[cfg(unix)]
    let cleanup: Result<()> = {
        group.signal(true);
        Ok(())
    };
    let reaped = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    diagnostics.abort();
    cleanup?;
    reaped.context("native runtime did not stop")??;
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        events.send(Event::Finished {
            outcome,
            usage: driver.usage(),
        }),
    )
    .await;
    result.map(|_| outcome)
}
