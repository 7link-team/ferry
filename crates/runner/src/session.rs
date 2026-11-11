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
/// Captured command output is kept, so it has to be bounded.
const OUTPUT_LIMIT: usize = 1024 * 1024;
/// A directory listing is metadata for a person to look at, not a data dump.
const ENTRY_LIMIT: usize = 5000;

pub async fn run(
    url: &str,
    hello: wire::SessionHello,
    kind: wire::SessionKind,
    state_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let mut socket = crate::transport::dial(url).await?;
    crate::transport::send(&mut socket, &hello).await?;
    let request: wire::Session = crate::transport::next_frame(&mut socket).await?;
    tracing::info!(?kind, "会话连接已建立");

    let outcome = dispatch(&mut socket, request, state_dir).await;
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

async fn dispatch(
    socket: &mut Socket,
    request: wire::Session,
    state_dir: &std::path::Path,
) -> anyhow::Result<()> {
    match request {
        wire::Session::TerminalStart { cols, rows } => terminal(socket, cols, rows).await,
        wire::Session::ExecStart {
            command,
            timeout_seconds,
        } => exec(socket, &command, timeout_seconds).await,
        wire::Session::ExecStreamStart {
            command,
            timeout_seconds,
        } => exec_stream(socket, &command, timeout_seconds).await,
        wire::Session::ReadFile { path } => read_file(socket, &path).await,
        wire::Session::WriteFile { path, mode } => write_file(socket, &path, mode).await,
        wire::Session::ListDir { path } => list_dir(socket, &path).await,
        wire::Session::CreateFile { path } => create_file(socket, &path).await,
        wire::Session::CreateDir { path } => create_dir(socket, &path).await,
        wire::Session::DeleteFile { path } => delete_file(socket, &path).await,
        wire::Session::Monitor => monitor(socket).await,
        wire::Session::AgentStart { request } => agent(socket, request, state_dir).await,
        wire::Session::ForwardStart { port } => forward(socket, port).await,
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
            let environment = crate::agent::provision::environment(
                state_dir,
                runtime,
                &directory,
                prepare,
                binding.as_ref(),
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

/// Pipes a loopback port through the session: TCP bytes become binary frames
/// and back. Either side ending ends both, so neither connection outlives the
/// other.
async fn forward(socket: &mut Socket, port: u16) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    anyhow::ensure!(port != 0, "预览端口必须在 1-65535 之间");
    let connect = |address: std::net::SocketAddr| {
        tokio::time::timeout(FORWARD_CONNECT, tokio::net::TcpStream::connect(address))
    };
    // A dev server that listens on "localhost" may have bound only one family.
    let v4 = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    let v6 = std::net::SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port));
    let stream = match connect(v4).await {
        Ok(Ok(stream)) => stream,
        first => match connect(v6).await {
            Ok(Ok(stream)) => stream,
            _ => match first {
                Ok(Err(error)) => anyhow::bail!("无法连接本机端口 {port}：{error}"),
                _ => anyhow::bail!("连接本机端口 {port} 超时"),
            },
        },
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
    let (events, mut receiver) = mpsc::channel(16);
    let (inputs, commands) = mpsc::channel(16);
    let (stop, cancelled) = watch::channel(false);
    let execution = crate::agent::run_managed(state_dir, request, events, commands, cancelled);
    tokio::pin!(execution);
    let outcome = loop {
        tokio::select! {
            result = &mut execution => {
                // All events were queued before the adapter returned.
                while let Ok(event) = receiver.try_recv() {
                    tokio::time::timeout(Duration::from_secs(10), crate::transport::send(socket, &wire::Session::AgentEvent { event })).await??;
                }
                return result.map(|_| ());
            }
            event = receiver.recv() => if let Some(event) = event {
                let sent = tokio::time::timeout(Duration::from_secs(10), crate::transport::send(socket, &wire::Session::AgentEvent { event })).await;
                if !matches!(sent, Ok(Ok(()))) { break Err(anyhow::anyhow!("Agent event connection unavailable")); }
            },
            frame = socket.next() => match frame {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<wire::Session>(&text) {
                    Ok(wire::Session::AgentInput { input: wire::agent::Input::Cancel }) => { let _ = stop.send(true); }
                    Ok(wire::Session::AgentInput { input }) => {
                        if inputs.try_send(input).is_err() { break Err(anyhow::anyhow!("Agent input queue unavailable")); }
                    }
                    _ => break Err(anyhow::anyhow!("invalid Agent session frame")),
                },
                Some(Ok(Message::Ping(data))) => if socket.send(Message::Pong(data)).await.is_err() { break Ok(()); },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(()),
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

pub(crate) fn shell_command(command: &str) -> tokio::process::Command {
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
    process
}

fn capped(bytes: &[u8]) -> String {
    if bytes.len() <= OUTPUT_LIMIT {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = String::from_utf8_lossy(&bytes[..OUTPUT_LIMIT]).into_owned();
    text.push_str("\n…输出过长，已截断。");
    text
}

async fn exec(socket: &mut Socket, command: &str, timeout_seconds: u64) -> anyhow::Result<()> {
    let within = Duration::from_secs(timeout_seconds.clamp(1, 24 * 3600));
    let output = tokio::time::timeout(within, shell_command(command).output())
        .await
        .map_err(|_| anyhow::anyhow!("命令执行超过 {} 秒，已终止。", within.as_secs()))?
        .context("无法启动命令")?;
    crate::transport::send(
        socket,
        &wire::Session::ExecDone {
            // A command killed by a signal has no exit code; -1 says "did not
            // exit normally" rather than pretending it succeeded.
            code: output.status.code().unwrap_or(-1),
            stdout: capped(&output.stdout),
            stderr: capped(&output.stderr),
        },
    )
    .await
}

async fn exec_stream(socket: &mut Socket, command: &str, seconds: u64) -> anyhow::Result<()> {
    let (output, mut chunks) = tokio::sync::mpsc::channel(16);
    let (stop, cancelled) = tokio::sync::watch::channel(false);
    let execution = crate::exec_stream::run(command, seconds, output, cancelled);
    tokio::pin!(execution);
    let mut exit = None;
    loop {
        tokio::select! {
            result = &mut execution, if exit.is_none() => { exit = Some(result?); }
            chunk = chunks.recv() => match chunk {
                Some((stream, data)) => {
                    let sent = tokio::time::timeout(std::time::Duration::from_secs(10), crate::transport::send(socket, &wire::Session::ExecOutput {stream, data})).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        let _ = stop.send(true);
                        if exit.is_none() { let _ = execution.await; }
                        return match sent { Ok(Err(error))=>Err(error), _=>Err(anyhow::anyhow!("流水线日志传输超时")) };
                    }
                }
                None => {
                    let code = match exit {Some(code) => code, None => execution.await?};
                    return crate::transport::send(socket, &wire::Session::ExecExit {code}).await;
                }
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) if matches!(serde_json::from_str::<wire::Session>(&text), Ok(wire::Session::ExecCancel)) => { let _ = stop.send(true); }
                Some(Ok(Message::Ping(data))) => { socket.send(Message::Pong(data)).await?; }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    let _ = stop.send(true);
                    if exit.is_none() { let _ = execution.await; }
                    return Ok(());
                }
                _ => {}
            }
        }
    }
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

async fn write_file(socket: &mut Socket, path: &str, mode: u32) -> anyhow::Result<()> {
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

    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => {
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

async fn list_dir(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    let mut reader = tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("无法打开目录 {path}"))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await.context("读取目录时中断")? {
        if entries.len() >= ENTRY_LIMIT {
            break;
        }
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

async fn terminal(socket: &mut Socket, cols: u16, rows: u16) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let pty = portable_pty::native_pty_system()
        .openpty(pty_size(cols, rows))
        .context("无法分配伪终端")?;
    let mut command = portable_pty::CommandBuilder::new(login_shell());
    command.env("TERM", "xterm-256color");
    #[cfg(windows)]
    command.cwd(terminal_directory());
    let child = pty.slave.spawn_command(command).context("无法启动 shell")?;
    // Dropping the slave is what lets the reader see EOF when the shell exits.
    drop(pty.slave);

    let mut reader = pty.master.try_clone_reader()?;
    let mut writer = pty.master.take_writer()?;
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

    let mut shell_exited = false;
    loop {
        tokio::select! {
            chunk = from_shell.recv() => match chunk {
                Some(bytes) => if socket.send(Message::binary(bytes)).await.is_err() { break },
                // The shell exited and its output is drained.
                None => { shell_exited = true; break }
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Binary(bytes))) => {
                    if to_shell.send(bytes.to_vec()).is_err() { break }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Ok(wire::Session::TerminalResize { cols, rows }) =
                        serde_json::from_str::<wire::Session>(&text)
                        && pty.master.resize(pty_size(cols, rows)).is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }

    // Closing the browser tab must not leave a shell running on the host.
    let mut child = child;
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
            native_session_id: None, permission: Permission::OnRequest, timeout_seconds: 120, platform_tools: false,
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
    fn output_is_bounded_but_short_output_is_untouched() {
        assert_eq!(capped(b"hello"), "hello");
        let long = vec![b'x'; OUTPUT_LIMIT + 10];
        let text = capped(&long);
        assert!(text.len() < OUTPUT_LIMIT + 100, "must be truncated");
        assert!(
            text.ends_with("已截断。"),
            "and say so: {}",
            &text[text.len() - 40..]
        );
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
        crate::transport::send(&mut client, &wire::Session::ForwardStart { port })
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
