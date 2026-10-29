//! One session, one connection.
//!
//! The server opens with exactly one request frame and this module answers it.
//! Nothing here multiplexes, correlates or buffers a whole transfer: the frame
//! says what to do, bytes flow in whichever direction it implies, and the
//! connection closes when it is done.

use crate::Socket;
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
) -> anyhow::Result<()> {
    let mut socket = crate::dial(url).await?;
    crate::send(&mut socket, &hello).await?;
    let request: wire::Session = crate::next_frame(&mut socket).await?;
    tracing::info!(?kind, "会话连接已建立");

    let outcome = dispatch(&mut socket, request).await;
    if let Err(error) = &outcome {
        // The operator sees this sentence in the browser, so say it rather
        // than closing the socket and leaving them to guess.
        let _ = crate::send(
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

async fn dispatch(socket: &mut Socket, request: wire::Session) -> anyhow::Result<()> {
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
        other => anyhow::bail!("服务下发了不是请求的帧：{other:?}"),
    }
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
    crate::send(socket, &wire::Session::MonitorResult { metrics }).await
}

async fn create_file(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .with_context(|| format!("无法创建文件 {path}，请检查名称是否重复及目录权限。"))?;
    crate::send(socket, &wire::Session::Done).await
}

async fn create_dir(socket: &mut Socket, path: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir(path)
        .await
        .with_context(|| format!("无法创建文件夹 {path}，请检查名称是否重复及目录权限。"))?;
    crate::send(socket, &wire::Session::Done).await
}

// ponytail: no path validation here. The server already rejects relative and
// `..` paths, and a channel that can run `exec` has nothing left to protect by
// vetting a path — the runner trusts its own server by construction.

// ------------------------------------------------------------------ 命令

pub(crate) fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    let mut process = {
        let mut process = tokio::process::Command::new("cmd");
        process.arg("/C").arg(command);
        // CREATE_NO_WINDOW: a scheduled job must not flash a console over
        // whatever the person at that desktop is doing.
        use std::os::windows::process::CommandExt;
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
    crate::send(
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
                    let sent = tokio::time::timeout(std::time::Duration::from_secs(10), crate::send(socket, &wire::Session::ExecOutput {stream, data})).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        let _ = stop.send(true);
                        if exit.is_none() { let _ = execution.await; }
                        return match sent { Ok(Err(error))=>Err(error), _=>Err(anyhow::anyhow!("流水线日志传输超时")) };
                    }
                }
                None => {
                    let code = match exit {Some(code) => code, None => execution.await?};
                    return crate::send(socket, &wire::Session::ExecExit {code}).await;
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
    crate::send(
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
    crate::send(socket, &wire::Session::Eof).await
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
    crate::send(socket, &wire::Session::Done).await
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
    crate::send(socket, &wire::Session::DirEntries { entries }).await
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
    crate::send(socket, &wire::Session::Done).await
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

async fn terminal(socket: &mut Socket, cols: u16, rows: u16) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let pty = portable_pty::native_pty_system()
        .openpty(pty_size(cols, rows))
        .context("无法分配伪终端")?;
    let mut command = portable_pty::CommandBuilder::new(login_shell());
    command.env("TERM", "xterm-256color");
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
        let _ = crate::send(socket, &wire::Session::Eof).await;
    }
    // A terminal that ended is not a failure, however it ended.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn streaming_wire_delivers_live_output_and_acknowledges_cancel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let stream = tokio_tungstenite::MaybeTlsStream::Plain(stream);
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let frame = crate::next_frame(&mut socket).await.unwrap();
            dispatch(&mut socket, frame).await.unwrap();
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        crate::send(
            &mut client,
            &wire::Session::ExecStreamStart {
                command: "printf ready; sleep 120".into(),
                timeout_seconds: 30,
            },
        )
        .await
        .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(2), crate::next_frame(&mut client))
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(frame, wire::Session::ExecOutput {stream, data} if stream == "stdout" && data == b"ready")
        );
        crate::send(&mut client, &wire::Session::ExecCancel)
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(12), crate::next_frame(&mut client))
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
}
