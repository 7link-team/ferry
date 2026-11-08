//! Bounded, cancellable command streaming for long-running pipeline workers.

use anyhow::Context;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, watch},
};

pub type Chunk = (String, Vec<u8>);

#[cfg(unix)]
pub(crate) struct ProcessGroup(pub(crate) u32);
#[cfg(unix)]
impl ProcessGroup {
    pub(crate) fn signal(&self, force: bool) {
        #[cfg(unix)]
        unsafe {
            libc::kill(
                -(self.0 as i32),
                if force { libc::SIGKILL } else { libc::SIGTERM },
            );
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.signal(true);
    }
}

/// Output stays as bytes until the server's per-stream decoder/redactor consumes it.
/// Closing the consumer/cancellation channel terminates the owned process group.
pub async fn run(
    command: &str,
    seconds: u64,
    output: mpsc::Sender<Chunk>,
    mut cancel: watch::Receiver<bool>,
) -> anyhow::Result<i32> {
    let mut process = crate::session::shell_command(command);
    process
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process.spawn().context("无法启动流水线命令")?;
    // A root PID can exit before taskkill visits its descendants. The job owns
    // those descendants independently, including when the Agent itself exits.
    #[cfg(windows)]
    let job = crate::process_job::ProcessJob::attach(&child)?;
    #[cfg(unix)]
    let group = ProcessGroup(child.id().context("命令没有进程标识")?);
    let (mut stdout, mut stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let (mut outbuf, mut errbuf) = ([0u8; 8192], [0u8; 8192]);
    let (mut out_open, mut err_open) = (true, true);
    let deadline = tokio::time::sleep(Duration::from_secs(seconds.clamp(1, 5 * 24 * 3600)));
    tokio::pin!(deadline);
    let mut status = None;
    let stopped = loop {
        if *cancel.borrow() {
            break -130;
        }
        if !out_open
            && !err_open
            && let Some(code) = status
        {
            return Ok(code);
        }
        let chunk = tokio::select! {
            value = child.wait(), if status.is_none() => { status = Some(value?.code().unwrap_or(-1)); continue; }
            _ = cancel.changed() => { break -130; }
            _ = &mut deadline => { break -124; }
            value = stdout.read(&mut outbuf), if out_open => {
                let size = value?; if size == 0 { out_open = false; continue; }
                ("stdout".to_owned(), outbuf[..size].to_vec())
            }
            value = stderr.read(&mut errbuf), if err_open => {
                let size = value?; if size == 0 { err_open = false; continue; }
                ("stderr".to_owned(), errbuf[..size].to_vec())
            }
        };
        tokio::select! {
            result = output.send(chunk) => { if result.is_err() { break -130; } }
            _ = cancel.changed() => { break -130; }
            _ = &mut deadline => { break -124; }
        }
    };
    #[cfg(unix)]
    group.signal(false);
    #[cfg(windows)]
    job.terminate()?;
    if status.is_none()
        && tokio::time::timeout(Duration::from_secs(70), child.wait())
            .await
            .is_err()
    {
        #[cfg(unix)]
        group.signal(true);
        #[cfg(windows)]
        job.terminate()?;
        let _ = child.wait().await;
    }
    Ok(stopped)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    async fn descendants_stop(cancel_early: bool) {
        let (output, mut chunks) = mpsc::channel(8);
        let (stop, cancelled) = watch::channel(false);
        let command = r#"powershell.exe -NoProfile -NonInteractive -Command "$p = Start-Process -FilePath ping.exe -ArgumentList '127.0.0.1','-t' -WindowStyle Hidden -PassThru; Write-Output $p.Id; Wait-Process -Id $p.Id""#;
        let task = tokio::spawn(run(
            command,
            if cancel_early { 30 } else { 3 },
            output,
            cancelled,
        ));
        let mut stdout = Vec::new();
        while !stdout.contains(&b'\n') {
            let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), chunks.recv())
                .await
                .unwrap()
                .unwrap();
            if kind == "stdout" {
                stdout.extend(bytes);
            }
        }
        let pid = String::from_utf8(stdout)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(
            !handle.is_null(),
            "descendant must be alive before termination"
        );
        if cancel_early {
            stop.send(true).unwrap();
        }
        let code = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ended = unsafe { WaitForSingleObject(handle, 5000) };
        unsafe {
            CloseHandle(handle);
        }
        assert_eq!(code, if cancel_early { -130 } else { -124 });
        assert_eq!(
            ended, 0,
            "command must not report termination while its descendant is alive"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_windows_descendants_even_after_root_exit() {
        descendants_stop(true).await;
    }

    #[tokio::test]
    async fn timeout_stops_windows_descendants_even_after_root_exit() {
        descendants_stop(false).await;
    }

    #[tokio::test]
    async fn successful_windows_commands_keep_exit_code_and_both_streams() {
        let (output, mut chunks) = mpsc::channel(8);
        let (_stop, cancelled) = watch::channel(false);
        let code = run(
            "echo stdout & echo stderr 1>&2 & exit /b 7",
            10,
            output,
            cancelled,
        )
        .await
        .unwrap();
        assert_eq!(code, 7);
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while let Some((kind, bytes)) = chunks.recv().await {
            if kind == "stdout" {
                stdout.extend(bytes);
            } else {
                stderr.extend(bytes);
            }
        }
        assert_eq!(String::from_utf8(stdout).unwrap().trim(), "stdout");
        assert_eq!(String::from_utf8(stderr).unwrap().trim(), "stderr");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::{mpsc, watch};

    #[tokio::test]
    async fn cancellation_allows_worker_cleanup_before_forced_exit() {
        let marker = std::env::temp_dir().join(format!(
            "stream-cleanup-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let command = format!(
            "exec sh -c 'trap \"sleep 6; touch {} ; exit 0\" TERM; echo ready; while :; do sleep 1; done'",
            marker.display()
        );
        let (output, mut chunks) = mpsc::channel(8);
        let (stop, cancelled) = watch::channel(false);
        let task = tokio::spawn(async move { run(&command, 30, output, cancelled).await });
        chunks.recv().await.expect("worker ready");
        stop.send(true).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(12), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            -130
        );
        let cleaned = marker.exists();
        let _ = std::fs::remove_file(marker);
        assert!(
            cleaned,
            "worker must finish its graceful cleanup before forced termination"
        );
    }

    #[tokio::test]
    async fn output_arrives_before_exit_and_cancel_stops_descendants() {
        let (output, mut chunks) = mpsc::channel(4);
        let (stop, cancelled) = watch::channel(false);
        let task = tokio::spawn(run(
            "sleep 120 & printf '%s\\n' \"$!\"; wait",
            30,
            output,
            cancelled,
        ));
        let (stream, bytes) = tokio::time::timeout(Duration::from_secs(2), chunks.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stream, "stdout");
        let pid = String::from_utf8(bytes).unwrap().trim().to_owned();
        assert!(
            !task.is_finished(),
            "output must arrive while command is alive"
        );
        stop.send(true).unwrap();
        assert_ne!(
            tokio::time::timeout(Duration::from_secs(12), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            0
        );
        for _ in 0..30 {
            let alive = tokio::process::Command::new("/bin/kill")
                .args(["-0", &pid])
                .output()
                .await
                .unwrap()
                .status
                .success();
            if !alive {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("cancelled command left its descendant alive");
    }

    #[tokio::test]
    async fn exit_code_and_both_streams_are_preserved() {
        let (output, mut chunks) = mpsc::channel(8);
        let (_stop, cancelled) = watch::channel(false);
        let code = run(
            "printf 'hello'; printf 'error' >&2; exit 7",
            10,
            output,
            cancelled,
        )
        .await
        .unwrap();
        assert_eq!(code, 7);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        while let Some((stream, bytes)) = chunks.recv().await {
            if stream == "stdout" {
                out.extend(bytes);
            } else {
                err.extend(bytes);
            }
        }
        assert_eq!(out, b"hello");
        assert_eq!(err, b"error");
    }

    #[tokio::test]
    async fn timeout_terminates_command_instead_of_only_stopping_wait() {
        let (output, _chunks) = mpsc::channel(4);
        let (_stop, cancelled) = watch::channel(false);
        let code = tokio::time::timeout(
            Duration::from_secs(12),
            run("sleep 120", 1, output, cancelled),
        )
        .await
        .unwrap()
        .unwrap();
        assert_ne!(code, 0);
    }
}
