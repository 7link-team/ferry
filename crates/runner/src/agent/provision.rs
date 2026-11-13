//! Opt-in, Runner-owned CLI installations. No global npm configuration is changed.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Policy {
    auto_install: bool,
    auto_upgrade: bool,
    isolate_config: bool,
    versions: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
pub(crate) struct Environment {
    pub executable: Option<PathBuf>,
    pub home: Option<PathBuf>,
}
impl Environment {
    pub fn apply(&self, command: &mut Command) {
        if let Some(home) = &self.home {
            for key in ["HOME", "USERPROFILE"] {
                command.env(key, native_path(home));
            }
            for (key, directory) in [
                ("APPDATA", "AppData/Roaming"),
                ("LOCALAPPDATA", "AppData/Local"),
                ("XDG_CONFIG_HOME", ".config"),
                ("XDG_CACHE_HOME", ".cache"),
                ("XDG_DATA_HOME", ".local/share"),
                ("XDG_STATE_HOME", ".local/state"),
                ("CODEX_HOME", ".codex"),
                ("CLAUDE_CONFIG_DIR", ".claude"),
                ("PI_CODING_AGENT_DIR", ".pi/agent"),
                ("DSH_HOME", ".dsh"),
                ("TMPDIR", "tmp"),
                ("TEMP", "tmp"),
                ("TMP", "tmp"),
            ] {
                command.env(key, native_path(&home.join(directory)));
            }
        }
    }
}

fn name(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Codex => "codex",
        Runtime::Claude => "claude",
        Runtime::Gemini => "gemini",
        Runtime::Qwen => "qwen",
        Runtime::Opencode => "opencode",
        Runtime::Pi => "pi",
        Runtime::Dsh => "dsh",
        Runtime::Kimi => "kimi",
    }
}
fn package(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Codex => "@openai/codex",
        Runtime::Claude => "@anthropic-ai/claude-code",
        Runtime::Gemini => "@google/gemini-cli",
        Runtime::Qwen => "@qwen-code/qwen-code",
        Runtime::Opencode => "opencode-ai",
        Runtime::Pi => "@mariozechner/pi-coding-agent",
        Runtime::Dsh => "@deepseek-ai/dsh",
        // Kimi is a Python CLI and never enters the npm installation path.
        Runtime::Kimi => "kimi-cli",
    }
}

fn policy(state: &Path) -> Result<Policy> {
    let path = state.join("agent-runtime.json");
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Policy::default()),
        Err(error) => return Err(error.into()),
    };
    use std::io::Read;
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "runtime policy must not be a symlink"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024, "runtime policy too large");
    let policy: Policy = serde_json::from_slice(&bytes).context("invalid Agent runtime policy")?;
    for (key, version) in &policy.versions {
        ensure!(Runtime::parse(key).is_some(), "unknown runtime policy key");
        valid_version(version)?;
    }
    Ok(policy)
}
fn valid_version(version: &str) -> Result<()> {
    ensure!(
        version.len() <= 128 && semver::Version::parse(version).is_ok(),
        "runtime requires an exact semantic version"
    );
    Ok(())
}

fn bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "runtime metadata too large");
    Ok(bytes)
}

/// Creates only plain directories underneath a canonical Runner-owned root.
async fn private_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
    let root = tokio::fs::canonicalize(root).await?;
    let mut path = root.clone();
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            anyhow::bail!("invalid private directory")
        };
        path.push(part);
        match tokio::fs::create_dir(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "private directory is not a plain directory"
        );
        let canonical = tokio::fs::canonicalize(&path).await?;
        ensure!(
            canonical.starts_with(&root),
            "private directory escaped Runner state"
        );
        path = canonical;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).await?;
        }
    }
    Ok(path)
}

