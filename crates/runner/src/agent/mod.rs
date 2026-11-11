//! Structured native CLI execution. The caller owns authorization, persistence,
//! directory exclusion and host/user/native-session binding; no shell is used.
pub(crate) mod platform;
pub(super) mod probe;
mod protocol;
pub(super) mod provision;
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
        request.runtime != Runtime::Pi || request.permission == Permission::ReadOnly,
        "Pi currently supports read-only conversations only"
    );
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

#[cfg(test)]
fn command(request: &Request) -> Command {
    command_with(
        request,
        &provision::Environment {
            executable: probe::executable(request.runtime),
            home: None,
        },
    )
}

fn command_with(request: &Request, environment: &provision::Environment) -> Command {
    let executable = &environment.executable;
    let mut command = match request.runtime {
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
                if request.permission == Permission::ReadOnly && !request.platform_tools {
                    ""
                } else if request.permission == Permission::ReadOnly {
                    "mcp__ferry__ferry_discover,mcp__ferry__ferry_request"
                } else {
                    "Read,Glob,Grep,Edit,Write,Bash,mcp__ferry__ferry_discover,mcp__ferry__ferry_request"
                },
                "--allowedTools",
                if request.platform_tools { "mcp__ferry__ferry_discover,mcp__ferry__ferry_request" } else { "" },
            ]);
            if let Some(model) = &request.model {
                c.arg("--model").arg(model);
            }
            if let Some(id) = &request.native_session_id {
                c.arg("--resume").arg(id);
            }
            c
        }
        Runtime::Gemini | Runtime::Qwen | Runtime::Opencode | Runtime::Dsh => {
            let name = match request.runtime {
                Runtime::Gemini => "gemini",
                Runtime::Qwen => "qwen",
                Runtime::Dsh => "dsh",
                _ => "opencode",
            };
            let mut c = native_command(executable.as_deref().unwrap_or(std::path::Path::new(name)));
            c.arg(match request.runtime {
                Runtime::Gemini => "--experimental-acp",
                Runtime::Qwen => "--acp",
                _ => "acp",
            });
            c
        }
        Runtime::Pi => {
            let mut c = native_command(executable.as_deref().unwrap_or(std::path::Path::new("pi")));
            // Pi's MCP integrations are extensions; disabling extensions also disables them.
            c.args([
                "--mode",
                "rpc",
                "--no-tools",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
            ]);
            if let Some(model) = &request.model {
                c.arg("--model").arg(model);
            }
            if let Some(id) = &request.native_session_id {
                c.arg("--session").arg(id);
            }
            c
        }
    };
    environment.apply(&mut command);
    command
}

fn native_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let program = std::path::Path::new(program.as_ref());
    let mut command = if program
        .extension()
        .is_some_and(|ext| matches!(ext.to_str(), Some("js" | "mjs" | "cjs")))
    {
        let node = std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .filter(|directory| directory.is_absolute())
                    .map(|directory| {
                        directory.join(if cfg!(windows) { "node.exe" } else { "node" })
                    })
                    .find(|path| path.is_file())
            })
            .unwrap_or_else(|| program.with_extension("requires-node-exe"));
        let mut command = Command::new(node);
        command.arg(native_path(program));
        command
    } else {
        Command::new(program)
    };
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
                | "DSH_HOME"
        ) {
            command.env(key, value);
        }
    }
    command
}

/// Node/npm's path utilities do not consistently handle Windows verbatim paths.
/// Keep canonical paths for binding checks; normalize only subprocess inputs.
fn native_path(path: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    if let Some(value) = path.to_str() {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}").into();
        }
        if let Some(disk) = value.strip_prefix(r"\\?\") {
            return disk.into();
        }
    }
    path.to_owned()
}

// Uses DSH's public permission service, rather than editing its persisted logs.
// No imported packages, credentials, or platform business logic are embedded.
const DSH_PERMISSIONS: &str = r#"
export const name = 'runtime-permissions';
export const inject = ['agents', 'permissionPresets'];
export function apply(ctx) {
  const enforce = ({agent}) => {
    ctx.permissionPresets.set(agent.session, 'read-only');
    process.stdout.write(JSON.stringify({jsonrpc:'2.0',method:'agent_runtime/permissions',
      params:{sessionId:agent.session.id,preset:'read-only'}}) + '\n');
  };
  ctx.on('agent/created', enforce);
  for (const agent of ctx.agents.list()) enforce({agent});
  ctx.on('session/event', (session, event) => {
    if (event.type === 'turn/end') {
      process.stdout.write(JSON.stringify({jsonrpc:'2.0',method:'agent_runtime/turn_end',
        params:{sessionId:session.id,reason:event.data.reason.kind}}) + '\n');
    }
  });
}
"#;

#[cfg(test)]
async fn run(
    request: Request,
    events: mpsc::Sender<Event>,
    inputs: mpsc::Receiver<Input>,
    cancel: watch::Receiver<bool>,
) -> Result<Outcome> {
    let state = tempfile::tempdir()?;
    run_managed(state.path(), request, events, inputs, cancel).await
}

