//! Non-model CLI inspection; raw login output is never returned or logged.
use super::*;
use ferry_protocol::agent::Probe;

pub(super) fn executable(runtime: Runtime) -> Option<std::path::PathBuf> {
    let name = match runtime {
        Runtime::Codex => "codex",
        Runtime::Claude => "claude",
    };
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        // An empty PATH entry would search the Agent's working directory.
        if !directory.is_absolute() {
            continue;
        }
        let path = directory.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        });
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

async fn capture(
    _runtime: Runtime,
    executable: &std::path::Path,
    args: &[&str],
) -> Result<(bool, String)> {
    let mut command = native_command(executable);
    // Use the exact executable inspected above, not a current-directory shim.
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let job = crate::process_job::ProcessJob::attach(&child)?;
    #[cfg(unix)]
    let group = crate::exec_stream::ProcessGroup(child.id().context("missing native process id")?);
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let read = |pipe: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
            let mut bytes = Vec::new();
            pipe.take(64 * 1024 + 1).read_to_end(&mut bytes).await?;
            ensure!(bytes.len() <= 64 * 1024, "CLI inspection output too large");
            Ok::<_, anyhow::Error>(bytes)
        };
        let (mut out, err) = tokio::try_join!(read(Box::new(stdout)), read(Box::new(stderr)))?;
        out.extend(err);
        let status = child.wait().await?;
        Ok::<_, anyhow::Error>((status.success(), String::from_utf8_lossy(&out).into_owned()))
    })
    .await;
    #[cfg(windows)]
    let cleanup = job.terminate();
    #[cfg(unix)]
    let cleanup: Result<()> = {
        group.signal(true);
        Ok(())
    };
    let reaped = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    cleanup?;
    reaped.context("CLI inspection did not stop")??;
    result.context("CLI inspection timed out")?
}

pub(crate) async fn probe(runtime: Runtime, cwd: &str) -> Result<Probe> {
    use sha2::{Digest, Sha256};
    let mut identity = Sha256::new();
    for key in [
        "USERNAME",
        "USER",
        "USERDOMAIN",
        "USERPROFILE",
        "HOME",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        identity.update(key.as_bytes());
        identity.update([0]);
        if let Some(value) = std::env::var_os(key) {
            identity.update(value.to_string_lossy().as_bytes());
        }
        identity.update([0]);
    }
    #[cfg(unix)]
    identity.update(unsafe { libc::geteuid() }.to_le_bytes());
    ensure!(
        (cwd.is_empty() || std::path::Path::new(cwd).is_absolute())
            && cwd.len() <= 4096
            && !cwd.contains('\0'),
        "invalid Agent directory"
    );
    let directory_exists = tokio::fs::metadata(cwd).await.is_ok_and(|m| m.is_dir());
    let mut status = Probe {
        runtime,
        execution_identity: format!("{:x}", identity.finalize()),
        execution_user: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default(),
        executable: None,
        version: None,
        authentication: "unknown".into(),
        directory_exists,
        error: None,
        canonical_directory: tokio::fs::canonicalize(cwd)
            .await
            .ok()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned()),
    };
    let Some(path) = executable(runtime) else {
        status.error = Some("missing".into());
        return Ok(status);
    };
    status.executable = Some(path.to_string_lossy().into_owned());
    match capture(runtime, &path, &["--version"]).await {
        Ok((true, text)) => {
            status.version = text
                .lines()
                .next()
                .filter(|line| line.len() <= 256 && !line.chars().any(char::is_control))
                .map(str::to_owned)
        }
        _ => {
            status.error = Some("failed".into());
            return Ok(status);
        }
    }
    let args: &[&str] = match runtime {
        Runtime::Codex => &["login", "status"],
        Runtime::Claude => &["auth", "status", "--json"],
    };
    if let Ok((success, text)) = capture(runtime, &path, args).await {
        status.authentication = match runtime {
            Runtime::Codex if success && text.contains("Logged in using") => "logged_in",
            Runtime::Codex if !success && text.contains("Not logged in") => "logged_out",
            Runtime::Claude => match serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["loggedIn"].as_bool())
            {
                Some(true) => "logged_in",
                Some(false) => "logged_out",
                None => "unknown",
            },
            _ => "unknown",
        }
        .into();
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires native CLI installations and the current user's login"]
    async fn native_probe_reports_version_directory_and_login_without_model_usage() {
        let dir = tempfile::tempdir().unwrap();
        for runtime in [Runtime::Codex, Runtime::Claude] {
            let result = probe(runtime, &dir.path().to_string_lossy()).await.unwrap();
            assert!(
                result.executable.is_some() && result.version.is_some() && result.directory_exists
            );
            let key = match runtime {
                Runtime::Codex => "AGENT_EXPECT_CODEX_AUTH",
                Runtime::Claude => "AGENT_EXPECT_CLAUDE_AUTH",
            };
            let expected = std::env::var(key).unwrap_or_else(|_| "logged_in".into());
            assert_eq!(
                result.authentication, expected,
                "native login state for {runtime:?}"
            );
            assert!(result.error.is_none());
            let missing = probe(runtime, &dir.path().join("missing").to_string_lossy())
                .await
                .unwrap();
            assert!(!missing.directory_exists);
        }
    }
}
