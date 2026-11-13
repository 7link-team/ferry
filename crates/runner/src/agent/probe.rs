//! Non-model CLI inspection; raw login output is never returned or logged.
use super::*;
use ferry_protocol::agent::Probe;

pub(super) fn executable(runtime: Runtime) -> Option<std::path::PathBuf> {
    let name = match runtime {
        Runtime::Codex => "codex",
        Runtime::Claude => "claude",
        Runtime::Gemini => "gemini",
        Runtime::Qwen => "qwen",
        Runtime::Opencode => "opencode",
        Runtime::Pi => "pi",
        Runtime::Dsh => "dsh",
        Runtime::Kimi => "kimi",
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
        #[cfg(windows)]
        {
            // npm's .cmd wrappers require a shell. Resolve only the named
            // package's declared JS entry and launch it through node.exe.
            let packages: &[&str] = match runtime {
                Runtime::Gemini => &["@google/gemini-cli"],
                Runtime::Qwen => &["@qwen-code/qwen-code"],
                Runtime::Dsh => &["@deepseek-ai/dsh"],
                Runtime::Pi => &[
                    "@earendil-works/pi-coding-agent",
                    "@mariozechner/pi-coding-agent",
                ],
                _ => &[],
            };
            if runtime == Runtime::Opencode {
                let package = if cfg!(target_arch = "aarch64") {
                    "opencode-windows-arm64"
                } else {
                    "opencode-windows-x64-baseline"
                };
                let bundled = directory
                    .join("node_modules")
                    .join(package)
                    .join("bin/opencode.exe");
                if bundled.is_file() {
                    return Some(bundled);
                }
                let bundled = directory.join("node_modules/opencode-ai/bin/opencode.exe");
                if bundled.is_file() {
                    return Some(bundled);
                }
            }
            for package in packages {
                let base = directory.join("node_modules").join(package);
                let metadata = base.join("package.json");
                if std::fs::metadata(&metadata).is_ok_and(|m| m.len() <= 128 * 1024) {
                    if let Ok(text) = std::fs::read_to_string(metadata) {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if let Some(bin) = value["bin"][name].as_str() {
                                let relative = std::path::Path::new(bin);
                                if !relative.is_absolute()
                                    && relative.components().all(|part| {
                                        matches!(
                                            part,
                                            std::path::Component::Normal(_)
                                                | std::path::Component::CurDir
                                        )
                                    })
                                    && relative.extension().is_some_and(|ext| {
                                        matches!(ext.to_str(), Some("js" | "mjs" | "cjs"))
                                    })
                                {
                                    let entry = base.join(relative);
                                    if entry.is_file() {
                                        return Some(entry);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub(super) async fn capture_with(
    executable: &std::path::Path,
    args: &[&str],
    environment: &provision::Environment,
) -> Result<(bool, String)> {
    let mut command = native_command(executable);
    environment.apply(&mut command);
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

#[cfg(test)]
pub(crate) async fn probe(runtime: Runtime, cwd: &str) -> Result<Probe> {
    probe_with(
        runtime,
        cwd,
        &provision::Environment {
            executable: executable(runtime),
            home: None,
        },
    )
    .await
}

pub(crate) async fn probe_with(
    runtime: Runtime,
    cwd: &str,
    environment: &provision::Environment,
) -> Result<Probe> {
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
    ]
    .into_iter()
    .chain((runtime == Runtime::Dsh).then_some("DSH_HOME"))
    {
        identity.update(key.as_bytes());
        identity.update([0]);
        if let Some(value) = std::env::var_os(key) {
            identity.update(value.to_string_lossy().as_bytes());
        }
        identity.update([0]);
    }
    #[cfg(unix)]
    identity.update(unsafe { libc::geteuid() }.to_le_bytes());
    if let Some(home) = &environment.home {
        identity.update(home.to_string_lossy().as_bytes());
    }
    ensure!(
        (cwd.is_empty() || std::path::Path::new(cwd).is_absolute())
            && cwd.len() <= 4096
            && !cwd.contains('\0'),
        "invalid Agent directory"
    );
    let directory_exists = tokio::fs::metadata(cwd).await.is_ok_and(|m| m.is_dir());
    let mut status = Probe {
        platform_tools: true,
        runtime,
        execution_identity: format!("{:x}", identity.finalize()),
        execution_user: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default(),
        executable: None,
        version: None,
        models: Vec::new(),
        authentication: "unknown".into(),
        directory_exists,
        error: None,
        canonical_directory: tokio::fs::canonicalize(cwd)
            .await
            .ok()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned()),
    };
    let Some(path) = environment.executable.as_ref() else {
        status.error = Some("missing".into());
        return Ok(status);
    };
    status.executable = Some(path.to_string_lossy().into_owned());
    // An empty cwd is an explicit catalog inspection, never a turn preflight.
    if cwd.is_empty() && environment.home.is_none() {
        status.models = native_models(runtime, &path).await;
    }
    match capture_with(path, &["--version"], environment).await {
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
        // No stable, non-inference authentication-status command is defined.
        // Preserve unknown; actual session admission reports authentication errors.
        _ => return Ok(status),
    };
    if let Ok((success, text)) = capture_with(path, args, environment).await {
        status.authentication = match runtime {
            Runtime::Codex if codex_authentication(success, &text) => "logged_in",
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

async fn native_models(runtime: Runtime, executable: &std::path::Path) -> Vec<String> {
    // Codex maintains this non-secret catalog for its own model picker. Never
    // inspect auth.json or return arbitrary native configuration to the server.
    if runtime == Runtime::Pi {
        return rpc_models(runtime, executable).await.unwrap_or_default();
    }
    if runtime != Runtime::Codex {
        return Vec::new();
    }
    let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) else {
        return Vec::new();
    };
    let path = std::path::PathBuf::from(home).join(".codex/models_cache.json");
    let Ok(file) = tokio::fs::File::open(path).await else {
        return rpc_models(runtime, executable).await.unwrap_or_default();
    };
    let mut bytes = Vec::new();
    if file
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .is_err()
        || bytes.len() > 1024 * 1024
    {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    catalog_ids(&value)
}

async fn rpc_models(runtime: Runtime, executable: &std::path::Path) -> Result<Vec<String>> {
    let mut command = native_command(executable);
    if runtime == Runtime::Pi {
        command.args([
            "--mode",
            "rpc",
            "--no-session",
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
        ]);
    } else {
        command.arg("app-server");
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let job = crate::process_job::ProcessJob::attach(&child)?;
    #[cfg(unix)]
    let group = crate::exec_stream::ProcessGroup(child.id().context("missing catalog process")?);
    let mut input = child.stdin.take().context("catalog stdin")?;
    let mut output = BufReader::new(child.stdout.take().context("catalog stdout")?);
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        write(&mut input, if runtime == Runtime::Pi { serde_json::json!({"id":1,"type":"get_available_models"}) }
            else { serde_json::json!({"id":0,"method":"initialize","params":{"clientInfo":{"name":"model_catalog","version":env!("CARGO_PKG_VERSION")}}}) }).await?;
        let mut buffer = Vec::new();
        let mut total = 0;
        while let Some(value) = read(&mut output, &mut buffer).await? {
            buffer.clear();
            total += value.to_string().len();
            ensure!(total <= 1024 * 1024, "catalog too large");
            if value["id"] == 0 {
                ensure!(value.get("error").is_none(), "catalog initialization failed");
                write(&mut input, serde_json::json!({"method":"initialized","params":{}})).await?;
                write(&mut input, serde_json::json!({"id":1,"method":"model/list","params":{"limit":2000}})).await?;
            } else if value["id"] == 1 {
                if runtime == Runtime::Pi {
                    ensure!(value["type"] == "response" && value["success"] == true, "Pi catalog failed");
                    let rows = value["data"]["models"].as_array().context("Pi catalog missing")?.iter().map(|model| {
                        let id = model["id"].as_str().unwrap_or("");
                        let id = model["provider"].as_str().filter(|provider| !provider.is_empty())
                            .map_or_else(|| id.to_owned(), |provider| format!("{provider}/{id}"));
                        serde_json::json!({"id":id})
                    }).collect::<Vec<_>>();
                    return Ok(catalog_ids(&serde_json::json!({"models":rows})));
                }
                ensure!(value.get("error").is_none(), "catalog request failed");
                return Ok(catalog_ids(&serde_json::json!({"models":value["result"]["data"]})));
            }
        }
        anyhow::bail!("catalog closed")
    }).await;
    #[cfg(windows)]
    job.terminate()?;
    #[cfg(unix)]
    group.signal(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    result.context("catalog timed out")?
}

fn catalog_ids(value: &Value) -> Vec<String> {
    let mut models = value["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["visibility"] != "hide")
        .filter_map(|v| v["slug"].as_str().or_else(|| v["id"].as_str()))
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && !id.starts_with('-')
                && !id.chars().any(char::is_control)
        })
        .take(2000)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    models
}

fn codex_authentication(success: bool, text: &str) -> bool {
    success
        && (text.contains("Logged in using")
            || text
                .lines()
                .any(|line| line.starts_with("API key configured - ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires an installed native Codex CLI and its execution user's configuration"]
    async fn native_codex_catalog_is_discoverable_without_inference() {
        let executable = executable(Runtime::Codex).expect("native Codex executable");
        let models = rpc_models(Runtime::Codex, &executable)
            .await
            .expect("native model/list");
        assert!(
            !models.is_empty(),
            "native model/list returned no identifiers"
        );
    }
    #[test]
    fn codex_accepts_configured_api_auth_without_exposing_status_text() {
        assert!(codex_authentication(true, "Logged in using ChatGPT"));
        assert!(codex_authentication(
            true,
            "API key configured - masked\nAuthentication is checked when you send a request."
        ));
        assert!(!codex_authentication(false, "API key configured - masked"));
        assert!(!codex_authentication(true, "Not logged in"));
        assert!(!codex_authentication(
            true,
            "Authentication is checked when you send a request."
        ));
    }
    #[tokio::test]
    async fn existing_runtime_bindings_keep_their_original_identity_hash() {
        use sha2::{Digest, Sha256};
        let mut legacy = Sha256::new();
        for key in [
            "USERNAME",
            "USER",
            "USERDOMAIN",
            "USERPROFILE",
            "HOME",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            legacy.update(key.as_bytes());
            legacy.update([0]);
            if let Some(value) = std::env::var_os(key) {
                legacy.update(value.to_string_lossy().as_bytes());
            }
            legacy.update([0]);
        }
        #[cfg(unix)]
        legacy.update(unsafe { libc::geteuid() }.to_le_bytes());
        let expected = format!("{:x}", legacy.finalize());
        for runtime in [
            Runtime::Codex,
            Runtime::Claude,
            Runtime::Gemini,
            Runtime::Qwen,
            Runtime::Opencode,
            Runtime::Pi,
        ] {
            let result = probe_with(runtime, "", &provision::Environment::default())
                .await
                .unwrap();
            assert_eq!(result.execution_identity, expected, "{runtime:?}");
        }
    }
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
                _ => unreachable!(),
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
