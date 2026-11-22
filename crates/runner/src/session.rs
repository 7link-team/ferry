//! One session, one connection.
//!
//! The server opens with exactly one request frame and this module answers it.
//! Nothing here multiplexes, correlates or buffers a whole transfer: the frame
//! says what to do, bytes flow in whichever direction it implies, and the
//! connection closes when it is done.

use crate::transport::Socket;
use anyhow::Context;
use ferry_protocol as wire;
use futures_util::{SinkExt, StreamExt};
use std::time::{Duration, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

/// Transfer chunk size. One of these is the runner's whole transfer footprint,
/// however large the file is.
const CHUNK: usize = 64 * 1024;
use wire::resources::{ResourcePolicy, allows};

pub async fn run(
    url: &str,
    hello: wire::SessionHello,
    kind: wire::SessionKind,
    state_dir: &std::path::Path,
    policy: ResourcePolicy,
) -> anyhow::Result<()> {
    let mut socket = crate::transport::dial_with_policy(url, &policy).await?;
    crate::transport::send(&mut socket, &hello).await?;
    let request: wire::Session = crate::transport::next_frame(&mut socket).await?;
    tracing::info!(?kind, "会话连接已建立");

    let outcome = dispatch_with_policy(&mut socket, request, state_dir, &policy).await;
    if let Err(error) = &outcome {
        // The operator sees this sentence in the browser, so say it rather
        // than closing the socket and leaving them to guess.
        let _ = crate::transport::send(
            &mut socket,
            &wire::Session::Failed {
                error: format!("{error}"),
            },
        )
        .await;
    }
    let _ = socket.close(None).await;
    outcome
}

#[cfg(test)]
async fn dispatch(
    socket: &mut Socket,
    request: wire::Session,
    state_dir: &std::path::Path,
) -> anyhow::Result<()> {
    dispatch_with_policy(socket, request, state_dir, &ResourcePolicy::default()).await
}

async fn dispatch_with_policy(
    socket: &mut Socket,
    request: wire::Session,
    state_dir: &std::path::Path,
    policy: &ResourcePolicy,
) -> anyhow::Result<()> {
    match request {
        wire::Session::TerminalStart {
            cols,
            rows,
            timezone,
        } => terminal(socket, cols, rows, timezone.as_deref()).await,
        wire::Session::ExecStart {
            command,
            timeout_seconds,
            timezone,
        } => {
            let _ = timeout_seconds; // Legacy wire value is no longer a quota.
            anyhow::ensure!(
                allows(policy.command_bytes, command.len()),
                "command exceeds size limit"
            );
            exec(socket, &command, policy, timezone.as_deref()).await
        }
        wire::Session::ExecStreamStart {
            command,
            timeout_seconds,
            timezone,
        } => {
            let _ = timeout_seconds;
            anyhow::ensure!(
                allows(policy.command_bytes, command.len()),
                "command exceeds size limit"
            );
            exec_stream(socket, &command, policy, timezone.as_deref()).await
        }
        wire::Session::ReadFile { path } => read_file(socket, &path).await,
        wire::Session::WriteFile { path, mode } => write_file(socket, &path, mode, policy).await,
        wire::Session::ListDir { path } => list_dir(socket, &path, policy).await,
        wire::Session::CreateFile { path } => create_file(socket, &path).await,
        wire::Session::CreateDir { path } => create_dir(socket, &path).await,
        wire::Session::DeleteFile { path } => delete_file(socket, &path).await,
        wire::Session::Monitor => monitor(socket).await,
        wire::Session::AgentStart { request } => agent(socket, request, state_dir).await,
        wire::Session::ForwardStart { port, host } => forward(socket, &host, port).await,
        wire::Session::AgentProbe {
            runtime,
            cwd,
            workspace,
            binding,
        } => {
            let prepare = workspace.is_some();
            let directory = if let Some(key) = workspace {
                anyhow::ensure!(
                    cwd.is_empty(),
                    "managed workspace cannot override a directory"
                );
                crate::agent::workspace(state_dir, &key)
                    .await?
                    .to_string_lossy()
                    .into_owned()
            } else {
                cwd
            };
            let environment = crate::agent::provision::environment_with_policy(
                state_dir,
                runtime,
                &directory,
                prepare,
                binding.as_ref(),
                policy,
            )
            .await?;
            let status = crate::agent::probe::probe_with(runtime, &directory, &environment).await?;
            crate::transport::send(socket, &wire::Session::AgentProbeResult { status }).await
        }
        other => anyhow::bail!("服务下发了不是请求的帧：{other:?}"),
    }
}

/// How long a preview may wait for the local service to accept.
const FORWARD_CONNECT: Duration = Duration::from_secs(5);

/// A preview may reach this host or its local network, never the internet,
/// link-local/cloud-metadata addresses or multicast.
pub(crate) fn forward_allowed(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                // Carrier-grade NAT, used by Tailscale and some LANs.
                || (a == 100 && (64..=127).contains(&b))
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forward_allowed(std::net::IpAddr::V4(v4));
            }
            // Loopback and unique-local (fc00::/7).
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Pipes a local port through the session: TCP bytes become binary frames
/// and back. Either side ending ends both, so neither connection outlives the
/// other. An empty `host` is this machine's loopback; otherwise it must
/// resolve, on this Runner, only to loopback or private-network addresses.
async fn forward(socket: &mut Socket, host: &str, port: u16) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    anyhow::ensure!(port != 0, "预览端口必须在 1-65535 之间");
    let connect = |address: std::net::SocketAddr| {
        tokio::time::timeout(FORWARD_CONNECT, tokio::net::TcpStream::connect(address))
    };
    let candidates: Vec<std::net::SocketAddr> = if host.is_empty() {
        // A dev server that listens on "localhost" may have bound only one family.
        vec![
            (std::net::Ipv4Addr::LOCALHOST, port).into(),
            (std::net::Ipv6Addr::LOCALHOST, port).into(),
        ]
    } else {
        anyhow::ensure!(
            host.len() <= 253
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)),
            "预览目标地址无效"
        );
        let name = host.trim_start_matches('[').trim_end_matches(']');
        let resolved: Vec<_> =
            tokio::time::timeout(FORWARD_CONNECT, tokio::net::lookup_host((name, port)))
                .await
                .map_err(|_| anyhow::anyhow!("解析预览目标 {host} 超时"))?
                .map_err(|error| anyhow::anyhow!("无法解析预览目标 {host}：{error}"))?
                .collect();
        // Every address must be local: a name that also resolves publicly is refused.
        anyhow::ensure!(
            !resolved.is_empty() && resolved.iter().all(|a| forward_allowed(a.ip())),
            "预览目标 {host} 不在本机或局域网地址范围内"
        );
        resolved
    };
    let mut last = None;
    let mut stream = None;
    for address in candidates {
        match connect(address).await {
            Ok(Ok(connected)) => {
                stream = Some(connected);
                break;
            }
            other => last = Some(other),
        }
    }
    let target = if host.is_empty() { "本机" } else { host };
    let stream = match (stream, last) {
        (Some(stream), _) => stream,
        (None, Some(Ok(Err(error)))) => anyhow::bail!("无法连接{target}端口 {port}：{error}"),
        _ => anyhow::bail!("连接{target}端口 {port} 超时"),
    };
    let _ = stream.set_nodelay(true);
    crate::transport::send(socket, &wire::Session::ForwardReady).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let n = read?;
                if n == 0 { break }
                socket.send(Message::Binary(buffer[..n].to_vec().into())).await?;
            }
            message = socket.next() => match message {
                Some(Ok(Message::Binary(bytes))) => writer.write_all(&bytes).await?,
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_))) => {}
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(error)) => return Err(error.into()),
            },
        }
    }
    let _ = writer.shutdown().await;
    Ok(())
}

