//! Host operations carried on dedicated Runner sessions.
use super::*;

// ----------------------------------------------------------- HostChannel

pub struct RunnerChannel {
    _lease: crate::hosts::runner::reservations::Lease,
    cluster: Option<Arc<crate::cluster::Cluster>>,
    host_id: String,
    /// Only for error messages: an operator reads names, not ids.
    name: String,
}

pub(crate) async fn cluster_open(host: &str, session: &str, kind: wire::SessionKind) -> Result<()> {
    let registration = live().get(host).cloned().ok_or_else(|| {
        crate::cluster::diagnostic("websocket", "open", "registration_missing");
        crate::cluster::unavailable()
    })?;
    if !registration
        .ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        crate::cluster::diagnostic("websocket", "open", "agent_not_ready");
        return Err(crate::cluster::unavailable());
    }
    if kind == wire::SessionKind::Monitor && !registration.supports_monitor {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.monitorUnsupported"),
        ));
    }
    if kind == wire::SessionKind::Agent && !registration.supports_agent {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.agentUpgrade"),
        ));
    }
    if kind == wire::SessionKind::AgentProbe && !registration.supports_agent_probe {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("agents.probeUnsupported"),
        ));
    }
    registration
        .control
        .send(wire::Control::OpenSession {
            session: session.into(),
            kind,
        })
        .await
        .map_err(|_| crate::cluster::unavailable())
}

/// The factory arm for `connection = 'runner'`. Fails fast when the agent is
/// not connected, so the operator gets a readable HTTP error instead of a
/// socket that opens and immediately closes.
pub async fn open(s: &AppState, host: &Host) -> Result<Box<dyn HostChannel>> {
    let lease =
        crate::hosts::runner::reservations::lease_with(s.cluster.as_ref(), &host.id.to_string())
            .await?;
    if s.cluster.is_none()
        && !live()
            .get(&host.id.to_string())
            .is_some_and(|r| r.ready.load(std::sync::atomic::Ordering::Acquire))
    {
        return Err(offline(&host.name));
    }
    if let Some(cluster) = &s.cluster {
        let owner = cluster
            .owner(&host.id.to_string())
            .await?
            .ok_or_else(|| offline(&host.name))?;
        if owner.workspace != host.workspace_id {
            return Err(ApiError::missing());
        }
    }
    Ok(Box::new(RunnerChannel {
        _lease: lease,
        cluster: s.cluster.clone(),
        host_id: host.id.to_string(),
        name: host.name.clone(),
    }))
}

