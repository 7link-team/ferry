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

    // Reconnect with backoff, forever: a server restart or a flapping link is
    // normal operation for an agent, not a reason to stop.
    let mut backoff = Duration::from_secs(1);
    let mut sessions = tokio::task::JoinSet::new();
    let mut activation = None;
    loop {
        let result = tokio::select! {
            biased;
            _ = stopped.wait_for(|value| *value) => break,
            result = hold(&cfg, &control_url, &mut sessions) => result,
        };
        match result {
            Ok(()) => {
                tracing::warn!("控制连接已关闭，准备重连");
                backoff = Duration::from_secs(1);
            }
            Err(error) if error.is::<update::Prepared>() => {
                activation = Some(error.downcast::<update::Prepared>()?.0);
                break;
            }
            Err(error) if error.is::<Rejected>() => {
                tracing::error!(%error, "接入被拒绝，不再重试");
                return Err(error);
            }
            Err(error) => tracing::warn!(%error, "控制连接失败，{}s 后重试", backoff.as_secs()),
        }
        tokio::select! {
            _ = stopped.wait_for(|value| *value) => break,
            _ = tokio::time::sleep(backoff) => {},
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
    tracing::info!(
        active_sessions = sessions.len(),
        "停止接收会话，等待在途会话结束"
    );
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
            supports_monitor: true,
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
                        wire::Control::Update { request, version, manifest, signature } => {
                            if !cfg.managed || !preparing.is_empty() {
                                send(&mut socket, &wire::Report::UpdateFailed { request, error: "代理不是托管服务或已有升级进行中".into(), message: Some(wire::SystemMessage::new("runner.updateUnavailable")) }).await?;
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
                            sessions.spawn(async move {
                                if let Err(error) = session::run(&url, hello, kind).await {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                confirmation: None,
                failed_version: None,
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
}