async fn agent(
    socket: &mut Socket,
    request: wire::agent::Request,
    state_dir: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::sync::{mpsc, watch};
    let (events, receiver) = mpsc::channel(16);
    let (inputs, commands) = mpsc::channel(16);
    let (stop, cancelled) = watch::channel(false);
    let execution = crate::agent::run_managed(state_dir, request, events, commands, cancelled);
    pump_agent(socket, execution, receiver, inputs, stop).await
}

async fn pump_agent(
    socket: &mut Socket,
    execution: impl std::future::Future<Output = anyhow::Result<wire::agent::Outcome>>,
    mut receiver: tokio::sync::mpsc::Receiver<wire::agent::Event>,
    inputs: tokio::sync::mpsc::Sender<wire::agent::Input>,
    stop: tokio::sync::watch::Sender<bool>,
) -> anyhow::Result<()> {
    let (mut sink, mut source) = socket.split();
    let (outbound, mut messages) = tokio::sync::mpsc::channel::<wire::Session>(1);
    let writing = async move {
        while let Some(frame) = messages.recv().await {
            sink.send(Message::Text(serde_json::to_string(&frame)?.into()))
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(writing, execution);
    let mut pending = std::collections::VecDeque::new();
    let mut output = None;
    let outcome = loop {
        tokio::select! {
            result = &mut writing => break result,
            permit = outbound.clone().reserve_owned(), if output.is_some() => {
                match permit {
                    Ok(permit) => { permit.send(output.take().unwrap()); }
                    Err(_) => break Err(anyhow::anyhow!("Agent event connection unavailable")),
                }
            },
            permit = inputs.reserve(), if !pending.is_empty() => {
                match permit {
                    Ok(permit) => { permit.send(pending.pop_front().unwrap()); }
                    Err(_) => break Err(anyhow::anyhow!("Agent input receiver closed")),
                }
            },
            result = &mut execution => {
                let draining = async {
                    if let Some(frame) = output.take() { outbound.send(frame).await?; }
                    while let Ok(event) = receiver.try_recv() {
                        outbound.send(wire::Session::AgentEvent { event }).await?;
                    }
                    drop(outbound);
                    Ok::<(), anyhow::Error>(())
                };
                tokio::try_join!(draining, &mut writing)?;
                return result.map(|_| ());
            },
            event = receiver.recv(), if output.is_none() => if let Some(event) = event {
                output = Some(wire::Session::AgentEvent { event });
            },
            frame = source.next() => match frame {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<wire::Session>(&text) {
                    Ok(wire::Session::AgentInput { input: wire::agent::Input::Cancel }) => { let _ = stop.send(true); }
                    Ok(wire::Session::AgentInput { input }) => { pending.push_back(input); }
                    _ => break Err(anyhow::anyhow!("invalid Agent session frame")),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(()),
                // Tungstenite queues protocol Pong replies while reading.
                _ => {},
            }
        }
    };
    let _ = stop.send(true);
    let _ = execution.await;
    outcome
}

async fn monitor(socket: &mut Socket) -> anyhow::Result<()> {
    let metrics = tokio::task::spawn_blocking(|| {
        let mut system = sysinfo::System::new();
        system.refresh_cpu_all();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        system.refresh_cpu_all();
        system.refresh_memory();
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let networks = sysinfo::Networks::new_with_refreshed_list();
        wire::HostMonitor {
            cpu_percent: f64::from(system.global_cpu_usage()),
            load1: sysinfo::System::load_average().one,
            memory_used: system.used_memory(),
            memory_total: system.total_memory(),
            disks: disks
                .list()
                .iter()
                .map(|disk| wire::DiskUsage {
                    name: disk.name().to_string_lossy().into_owned(),
                    mount: disk.mount_point().to_string_lossy().into_owned(),
                    total: disk.total_space(),
                    used: disk.total_space().saturating_sub(disk.available_space()),
                })
                .collect(),
            networks: networks
                .list()
                .iter()
                .filter(|(name, _)| !name.starts_with("lo") && !name.starts_with("Loopback"))
                .map(|(name, data)| wire::NetworkUsage {
                    name: name.clone(),
                    received: data.total_received(),
                    transmitted: data.total_transmitted(),
                })
                .collect(),
        }
    })
    .await?;
    crate::transport::send(socket, &wire::Session::MonitorResult { metrics }).await
}

async fn create_file(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .with_context(|| format!("无法创建文件 {path}，请检查名称是否重复及目录权限。"))?;
    crate::transport::send(socket, &wire::Session::Done).await
}

async fn create_dir(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir(path)
        .await
        .with_context(|| format!("无法创建文件夹 {path}，请检查名称是否重复及目录权限。"))?;
    crate::transport::send(socket, &wire::Session::Done).await
}

// ponytail: no path validation here. The server already rejects relative and
// `..` paths, and a channel that can run `exec` has nothing left to protect by
// vetting a path — the runner trusts its own server by construction.

// ------------------------------------------------------------------ 命令

#[cfg(test)]
pub(crate) fn shell_command(command: &str) -> tokio::process::Command {
    shell_command_with_timezone(command, None)
}

pub(crate) fn shell_command_with_timezone(
    command: &str,
    timezone: Option<&str>,
) -> tokio::process::Command {
    #[cfg(windows)]
    let mut process = {
        use std::os::windows::process::CommandExt;
        let mut process = tokio::process::Command::new("cmd");
        // The command reaches cmd.exe exactly as written: `arg` would escape
        // inner quotes MSVC-style (\"), which cmd does not understand, so any
        // quoted path with spaces failed. /S makes cmd strip only the outer
        // pair of quotes added here.
        process
            .arg("/S")
            .arg("/C")
            .raw_arg(format!("\"{command}\""));
        // CREATE_NO_WINDOW: a scheduled job must not flash a console over
        // whatever the person at that desktop is doing.
        process.creation_flags(0x0800_0000);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = tokio::process::Command::new("/bin/sh");
        process.arg("-c").arg(command);
        process
    };
    process.stdin(std::process::Stdio::null());
    crate::timezone::apply(&mut process, timezone);
    process
}

fn capped(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

async fn exec(
    socket: &mut Socket,
    command: &str,
    policy: &ResourcePolicy,
    timezone: Option<&str>,
) -> anyhow::Result<()> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    if let Some(zone) = timezone {
        anyhow::ensure!(crate::timezone::valid(zone), "invalid process timezone");
    }
    let mut process = shell_command_with_timezone(command, timezone);
    process
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process.spawn().context("cannot start command")?;
    #[cfg(windows)]
    let job = crate::process_job::ProcessJob::attach(&child)?;
    #[cfg(unix)]
    let group = crate::exec_stream::ProcessGroup(child.id().context("missing command process id")?);
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let result: anyhow::Result<Option<std::process::Output>> = {
        let capture = async {
            let mut stdout_bytes = Vec::new();
            let mut stderr_bytes = Vec::new();
            let (status, _, _) = tokio::try_join!(
                child.wait(),
                stdout.read_to_end(&mut stdout_bytes),
                stderr.read_to_end(&mut stderr_bytes),
            )?;
            Ok::<_, anyhow::Error>(std::process::Output {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
            })
        };
        let deadline = crate::transport::deadline(policy.task_timeout_seconds);
        tokio::pin!(capture, deadline);
        loop {
            tokio::select! {
                output = &mut capture => break output.map(Some),
                _ = &mut deadline => break Err(anyhow::anyhow!("command execution exceeded configured timeout")),
                message = socket.next() => match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(None),
                    _ => {},
                }
            }
        }
    };
    // All outcomes own their descendants, including a client that vanished
    // while a command with no deadline was still producing captured output.
    #[cfg(windows)]
    let cleanup = job.terminate();
    #[cfg(unix)]
    let cleanup: anyhow::Result<()> = {
        group.signal(true);
        Ok(())
    };
    let _ = child.start_kill();
    let reaped = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    cleanup?;
    reaped.context("command did not stop")??;
    let Some(output) = result? else {
        return Ok(());
    };
    anyhow::ensure!(
        allows(
            policy.command_output_bytes,
            output.stdout.len().saturating_add(output.stderr.len())
        ),
        "command output exceeds size limit"
    );
    crate::transport::send(
        socket,
        &wire::Session::ExecDone {
            code: output.status.code().unwrap_or(-1),
            stdout: capped(&output.stdout),
            stderr: capped(&output.stderr),
        },
    )
    .await
}

async fn exec_stream(
    socket: &mut Socket,
    command: &str,
    policy: &ResourcePolicy,
    timezone: Option<&str>,
) -> anyhow::Result<()> {
    let (output, mut chunks) = tokio::sync::mpsc::channel(16);
    let (stop, cancelled) = tokio::sync::watch::channel(false);
    let execution = crate::exec_stream::run_with_timezone(
        command,
        policy.task_timeout_seconds.unwrap_or(0),
        output,
        cancelled,
        timezone,
    );
    let (mut sink, mut source) = socket.split();
    let (outbound, mut messages) = tokio::sync::mpsc::channel::<wire::Session>(1);
    let writing = async move {
        while let Some(frame) = messages.recv().await {
            sink.send(Message::Text(serde_json::to_string(&frame)?.into()))
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::pin!(execution, writing);
    let mut exit = None;
    let mut total = 0usize;
    let mut pending = None;
    let outcome = loop {
        tokio::select! {
            result = &mut writing => break result,
            result = &mut execution, if exit.is_none() => { exit = Some(result?); }
            permit = outbound.clone().reserve_owned(), if pending.is_some() => {
                match permit {
                    Ok(permit) => { permit.send(pending.take().unwrap()); }
                    Err(_) => break Err(anyhow::anyhow!("command output connection unavailable")),
                }
            },
            chunk = chunks.recv(), if pending.is_none() => match chunk {
                Some((stream, data)) => {
                    total = total.saturating_add(data.len());
                    if !allows(policy.command_output_bytes, total) || !allows(policy.log_output_bytes, total) {
                        break Err(anyhow::anyhow!("command output exceeds size limit"));
                    }
                    pending = Some(wire::Session::ExecOutput {stream, data});
                }
                None => {
                    let code = match exit { Some(code) => code, None => execution.await? };
                    let ending = async {
                        outbound.send(wire::Session::ExecExit { code }).await?;
                        drop(outbound);
                        Ok::<(), anyhow::Error>(())
                    };
                    tokio::try_join!(ending, &mut writing)?;
                    return Ok(());
                }
            },
            message = source.next() => match message {
                Some(Ok(Message::Text(text))) if matches!(serde_json::from_str::<wire::Session>(&text), Ok(wire::Session::ExecCancel)) => { let _ = stop.send(true); }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(()),
                _ => {}
            }
        }
    };
    let _ = stop.send(true);
    if exit.is_none() {
        let _ = execution.await;
    }
    outcome
}

// ------------------------------------------------------------------ 文件

fn millis(time: std::io::Result<std::time::SystemTime>) -> i64 {
    time.ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

fn mode_of(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

async fn read_file(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncReadExt;
    let meta = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("无法读取 {path}"))?;
    anyhow::ensure!(!meta.is_dir(), "{path} 是目录，不能直接下载。");
    crate::transport::send(
        socket,
        &wire::Session::FileMeta {
            size: meta.len(),
            mode: mode_of(&meta),
            modified_at: millis(meta.modified()),
        },
    )
    .await?;

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("无法打开 {path}"))?;
    // One buffer for the whole transfer, and `send().await` is the
    // backpressure: a 2 GB file costs 64 KiB of memory on both ends.
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut buffer).await.context("读取文件时中断")?;
        if read == 0 {
            break;
        }
        socket
            .send(Message::binary(buffer[..read].to_vec()))
            .await
            .context("发送文件内容时中断")?;
    }
    crate::transport::send(socket, &wire::Session::Eof).await
}

async fn write_file(
    socket: &mut Socket,
    path: &str,
    mode: u32,
    policy: &ResourcePolicy,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        // A mode of 0 would create a file nobody can read, including the
        // service that is about to use it.
        options.mode(if mode == 0 { 0o644 } else { mode });
    }
    let mut file = options
        .open(path)
        .await
        .with_context(|| format!("无法写入 {path}，请检查目录是否存在及权限。"))?;

    let mut total = 0usize;
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => {
                total = total.saturating_add(bytes.len());
                anyhow::ensure!(
                    allows(policy.file_upload_bytes, total),
                    "file upload exceeds size limit"
                );
                file.write_all(&bytes).await.context("写入文件时中断")?;
            }
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<wire::Session>(&text) {
                Ok(wire::Session::Eof) => break,
                Ok(wire::Session::Failed { error }) => anyhow::bail!("上传方中断：{error}"),
                _ => {}
            },
            Some(Ok(Message::Close(_))) | None => {
                // Closing without `Eof` means the file on disk is incomplete.
                // Say so instead of reporting a successful short upload.
                anyhow::bail!("上传中断：连接在传输完成前关闭，{path} 内容不完整。");
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error).context("上传中断"),
        }
    }
    // The operator asked for the file to be on this host, so it is on the disk
    // before the answer goes back.
    file.flush().await?;
    file.sync_all().await?;
    crate::transport::send(socket, &wire::Session::Done).await
}