impl RunnerChannel {
    /// Asks the runner to dial a dedicated connection and waits for it.
    #[tracing::instrument(skip(self), fields(host_id = %self.host_id))]
    async fn session(&self, kind: wire::SessionKind) -> Result<Socket> {
        if !self._lease.valid() {
            crate::cluster::diagnostic("reservation", "open", "lease_invalid");
            return Err(crate::cluster::unavailable());
        }
        if let Some(cluster) = &self.cluster {
            let owner = cluster
                .owner(&self.host_id)
                .await
                .inspect_err(|_| {
                    crate::cluster::diagnostic("nats", "open", "ownership_lookup_failed");
                })?
                .ok_or_else(|| {
                    crate::cluster::diagnostic("nats", "open", "owner_missing");
                    offline(&self.name)
                })?;
            let id = secret();
            let route = crate::cluster::SessionRoute {
                host: self.host_id.clone(),
                owner: owner.clone(),
            };
            let mut socket = cluster.socket(&id, false, &route).await.inspect_err(|_| {
                crate::cluster::diagnostic("nats", "open", "stream_setup_failed");
            })?;
            socket.hold(self._lease.clone());
            cluster.new_session(&id, &route).await.inspect_err(|_| {
                crate::cluster::diagnostic("nats", "open", "route_registration_failed");
            })?;
            let _: Value = cluster
                .rpc(
                    &owner.node,
                    &crate::cluster::Request::Open {
                        host: self.host_id.clone(),
                        fence: owner.fence,
                        session: id.clone(),
                        kind,
                    },
                )
                .await
                .inspect_err(|_| {
                    crate::cluster::diagnostic("nats", "open", "owner_rpc_failed");
                })?;
            cluster.wait_session(&id).await.inspect_err(|_| {
                crate::cluster::diagnostic("nats", "wait", "session_not_ready");
            })?;
            return Ok(socket);
        }
        let registration = live()
            .get(&self.host_id)
            .cloned()
            .ok_or_else(|| offline(&self.name))?;
        if kind == wire::SessionKind::Monitor && !registration.supports_monitor {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("runner.monitorUpgrade"),
            ));
        }
        if kind == wire::SessionKind::Agent && !registration.supports_agent {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("runner.agentUpgrade"),
            ));
        }
        if kind == wire::SessionKind::AgentProbe && !registration.supports_agent_probe {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("agents.probeUnsupported"),
            ));
        }
        let id = secret();
        let (socket, dialled) = oneshot::channel();
        pending().insert(
            id.clone(),
            Pending {
                host_id: self.host_id.clone(),
                socket,
            },
        );
        let asked = registration
            .control
            .send(wire::Control::OpenSession {
                session: id.clone(),
                kind,
            })
            .await;
        if asked.is_err() {
            crate::cluster::diagnostic("websocket", "open", "control_channel_closed");
            pending().remove(&id);
            return Err(offline(&self.name));
        }
        match tokio::time::timeout(DIAL_TIMEOUT, dialled).await {
            Ok(Ok(Ok(socket))) => Ok(socket),
            Ok(Ok(Err(error))) => {
                crate::cluster::diagnostic("websocket", "wait", "session_rejected");
                pending().remove(&id);
                Err(error)
            }
            // Dropped sender or timeout: either way nothing will arrive.
            Ok(Err(_)) | Err(_) => {
                crate::cluster::diagnostic("websocket", "wait", "session_not_ready");
                pending().remove(&id);
                Err(offline(&self.name))
            }
        }
    }

    /// Opens a session, sends one request and returns its first answer, with
    /// the socket in case the answer is a header followed by bytes.
    async fn ask(
        &self,
        kind: wire::SessionKind,
        request: wire::Session,
        within: Duration,
    ) -> Result<(wire::Session, Socket)> {
        let mut socket = self.session(kind).await?;
        send_frame(&mut socket, &request).await?;
        let answer = tokio::time::timeout(within, next_frame(&mut socket))
            .await
            .map_err(|_| timed_out())??;
        if let wire::Session::Failed { error } = answer {
            return Err(remote_failed(error));
        }
        Ok((answer, socket))
    }
}

