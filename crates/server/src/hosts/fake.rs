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
    /// Scripted Agent executions, one per `agent` call in order; an exhausted queue
    /// finishes immediately as completed without text.
    pub agent_scripts: Mutex<std::collections::VecDeque<Vec<AgentStep>>>,
    /// Every Agent request received, in order.
    pub agent_requests: Mutex<Vec<ferry_protocol::agent::Request>>,
    pub process_timezones: Mutex<Vec<Option<String>>>,
    pub stream_commands: AtomicBool,
    /// Every input the server sent to a running Agent, in order.
    pub agent_inputs: Mutex<Vec<ferry_protocol::agent::Input>>,
    /// Set once the server requested cancellation of a running Agent.
    pub agent_cancelled: AtomicBool,
    /// Overrides the default probe answer (a usable, signed-in CLI with platform tools).
    pub agent_probe: Mutex<Option<ferry_protocol::agent::Probe>>,
    /// When set, `list_dir` and `read_file` serve `files` as a directory tree
    /// (off by default so deployment fakes keep their empty listings).
    pub browsable: AtomicBool,
    /// Paths `list_dir` reports as symbolic links.
    pub symlinks: Mutex<std::collections::BTreeSet<String>>,
}

/// One step of a scripted fake Agent execution.
pub(crate) enum AgentStep {
    /// Emitted to the server as is.
    Event(ferry_protocol::agent::Event),
    /// Waits for the next server input (recorded), or for cancellation.
    Input,
    /// Meets the test at this barrier, so the test can change state mid-run.
    Barrier(Arc<tokio::sync::Barrier>),
    /// Runs until cancelled.
    Hang,
    /// Writes a file relative to the request's working directory, as the CLI would.
    Write(String, String),
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
    Ok(Box::new(Channel(fake, None)))
}

fn connection_lost(address: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_GATEWAY,
        crate::i18n::Message::new("ssh.unreachable")
            .with("address", address)
            .with("detail", "fake host unreachable"),
    )
}

struct Channel(Arc<FakeHost>, Option<String>);

fn ok(stdout: String) -> Output {
    Output {
        code: 0,
        stdout,
        stderr: String::new(),
    }
}

#[async_trait::async_trait]
impl HostChannel for Channel {
    fn set_timezone(&mut self, timezone: Option<String>) -> Result<()> {
        self.1 = timezone;
        Ok(())
    }