async fn list_dir(socket: &mut Socket, path: &str, policy: &ResourcePolicy) -> anyhow::Result<()> {
    let mut reader = tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("无法打开目录 {path}"))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await.context("读取目录时中断")? {
        anyhow::ensure!(
            allows(policy.directory_entries, entries.len().saturating_add(1)),
            "directory exceeds entry limit"
        );
        // Does not follow the link: a dangling symlink should still be listed
        // rather than failing the whole directory.
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        entries.push(wire::Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: if meta.is_symlink() {
                "symlink"
            } else if meta.is_dir() {
                "directory"
            } else {
                "file"
            }
            .to_owned(),
            size: meta.len(),
            mode: mode_of(&meta),
            modified_at: millis(meta.modified()),
        });
    }
    // Directories first, then by name: the order a file manager wants, decided
    // once here rather than in every client.
    entries.sort_by(|a, b| {
        (a.kind != "directory")
            .cmp(&(b.kind != "directory"))
            .then_with(|| a.name.cmp(&b.name))
    });
    crate::transport::send(socket, &wire::Session::DirEntries { entries }).await
}

async fn delete_file(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .with_context(|| format!("{path} 不存在或无法访问。"))?;
    if meta.is_dir() {
        // Not recursive, deliberately: one mistyped path must not take a tree
        // with it. An operator who means that can say so with a command.
        tokio::fs::remove_dir(path)
            .await
            .with_context(|| format!("无法删除目录 {path}，请确认目录为空。"))?;
    } else {
        tokio::fs::remove_file(path)
            .await
            .with_context(|| format!("无法删除 {path}"))?;
    }
    crate::transport::send(socket, &wire::Session::Done).await
}