async fn private_home(root: &Path, relative: &Path) -> Result<PathBuf> {
    let home = private_directory(root, relative).await?;
    for directory in [
        "tmp",
        ".codex",
        ".claude",
        ".config",
        ".cache",
        ".local/share",
        ".local/state",
        ".pi/agent",
        ".dsh",
        "AppData/Roaming",
        "AppData/Local",
    ] {
        private_directory(&home, Path::new(directory)).await?;
    }
    Ok(home)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    version: String,
    directory: String,
    checked_at: u64,
    #[serde(default)]
    requested: Option<String>,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn installed(root: &Path) -> Result<Option<Installed>> {
    let path = root.join("current.json");
    let bytes = match bounded_file(&path, 4096) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    ensure!(
        bytes.len() <= 4096 && !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "invalid installed runtime record"
    );
    let record: Installed = serde_json::from_slice(&bytes)?;
    valid_version(&record.version)?;
    ensure!(
        uuid::Uuid::parse_str(&record.directory).is_ok(),
        "invalid runtime installation directory"
    );
    Ok(Some(record))
}
fn publish(root: &Path, record: &Installed) -> Result<()> {
    use std::io::Write;
    let mut pending = tempfile::NamedTempFile::new_in(root)?;
    pending.write_all(&serde_json::to_vec(record)?)?;
    pending.as_file().sync_all()?;
    pending.persist(root.join("current.json"))?;
    Ok(())
}

fn platform_package(runtime: Runtime) -> String {
    let os = match std::env::consts::OS {
        "windows" => {
            if runtime == Runtime::Claude {
                "win32"
            } else {
                "windows"
            }
        }
        "macos" => "darwin",
        "linux" => "linux",
        _ => "unsupported",
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => "unsupported",
    };
    let musl = if cfg!(target_env = "musl") {
        "-musl"
    } else {
        ""
    };
    let baseline = if runtime == Runtime::Opencode && arch == "x64" {
        "-baseline"
    } else {
        ""
    };
    if runtime == Runtime::Claude {
        format!("@anthropic-ai/claude-code-{os}-{arch}{musl}")
    } else {
        format!("opencode-{os}-{arch}{baseline}{musl}")
    }
}

fn entry(root: &Path, runtime: Runtime) -> Result<PathBuf> {
    ensure!(
        !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
        "runtime installation must not be a symlink"
    );
    let base = root.join("node_modules").join(package(runtime));
    let metadata = base.join("package.json");
    ensure!(
        std::fs::metadata(&metadata)?.len() <= 128 * 1024,
        "CLI package metadata too large"
    );
    let value: Value = serde_json::from_slice(&bounded_file(&metadata, 128 * 1024)?)?;
    ensure!(
        value["name"].as_str() == Some(package(runtime)),
        "unexpected CLI package"
    );
    // These two packages publish native optional dependencies; their npm lifecycle
    // scripts copy the binary. Resolve the installed binary directly instead.
    if matches!(runtime, Runtime::Claude | Runtime::Opencode) {
        let platform = platform_package(runtime);
        let base = root.join("node_modules").join(&platform);
        if base.is_dir() {
            let metadata: Value =
                serde_json::from_slice(&bounded_file(&base.join("package.json"), 128 * 1024)?)?;
            ensure!(
                metadata["name"].as_str() == Some(&platform)
                    && metadata["version"] == value["version"],
                "native CLI package version mismatch"
            );
            let filename = format!(
                "{}{}",
                name(runtime),
                if cfg!(windows) { ".exe" } else { "" }
            );
            let binary = if runtime == Runtime::Claude {
                base.join(filename)
            } else {
                base.join("bin").join(filename)
            };
            let binary = std::fs::canonicalize(binary)?;
            ensure!(
                binary.is_file() && binary.starts_with(std::fs::canonicalize(root)?),
                "native CLI escaped installation"
            );
            return Ok(binary);
        }
    }
    let bin = value["bin"]
        .as_str()
        .or_else(|| value["bin"][name(runtime)].as_str())
        .context("CLI package has no executable")?;
    let relative = Path::new(bin);
    ensure!(
        !relative.is_absolute()
            && relative.components().all(|c| matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )),
        "invalid CLI package entry"
    );
    let result = std::fs::canonicalize(base.join(relative))?;
    ensure!(
        result.is_file() && result.starts_with(std::fs::canonicalize(root)?),
        "CLI executable escaped installation"
    );
    Ok(result)
}

