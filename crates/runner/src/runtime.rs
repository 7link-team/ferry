//! Foreground lifecycle, authenticated control connection, reconnect and session drain.

use crate::{
    cli::Config,
    enrollment::Enrolment,
    service, session,
    transport::{dial, next_frame, send, ws_url},
    update,
};
use anyhow::Context;
use ferry_protocol as wire;
use futures_util::StreamExt;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// A refusal the server will give again on the next attempt. Retrying a wrong
/// version or a dead key forever only hides the problem from the operator.
#[derive(Debug)]
struct Rejected(String);
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "服务拒绝接入：{}", self.0)
    }
}
impl std::error::Error for Rejected {}

fn facts() -> (String, String, String) {
    (
        hostname(),
        std::env::consts::OS.to_owned(),
        std::env::consts::ARCH.to_owned(),
    )
}

fn hostname() -> String {
    // sysinfo already knows, and it is the only portable answer available
    // without another dependency.
    sysinfo::System::host_name().unwrap_or_else(|| "unknown".into())
}

/// Load, memory and the filesystem the host actually runs from.
fn metrics() -> wire::Report {
    let mut system = sysinfo::System::new();
    system.refresh_cpu_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_all();
    system.refresh_memory();
    let load = sysinfo::System::load_average();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // The root filesystem is the one an operator means by "disk". Longest
    // matching mount point wins, so a separate /var is reported for /var.
    let root = disks
        .list()
        .iter()
        .filter(|disk| disk.mount_point() == std::path::Path::new("/"))
        .chain(
            disks
                .list()
                .iter()
                .filter(|disk| disk.mount_point() == std::path::Path::new("C:\\")),
        )
        .next();
    let (disk_total, disk_used) = root
        .map(|disk| {
            (
                disk.total_space(),
                disk.total_space().saturating_sub(disk.available_space()),
            )
        })
        .unwrap_or((0, 0));
    wire::Report::Metrics {
        load1: load.one,
        cpu_percent: f64::from(system.global_cpu_usage()),
        memory_used: system.used_memory(),
        memory_total: system.total_memory(),
        disk_used,
        disk_total,
    }
}

pub(super) fn foreground(cfg: Config) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cfg))
}

async fn run(cfg: Config) -> anyhow::Result<()> {
    // The CLI search path may ask the login shell; do it now, off the Hello path.
    crate::agent::paths::warm();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = stop.send(true);
    });
    run_with_shutdown(cfg, stopped).await
}

pub(super) async fn run_with_shutdown(
    cfg: Config,
    mut stopped: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        cfg.token.is_some() || Enrolment::load(&cfg.state_dir).is_some(),
        "首次运行需要 --token，或已有 {} 中的注册信息。",
        cfg.state_dir.display()
    );
    let control_url = ws_url(&cfg.server, "/api/runners/connect")?;
    tracing::info!(server = %cfg.server, protocol = wire::VERSION, "ferry-runner 启动");

    // Reconnect forever: a server restart or a flapping link is normal
    // operation for an agent, not a reason to stop. See `reconnect_delay`.
    let mut attempt = 0usize;
    let mut sessions = tokio::task::JoinSet::new();
    // Preview forwards are open-ended byte pipes with no state to finish, so
    // they are kept apart and cut on stop instead of being waited for.
    let mut forwards = tokio::task::JoinSet::new();
    let mut activation = None;
    loop {
        let connected_at = tokio::time::Instant::now();
        let result = tokio::select! {
            biased;
            _ = stopped.wait_for(|value| *value) => break,
            result = hold(&cfg, &control_url, &mut sessions, &mut forwards) => result,
        };
        // A connection that worked for a while and then broke (e.g. TLS cut by
        // the network) is a fresh incident: start the schedule again.
        if connected_at.elapsed() >= HEALTHY_CONNECTION {
            attempt = 0;
        }
        match result {
            Ok(()) => {
                tracing::warn!("控制连接已关闭，准备重连");
                attempt = 0;
            }
            Err(error) if error.is::<update::Prepared>() => {
                activation = Some(error.downcast::<update::Prepared>()?.0);
                break;
            }
            Err(error) if error.is::<Rejected>() => {
                tracing::error!(%error, "接入被拒绝，不再重试");
                return Err(error);
            }
            Err(_) => {}
        }
        let delay = reconnect_delay(attempt, jitter_seed());
        attempt = attempt.saturating_add(1);
        if let Err(error) = &result {
            tracing::warn!(%error, "控制连接失败，{:.1}s 后重试", delay.as_secs_f64());
        }
        tokio::select! {
            _ = stopped.wait_for(|value| *value) => break,
            _ = tokio::time::sleep(delay) => {},
        }
    }
    tracing::info!(
        active_sessions = sessions.len(),
        "停止接收会话，等待在途会话结束"
    );
    forwards.abort_all();
    while forwards.join_next().await.is_some() {}
    while sessions.join_next().await.is_some() {}
    if let Some(candidate) = activation {
        service::supervisor::emit(&service::supervisor::Event::Activate {
            version: candidate.version,
            digest: candidate.digest,
        })?;
    }
    Ok(())
}