// ------------------------------------------------------------------ 终端

fn login_shell() -> String {
    #[cfg(windows)]
    return std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    #[cfg(not(windows))]
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

fn pty_size(cols: u16, rows: u16) -> portable_pty::PtySize {
    portable_pty::PtySize {
        rows: rows.max(1),
        cols: cols.max(2),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// A service's profile is not a useful working directory for an operator.
#[cfg(windows)]
fn is_service_profile(path: &std::path::Path) -> bool {
    let path = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    path.trim_end_matches('\\').ends_with("\\systemprofile") || path.contains("\\serviceprofiles\\")
}

#[cfg(windows)]
fn terminal_directory() -> std::path::PathBuf {
    use std::{os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle, System::RemoteDesktop::*, UI::Shell::GetUserProfileDirectoryW,
    };

    // Ordinary standalone runners keep their own user's home. LocalSystem can
    // query the active console/RDP user's profile without changing shell identity.
    if let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from) {
        if home.is_dir() && !is_service_profile(&home) {
            return home;
        }
    }
    unsafe {
        let mut sessions = ptr::null_mut();
        let mut count = 0;
        if WTSEnumerateSessionsW(ptr::null_mut(), 0, 1, &mut sessions, &mut count) != 0 {
            let console = WTSGetActiveConsoleSessionId();
            let entries = if count == 0 || sessions.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(sessions, count as usize)
            };
            let mut active: Vec<_> = entries
                .iter()
                .filter(|session| session.State == WTSActive)
                .map(|session| session.SessionId)
                .collect();
            WTSFreeMemory(sessions.cast());
            active.sort_by_key(|id| (*id != console, *id));
            for session in active {
                let mut token = ptr::null_mut();
                if WTSQueryUserToken(session, &mut token) == 0 {
                    continue;
                }
                let mut size = 0;
                GetUserProfileDirectoryW(token, ptr::null_mut(), &mut size);
                let mut buffer = vec![0u16; size as usize];
                let found = size > 0
                    && GetUserProfileDirectoryW(token, buffer.as_mut_ptr(), &mut size) != 0;
                CloseHandle(token);
                if found {
                    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
                    let home =
                        std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end]));
                    if home.is_dir() && !is_service_profile(&home) {
                        return home;
                    }
                }
            }
        }
    }
    // Headless hosts may have no interactive user. Use the public user folder,
    // then the system drive, rather than a private Windows service directory.
    std::env::var_os("PUBLIC")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!(
                "{}\\",
                std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
            ))
        })
}

