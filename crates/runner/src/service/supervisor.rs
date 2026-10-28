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
    io::{AsyncBufReadExt, BufReader},
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
        tmp.persist(root.join("versions.json"))
            .map_err(|e| e.error)?;
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

fn spawn(binary: &Path, config: &Path, name: &str) -> anyhow::Result<Child> {
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
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    Ok(command.spawn()?)
}

enum Outcome {
    Stopped,
    Activate(Installed),
    Failed,
}

async fn monitor(
    child: &mut Child,
    expected: &str,
    checking: bool,
    timeout: Duration,
    stopped: &mut watch::Receiver<bool>,
    mut ready: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Outcome> {
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
                drop(child.stdin.take());
                child.wait().await?;
                return Ok(Outcome::Stopped);
            }
            status = child.wait() => {
                let status = status?;
                // Stdout may already contain activation when process exit wins.
                if output_open {
                    while let Some(line) = output.next_line().await? {
                        if let Event::Activate { version, digest } = parse(&line)? {
                            activation = Some(Installed { version, digest });
                        }
                    }
                }
                return Ok(if status.success() && confirmed {
                    activation.map(Outcome::Activate).unwrap_or(Outcome::Failed)
                } else { Outcome::Failed });
            }
            line = output.next_line(), if output_open => {
                match line? {
                    Some(line) => match parse(&line)? {
                        Event::Ready { version } => {
                            ensure!(version == expected, "新代理报告的版本与签名清单不一致");
                            if !confirmed {
                                ensure!(child.try_wait()?.is_none(), "新代理已退出");
                                ready()?;
                                confirmed = true;
                            }
                        }
                        Event::Activate { version, digest } => activation = Some(Installed { version, digest }),
                    },
                    None => output_open = false,
                }
            }
            _ = &mut deadline, if !confirmed => {
                // An unconfirmed child has never been admitted to new work by
                // the server's exclusive update reservation. Terminate it before rollback.
                child.kill().await?;
                child.wait().await?;
                return Ok(Outcome::Failed);
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
    if journal.pending.is_some() {
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
            Some(installed) => crate::update::staged_binary(root, installed)?,
            None => std::env::current_exe()?,
        };
        let execution = async {
            let mut child = spawn(&binary, &config, &settings.service_name)?;
            let result = monitor(
                &mut child,
                &expected,
                checking,
                Duration::from_secs(60),
                &mut stopped,
                || {
                    journal.confirm();
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
                tracing::warn!(error=?outcome.err(), "新代理未通过健康检查，恢复已确认版本");
                journal.rollback();
                journal.store(root)?;
            }
            Err(error) => return Err(error),
            Ok(Outcome::Failed) => anyhow::bail!("代理进程意外退出，由系统服务恢复"),
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
    async fn wrong_version_and_failed_child_are_never_confirmed() {
        for script in [
            "import sys; sys.exit(1)",
            "import sys; print('{\"event\":\"ready\",\"version\":\"1.0.0\"}',flush=True); sys.stdin.read()",
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
            assert!(result.is_err() || matches!(result, Ok(Outcome::Failed)));
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
        assert!(matches!(result, Outcome::Failed));
        assert!(child.try_wait().unwrap().is_some());
    }
}
