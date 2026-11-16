//! Stable native-service parent. Only authenticated, live workers become current.
use super::Settings;
use crate::update::Installed;
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::watch,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Journal {
    current: Option<Installed>,
    previous: Option<Installed>,
    pending: Option<Installed>,
    failed_version: Option<String>,
}
impl Journal {
    fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join("versions.json");
        if !super::regular_file(&path)? {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    fn store(&self, root: &Path) -> anyhow::Result<()> {
        let mut tmp = tempfile::NamedTempFile::new_in(root)?;
        tmp.write_all(&serde_json::to_vec(self)?)?;
        tmp.as_file().sync_all()?;
        let temp = tmp.into_temp_path();
        crate::update::durable_rename(&temp, &root.join("versions.json"), true)?;
        #[cfg(unix)]
        std::fs::File::open(root)?.sync_all()?;
        Ok(())
    }
    fn rollback(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.failed_version = Some(pending.version);
        }
    }
    fn confirm(&mut self) {
        self.previous = self.current.take();
        self.current = self.pending.take();
        self.failed_version = None;
    }
    /// A reinstalled service binary at least as new as the staged versions wins:
    /// staged older versions stop being selected, and a failure record only
    /// survives when it concerns a newer version. Returns whether anything changed.
    fn reconcile(&mut self, installed: &str) -> bool {
        let Ok(installed) = semver::Version::parse(installed) else {
            return false;
        };
        // An unreadable staged version cannot outrank the installed binary.
        let not_newer = |version: &str| {
            semver::Version::parse(version)
                .map(|version| installed.cmp_precedence(&version).is_ge())
                .unwrap_or(true)
        };
        let mut changed = false;
        if self.pending.as_ref().is_some_and(|v| not_newer(&v.version)) {
            self.pending = None;
            changed = true;
        }
        if self.current.as_ref().is_some_and(|v| not_newer(&v.version)) {
            self.current = None;
            self.previous = None;
            changed = true;
        }
        if self.current.is_none()
            && self
                .previous
                .as_ref()
                .is_some_and(|v| not_newer(&v.version))
        {
            self.previous = None;
            changed = true;
        }
        if self.failed_version.as_deref().is_some_and(not_newer) {
            self.failed_version = None;
            changed = true;
        }
        changed
    }
}

/// Makes the installed binary win over older staged versions; used when the
/// service is (re)installed and whenever the supervisor starts.
pub(super) fn reconcile_installed(root: &Path, installed: &str) -> anyhow::Result<()> {
    let mut journal = Journal::load(root)?;
    if journal.reconcile(installed) {
        journal.store(root)?;
        clear_failure(root);
    }
    Ok(())
}

/// Why the last activation failed, kept beside the journal (older supervisors
/// reject unknown journal fields) and handed to the restarted worker.
#[derive(Serialize, Deserialize)]
struct Failure {
    version: String,
    reason: String,
}
const FAILURE_FILE: &str = "activation-failure.json";
fn sanitize(reason: &str) -> String {
    reason
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn record_failure(root: &Path, version: &str, reason: &str) {
    let failure = Failure {
        version: version.into(),
        reason: sanitize(reason),
    };
    if let Ok(bytes) = serde_json::to_vec(&failure) {
        let _ = std::fs::write(root.join(FAILURE_FILE), bytes);
    }
}
fn failure_reason(root: &Path, version: Option<&str>) -> Option<String> {
    let version = version?;
    let path = root.join(FAILURE_FILE);
    if !super::regular_file(&path).unwrap_or(false) {
        return None;
    }
    let failure: Failure = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (failure.version == version).then(|| sanitize(&failure.reason))
}
fn clear_failure(root: &Path) {
    let _ = std::fs::remove_file(root.join(FAILURE_FILE));
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Event {
    Ready { version: String },
    Activate { version: String, digest: String },
}

pub(crate) fn emit(event: &Event) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, event)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

// Reuse the same signal semantics as the foreground runner, without assuming a
// child receives SCM stop controls or Unix service-manager signals itself.
pub(super) fn foreground(config: PathBuf) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let (stop, stopped) = watch::channel(false);
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    let mut term =
                        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                            .expect("SIGTERM handler");
                    tokio::select! { _=term.recv()=>{}, _=tokio::signal::ctrl_c()=>{} }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                stop.send_replace(true);
            });
            run(config, stopped).await
        })
}