    async fn monitor(&self) -> Result<ferry_protocol::HostMonitor> {
        Err(ApiError::bad("fake host has no monitor"))
    }
    async fn exec(&self, command: &str, _: Duration) -> Result<Output> {
        // A command never reaches an unreachable host, so it is not recorded.
        if self.0.unreachable.load(Ordering::SeqCst) {
            return Err(connection_lost("fake"));
        }
        self.0
            .process_timezones
            .lock()
            .unwrap()
            .push(self.1.clone());
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
        command: &str,
        _: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        _: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        if !self.0.stream_commands.load(Ordering::SeqCst) {
            return Err(ApiError::bad("fake host does not stream"));
        }
        let result = self.exec(command, Duration::ZERO).await?;
        if !result.stdout.is_empty() {
            let _ = output
                .send(("stdout".into(), result.stdout.into_bytes()))
                .await;
        }
        Ok(result.code)
    }
    async fn open_pty(&self, _: PtySize) -> Result<Box<dyn Pty>> {
        Err(ApiError::bad("fake host has no pty"))
    }
    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        if !self.0.browsable.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        let prefix = format!("{}/", path.trim_end_matches('/'));
        let files = self.0.files.lock().unwrap();
        let symlinks = self.0.symlinks.lock().unwrap();
        let mut entries: BTreeMap<String, DirEntry> = BTreeMap::new();
        for (file, content) in files.iter() {
            let Some(rest) = file.strip_prefix(&prefix) else {
                continue;
            };
            let (name, nested) = match rest.split_once('/') {
                Some((name, _)) => (name, true),
                None => (rest, false),
            };
            let kind = if symlinks.contains(&format!("{prefix}{name}")) {
                "symlink"
            } else if nested {
                "directory"
            } else {
                "file"
            };
            entries.insert(
                name.into(),
                DirEntry {
                    name: name.into(),
                    kind: kind.into(),
                    size: if nested { 0 } else { content.len() as u64 },
                    mode: 0o644,
                    modified_at: 0,
                },
            );
        }
        if entries.is_empty() && !files.keys().any(|file| file.starts_with(&prefix)) {
            return Err(ApiError::bad("no such directory"));
        }
        Ok(entries.into_values().collect())
    }
    async fn create_file(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn create_dir(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)> {
        if !self.0.browsable.load(Ordering::SeqCst) {
            return Err(ApiError::bad("fake host does not stream files"));
        }
        let content = self
            .0
            .files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| ApiError::bad("no such file"))?;
        let meta = FileMeta {
            size: content.len() as u64,
            mode: 0o644,
            modified_at: 0,
        };
        let chunk: Result<bytes::Bytes> = Ok(bytes::Bytes::from(content.into_bytes()));
        Ok((meta, Box::pin(futures::stream::iter([chunk]))))
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
    async fn agent_probe(
        &self,
        runtime: ferry_protocol::agent::Runtime,
        cwd: &str,
        _: Option<&ferry_protocol::agent::Binding>,
    ) -> Result<ferry_protocol::agent::Probe> {
        Ok(self.probe(runtime, cwd))
    }
    async fn agent_workspace(
        &self,
        runtime: ferry_protocol::agent::Runtime,
        key: &str,
    ) -> Result<ferry_protocol::agent::Probe> {
        Ok(self.probe(runtime, &format!("/fake/workspaces/{key}")))
    }
    async fn agent(
        &self,
        request: ferry_protocol::agent::Request,
        events: tokio::sync::mpsc::Sender<ferry_protocol::agent::Event>,
        mut inputs: tokio::sync::mpsc::Receiver<ferry_protocol::agent::Input>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        use ferry_protocol::agent::{Event, Input, Outcome};
        let cwd = request.cwd.trim_end_matches('/').to_owned();
        self.0.agent_requests.lock().unwrap().push(request);
        let script = self
            .0
            .agent_scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        let cancelled = |fake: &FakeHost| fake.agent_cancelled.store(true, Ordering::SeqCst);
        for step in script {
            if *cancel.borrow() {
                cancelled(&self.0);
                break;
            }
            match step {
                AgentStep::Event(event) => {
                    let _ = events.send(event).await;
                }
                AgentStep::Barrier(barrier) => {
                    barrier.wait().await;
                }
                AgentStep::Input => {
                    tokio::select! {
                        input = inputs.recv() => match input {
                            Some(Input::Cancel) | None => { cancelled(&self.0); break; }
                            Some(input) => self.0.agent_inputs.lock().unwrap().push(input),
                        },
                        _ = cancel.wait_for(|value| *value) => { cancelled(&self.0); break; }
                    }
                }
                AgentStep::Hang => {
                    let _ = cancel.wait_for(|value| *value).await;
                    cancelled(&self.0);
                    break;
                }
                AgentStep::Write(relative, content) => {
                    self.0
                        .files
                        .lock()
                        .unwrap()
                        .insert(format!("{cwd}/{relative}"), content);
                }
            }
        }
        if self.0.agent_cancelled.load(Ordering::SeqCst) {
            let _ = events
                .send(Event::Finished {
                    outcome: Outcome::Cancelled,
                    usage: None,
                })
                .await;
        }
        Ok(())
    }
}

impl Channel {
    fn probe(
        &self,
        runtime: ferry_protocol::agent::Runtime,
        cwd: &str,
    ) -> ferry_protocol::agent::Probe {
        self.0
            .agent_probe
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| probe_answer(runtime, cwd))
    }
}

/// The default probe answer: a usable, signed-in CLI with platform tools.
pub(crate) fn probe_answer(
    runtime: ferry_protocol::agent::Runtime,
    cwd: &str,
) -> ferry_protocol::agent::Probe {
    ferry_protocol::agent::Probe {
        platform_tools: true,
        runtime,
        execution_user: "fake".into(),
        execution_identity: "fake-identity".into(),
        executable: Some("/usr/bin/fake-cli".into()),
        version: Some("1.0.0".into()),
        models: vec![],
        authentication: "logged_in".into(),
        directory_exists: true,
        error: None,
        canonical_directory: Some(cwd.into()),
        write_workdir: true,
        file_tools: vec![],
    }
}

/// A host row served by a fake channel (connection `fake`).
pub(crate) fn host_row(
    workspace: i64,
    id: i64,
    name: &str,
    state: &str,
) -> crate::entities::hosts::Model {
    crate::entities::hosts::Model {
        id,
        workspace_id: workspace,
        name: name.into(),
        address: format!("{name}.test"),
        port: 0,
        username: String::new(),
        connection: "fake".into(),
        state: state.into(),
        group_id: 0,
        description: String::new(),
        host_key: String::new(),
        runner_key_hash: format!("key-{id}"),
        enroll_token: String::new(),
        enroll_token_hash: String::new(),
        jump_host_id: 0,
        seen_at: 0,
        created_at: 0,
        updated_at: 0,
        version: 1,
        os: "linux".into(),
        arch: "x86_64".into(),
        agent_version: String::new(),
        agent_update: None,
        runner_auto_update: "inherit".into(),
        metrics: String::new(),
        credential_id: 0,
        timezone: String::new(),
        detected_timezone: String::new(),
        timezone_supported: 1,
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