#[async_trait::async_trait]
impl HostChannel for RunnerChannel {
    async fn agent(
        &self,
        request: wire::agent::Request,
        events: tokio::sync::mpsc::Sender<wire::agent::Event>,
        mut inputs: tokio::sync::mpsc::Receiver<wire::agent::Input>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        let mut socket = self.session(wire::SessionKind::Agent).await?;
        let deadline = tokio::time::sleep(Duration::from_secs(request.timeout_seconds + 85));
        tokio::pin!(deadline);
        send_frame(&mut socket, &wire::Session::AgentStart { request }).await?;
        let mut stopping = false;
        loop {
            if *cancel.borrow() && !stopping {
                send_frame(
                    &mut socket,
                    &wire::Session::AgentInput {
                        input: wire::agent::Input::Cancel,
                    },
                )
                .await?;
                stopping = true;
            }
            tokio::select! {
                _ = cancel.changed(), if !stopping => {
                    send_frame(&mut socket, &wire::Session::AgentInput { input: wire::agent::Input::Cancel }).await?;
                    stopping = true;
                }
                input = inputs.recv(), if !stopping => match input {
                    Some(input) => send_frame(&mut socket, &wire::Session::AgentInput { input }).await?,
                    None => { send_frame(&mut socket, &wire::Session::AgentInput { input: wire::agent::Input::Cancel }).await?; stopping = true; }
                },
                _ = &mut deadline => return Err(ApiError::localized(StatusCode::GATEWAY_TIMEOUT, crate::i18n::Message::new("runner.cleanupTimeout"))),
                frame = next_frame(&mut socket) => match frame? {
                    wire::Session::AgentEvent { event } => {
                        let finished = matches!(event, wire::agent::Event::Finished { .. });
                        // Backpressure must never strand an owned remote process.
                        match tokio::time::timeout(Duration::from_secs(10), events.send(event)).await {
                            Ok(Ok(())) => {},
                            _ => { let _ = socket.send(Message::Close(None)).await; return Err(garbled()); }
                        }
                        if finished { return Ok(()); }
                    }
                    wire::Session::Failed { error } => return Err(remote_failed(error)),
                    _ => return Err(garbled()),
                }
            }
        }
    }
    async fn agent_probe(
        &self,
        runtime: wire::agent::Runtime,
        cwd: &str,
    ) -> Result<wire::agent::Probe> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::AgentProbe,
                wire::Session::AgentProbe {
                    runtime,
                    cwd: cwd.into(),
                    workspace: None,
                },
                Duration::from_secs(35),
            )
            .await?;
        match answer {
            wire::Session::AgentProbeResult { status } => Ok(status),
            _ => Err(garbled()),
        }
    }
    async fn agent_workspace(
        &self,
        runtime: wire::agent::Runtime,
        key: &str,
    ) -> Result<wire::agent::Probe> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::AgentProbe,
                wire::Session::AgentProbe {
                    runtime,
                    cwd: String::new(),
                    workspace: Some(key.into()),
                },
                Duration::from_secs(35),
            )
            .await?;
        match answer {
            wire::Session::AgentProbeResult { status } => Ok(status),
            _ => Err(garbled()),
        }
    }
    async fn monitor(&self) -> Result<wire::HostMonitor> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Monitor,
                wire::Session::Monitor,
                Duration::from_secs(15),
            )
            .await?;
        match answer {
            wire::Session::MonitorResult { metrics } => Ok(metrics),
            _ => Err(garbled()),
        }
    }
    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output> {
        let seconds = timeout.as_secs().max(1);
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Exec,
                wire::Session::ExecStart {
                    command: command.to_owned(),
                    timeout_seconds: seconds,
                },
                // The runner enforces the timeout; this is the backstop for a
                // runner that dies holding the command.
                timeout + REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::ExecDone {
                code,
                stdout,
                stderr,
            } => Ok(Output {
                code,
                stdout,
                stderr,
            }),
            _ => Err(garbled()),
        }
    }

    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        let mut socket = self.session(wire::SessionKind::Exec).await?;
        send_frame(
            &mut socket,
            &wire::Session::ExecStreamStart {
                command: command.into(),
                timeout_seconds: timeout.as_secs().max(1),
            },
        )
        .await?;
        let deadline = tokio::time::sleep(timeout + Duration::from_secs(85));
        tokio::pin!(deadline);
        let mut stopping = false;
        loop {
            if *cancel.borrow() && !stopping {
                send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                stopping = true;
            }
            tokio::select! {
                _ = cancel.changed(), if !stopping => {
                    send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                    stopping = true;
                }
                _ = &mut deadline => {
                    let _ = socket.send(Message::Close(None)).await;
                    return Err(ApiError::localized(StatusCode::GATEWAY_TIMEOUT, crate::i18n::Message::new("runner.cleanupTimeout")));
                }
                frame = next_frame(&mut socket) => match frame? {
                    wire::Session::ExecOutput {stream, data} => {
                        if !["stdout", "stderr"].contains(&stream.as_str()) || data.len() > 65536 {return Err(garbled());}
                        if !stopping {
                            let should_stop = tokio::select! {
                                sent = output.send((stream, data)) => sent.is_err(),
                                _ = cancel.changed() => true,
                                _ = &mut deadline => return Err(ApiError::localized(StatusCode::GATEWAY_TIMEOUT, crate::i18n::Message::new("runner.cleanupTimeout"))),
                            };
                            if should_stop {
                                send_frame(&mut socket, &wire::Session::ExecCancel).await?;
                                stopping = true;
                            }
                        }
                    }
                    wire::Session::ExecExit {code} => return Ok(code),
                    wire::Session::Failed {error} => return Err(remote_failed(error)),
                    _ => return Err(garbled()),
                }
            }
        }
    }

    async fn open_pty(&self, size: PtySize) -> Result<Box<dyn Pty>> {
        let size = size.sane();
        let mut socket = self.session(wire::SessionKind::Terminal).await?;
        send_frame(
            &mut socket,
            &wire::Session::TerminalStart {
                cols: size.cols,
                rows: size.rows,
            },
        )
        .await?;
        // One task owns the socket and the two directions meet it through
        // channels: `Pty::read` is selected over in `remote::pump`, and an
        // mpsc receiver is cancel-safe where a half-read frame would not be.
        let (output, from_host) = mpsc::channel(32);
        let (to_host, input) = mpsc::channel(32);
        let lease = self._lease.clone();
        tokio::spawn(async move {
            let _lease = lease;
            pump_pty(socket, output, input).await;
        });
        Ok(Box::new(RunnerPty {
            output: from_host,
            input: to_host,
        }))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::ListDir {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::DirEntries { entries } => Ok(entries
                .into_iter()
                .map(|entry| DirEntry {
                    name: entry.name,
                    kind: entry.kind,
                    size: entry.size,
                    mode: entry.mode,
                    modified_at: entry.modified_at,
                })
                .collect()),
            _ => Err(garbled()),
        }
    }

    async fn create_file(&self, path: &str) -> Result<()> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::CreateFile {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::Done => Ok(()),
            _ => Err(garbled()),
        }
    }

    async fn create_dir(&self, path: &str) -> Result<()> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::CreateDir {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::Done => Ok(()),
            _ => Err(garbled()),
        }
    }

    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)> {
        let (answer, socket) = self
            .ask(
                wire::SessionKind::Download,
                wire::Session::ReadFile {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        let wire::Session::FileMeta {
            size,
            mode,
            modified_at,
        } = answer
        else {
            return Err(garbled());
        };
        // One frame at a time: the server's memory stays at one chunk no
        // matter how large the file is.
        let stream =
            futures::stream::unfold(Some((socket, self._lease.clone())), |state| async move {
                let (mut socket, lease) = state?;
                loop {
                    match socket.recv().await {
                        Some(Ok(Message::Binary(bytes))) => {
                            return Some((Ok(bytes), Some((socket, lease))));
                        }
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<wire::Session>(&text) {
                                Ok(wire::Session::Eof) => return None,
                                Ok(wire::Session::Failed { error }) => {
                                    return Some((Err(remote_failed(error)), None));
                                }
                                _ => continue,
                            }
                        }
                        Some(Ok(_)) => continue,
                        // Closing without `Eof` means a truncated file. Reporting
                        // it as an error is the difference between a failed
                        // download and a silently short one.
                        Some(Err(_)) | None => {
                            return Some((
                                Err(ApiError::localized(
                                    StatusCode::BAD_GATEWAY,
                                    crate::i18n::Message::new("runner.downloadClosed"),
                                )),
                                None,
                            ));
                        }
                    }
                }
            });
        Ok((
            FileMeta {
                size,
                mode,
                modified_at,
            },
            Box::pin(stream),
        ))
    }

    async fn write_file(&self, path: &str, mode: u32, mut body: ByteStream) -> Result<()> {
        let mut socket = self.session(wire::SessionKind::Upload).await?;
        send_frame(
            &mut socket,
            &wire::Session::WriteFile {
                path: path.to_owned(),
                mode,
            },
        )
        .await?;
        while let Some(chunk) = body.next().await {
            let mut chunk = chunk?;
            while !chunk.is_empty() {
                // Awaiting each send is the backpressure: the uploader cannot
                // run ahead of the host, so nothing queues up here.
                let piece = chunk.split_to(chunk.len().min(CHUNK));
                socket
                    .send(Message::Binary(piece))
                    .await
                    .map_err(remote_failed)?;
            }
        }
        send_frame(&mut socket, &wire::Session::Eof).await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, next_frame(&mut socket))
            .await
            .map_err(|_| timed_out())??
        {
            wire::Session::Done => Ok(()),
            wire::Session::Failed { error } => Err(remote_failed(error)),
            _ => Err(garbled()),
        }
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
        let (answer, _socket) = self
            .ask(
                wire::SessionKind::Files,
                wire::Session::DeleteFile {
                    path: path.to_owned(),
                },
                REQUEST_TIMEOUT,
            )
            .await?;
        match answer {
            wire::Session::Done => Ok(()),
            _ => Err(garbled()),
        }
    }
}