fn terminal_command(timezone: Option<&str>) -> anyhow::Result<portable_pty::CommandBuilder> {
    let mut command = portable_pty::CommandBuilder::new(login_shell());
    command.env("TERM", "xterm-256color");
    if let Some(zone) = timezone {
        anyhow::ensure!(crate::timezone::valid(zone), "invalid process timezone");
        crate::timezone::apply_pty(&mut command, Some(zone));
    }
    #[cfg(windows)]
    command.cwd(terminal_directory());
    Ok(command)
}

async fn terminal(
    socket: &mut Socket,
    cols: u16,
    rows: u16,
    timezone: Option<&str>,
) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let pty = portable_pty::native_pty_system()
        .openpty(pty_size(cols, rows))
        .context("无法分配伪终端")?;
    let command = terminal_command(timezone)?;
    let mut child = pty.slave.spawn_command(command).context("无法启动 shell")?;
    // Dropping the slave is what lets the reader see EOF when the shell exits.
    drop(pty.slave);

    let mut reader = pty.master.try_clone_reader()?;
    let mut writer = pty.master.take_writer()?;
    let mut master = Some(pty.master);
    // portable-pty is blocking, and these two threads live as long as the
    // session, so they are dedicated threads rather than `spawn_blocking`
    // slots the runtime needs back.
    let (output, mut from_shell) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if output.blocking_send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let (to_shell, keystrokes) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(data) = keystrokes.recv() {
            if writer.write_all(&data).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let shell_exited = {
        let (mut sink, mut incoming) = socket.split();
        // A slow output flush must not prevent receiving Ctrl-C or resize.
        // The existing bounded PTY queue still applies backpressure to logs.
        let outgoing = async {
            while let Some(bytes) = from_shell.recv().await {
                sink.send(Message::binary(bytes)).await?;
            }
            Ok::<(), tokio_tungstenite::tungstenite::Error>(())
        };
        tokio::pin!(outgoing);
        let mut child_poll = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            tokio::select! {
                result = &mut outgoing => break result.is_ok(),
                _ = child_poll.tick(), if cfg!(windows) && master.is_some() => {
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            // ConPTY keeps its output pipe open after cmd exits
                            // while the master still owns the pseudo console.
                            // Closing it allows the reader to drain and see EOF.
                            let closing = master.take();
                            tokio::task::spawn_blocking(move || drop(closing));
                        }
                        Ok(None) => {}
                        Err(_) => break false,
                    }
                },
                message = incoming.next() => match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if to_shell.send(bytes.to_vec()).is_err() { break false; }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(wire::Session::TerminalResize { cols, rows }) =
                            serde_json::from_str::<wire::Session>(&text)
                            && let Some(master) = &master
                            && master.resize(pty_size(cols, rows)).is_err()
                        {
                            break false;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break false,
                    Some(Ok(_)) => {}
                },
            }
        }
    };

    // Closing the browser tab must not leave a shell running on the host.
    tokio::task::spawn_blocking(move || {
        let _ = child.kill();
        let _ = child.wait();
    });
    drop(to_shell);
    // Only worth telling the server the shell is gone if the server is still
    // listening: when the browser closed first, the socket is already closing
    // and a frame sent into it is a protocol error, not news.
    if shell_exited {
        let _ = crate::transport::send(socket, &wire::Session::Eof).await;
    }
    // A terminal that ended is not a failure, however it ended.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captured_exec_preserves_large_output_stderr_and_exit_code_with_no_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("output.mjs");
        let size = 1024 * 1024 + 123;
        std::fs::write(&script, format!("process.stdout.write('x'.repeat({size})); process.stderr.write('trace'); process.exitCode=7;\n")).unwrap();
        let command = format!(r#"node "{}""#, script.to_string_lossy().replace('\\', "/"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::transport::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame, std::path::Path::new(".")).await
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        crate::transport::send(
            &mut client,
            &wire::Session::ExecStart {
                command,
                timeout_seconds: 0,
                timezone: None,
            },
        )
        .await
        .unwrap();
        let frame: wire::Session = tokio::time::timeout(
            Duration::from_secs(5),
            crate::transport::next_frame(&mut client),
        )
        .await
        .unwrap()
        .unwrap();
        let wire::Session::ExecDone {
            code,
            stdout,
            stderr,
        } = frame
        else {
            panic!("ExecDone frame")
        };
        assert_eq!(code, 7);
        assert_eq!(stdout.len(), size);
        assert!(stdout.bytes().all(|byte| byte == b'x'));
        assert_eq!(stderr, "trace");
        served.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn captured_exec_disconnect_stops_command_and_descendants_without_a_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("parent.mjs");
        let child = dir.path().join("child.mjs");
        let ready = dir.path().join("ready");
        let parent_marker = dir.path().join("parent-late");
        let child_marker = dir.path().join("child-late");
        // Self-expiry keeps the regression fixture contained even before the fix.
        std::fs::write(
            &script,
            r#"import fs from 'node:fs'; import {spawn} from 'node:child_process';
const [marker,ready,child,childMarker]=process.argv.slice(2);
spawn(process.execPath,[child,childMarker],{windowsHide:true,stdio:'ignore'});
fs.writeFileSync(ready,'ready');
setTimeout(()=>fs.writeFileSync(marker,'late'),1000);
setTimeout(()=>process.exit(0),2500);
"#,
        )
        .unwrap();
        std::fs::write(
            &child,
            r#"import fs from 'node:fs';
setTimeout(()=>fs.writeFileSync(process.argv[2],'late'),1000);
setTimeout(()=>process.exit(0),2500);
"#,
        )
        .unwrap();
        let quoted =
            |path: &std::path::Path| format!("\"{}\"", path.to_string_lossy().replace('\\', "/"));
        let command = format!(
            "node {} {} {} {} {}",
            quoted(&script),
            quoted(&parent_marker),
            quoted(&ready),
            quoted(&child),
            quoted(&child_marker)
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut served = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::transport::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame, std::path::Path::new(".")).await
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        crate::transport::send(
            &mut client,
            &wire::Session::ExecStart {
                command,
                timeout_seconds: 0,
                timezone: None,
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        client.close(None).await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(1), &mut served).await;
        let prompt = matches!(&ended, Ok(Ok(Ok(()))));
        if ended.is_err() {
            served.abort();
            let _ = served.await;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(prompt, "captured command ignored its closed session");
        assert!(
            !parent_marker.exists(),
            "command survived socket cancellation"
        );
        assert!(
            !child_marker.exists(),
            "descendant survived socket cancellation"
        );
    }

    #[tokio::test]
    async fn agent_cancellation_reaches_execution_while_socket_output_is_backpressured() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (blocked, blocked_rx) = tokio::sync::oneshot::channel();
        let (stopped, stopped_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (events, receiver) = tokio::sync::mpsc::channel(1);
            let (inputs, _commands) = tokio::sync::mpsc::channel(1);
            let (stop, mut cancelled) = tokio::sync::watch::channel(false);
            let execution = async move {
                let produce = async {
                    for id in 0..64 {
                        events
                            .send(wire::agent::Event::Text {
                                item_id: id.to_string(),
                                text: "x".repeat(1024 * 1024),
                                replace: false,
                            })
                            .await
                            .unwrap();
                    }
                };
                assert!(
                    tokio::time::timeout(Duration::from_millis(200), produce)
                        .await
                        .is_err(),
                    "unread socket must backpressure event production"
                );
                blocked.send(()).unwrap();
                cancelled.wait_for(|value| *value).await.unwrap();
                stopped.send(()).unwrap();
                Ok(wire::agent::Outcome::Cancelled)
            };
            pump_agent(&mut socket, execution, receiver, inputs, stop).await
        });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), blocked_rx)
            .await
            .unwrap()
            .unwrap();
        crate::transport::send(
            &mut socket,
            &wire::Session::AgentInput {
                input: wire::agent::Input::Cancel,
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), stopped_rx)
            .await
            .unwrap()
            .unwrap();
        drop(socket);
        let _ = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn saturated_agent_inputs_preserve_order_and_cancel_without_failure() {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (_events, receiver) = tokio::sync::mpsc::channel(1);
                let (inputs, mut commands) = tokio::sync::mpsc::channel(1);
                let (stop, mut cancelled) = tokio::sync::watch::channel(false);
                let execution = async move {
                    if cancel {
                        cancelled.wait_for(|value| *value).await.unwrap();
                        assert_eq!(commands.len(), 1, "input channel was saturated");
                        Ok(wire::agent::Outcome::Cancelled)
                    } else {
                        for id in 0..64 {
                            let Some(wire::agent::Input::PlatformResult { request_id, .. }) =
                                commands.recv().await
                            else {
                                panic!("missing queued input")
                            };
                            assert_eq!(request_id, id.to_string());
                            tokio::task::yield_now().await;
                        }
                        Ok(wire::agent::Outcome::Completed)
                    }
                };
                pump_agent(&mut socket, execution, receiver, inputs, stop).await
            });
            let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap();
            for id in 0..64 {
                crate::transport::send(
                    &mut socket,
                    &wire::Session::AgentInput {
                        input: wire::agent::Input::PlatformResult {
                            request_id: id.to_string(),
                            result: serde_json::json!({}),
                        },
                    },
                )
                .await
                .unwrap();
            }
            if cancel {
                crate::transport::send(
                    &mut socket,
                    &wire::Session::AgentInput {
                        input: wire::agent::Input::Cancel,
                    },
                )
                .await
                .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "requires installed, authenticated native CLI; real model usage"]
    async fn live_native_agent_wire_forwards_approval_and_cancellation() {
        use wire::agent::{Event, Input, Outcome, Permission, Request, Runtime};
        let runtime = if std::env::var("AGENT_TEST_RUNTIME").as_deref() == Ok("claude") {
            Runtime::Claude
        } else {
            Runtime::Codex
        };
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::transport::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame, std::path::Path::new("."))
                .await
                .unwrap();
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        crate::transport::send(&mut client, &wire::Session::AgentStart { request: Request {
            binding: None,
            runtime, cwd: dir.path().to_string_lossy().into_owned(), model: None,
            native_session_id: None, permission: Permission::OnRequest, timeout_seconds: 120, resource_policy: None, platform_tools: false, provider: None, timezone: None, write_workdir: false,
            prompt: if runtime == Runtime::Claude {
                "Use only the Write tool to create wire-marker.txt containing CANCELLED.".into()
            } else {
                "Use only the shell tool to run exactly: powershell.exe -NoProfile -NonInteractive -Command \"[System.IO.File]::WriteAllText('wire-marker.txt','CANCELLED')\". Do not use apply_patch.".into()
            },
        }}).await.unwrap();
        let mut bound = false;
        let mut asked = false;
        loop {
            let frame: wire::Session = tokio::time::timeout(
                Duration::from_secs(130),
                crate::transport::next_frame(&mut client),
            )
            .await
            .unwrap()
            .unwrap();
            match frame {
                wire::Session::AgentEvent {
                    event: Event::Session { .. },
                } => bound = true,
                wire::Session::AgentEvent {
                    event: Event::Approval { .. },
                } => {
                    asked = true;
                    crate::transport::send(
                        &mut client,
                        &wire::Session::AgentInput {
                            input: Input::Cancel,
                        },
                    )
                    .await
                    .unwrap();
                }
                wire::Session::AgentEvent {
                    event: Event::Finished { outcome, .. },
                } => {
                    assert_eq!(outcome, Outcome::Cancelled);
                    break;
                }
                wire::Session::AgentEvent { .. } => {}
                other => panic!("unexpected frame {other:?}"),
            }
        }
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
        assert!(
            bound && asked,
            "actual native session and approval must cross the WebSocket"
        );
        assert!(!dir.path().join("wire-marker.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streaming_wire_delivers_live_output_and_acknowledges_cancel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::transport::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame, std::path::Path::new("."))
                .await
                .unwrap();
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        crate::transport::send(
            &mut client,
            &wire::Session::ExecStreamStart {
                command: "printf ready; sleep 120".into(),
                timeout_seconds: 30,
                timezone: None,
            },
        )
        .await
        .unwrap();
        let frame = tokio::time::timeout(
            Duration::from_secs(2),
            crate::transport::next_frame(&mut client),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(frame, wire::Session::ExecOutput {stream, data} if stream == "stdout" && data == b"ready")
        );
        crate::transport::send(&mut client, &wire::Session::ExecCancel)
            .await
            .unwrap();
        let frame = tokio::time::timeout(
            Duration::from_secs(12),
            crate::transport::next_frame(&mut client),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(frame, wire::Session::ExecExit {code} if code != 0));
        server.await.unwrap();
    }

    #[test]
    fn output_is_unlimited_by_default() {
        assert_eq!(capped(b"hello"), "hello");
        let long = vec![b'x'; 1024 * 1024 + 10];
        assert_eq!(capped(&long).as_bytes(), long);
    }

    #[test]
    fn a_pty_never_gets_a_size_a_shell_cannot_use() {
        let tiny = pty_size(0, 0);
        assert!(tiny.cols >= 2 && tiny.rows >= 1);
        let normal = pty_size(120, 40);
        assert_eq!((normal.cols, normal.rows), (120, 40));
    }

    #[cfg(windows)]
    #[test]
    fn terminal_starts_in_a_user_directory_not_a_service_profile() {
        assert!(is_service_profile(std::path::Path::new(
            r"C:\Windows\System32\config\systemprofile"
        )));
        assert!(is_service_profile(std::path::Path::new(
            r"C:\Windows\ServiceProfiles\LocalService"
        )));
        assert!(is_service_profile(std::path::Path::new(
            "C:/WINDOWS/system32/config/SYSTEMPROFILE/"
        )));
        let home = terminal_directory();
        assert!(home.is_dir());
        assert!(!is_service_profile(&home));
        if let Some(expected) = std::env::var_os("USERPROFILE") {
            let expected = std::path::PathBuf::from(expected);
            if expected.is_dir() && !is_service_profile(&expected) {
                assert_eq!(home, expected);
            }
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn quoted_paths_with_spaces_and_quoted_arguments_reach_cmd_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("dir with space");
        std::fs::create_dir(&folder).unwrap();
        let script = folder.join("say.cmd");
        std::fs::write(&script, "@echo [%~1]\r\n").unwrap();
        let command = format!("\"{}\" \"hello world\"", script.display());
        let output = shell_command(&command).output().await.unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", capped(&output.stderr));
        assert_eq!(capped(&output.stdout).trim(), "[hello world]");
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_real_exit_code() {
        let command = if cfg!(windows) {
            "echo out & echo err 1>&2 & exit /b 3"
        } else {
            "echo out; echo err 1>&2; exit 3"
        };
        let output = shell_command(command).output().await.unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(capped(&output.stdout).trim(), "out");
        assert_eq!(capped(&output.stderr).trim(), "err");
    }

    #[test]
    fn directories_sort_before_files() {
        let entry = |name: &str, kind: &str| wire::Entry {
            name: name.into(),
            kind: kind.into(),
            size: 0,
            mode: 0,
            modified_at: 0,
        };
        let mut entries = vec![
            entry("b.txt", "file"),
            entry("zz", "directory"),
            entry("a.txt", "file"),
            entry("aa", "directory"),
        ];
        entries.sort_by(|a, b| {
            (a.kind != "directory")
                .cmp(&(b.kind != "directory"))
                .then_with(|| a.name.cmp(&b.name))
        });
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["aa", "zz", "a.txt", "b.txt"]);
    }

    /// Serves one session request on a throwaway WebSocket and hands back the
    /// client end plus the dispatch outcome.
    async fn forward_session(
        port: u16,
    ) -> (
        crate::transport::Socket,
        tokio::task::JoinHandle<anyhow::Result<()>>,
    ) {
        forward_session_to(port, "").await
    }

    async fn forward_session_to(
        port: u16,
        host: &str,
    ) -> (
        crate::transport::Socket,
        tokio::task::JoinHandle<anyhow::Result<()>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::transport::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame, std::path::Path::new(".")).await
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        crate::transport::send(
            &mut client,
            &wire::Session::ForwardStart {
                port,
                host: host.to_owned(),
            },
        )
        .await
        .unwrap();
        (client, served)
    }

    #[tokio::test]
    async fn forward_pipes_bytes_both_ways_and_ends_with_the_local_service() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        let service = tokio::spawn(async move {
            let (mut stream, _) = local.accept().await.unwrap();
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
            // Closing here must end the session rather than leave it open.
        });
        let (mut client, served) = forward_session(port).await;
        let ready: wire::Session = crate::transport::next_frame(&mut client).await.unwrap();
        assert!(matches!(ready, wire::Session::ForwardReady));
        client
            .send(Message::Binary(b"ping".to_vec().into()))
            .await
            .unwrap();
        let mut received = Vec::new();
        while let Some(Ok(message)) = client.next().await {
            if let Message::Binary(bytes) = message {
                received.extend_from_slice(&bytes);
            }
        }
        assert_eq!(received, b"pong");
        service.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), served)
            .await
            .expect("session must end when the local service closes")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn forward_targets_are_limited_to_this_host_and_private_networks() {
        for allowed in [
            "127.0.0.1",
            "127.0.0.2",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.20",
            "100.64.0.1",
            "100.127.255.255",
            "::1",
            "fd12::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(forward_allowed(allowed.parse().unwrap()), "{allowed}");
        }
        for refused in [
            "8.8.8.8",
            "169.254.169.254",
            "172.32.0.1",
            "100.128.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "fe80::1",
            "2001:db8::1",
            "::",
            "::ffff:8.8.8.8",
        ] {
            assert!(!forward_allowed(refused.parse().unwrap()), "{refused}");
        }
    }

    #[tokio::test]
    async fn forward_reaches_a_named_local_target_and_refuses_public_ones() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        let service = tokio::spawn(async move {
            let (mut stream, _) = local.accept().await.unwrap();
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            stream.write_all(b"pong").await.unwrap();
        });
        // An explicit address goes through resolution and the private-range check.
        let (mut client, served) = forward_session_to(port, "127.0.0.1").await;
        let ready: wire::Session = crate::transport::next_frame(&mut client).await.unwrap();
        assert!(matches!(ready, wire::Session::ForwardReady));
        client
            .send(Message::Binary(b"ping".to_vec().into()))
            .await
            .unwrap();
        let mut received = Vec::new();
        while let Some(Ok(message)) = client.next().await {
            if let Message::Binary(bytes) = message {
                received.extend_from_slice(&bytes);
            }
        }
        assert_eq!(received, b"pong");
        service.await.unwrap();
        served.await.unwrap().unwrap();
        for public in ["8.8.8.8", "169.254.169.254", "a b"] {
            let (_client, served) = forward_session_to(port, public).await;
            let error = tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .expect("refusal is immediate")
                .unwrap()
                .unwrap_err();
            assert!(error.to_string().contains("预览目标"), "{public}: {error}");
        }
    }

    #[tokio::test]
    async fn closing_the_session_closes_the_local_connection() {
        use tokio::io::AsyncReadExt;
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        let service = tokio::spawn(async move {
            let (mut stream, _) = local.accept().await.unwrap();
            let mut rest = Vec::new();
            // Returns only once the runner releases its side.
            stream.read_to_end(&mut rest).await.unwrap();
            rest
        });
        let (mut client, served) = forward_session(port).await;
        let ready: wire::Session = crate::transport::next_frame(&mut client).await.unwrap();
        assert!(matches!(ready, wire::Session::ForwardReady));
        client
            .send(Message::Binary(b"partial".to_vec().into()))
            .await
            .unwrap();
        client.close(None).await.unwrap();
        let rest = tokio::time::timeout(Duration::from_secs(5), service)
            .await
            .expect("local connection must be released")
            .unwrap();
        assert_eq!(rest, b"partial");
        served.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn forward_to_a_closed_port_fails_instead_of_hanging() {
        let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = unused.local_addr().unwrap().port();
        drop(unused);
        let (_client, served) = forward_session(port).await;
        let outcome = tokio::time::timeout(Duration::from_secs(12), served)
            .await
            .unwrap()
            .unwrap();
        assert!(outcome.unwrap_err().to_string().contains(&port.to_string()));
    }

    #[tokio::test]
    async fn forward_rejects_port_zero() {
        let (_client, served) = forward_session(0).await;
        assert!(served.await.unwrap().is_err());
    }
}