fn npm() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("Node/npm is required for managed CLIs")?;
    for directory in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
        // npm ships a JS entry; never execute a .cmd wrapper or shell string.
        for candidate in [
            directory.join("node_modules/npm/bin/npm-cli.js"),
            directory.join("../lib/node_modules/npm/bin/npm-cli.js"),
            directory.join("../share/nodejs/npm/bin/npm-cli.js"),
        ] {
            if candidate.is_file() {
                return Ok(std::fs::canonicalize(candidate)?);
            }
        }
        #[cfg(unix)]
        {
            let candidate = directory.join("npm");
            if candidate.is_file() {
                let candidate = std::fs::canonicalize(candidate)?;
                if candidate.file_name().is_some_and(|n| n == "npm-cli.js") {
                    return Ok(candidate);
                }
            }
        }
    }
    anyhow::bail!("Node/npm is required for managed CLIs")
}

async fn latest(runtime: Runtime) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .get(format!(
            "https://registry.npmjs.org/{}/latest",
            package(runtime)
        ))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "CLI registry response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["name"].as_str() == Some(package(runtime)),
        "unexpected registry package"
    );
    let version = value["version"].as_str().context("missing CLI version")?;
    valid_version(version)?;
    Ok(version.into())
}

async fn install(root: &Path, runtime: Runtime, version: &str) -> Result<Installed> {
    valid_version(version)?;
    let directory = uuid::Uuid::new_v4().to_string();
    let target = private_directory(root, Path::new(&directory)).await?;
    let outcome = async {
        let home = private_home(&target, Path::new("installer-home")).await?;
        tokio::fs::write(home.join("empty.npmrc"), b"").await?;
        tokio::fs::write(home.join("global.npmrc"), b"").await?;
        let mut command = native_command(npm()?.as_os_str());
        Environment {
            executable: None,
            home: Some(home.clone()),
        }
        .apply(&mut command);
        command
            .args([
                "install",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--no-update-notifier",
                "--package-lock=false",
                "--registry=https://registry.npmjs.org",
            ])
            .arg("--userconfig")
            .arg(native_path(&home.join("empty.npmrc")))
            .arg("--globalconfig")
            .arg(native_path(&home.join("global.npmrc")))
            .arg("--cache")
            .arg(native_path(&home.join("cache")))
            .arg("--prefix")
            .arg(native_path(&target))
            .arg(format!("{}@{version}", package(runtime)))
            .current_dir(native_path(&target))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if runtime == Runtime::Opencode {
            // Both baseline and AVX2 variants match npm's os/cpu filters. Install
            // only the compatible native variant as an explicit dependency.
            command
                .arg("--omit=optional")
                .arg(format!("{}@{version}", platform_package(runtime)));
        }
        if runtime == Runtime::Dsh {
            command.arg("--engine-strict");
        }
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(test)]
        let mut diagnostic = tempfile::tempfile()?;
        #[cfg(test)]
        command.stderr(Stdio::from(diagnostic.try_clone()?));
        let mut child = command
            .spawn()
            .context("cannot start managed CLI installer")?;
        #[cfg(windows)]
        let job = crate::process_job::ProcessJob::attach(&child)?;
        #[cfg(unix)]
        let group =
            crate::exec_stream::ProcessGroup(child.id().context("missing installer process")?);
        let result = tokio::time::timeout(Duration::from_secs(300), child.wait()).await;
        #[cfg(windows)]
        job.terminate()?;
        #[cfg(unix)]
        group.signal(true);
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        #[cfg(test)]
        if !matches!(&result, Ok(Ok(status)) if status.success()) {
            use std::io::{Read, Seek};
            diagnostic.rewind()?;
            let mut bytes = Vec::new();
            diagnostic.take(8192).read_to_end(&mut bytes)?;
            eprintln!(
                "isolated installer status: {result:?}; diagnostic: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        ensure!(
            result.context("managed CLI install timed out")??.success(),
            "managed CLI installation failed"
        );
        let environment = Environment {
            executable: Some(entry(&target, runtime)?),
            home: Some(home),
        };
        let (success, text) = super::probe::capture_with(
            environment.executable.as_ref().unwrap(),
            &["--version"],
            &environment,
        )
        .await?;
        ensure!(
            success
                && text
                    .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
                    .any(|part| part == version),
            "installed CLI version check failed"
        );
        Ok(Installed {
            version: version.into(),
            directory: directory.clone(),
            checked_at: now(),
            requested: None,
        })
    }
    .await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_dir_all(&target).await;
    }
    outcome
}