/// One life of the control connection: handshake, then forward until it ends.
async fn hold(
    cfg: &Config,
    control_url: &str,
    sessions: &mut tokio::task::JoinSet<()>,
    forwards: &mut tokio::task::JoinSet<()>,
) -> anyhow::Result<()> {
    let mut socket = dial(control_url).await?;
    let (hostname, os, arch) = facts();
    let stored = Enrolment::load(&cfg.state_dir);
    // A stored key wins over a token: re-running the install command on an
    // already-enrolled host must not burn a second token.
    let auth = match (&stored, &cfg.token) {
        (Some(enrolment), _) => wire::Auth::Key {
            host_id: enrolment.host_id.clone(),
            key: enrolment.key.clone(),
        },
        (None, Some(token)) => wire::Auth::Enroll {
            token: token.clone(),
        },
        (None, None) => anyhow::bail!("没有可用的注册信息。"),
    };
    send(
        &mut socket,
        &wire::Hello {
            version: wire::VERSION,
            auth,
            hostname,
            ip_address: local_ip_address::local_ip()
                .ok()
                .filter(|address| !address.is_loopback() && !address.is_unspecified())
                .map(|address| address.to_string())
                .unwrap_or_default(),
            os,
            arch,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            managed_updates: cfg.managed,
            installation_scope: if cfg.managed {
                cfg.installation_scope.clone()
            } else {
                Some("foreground".into())
            },
            supports_monitor: true,
            supports_agent: true,
            supports_agent_probe: true,
            supports_forward: true,
            supports_timezone: true,
            timezone: crate::timezone::detected(),
        },
    )
    .await?;

    let (host_id, heartbeat_seconds, metrics_seconds) = match next_frame(&mut socket).await? {
        wire::Control::Welcome {
            host_id,
            key,
            heartbeat_seconds,
            metrics_seconds,
        } => {
            if let Some(key) = key {
                Enrolment {
                    host_id: host_id.clone(),
                    key,
                }
                .save(&cfg.state_dir)?;
                tracing::info!(%host_id, "注册成功，已保存长期凭据");
            }
            (host_id, heartbeat_seconds, metrics_seconds)
        }
        wire::Control::Rejected { reason } => return Err(Rejected(reason).into()),
        other => anyhow::bail!("服务返回了意外的握手帧：{other:?}"),
    };
    let enrolment = Enrolment::load(&cfg.state_dir).context("注册信息丢失")?;
    tracing::info!(%host_id, "控制连接已建立");
    if cfg.managed {
        service::supervisor::emit(&service::supervisor::Event::Ready {
            version: env!("CARGO_PKG_VERSION").into(),
        })?;
        let mut confirmation = cfg.confirmation.clone().context("缺少服务确认通道")?;
        confirmation
            .wait_for(|confirmed| *confirmed)
            .await
            .context("服务启动器未确认版本")?;
        send(
            &mut socket,
            &wire::Report::UpdateHealth {
                version: env!("CARGO_PKG_VERSION").into(),
                failed_version: cfg.failed_version.clone(),
                failed_reason: cfg.failed_reason.clone(),
            },
        )
        .await?;
    }

    let session_base = cfg.server.clone();
    let mut preparing = tokio::task::JoinSet::new();
    let mut report = tokio::time::interval(Duration::from_secs(metrics_seconds.max(5)));
    // Report once immediately so the dashboard is populated before the first
    // interval elapses.
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Silence from the server for this long means the link is dead even though
    // the socket still looks open — the other half of the heartbeat.
    let silence = Duration::from_secs(heartbeat_seconds.max(5) * 3);

    loop {
        tokio::select! {
            incoming = tokio::time::timeout(silence, socket.next()) => {
                let Ok(incoming) = incoming else {
                    anyhow::bail!("{}s 内没有收到服务的任何帧，判定连接已失效。", silence.as_secs());
                };
                let Some(message) = incoming else { return Ok(()) };
                match message? {
                    Message::Text(text) => match serde_json::from_str::<wire::Control>(&text)? {
                        wire::Control::Ping => send(&mut socket, &wire::Report::Pong).await?,
                        // Applies to processes started from now on; nothing restarts.
                        wire::Control::Settings { timezone } => crate::timezone::pin(timezone),
                        wire::Control::Update { request, version, manifest, signature } => {
                            if !cfg.managed {
                                send(&mut socket, &wire::Report::UpdateFailed { request, error: "代理不是托管服务，无法自动升级".into(), message: Some(wire::SystemMessage::new("runner.updateNotManaged")) }).await?;
                            } else if !preparing.is_empty() {
                                send(&mut socket, &wire::Report::UpdateFailed { request, error: "已有升级正在进行".into(), message: Some(wire::SystemMessage::new("runner.updateInProgress")) }).await?;
                            } else if cfg.failed_version.as_deref() == Some(&version) {
                                send(&mut socket, &wire::Report::UpdateFailed { request, error: "该版本启动失败，已回退，不能自动重复尝试".into(), message: Some(wire::SystemMessage::new("runner.failedTarget")) }).await?;
                            } else {
                                let root = cfg.state_dir.clone();
                                preparing.spawn(async move { (request, update::download(&root, &manifest, &signature, &version).await) });
                            }
                        }
                        wire::Control::OpenSession { session: id, kind } => {
                            if !preparing.is_empty() {
                                send(&mut socket, &wire::Report::SessionFailed { session: id, error: "代理正在准备升级".into(), message: Some(wire::SystemMessage::new("runnerExtra.preparingUpgrade")) }).await?;
                                continue;
                            }
                            let url = ws_url(&session_base, &format!("/api/runners/sessions/{id}"))?;
                            let hello = wire::SessionHello {
                                version: wire::VERSION,
                                host_id: enrolment.host_id.clone(),
                                key: enrolment.key.clone(),
                            };
                            // Dialled on its own task: a slow transfer must not
                            // stall heartbeats or the next session request.
                            let root = cfg.state_dir.clone();
                            let set = if kind == wire::SessionKind::Forward { &mut *forwards } else { &mut *sessions };
                            set.spawn(async move {
                                if let Err(error) = session::run(&url, hello, kind, &root).await {
                                    tracing::warn!(%id, ?kind, %error, "会话结束于错误");
                                }
                            });
                        }
                        wire::Control::Rejected { reason } => return Err(Rejected(reason).into()),
                        wire::Control::Welcome { .. } => {}
                    },
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
            completed = preparing.join_next(), if !preparing.is_empty() => {
                let (request, result) = completed.context("更新任务丢失")??;
                match result {
                    Ok(candidate) => return Err(update::Prepared(candidate).into()),
                    Err(error) => {
                        tracing::warn!(error=?error, "更新验证或下载失败");
                        send(&mut socket, &wire::Report::UpdateFailed { request, error: "更新下载或校验失败，保留原版本".into(), message: Some(wire::SystemMessage::new("runner.updateFailed")) }).await?;
                    }
                }
            }
            _ = report.tick() => send(&mut socket, &metrics()).await?,
            _ = sessions.join_next(), if !sessions.is_empty() => {},
            _ = forwards.join_next(), if !forwards.is_empty() => {},
        }
    }
}

/// How long a control connection must have lasted to count as healthy, so
/// that its failure restarts the reconnect schedule from the beginning.
const HEALTHY_CONNECTION: Duration = Duration::from_secs(60);
/// Reconnect schedule in seconds: a network blip is usually over within
/// seconds (1, 3, 5), a server restart or upgrade within 10-30 s (10, 15),
/// and a long outage is retried every 30 s so it adds no load.
const RECONNECT_SCHEDULE: [u64; 6] = [1, 3, 5, 10, 15, 30];

/// The wait before reconnect attempt `attempt` (0-based), spread by ±20 % so
/// that many Runners do not reconnect in the same second after a restart.
/// `seed` is any value in 0..1000.
fn reconnect_delay(attempt: usize, seed: u64) -> Duration {
    let base = RECONNECT_SCHEDULE[attempt.min(RECONNECT_SCHEDULE.len() - 1)] as f64;
    let spread = (seed % 1000) as f64 / 1000.0 * 0.4 - 0.2;
    Duration::from_secs_f64(base * (1.0 + spread))
}

/// Cheap per-attempt variation; no randomness dependency is needed for jitter.
fn jitter_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.subsec_nanos() as u64)
        .unwrap_or(0);
    (nanos ^ (u64::from(std::process::id()) * 2_654_435_761)) % 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnects_quickly_at_first_then_backs_off_to_30_seconds_with_jitter() {
        let secs = |attempt, seed| super::reconnect_delay(attempt, seed).as_secs_f64();
        // Middle of the jitter range (seed 500) is the schedule itself.
        let schedule: Vec<f64> = (0..8).map(|attempt| secs(attempt, 500)).collect();
        assert_eq!(schedule, [1.0, 3.0, 5.0, 10.0, 15.0, 30.0, 30.0, 30.0]);
        // Jitter stays within ±20 %.
        assert!((secs(0, 0) - 0.8).abs() < 1e-9);
        assert!((secs(5, 999) - 35.988).abs() < 1e-9);
        // The first three tries cover a brief network blip within ~10 s.
        assert!(secs(0, 999) + secs(1, 999) + secs(2, 999) < 11.0);
    }
    use futures_util::SinkExt;

    #[tokio::test]
    async fn graceful_stop_waits_for_a_running_session() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let running = tokio::spawn(run_with_shutdown(
            Config {
                managed: false,
                installation_scope: None,
                confirmation: None,
                failed_version: None,
                failed_reason: None,
                server: format!("http://{address}"),
                token: Some("fixture-token".into()),
                state_dir: dir.path().to_owned(),
            },
            stopped,
        ));
        let (tcp, _) = listener.accept().await.unwrap();
        let mut control = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let hello = control.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(hello.contains("fixture-token"));
        control
            .send(Message::Text(
                serde_json::to_string(&wire::Control::Welcome {
                    host_id: "1".into(),
                    key: Some("fixture-key".into()),
                    heartbeat_seconds: 20,
                    metrics_seconds: 30,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        control
            .send(Message::Text(
                serde_json::to_string(&wire::Control::OpenSession {
                    session: "job".into(),
                    kind: wire::SessionKind::Exec,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let (tcp, _) = listener.accept().await.unwrap();
        let mut session = tokio_tungstenite::accept_async(tcp).await.unwrap();
        session.next().await.unwrap().unwrap();
        let command = if cfg!(windows) {
            "echo BEFORE & ping -n 3 127.0.0.1 >NUL & echo AFTER"
        } else {
            "printf BEFORE; sleep 1; printf AFTER"
        };
        session
            .send(Message::Text(
                serde_json::to_string(&wire::Session::ExecStreamStart {
                    command: command.into(),
                    timeout_seconds: 15,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let first = session.next().await.unwrap().unwrap().into_text().unwrap();
        let first: wire::Session = serde_json::from_str(&first).unwrap();
        assert!(
            matches!(first, wire::Session::ExecOutput { data, .. } if String::from_utf8_lossy(&data).contains("BEFORE"))
        );
        stop.send(true).unwrap();
        tokio::task::yield_now().await;
        assert!(
            !running.is_finished(),
            "stop must wait for executing session"
        );
        let mut output = Vec::new();
        let mut exit = None;
        while let Some(Ok(message)) = session.next().await {
            if let Message::Text(text) = message {
                match serde_json::from_str::<wire::Session>(&text).unwrap() {
                    wire::Session::ExecOutput { data, .. } => output.extend(data),
                    wire::Session::ExecExit { code } => exit = Some(code),
                    other => panic!("unexpected session frame: {other:?}"),
                }
            }
        }
        assert!(
            String::from_utf8_lossy(&output).contains("AFTER"),
            "in-flight command was interrupted"
        );
        assert_eq!(exit, Some(0));
        tokio::time::timeout(Duration::from_secs(5), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn host_facts_are_reported_not_invented() {
        let (hostname, os, arch) = facts();
        assert!(!hostname.is_empty());
        assert!(!os.is_empty() && !arch.is_empty());
        let wire::Report::Metrics {
            memory_total,
            disk_total,
            ..
        } = metrics()
        else {
            panic!("metrics() must report metrics");
        };
        assert!(memory_total > 0, "a machine has memory");
        assert!(disk_total > 0, "and a root filesystem");
    }

    #[tokio::test]
    async fn graceful_stop_cuts_open_forward_sessions() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let local = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let running = tokio::spawn(run_with_shutdown(
            Config {
                managed: false,
                installation_scope: None,
                confirmation: None,
                failed_version: None,
                failed_reason: None,
                server: format!("http://{address}"),
                token: Some("fixture-token".into()),
                state_dir: dir.path().to_owned(),
            },
            stopped,
        ));
        let (tcp, _) = listener.accept().await.unwrap();
        let mut control = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let hello = control.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(hello.contains(r#""supports_forward":true"#), "{hello}");
        for frame in [
            wire::Control::Welcome {
                host_id: "1".into(),
                key: Some("fixture-key".into()),
                heartbeat_seconds: 20,
                metrics_seconds: 30,
            },
            wire::Control::OpenSession {
                session: "preview".into(),
                kind: wire::SessionKind::Forward,
            },
        ] {
            control
                .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
                .await
                .unwrap();
        }
        let (tcp, _) = listener.accept().await.unwrap();
        let mut session = tokio_tungstenite::accept_async(tcp).await.unwrap();
        session.next().await.unwrap().unwrap();
        session
            .send(Message::Text(
                serde_json::to_string(&wire::Session::ForwardStart {
                    port,
                    host: String::new(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let (mut held, _) = local.accept().await.unwrap();
        let ready = session.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(ready.contains("forward_ready"), "{ready}");
        // An idle preview connection would otherwise hold the stop forever.
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), running)
            .await
            .expect("stop must not wait for an open forward")
            .unwrap()
            .unwrap();
        let mut rest = Vec::new();
        use tokio::io::AsyncReadExt;
        tokio::time::timeout(Duration::from_secs(5), held.read_to_end(&mut rest))
            .await
            .expect("local connection must be released on stop")
            .unwrap();
    }
}