// ------------------------------------------------------------------ 终端

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_runner_refuses_agent_before_opening_local_or_cluster_session() {
        let host = uuid::Uuid::new_v4().to_string();
        let (control, mut commands) = mpsc::channel(4);
        let (disconnect, _) = watch::channel(false);
        live().insert(
            host.clone(),
            Arc::new(Live {
                owner: None,
                owner_revision: std::sync::atomic::AtomicU64::new(0),
                key_hash: String::new(),
                control,
                disconnect,
                managed: false,
            installation_scope: None,
                supports_monitor: true,
                supports_agent: false,
                supports_agent_probe: false,
                version: "legacy".into(),
                ready: std::sync::atomic::AtomicBool::new(true),
                failed_version: Mutex::new(None),
                update_error: Mutex::new(None),
            }),
        );
        let channel = RunnerChannel {
            _lease: crate::hosts::runner::reservations::lease(&host).await,
            cluster: None,
            host_id: host.clone(),
            name: "fixture".into(),
        };
        let local = channel
            .session(wire::SessionKind::Agent)
            .await
            .err()
            .expect("legacy local runner must refuse Agent");
        let remote = cluster_open(&host, "unused", wire::SessionKind::Agent)
            .await
            .unwrap_err();
        assert!(
            commands.try_recv().is_err(),
            "unsupported work must never be dispatched"
        );
        live().remove(&host);
        for error in [local, remote] {
            assert_eq!(error.0, StatusCode::CONFLICT);
            for locale in crate::i18n::Locale::ALL {
                assert_eq!(
                    error.render(locale),
                    crate::i18n::render(locale, &crate::i18n::Message::new("runner.agentUpgrade"))
                );
            }
        }
    }

    #[tokio::test]
    async fn legacy_runner_monitor_does_not_send_an_unknown_session_kind() {
        let host_id = "legacy-monitor-fixture";
        let (control, mut receiver) = mpsc::channel(1);
        let (disconnect, _) = watch::channel(false);
        live().insert(
            host_id.into(),
            Arc::new(Live {
                owner: None,
                owner_revision: std::sync::atomic::AtomicU64::new(0),
                key_hash: String::new(),
                control,
                disconnect,
                managed: false,
            installation_scope: None,
                supports_monitor: false,
                supports_agent: false,
                supports_agent_probe: false,
                version: "0.1.0-alpha.13".into(),
                ready: std::sync::atomic::AtomicBool::new(true),
                failed_version: Mutex::new(None),
                update_error: Mutex::new(None),
            }),
        );
        let channel = RunnerChannel {
            _lease: crate::hosts::runner::reservations::lease(host_id).await,
            cluster: None,
            host_id: host_id.into(),
            name: "旧版主机".into(),
        };
        let error = channel.monitor().await.unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert!(
            error
                .render(crate::i18n::Locale::ZhCn)
                .contains("升级 ferry-runner")
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        live().remove(host_id);
    }
}