async fn selected(
    root: &Path,
    runtime: Runtime,
    policy: &Policy,
    prepare: bool,
) -> Result<Option<PathBuf>> {
    if runtime == Runtime::Kimi {
        ensure!(
            !policy.versions.contains_key("kimi"),
            "Kimi versions are managed by its native Python installer"
        );
        return Ok(None);
    }
    let mut record = installed(root)?;
    if prepare
        && ((record.is_none() && policy.auto_install) || (record.is_some() && policy.auto_upgrade))
    {
        // One installation/pointer publication per runtime across Runner processes.
        let _lock = directory_lock(root).context("another CLI installation is in progress")?;
        record = installed(root)?;
        let desired = policy.versions.get(name(runtime));
        let due = record.as_ref().is_none_or(|r| {
            now().saturating_sub(r.checked_at) >= 86400 || desired != r.requested.as_ref()
        });
        if due
            && ((record.is_none() && policy.auto_install)
                || (record.is_some() && policy.auto_upgrade))
        {
            let update = async {
                let version = match desired {
                    Some(version) => version.clone(),
                    None => latest(runtime).await?,
                };
                if let Some(current) = &record {
                    if current.version == version {
                        let refreshed = Installed {
                            version,
                            directory: current.directory.clone(),
                            checked_at: now(),
                            requested: desired.cloned(),
                        };
                        publish(root, &refreshed)?;
                        return Ok(refreshed);
                    }
                }
                let mut installed = install(root, runtime, &version).await?;
                installed.requested = desired.cloned();
                publish(root, &installed)?;
                tracing::info!(runtime = name(runtime), version, "managed CLI installed");
                Ok::<_, anyhow::Error>(installed)
            }
            .await;
            match update {
                Ok(updated) => record = Some(updated),
                Err(error) if record.is_none() => return Err(error),
                Err(_) => {
                    tracing::warn!(
                        runtime = name(runtime),
                        "CLI upgrade failed; preserving installed version"
                    );
                    // Back off failures as well; do not hammer registry for each conversation.
                    if let Some(current) = &mut record {
                        current.checked_at = now();
                        current.requested = desired.cloned();
                        publish(root, current)?;
                    }
                }
            }
        }
    }
    record
        .map(|r| entry(&root.join(r.directory), runtime))
        .transpose()
}

