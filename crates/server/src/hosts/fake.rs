//! Test-only host channel: hosts whose `connection` is `fake` are served from an in-memory
//! registry keyed by host id, so deployment flows can run without SSH or a runner.
use crate::error::{ApiError, Result};
use crate::hosts::channel::{ByteStream, DirEntry, FileMeta, HostChannel, Output, Pty, PtySize};
use crate::hosts::remote::Host;
use futures::StreamExt;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

/// Fixed digest every `docker image inspect` reports.
pub(crate) const DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

#[derive(Default)]
pub(crate) struct FakeHost {
    pub commands: Mutex<Vec<String>>,
    /// Remote path → content written by `write_file`.
    pub files: Mutex<BTreeMap<String, String>>,
    /// When set, deployment scripts (`set -eu; ...`) exit non-zero before changing anything,
    /// as on a host without a usable container runtime; image pulls still succeed.
    pub fail: AtomicBool,
    /// Canned stdout: the first entry whose pattern the command contains answers it.
    pub responses: Mutex<Vec<(String, String)>>,
    /// Canned failures: a command containing the pattern exits 1 with this stderr; checked
    /// before `responses`.
    pub failures: Mutex<Vec<(String, String)>>,
    /// When set, opening a channel or running a command fails with a connection error, as for
    /// a host that went down.
    pub unreachable: AtomicBool,
}

impl FakeHost {
    /// Answers every later command containing `pattern` with `stdout` (exit 0).
    pub fn respond(&self, pattern: &str, stdout: &str) {
        self.responses
            .lock()
            .unwrap()
            .push((pattern.into(), stdout.into()));
    }
    /// Fails every later command containing `pattern` (exit 1, `stderr`).
    pub fn fail_on(&self, pattern: &str, stderr: &str) {
        self.failures
            .lock()
            .unwrap()
            .push((pattern.into(), stderr.into()));
    }
}

static HOSTS: LazyLock<Mutex<HashMap<i64, Arc<FakeHost>>>> = LazyLock::new(Default::default);

/// Registers (or replaces) the fake behind host `id`; ids must be unique per test.
pub(crate) fn register(id: i64) -> Arc<FakeHost> {
    let host = Arc::new(FakeHost::default());
    HOSTS.lock().unwrap().insert(id, host.clone());
    host
}

pub(crate) fn open(host: &Host) -> Result<Box<dyn HostChannel>> {
    let fake = HOSTS
        .lock()
        .unwrap()
        .get(&i64::from(host.id))
        .cloned()
        // An unregistered host is unreachable, as a real SSH host that refuses connections.
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_GATEWAY,
                crate::i18n::Message::new("ssh.unreachable")
                    .with("address", &host.address)
                    .with("detail", "fake host not registered"),
            )
        })?;
    if fake.unreachable.load(Ordering::SeqCst) {
        return Err(connection_lost(&host.address));
    }
    Ok(Box::new(Channel(fake)))
}

fn connection_lost(address: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_GATEWAY,
        crate::i18n::Message::new("ssh.unreachable")
            .with("address", address)
            .with("detail", "fake host unreachable"),
    )
}

struct Channel(Arc<FakeHost>);

fn ok(stdout: String) -> Output {
    Output {
        code: 0,
        stdout,
        stderr: String::new(),
    }
}

#[async_trait::async_trait]
impl HostChannel for Channel {
    async fn monitor(&self) -> Result<ferry_protocol::HostMonitor> {
        Err(ApiError::bad("fake host has no monitor"))
    }
    async fn exec(&self, command: &str, _: Duration) -> Result<Output> {
        // A command never reaches an unreachable host, so it is not recorded.
        if self.0.unreachable.load(Ordering::SeqCst) {
            return Err(connection_lost("fake"));
        }
        self.0.commands.lock().unwrap().push(command.into());
        if command.starts_with("set -eu") && self.0.fail.load(Ordering::SeqCst) {
            return Ok(Output {
                code: 1,
                stdout: String::new(),
                stderr: "fake failure".into(),
            });
        }
        let failed = self
            .0
            .failures
            .lock()
            .unwrap()
            .iter()
            .find(|(pattern, _)| command.contains(pattern.as_str()))
            .map(|(_, stderr)| stderr.clone());
        if let Some(stderr) = failed {
            return Ok(Output {
                code: 1,
                stdout: String::new(),
                stderr,
            });
        }
        let canned = self
            .0
            .responses
            .lock()
            .unwrap()
            .iter()
            .find(|(pattern, _)| command.contains(pattern.as_str()))
            .map(|(_, stdout)| stdout.clone());
        if let Some(stdout) = canned {
            return Ok(ok(stdout));
        }
        if command.starts_with("docker image inspect") {
            return Ok(ok(format!("[\"example/image@{DIGEST}\"]\n")));
        }
        if command.starts_with("if [ -e ") {
            // `cat` of an existing remote file, or nothing when it does not exist.
            let files = self.0.files.lock().unwrap();
            let found = files
                .iter()
                .find(|(path, _)| command.contains(&crate::deployments::quote(path)));
            return Ok(ok(found.map(|(_, text)| text.clone()).unwrap_or_default()));
        }
        Ok(ok(String::new()))
    }
    async fn exec_stream(
        &self,
        _: &str,
        _: Duration,
        _: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        _: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        Err(ApiError::bad("fake host does not stream"))
    }
    async fn open_pty(&self, _: PtySize) -> Result<Box<dyn Pty>> {
        Err(ApiError::bad("fake host has no pty"))
    }
    async fn list_dir(&self, _: &str) -> Result<Vec<DirEntry>> {
        Ok(Vec::new())
    }
    async fn create_file(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn create_dir(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn read_file(&self, _: &str) -> Result<(FileMeta, ByteStream)> {
        Err(ApiError::bad("fake host does not stream files"))
    }
    async fn write_file(&self, path: &str, _: u32, mut body: ByteStream) -> Result<()> {
        let mut bytes = Vec::new();
        while let Some(chunk) = body.next().await {
            bytes.extend_from_slice(&chunk?);
        }
        self.0
            .files
            .lock()
            .unwrap()
            .insert(path.into(), String::from_utf8_lossy(&bytes).into_owned());
        Ok(())
    }
    async fn delete_file(&self, path: &str) -> Result<()> {
        self.0.files.lock().unwrap().remove(path);
        Ok(())
    }
}

/// The fake registered for host `id`.
pub(crate) fn get(id: i64) -> Arc<FakeHost> {
    HOSTS.lock().unwrap()[&id].clone()
}

/// Removes the fake behind host `id`, so opening a channel to it fails as for an offline host.
pub(crate) fn unregister(id: i64) {
    HOSTS.lock().unwrap().remove(&id);
}