pub(super) async fn run_managed(
    state_dir: &std::path::Path,
    request: Request,
    events: mpsc::Sender<Event>,
    inputs: mpsc::Receiver<Input>,
    cancel: watch::Receiver<bool>,
) -> Result<Outcome> {
    validate(&request)?;
    let environment = provision::environment(
        state_dir,
        request.runtime,
        &request.cwd,
        false,
        request.binding.as_ref(),
    )
    .await?;
    // A kernel lock stays held until native process cleanup completes. Canonical
    // paths also exclude symlink/drive aliases and separate Runner processes.
    let directory = tokio::fs::canonicalize(&request.cwd).await?;
    let _lock = directory_lock(&directory)?;
    if let Some(binding) = &request.binding {
        let inspected = probe::probe_with(request.runtime, &request.cwd, &environment).await?;
        ensure!(
            inspected.canonical_directory.as_deref() == Some(&binding.canonical_directory)
                && inspected.execution_user == binding.execution_user
                && inspected.execution_identity == binding.execution_identity
                && inspected.executable.as_deref() == Some(&binding.executable),
            "Agent execution identity changed; create a new conversation"
        );
    }
    let command = command_with(&request, &environment);
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
    bridge: Option<&platform::Bridge>,
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
                Some(Input::PlatformResult{request_id,result})=>{
                    if let Some(bridge)=bridge { bridge.resolve(&request_id,result); }
                    protocol::Step::default()
                },
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
    let bridge = if request.platform_tools {
        Some(platform::Bridge::start(events.clone()).await?)
    } else {
        None
    };
    let mut mcp = None;
    let mut _extension = None;
    let mut _runtime_policy = None;
    if request.runtime == Runtime::Dsh {
        // ACP has no mode setter. Apply the official permission service to every
        // new or restored Agent before admitting a prompt, including descendants.
        // A read-only sandbox can escalate only through our per-call approval path.
        let plugin = tempfile::Builder::new()
            .prefix("runtime-permissions-")
            .suffix(".mjs")
            .tempfile()?;
        tokio::fs::write(plugin.path(), DSH_PERMISSIONS).await?;
        let url = reqwest::Url::from_file_path(native_path(plugin.path()))
            .map_err(|_| anyhow::anyhow!("invalid runtime policy path"))?;
        let patch = tempfile::Builder::new()
            .prefix("runtime-permissions-")
            .suffix(".yml")
            .tempfile()?;
        let body = serde_json::json!([{"insert":[{"id":"runtime-permissions","name":url.as_str(),
            "inject":["agents","permissionPresets"]}]}]);
        tokio::fs::write(patch.path(), serde_json::to_vec(&body)?).await?;
        command.arg("--patch").arg(native_path(patch.path()));
        command.env("DSH_PERMISSION_MODE", "read-only");
        _runtime_policy = Some((plugin, patch));
    }
    if let Some(bridge) = &bridge {
        command
            .env("FERRY_PLATFORM_BRIDGE_URL", &bridge.url)
            .env("FERRY_PLATFORM_BRIDGE_TOKEN", &bridge.token);
        let executable = std::env::current_exe()?
            .to_string_lossy()
            .replace('\\', "/");
        mcp = Some(
            serde_json::json!({"name":"ferry","command":executable,"args":["ferry-mcp"],"env":[
            {"name":"FERRY_PLATFORM_BRIDGE_URL","value":bridge.url}, {"name":"FERRY_PLATFORM_BRIDGE_TOKEN","value":bridge.token}]}),
        );
        match request.runtime {
            Runtime::Codex => {
                for (key, value) in [
                    ("command", serde_json::json!(executable)),
                    ("args", serde_json::json!(["ferry-mcp"])),
                    (
                        "env_vars",
                        serde_json::json!([
                            "FERRY_PLATFORM_BRIDGE_URL",
                            "FERRY_PLATFORM_BRIDGE_TOKEN"
                        ]),
                    ),
                    ("required", serde_json::json!(true)),
                    // Only the Ferry transport is pre-approved. The server performs
                    // live RBAC and its own one-time confirmation for every write.
                    (
                        "tools.ferry_discover.approval_mode",
                        serde_json::json!("approve"),
                    ),
                    (
                        "tools.ferry_request.approval_mode",
                        serde_json::json!("approve"),
                    ),
                    ("tool_timeout_sec", serde_json::json!(3600)),
                ] {
                    command
                        .arg("-c")
                        .arg(format!("mcp_servers.ferry.{key}={value}"));
                }
            }
            Runtime::Claude => {
                command.arg("--mcp-config").arg(serde_json::json!({"mcpServers":{"ferry":{"command":executable,"args":["ferry-mcp"]}}}).to_string());
            }
            Runtime::Pi => {
                let file = tempfile::Builder::new()
                    .prefix("platform-tools-")
                    .suffix(".mjs")
                    .tempfile()?;
                tokio::fs::write(file.path(), platform::PI_EXTENSION).await?;
                command.arg("--extension").arg(file.path());
                _extension = Some(file);
            }
            _ => {}
        }
    }
    command
        .current_dir(native_path(std::path::Path::new(&request.cwd)))
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
    driver.mcp_server = mcp;
    let result = {
        let execution = drive(
            &mut driver,
            &mut stdin,
            stdout,
            &events,
            inputs,
            bridge.as_ref(),
        );
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