fn spawn(
    binary: &Path,
    config: &Path,
    name: &str,
    failed: Option<&str>,
    reason: Option<&str>,
) -> anyhow::Result<Child> {
    let mut command = Command::new(binary);
    command
        .args(["service", "worker", "--config"])
        .arg(config)
        .arg("--name")
        .arg(name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(version) = failed {
        command.arg("--failed-version").arg(version);
    }
    // An environment variable, not a flag: the worker restarted after a rollback
    // is the previous version, which may not know any new flag.
    command.env_remove("FERRY_FAILED_REASON");
    if let Some(reason) = reason.filter(|_| failed.is_some()) {
        command.env("FERRY_FAILED_REASON", reason);
    }
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let log = config.with_file_name("service.log");
        super::regular_file(&log)?;
        command.stderr(Stdio::from(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)?,
        ));
    }
    Ok(command.spawn()?)
}

enum Outcome {
    Retry,
    Stopped,
    Activate(Installed),
    /// Why the worker did not become (or stay) healthy, as `code[:detail]`.
    Failed(String),
}

fn exited(status: std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal:{signal}");
        }
    }
    match status.code() {
        Some(code) => format!("exit:{code}"),
        None => "exit:unknown".into(),
    }
}

async fn monitor(
    child: &mut Child,
    expected: &str,
    checking: bool,
    timeout: Duration,
    stopped: &mut watch::Receiver<bool>,
    mut ready: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Outcome> {
    // Child::wait closes Child.stdin eagerly; retain the control pipe separately.
    let mut input = child.stdin.take();
    let mut output =
        BufReader::new(child.stdout.take().context("worker stdout unavailable")?).lines();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut confirmed = !checking;
    let mut activation = None;
    let mut output_open = true;
    loop {
        tokio::select! {
            biased;
            _ = stopped.wait_for(|value| *value) => {
                // EOF requests graceful session drain, including after parent death.
                drop(input.take());
                child.wait().await?;
                return Ok(Outcome::Stopped);
            }
            status = child.wait() => {
                let status = status?;
                // Stdout may already contain activation when process exit wins.
                if output_open {
                    while let Some(line) = output.next_line().await? {
                        match parse(&line) {
                            Ok(Event::Activate { version, digest }) => activation = Some(Installed { version, digest }),
                            Ok(_) => {}
                            Err(_) => return Ok(Outcome::Failed("invalid_event".into())),
                        }
                    }
                }
                return Ok(if status.success() && confirmed {
                    activation.map(Outcome::Activate).unwrap_or_else(|| Outcome::Failed(exited(status)))
                } else { Outcome::Failed(exited(status)) });
            }
            line = output.next_line(), if output_open => {
                match line? {
                    Some(line) => match parse(&line) {
                        Err(_) => {
                            child.kill().await?;
                            child.wait().await?;
                            return Ok(Outcome::Failed("invalid_event".into()));
                        }
                        Ok(Event::Ready { version }) => {
                            if version != expected {
                                child.kill().await?;
                                child.wait().await?;
                                return Ok(Outcome::Failed(format!("version_mismatch:{version}")));
                            }
                            if !confirmed {
                                ensure!(child.try_wait()?.is_none(), "新代理已退出");
                                ready()?;
                                confirmed = true;
                            }
                            input.as_mut().context("worker stdin unavailable")?.write_all(b"confirm\n").await?;
                        }
                        Ok(Event::Activate { version, digest }) => activation = Some(Installed { version, digest }),
                    },
                    None => output_open = false,
                }
            }
            _ = &mut deadline, if !confirmed => {
                // An unconfirmed child has never been admitted to new work by
                // the server's exclusive update reservation. Terminate it before rollback.
                child.kill().await?;
                child.wait().await?;
                return Ok(Outcome::Failed(format!("timeout:{}", timeout.as_secs())));
            }
        }
    }
}

fn parse(line: &str) -> anyhow::Result<Event> {
    ensure!(line.len() <= 4096, "代理控制消息过长");
    Ok(serde_json::from_str(line)?)
}

pub(super) async fn run(config: PathBuf, mut stopped: watch::Receiver<bool>) -> anyhow::Result<()> {
    let settings = Settings::load(&config)?;
    let root = &settings.state_dir;
    let mut journal = Journal::load(root)?;
    // A reinstalled newer service binary must not keep running an older staged version.
    if journal.reconcile(env!("CARGO_PKG_VERSION")) {
        tracing::info!("已安装的代理不低于暂存版本，改用已安装版本");
        clear_failure(root);
        journal.store(root)?;
    }
    if let Some(pending) = &journal.pending {
        record_failure(root, &pending.version, "interrupted");
        journal.rollback();
        journal.store(root)?;
    }
    loop {
        if *stopped.borrow() {
            return Ok(());
        }
        let checking = journal.pending.is_some();
        let selected = journal.pending.as_ref().or(journal.current.as_ref());
        let expected = selected
            .map(|v| v.version.as_str())
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_owned();
        let binary = match selected {
            Some(installed) => match crate::update::staged_binary(root, installed) {
                Ok(binary) => binary,
                Err(error) => {
                    tracing::warn!(%error, "版本文件不可用，恢复上一个已确认版本");
                    if let Some(pending) = &journal.pending {
                        record_failure(root, &pending.version, "missing_binary");
                        journal.rollback();
                    } else {
                        journal.failed_version = journal.current.take().map(|v| v.version);
                        journal.current = journal.previous.take();
                    }
                    journal.store(root)?;
                    continue;
                }
            },
            None => std::env::current_exe()?,
        };
        let execution = async {
            let reason = failure_reason(root, journal.failed_version.as_deref());
            let mut child = match spawn(
                &binary,
                &config,
                &settings.service_name,
                journal.failed_version.as_deref(),
                reason.as_deref(),
            ) {
                Ok(child) => child,
                Err(error) if checking => return Ok(Outcome::Failed(format!("spawn:{error}"))),
                Err(error) if journal.pending.is_none() && journal.current.is_some() => {
                    tracing::warn!(%error, "已确认版本无法启动，恢复上一版本");
                    journal.failed_version = journal.current.take().map(|v| v.version);
                    journal.current = journal.previous.take();
                    journal.store(root)?;
                    return Ok(Outcome::Retry);
                }
                Err(error) => return Err(error),
            };
            #[cfg(windows)]
            let _job = crate::process_job::ProcessJob::attach(&child)?;
            let result = monitor(
                &mut child,
                &expected,
                checking,
                Duration::from_secs(60),
                &mut stopped,
                || {
                    journal.confirm();
                    clear_failure(root);
                    journal.store(root)
                },
            )
            .await;
            if result.is_err() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            result
        }
        .await;
        match execution {
            Ok(Outcome::Retry) => continue,
            Ok(Outcome::Stopped) => return Ok(()),
            Ok(Outcome::Activate(candidate)) => {
                let current = journal
                    .current
                    .as_ref()
                    .map(|v| v.version.as_str())
                    .unwrap_or(env!("CARGO_PKG_VERSION"));
                ensure!(
                    semver::Version::parse(&candidate.version)?
                        .cmp_precedence(&semver::Version::parse(current)?)
                        .is_gt(),
                    "升级版本必须高于当前版本"
                );
                ensure!(
                    journal.failed_version.as_deref() != Some(&candidate.version),
                    "该版本升级失败，不能自动重复尝试"
                );
                crate::update::staged_binary(root, &candidate)?;
                journal.pending = Some(candidate);
                journal.store(root)?;
            }
            outcome if journal.pending.is_some() => {
                let reason = match outcome {
                    Ok(Outcome::Failed(reason)) => reason,
                    Err(error) => format!("error:{error:#}"),
                    _ => "unknown".into(),
                };
                tracing::warn!(%reason, "新代理未通过健康检查，恢复已确认版本");
                if let Some(pending) = &journal.pending {
                    record_failure(root, &pending.version, &reason);
                }
                journal.rollback();
                journal.store(root)?;
            }
            Err(error) => return Err(error),
            Ok(Outcome::Failed(reason)) => {
                anyhow::bail!("代理进程意外退出（{reason}），由系统服务恢复")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn installed(version: &str) -> Installed {
        Installed {
            version: version.into(),
            digest: "a".repeat(64),
        }
    }
    #[test]
    fn interrupted_activation_keeps_last_confirmed_version() {
        let dir = tempfile::tempdir().unwrap();
        let state = Journal {
            current: Some(installed("1.0.0")),
            pending: Some(installed("2.0.0")),
            ..Default::default()
        };
        state.store(dir.path()).unwrap();
        let mut recovered = Journal::load(dir.path()).unwrap();
        recovered.rollback();
        recovered.store(dir.path()).unwrap();
        let result = Journal::load(dir.path()).unwrap();
        assert_eq!(result.current.unwrap().version, "1.0.0");
        assert!(result.pending.is_none());
        assert_eq!(result.failed_version.as_deref(), Some("2.0.0"));
    }
    #[test]
    fn a_reinstalled_newer_binary_wins_over_older_staged_versions() {
        let dir = tempfile::tempdir().unwrap();
        Journal {
            current: Some(installed("0.4.0-alpha")),
            previous: Some(installed("0.3.0-alpha")),
            pending: Some(installed("0.8.0-alpha")),
            failed_version: Some("0.8.0-alpha".into()),
        }
        .store(dir.path())
        .unwrap();
        record_failure(dir.path(), "0.8.0-alpha", "timeout:60");
        reconcile_installed(dir.path(), "0.9.0-alpha").unwrap();
        let journal = Journal::load(dir.path()).unwrap();
        // Nothing staged is selected any more, so the installed binary runs.
        assert!(
            journal.current.is_none() && journal.pending.is_none() && journal.previous.is_none()
        );
        assert!(journal.failed_version.is_none());
        assert!(failure_reason(dir.path(), Some("0.8.0-alpha")).is_none());
        // Reinstalling the same version as the confirmed current also uses the installed binary.
        let mut same = Journal {
            current: Some(installed("0.9.0-alpha")),
            ..Default::default()
        };
        assert!(same.reconcile("0.9.0-alpha"));
        assert!(same.current.is_none());
    }
    #[test]
    fn a_newer_confirmed_version_and_a_newer_failure_are_kept() {
        let mut journal = Journal {
            current: Some(installed("0.9.0-alpha")),
            previous: Some(installed("0.8.2-alpha")),
            failed_version: Some("1.0.0-alpha".into()),
            ..Default::default()
        };
        assert!(!journal.reconcile("0.8.2-alpha"));
        assert_eq!(journal.current.as_ref().unwrap().version, "0.9.0-alpha");
        assert_eq!(journal.previous.as_ref().unwrap().version, "0.8.2-alpha");
        assert_eq!(journal.failed_version.as_deref(), Some("1.0.0-alpha"));
        // An older failure no longer blocks anything once a newer binary is installed.
        let mut older = Journal {
            current: Some(installed("1.1.0-alpha")),
            failed_version: Some("0.9.0-alpha".into()),
            ..Default::default()
        };
        assert!(older.reconcile("1.0.0-alpha"));
        assert!(older.failed_version.is_none());
        assert_eq!(older.current.as_ref().unwrap().version, "1.1.0-alpha");
    }
    #[test]
    fn the_failure_reason_is_kept_per_version_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        record_failure(
            dir.path(),
            "2.0.0",
            &format!("spawn:bad\nline{}", "x".repeat(400)),
        );
        let reason = failure_reason(dir.path(), Some("2.0.0")).unwrap();
        assert!(reason.starts_with("spawn:bad line"));
        assert!(reason.chars().count() <= 200);
        assert!(failure_reason(dir.path(), Some("3.0.0")).is_none());
        assert!(failure_reason(dir.path(), None).is_none());
        clear_failure(dir.path());
        assert!(failure_reason(dir.path(), Some("2.0.0")).is_none());
    }
    #[test]
    fn confirmation_preserves_previous_version() {
        let mut state = Journal {
            current: Some(installed("1.0.0")),
            pending: Some(installed("2.0.0")),
            ..Default::default()
        };
        state.confirm();
        assert_eq!(state.current.unwrap().version, "2.0.0");
        assert_eq!(state.previous.unwrap().version, "1.0.0");
        assert!(state.pending.is_none());
    }
    fn fixture(script: &str) -> Child {
        Command::new(if cfg!(windows) { "python" } else { "python3" })
            .args(["-u", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }
    #[tokio::test]
    async fn live_matching_worker_is_confirmed_and_stopped_gracefully() {
        let mut child = fixture(
            "import sys; print('{\"event\":\"ready\",\"version\":\"2.0.0\"}',flush=True); sys.stdin.read(); sys.exit(0)",
        );
        let (stop, mut stopped) = watch::channel(false);
        let mut confirmed = false;
        let result = monitor(
            &mut child,
            "2.0.0",
            true,
            Duration::from_secs(5),
            &mut stopped,
            || {
                confirmed = true;
                stop.send_replace(true);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(confirmed && matches!(result, Outcome::Stopped));
        assert!(child.try_wait().unwrap().unwrap().success());
    }
    #[tokio::test]
    async fn wrong_version_and_failed_child_are_never_confirmed_and_say_why() {
        for (script, reason) in [
            ("import sys; sys.exit(1)", "exit:1"),
            (
                "import sys; print('{\"event\":\"ready\",\"version\":\"1.0.0\"}',flush=True); sys.stdin.read()",
                "version_mismatch:1.0.0",
            ),
            (
                "import sys; print('not json',flush=True); sys.stdin.read()",
                "invalid_event",
            ),
        ] {
            let mut child = fixture(script);
            let (_stop, mut stopped) = watch::channel(false);
            let mut confirmed = false;
            let result = monitor(
                &mut child,
                "2.0.0",
                true,
                Duration::from_secs(5),
                &mut stopped,
                || {
                    confirmed = true;
                    Ok(())
                },
            )
            .await;
            assert!(!confirmed);
            assert!(
                matches!(&result, Ok(Outcome::Failed(why)) if why == reason),
                "{script}"
            );
        }
    }
    #[tokio::test]
    async fn missing_readiness_times_out_before_confirmation() {
        let mut child = fixture("import sys; sys.stdin.read()");
        let (_stop, mut stopped) = watch::channel(false);
        let result = monitor(
            &mut child,
            "2.0.0",
            true,
            Duration::from_millis(100),
            &mut stopped,
            || panic!("unhealthy worker confirmed"),
        )
        .await
        .unwrap();
        assert!(matches!(&result, Outcome::Failed(why) if why == "timeout:0"));
        assert!(child.try_wait().unwrap().is_some());
    }
    #[tokio::test]
    async fn waiting_for_process_exit_does_not_close_its_control_pipe() {
        let mut child = fixture("import sys; sys.stdin.read(); sys.exit(0)");
        let (stop, mut stopped) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            stop.send_replace(true);
        });
        let start = std::time::Instant::now();
        let result = monitor(
            &mut child,
            "1.0.0",
            false,
            Duration::from_secs(5),
            &mut stopped,
            || Ok(()),
        )
        .await
        .unwrap();
        assert!(matches!(result, Outcome::Stopped));
        assert!(start.elapsed() >= Duration::from_millis(200));
    }
}