pub(crate) async fn environment(
    state: &Path,
    runtime: Runtime,
    cwd: &str,
    prepare: bool,
    binding: Option<&ferry_protocol::agent::Binding>,
) -> Result<Environment> {
    let result: Result<Environment> = async {
        let policy = policy(state)?;
        ensure!(
            !policy.isolate_config || cwd.is_empty() || prepare || binding.is_some(),
            "configuration isolation requires the default managed conversation directory"
        );
        let root =
            private_directory(state, &Path::new("agent-runtimes").join(name(runtime))).await?;
        let mut executable = selected(&root, runtime, &policy, prepare)
            .await?
            .or_else(|| super::probe::executable(runtime));
        if let Some(binding) = binding {
            let bound = PathBuf::from(&binding.executable);
            if bound.starts_with(&root) {
                let relative = bound.strip_prefix(&root)?;
                let directory = relative
                    .components()
                    .next()
                    .context("invalid runtime binding")?;
                let std::path::Component::Normal(directory) = directory else {
                    anyhow::bail!("invalid runtime binding")
                };
                ensure!(
                    uuid::Uuid::parse_str(&directory.to_string_lossy()).is_ok(),
                    "invalid runtime binding"
                );
                let trusted = entry(&root.join(directory), runtime)?;
                ensure!(
                    std::fs::canonicalize(&bound)? == trusted,
                    "runtime binding changed"
                );
                executable = Some(trusted);
            } else if super::probe::executable(runtime).as_ref() == Some(&bound) {
                // Conversations created before managed installation keep their exact
                // native executable too. Never accept arbitrary server-supplied paths.
                executable = Some(bound);
            }
        }
        let home = if policy.isolate_config {
            let key = if cwd.is_empty() {
                format!("catalog-{}", name(runtime))
            } else {
                let cwd = tokio::fs::canonicalize(cwd).await?;
                let base = tokio::fs::canonicalize(state.join("workspaces/conversations")).await?;
                let key = cwd
                    .strip_prefix(&base)
                    .context("configuration isolation requires a managed conversation directory")?;
                ensure!(
                    key.components().count() == 1,
                    "configuration isolation requires a managed conversation directory"
                );
                // Reuse workspace's strict ID and symlink checks before accepting a manually supplied path.
                let trusted = workspace(state, &key.to_string_lossy()).await?;
                ensure!(trusted == cwd, "isolated directory changed");
                key.to_string_lossy()
                    .split_once('-')
                    .context("invalid managed Agent directory")?
                    .1
                    .to_owned()
            };
            let home = private_home(state, &Path::new("agent-configs").join(key)).await?;
            Some(home)
        } else {
            None
        };
        Ok(Environment { executable, home })
    }
    .await;
    result.map_err(|error| {
        tracing::warn!(runtime = name(runtime), error = %error, "Agent runtime preparation failed");
        error.context("agents.runtimeSetupFailed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_is_opt_in_and_rejects_unknown_or_unbounded_input() {
        let dir = tempfile::tempdir().unwrap();
        let p = policy(dir.path()).unwrap();
        assert!(!p.auto_install && !p.auto_upgrade && !p.isolate_config);
        for invalid in [
            r#"{"auto_instal":true}"#,
            r#"{"versions":{"codex":"latest"}}"#,
            r#"{"versions":{"other":"1.0.0"}}"#,
        ] {
            std::fs::write(dir.path().join("agent-runtime.json"), invalid).unwrap();
            assert!(policy(dir.path()).is_err());
        }
        std::fs::write(dir.path().join("agent-runtime.json"), vec![b' '; 16385]).unwrap();
        assert!(policy(dir.path()).is_err());
    }
    #[tokio::test]
    async fn agents_have_distinct_homes_and_conversations_reuse_only_the_same_agent_home() {
        let state = tempfile::tempdir().unwrap();
        std::fs::write(
            state.path().join("agent-runtime.json"),
            r#"{"isolate_config":true}"#,
        )
        .unwrap();
        let a = workspace(state.path(), "1-2").await.unwrap();
        let b = workspace(state.path(), "3-4").await.unwrap();
        assert!(
            environment(
                state.path(),
                Runtime::Codex,
                a.to_str().unwrap(),
                false,
                None
            )
            .await
            .is_err()
        );
        let binding = ferry_protocol::agent::Binding {
            execution_user: "fixture".into(),
            execution_identity: "fixture".into(),
            canonical_directory: a.to_string_lossy().into_owned(),
            executable: "fixture".into(),
        };
        let preflight = environment(
            state.path(),
            Runtime::Codex,
            a.to_str().unwrap(),
            false,
            Some(&binding),
        )
        .await
        .unwrap();
        let a = environment(
            state.path(),
            Runtime::Codex,
            a.to_str().unwrap(),
            true,
            None,
        )
        .await
        .unwrap();
        assert_eq!(preflight.home, a.home);
        let b = environment(
            state.path(),
            Runtime::Codex,
            b.to_str().unwrap(),
            true,
            None,
        )
        .await
        .unwrap();
        assert_ne!(a.home, b.home);
        let same_agent = workspace(state.path(), "5-2").await.unwrap();
        let same_agent = environment(
            state.path(),
            Runtime::Codex,
            same_agent.to_str().unwrap(),
            true,
            None,
        )
        .await
        .unwrap();
        assert_eq!(a.home, same_agent.home);
        assert!(
            environment(
                state.path(),
                Runtime::Codex,
                same_agent.home.as_ref().unwrap().to_str().unwrap(),
                false,
                None
            )
            .await
            .is_err()
        );
        assert!(a.home.as_ref().unwrap().join(".codex").is_dir());
        let mut command = native_command("not-executed");
        a.apply(&mut command);
        let home = command
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == "HOME")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(home, native_path(a.home.as_ref().unwrap()).as_os_str());
        let shared = tempfile::tempdir().unwrap();
        assert!(
            environment(
                state.path(),
                Runtime::Codex,
                shared.path().to_str().unwrap(),
                false,
                None
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn disabled_install_and_upgrade_never_use_network() {
        let state = tempfile::tempdir().unwrap();
        let root = private_directory(state.path(), Path::new("runtimes"))
            .await
            .unwrap();
        assert!(
            selected(&root, Runtime::Codex, &Policy::default(), true)
                .await
                .unwrap()
                .is_none()
        );
        assert!(installed(&root).unwrap().is_none());
    }
    #[tokio::test]
    async fn private_directories_reject_parent_components() {
        let state = tempfile::tempdir().unwrap();
        assert!(
            private_directory(state.path(), Path::new("../escape"))
                .await
                .is_err()
        );
    }
    #[test]
    fn atomic_record_replacement_and_invalid_records() {
        let state = tempfile::tempdir().unwrap();
        for version in ["1.0.0", "1.0.1"] {
            publish(
                state.path(),
                &Installed {
                    version: version.into(),
                    directory: uuid::Uuid::new_v4().to_string(),
                    checked_at: 1,
                    requested: None,
                },
            )
            .unwrap();
            assert_eq!(installed(state.path()).unwrap().unwrap().version, version);
        }
        std::fs::write(
            state.path().join("current.json"),
            r#"{"version":"1.0.0","directory":"../escape","checked_at":0}"#,
        )
        .unwrap();
        assert!(installed(state.path()).is_err());
    }

    async fn fixture_install(root: &Path, version: &str) -> Installed {
        let directory = uuid::Uuid::new_v4().to_string();
        let base = private_directory(
            root,
            &Path::new(&directory).join("node_modules/@openai/codex/bin"),
        )
        .await
        .unwrap();
        std::fs::write(base.join("codex.js"), "// fixture, never executed").unwrap();
        std::fs::write(base.parent().unwrap().join("package.json"), serde_json::json!({"name":"@openai/codex","version":version,"bin":{"codex":"bin/codex.js"}}).to_string()).unwrap();
        Installed {
            version: version.into(),
            directory,
            checked_at: now(),
            requested: None,
        }
    }

    #[tokio::test]
    async fn upgrading_pointer_preserves_old_binding_and_disabled_policy_is_offline() {
        let state = tempfile::tempdir().unwrap();
        let root = private_directory(state.path(), Path::new("agent-runtimes/codex"))
            .await
            .unwrap();
        let old = fixture_install(&root, "1.0.0").await;
        publish(&root, &old).unwrap();
        let old_path = entry(&root.join(&old.directory), Runtime::Codex).unwrap();
        let binding = ferry_protocol::agent::Binding {
            execution_user: "fixture".into(),
            execution_identity: "fixture".into(),
            canonical_directory: "fixture".into(),
            executable: old_path.to_string_lossy().into_owned(),
        };
        let new = fixture_install(&root, "2.0.0").await;
        publish(&root, &new).unwrap();
        let current = environment(state.path(), Runtime::Codex, "", false, None)
            .await
            .unwrap();
        assert_ne!(current.executable.as_ref(), Some(&old_path));
        let resumed = environment(state.path(), Runtime::Codex, "", false, Some(&binding))
            .await
            .unwrap();
        assert_eq!(resumed.executable, Some(old_path));
        let mut disabled = Policy::default();
        disabled
            .versions
            .insert("codex".into(), "999999.0.0".into());
        assert_eq!(
            selected(&root, Runtime::Codex, &disabled, true)
                .await
                .unwrap(),
            current.executable
        );
        let _held = directory_lock(&root).unwrap();
        disabled.auto_upgrade = true;
        assert!(
            selected(&root, Runtime::Codex, &disabled, true)
                .await
                .is_err()
        );
        assert_eq!(installed(&root).unwrap().unwrap().version, "2.0.0");
    }

    #[tokio::test]
    async fn package_entry_rejects_path_traversal() {
        let state = tempfile::tempdir().unwrap();
        let record = fixture_install(state.path(), "1.0.0").await;
        let root = state.path().join(record.directory);
        std::fs::write(
            root.join("node_modules/@openai/codex/package.json"),
            r#"{"name":"@openai/codex","bin":{"codex":"../../../../escape"}}"#,
        )
        .unwrap();
        assert!(entry(&root, Runtime::Codex).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_directories_reject_symlinks() {
        let state = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), state.path().join("agent-configs")).unwrap();
        assert!(
            private_home(state.path(), Path::new("agent-configs/test"))
                .await
                .is_err()
        );
        assert!(!outside.path().join("test").exists());
    }

    // Explicit opt-in: installs official CLI packages into a temporary Runner
    // state, never runs inference or copies this user's authentication files.
    #[tokio::test]
    #[ignore = "downloads official CLI packages; requires Node/npm and network"]
    async fn live_managed_install_isolation_and_failed_upgrade() {
        let runtime =
            Runtime::parse(&std::env::var("AGENT_TEST_RUNTIME").unwrap_or_else(|_| "codex".into()))
                .unwrap();
        let state = tempfile::tempdir().unwrap();
        let cwd = workspace(state.path(), "90001-90002").await.unwrap();
        let policy_path = state.path().join("agent-runtime.json");
        let mut initial = serde_json::json!({"auto_install":true,"isolate_config":true});
        if let Ok(version) = std::env::var("AGENT_TEST_VERSION") {
            initial["versions"] = serde_json::json!({name(runtime):version});
        }
        std::fs::write(&policy_path, initial.to_string()).unwrap();
        let first = environment(state.path(), runtime, cwd.to_str().unwrap(), true, None)
            .await
            .unwrap();
        let executable = first.executable.clone().unwrap();
        assert!(executable.starts_with(std::fs::canonicalize(state.path()).unwrap()));
        let probe = super::super::probe::probe_with(runtime, cwd.to_str().unwrap(), &first)
            .await
            .unwrap();
        assert_eq!(probe.error, None);
        assert!(probe.version.as_ref().is_some_and(|v| !v.is_empty()));
        if matches!(runtime, Runtime::Codex | Runtime::Claude) {
            assert_ne!(probe.authentication, "logged_in");
        }
        let binding = ferry_protocol::agent::Binding {
            execution_user: probe.execution_user,
            execution_identity: probe.execution_identity,
            executable: executable.to_string_lossy().into_owned(),
            canonical_directory: probe.canonical_directory.unwrap(),
        };
        // Disable both switches, including with an unavailable requested version.
        let unavailable = serde_json::json!({"auto_install":false,"auto_upgrade":false,"isolate_config":true,"versions":{name(runtime):"999999.0.0"}});
        std::fs::write(&policy_path, unavailable.to_string()).unwrap();
        assert_eq!(
            environment(state.path(), runtime, cwd.to_str().unwrap(), true, None)
                .await
                .unwrap()
                .executable,
            Some(executable.clone())
        );
        // A failed upgrade leaves the same binary usable and backs off retries.
        let mut upgrade = unavailable;
        upgrade["auto_upgrade"] = Value::Bool(true);
        std::fs::write(&policy_path, upgrade.to_string()).unwrap();
        let second = environment(state.path(), runtime, cwd.to_str().unwrap(), true, None)
            .await
            .unwrap();
        assert_eq!(second.executable, Some(executable.clone()));
        let root = std::fs::canonicalize(state.path())
            .unwrap()
            .join("agent-runtimes")
            .join(name(runtime));
        let record = installed(&root).unwrap().unwrap();
        assert_eq!(record.requested.as_deref(), Some("999999.0.0"));
        // The original conversation binding still resolves after policy changes.
        let resumed = environment(
            state.path(),
            runtime,
            cwd.to_str().unwrap(),
            false,
            Some(&binding),
        )
        .await
        .unwrap();
        assert_eq!(resumed.executable, Some(executable));
        assert_eq!(resumed.home, first.home);
        let probe = super::super::probe::probe_with(runtime, cwd.to_str().unwrap(), &resumed)
            .await
            .unwrap();
        assert_eq!(probe.execution_identity, binding.execution_identity);
        println!(
            "managed {:?}: version={} isolated=true disabled=true failed_upgrade_preserved=true",
            runtime,
            probe.version.unwrap()
        );
    }
}