#[cfg(test)]
mod process_timezone_tests {
    use super::*;
    #[tokio::test]
    async fn ordinary_process_timezone_is_child_only_and_streaming_matches() {
        let _serial = crate::timezone::TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        crate::timezone::pin(None);
        let parent = std::env::var_os("TZ");
        let zone = if crate::timezone::detected() == "Pacific/Chatham" {
            "Asia/Kathmandu"
        } else {
            "Pacific/Chatham"
        };
        let command = "node -p process.env.TZ";
        let output = shell_command_with_timezone(command, Some(zone))
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), zone);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let (_stop, cancelled) = tokio::sync::watch::channel(false);
        let (result, stdout) = tokio::join!(
            crate::exec_stream::run_with_timezone(command, 10, tx, cancelled, Some(zone)),
            async {
                let mut bytes = Vec::new();
                while let Some((kind, chunk)) = rx.recv().await {
                    if kind == "stdout" {
                        bytes.extend(chunk)
                    }
                }
                bytes
            }
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(String::from_utf8(stdout).unwrap().trim(), zone);
        assert_eq!(std::env::var_os("TZ"), parent);
        assert_eq!(
            terminal_command(Some(zone)).unwrap().get_env("TZ"),
            Some(std::ffi::OsStr::new(zone))
        );
        let custom = if cfg!(windows) {
            "set TZ=UTC&& node -p process.env.TZ"
        } else {
            "TZ=UTC node -p process.env.TZ"
        };
        let overridden = shell_command_with_timezone(custom, Some(zone))
            .output()
            .await
            .unwrap();
        assert_eq!(String::from_utf8(overridden.stdout).unwrap().trim(), "UTC");
        let mut inherited = shell_command(command);
        assert!(
            !inherited
                .as_std_mut()
                .get_envs()
                .any(|(key, _)| key == "TZ")
        );
        assert!(
            crate::exec_stream::run_with_timezone(
                command,
                10,
                tokio::sync::mpsc::channel(1).0,
                tokio::sync::watch::channel(false).1,
                Some("UTC;echo unsafe")
            )
            .await
            .is_err()
        );
    }
}
