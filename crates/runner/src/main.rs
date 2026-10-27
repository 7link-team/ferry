//! Ferry runner: one command enrols a host, then holds an outbound control
//! connection so the server can reach it without an inbound SSH route.
//!
//! The control connection carries no bulk data. For a terminal, a transfer or
//! a command the server sends `OpenSession` and the runner dials a second,
//! dedicated connection for it.
//
// ponytail: one connection per session instead of a multiplexer. An ops host
// runs a handful of concurrent sessions; move to yamux only if that stops
// being true.

mod exec_stream;
mod session;

use anyhow::Context;
use clap::Parser;
use ferry_protocol as wire;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(version, about = "Ferry 运维代理")]
struct Config {
    /// Server base URL, e.g. https://ops.example.com
    #[arg(long, env = "FERRY_SERVER")]
    server: String,
    /// Enrollment token from the host page, used once on the first connection.
    #[arg(long, env = "FERRY_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Where the runner keeps its host id and key after enrollment.
    #[arg(long, env = "FERRY_STATE_DIR", default_value = "/var/lib/ferry-runner")]
    state_dir: PathBuf,
}

/// What survives a restart. Written with 0600: it is the host's credential.
#[derive(Serialize, Deserialize)]
struct Enrolment {
    host_id: String,
    key: String,
}

impl Enrolment {
    fn path(state_dir: &std::path::Path) -> PathBuf {
        state_dir.join("runner.json")
    }

    fn load(state_dir: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(state_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self, state_dir: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("无法创建状态目录 {}", state_dir.display()))?;
        let path = Self::path(state_dir);
        std::fs::write(&path, serde_json::to_vec(self)?)
            .with_context(|| format!("无法写入 {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// `http://host:port` -> `ws://host:port/path`, `https://` -> `wss://`.
/// Taking a base URL rather than a WebSocket URL keeps the one-command install
/// honest: the operator pastes the address they already use in a browser.
fn ws_url(base: &str, path: &str) -> anyhow::Result<String> {
    let base = base.trim().trim_end_matches('/');
    let rest = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_owned()
    } else {
        anyhow::bail!("服务地址需要以 http:// 或 https:// 开头，当前为「{base}」。");
    };
    anyhow::ensure!(
        !rest.ends_with("://") && rest.split("://").nth(1).is_some_and(|h| !h.is_empty()),
        "服务地址缺少主机名：「{base}」。"
    );
    Ok(format!("{rest}{path}"))
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn dial(url: &str) -> anyhow::Result<Socket> {
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    Ok(socket)
}

async fn send<T: Serialize>(socket: &mut Socket, frame: &T) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(frame)?.into()))
        .await?;
    Ok(())
}

/// Reads frames until a JSON text frame arrives, which is the only shape
/// control information ever takes.
async fn next_frame<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> anyhow::Result<T> {
    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .with_context(|| format!("无法解析服务下发的帧：{text}"));
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    anyhow::bail!("连接已关闭，未收到预期的帧。")
}

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
        memory_used: system.used_memory(),
        memory_total: system.total_memory(),
        disk_used,
        disk_total,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferry_runner=info".into()),
        )
        .init();
    let cfg = Config::parse();
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
    loop {
        match hold(&cfg, &control_url).await {
            Ok(()) => {
                tracing::warn!("控制连接已关闭，准备重连");
                backoff = Duration::from_secs(1);
            }
            Err(error) if error.is::<Rejected>() => {
                tracing::error!(%error, "接入被拒绝，不再重试");
                return Err(error);
            }
            Err(error) => tracing::warn!(%error, "控制连接失败，{}s 后重试", backoff.as_secs()),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// One life of the control connection: handshake, then forward until it ends.
async fn hold(cfg: &Config, control_url: &str) -> anyhow::Result<()> {
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
            os,
            arch,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
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

    let session_base = cfg.server.clone();
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
                        wire::Control::OpenSession { session: id, kind } => {
                            let url = ws_url(&session_base, &format!("/api/runners/sessions/{id}"))?;
                            let hello = wire::SessionHello {
                                version: wire::VERSION,
                                host_id: enrolment.host_id.clone(),
                                key: enrolment.key.clone(),
                            };
                            // Dialled on its own task: a slow transfer must not
                            // stall heartbeats or the next session request.
                            tokio::spawn(async move {
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
            _ = report.tick() => send(&mut socket, &metrics()).await?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_url_becomes_the_matching_websocket_url() {
        assert_eq!(
            ws_url("http://127.0.0.1:4290", "/api/runners/connect").unwrap(),
            "ws://127.0.0.1:4290/api/runners/connect"
        );
        // A trailing slash is what an operator pastes out of a browser.
        assert_eq!(
            ws_url("https://ops.example.com/", "/api/runners/connect").unwrap(),
            "wss://ops.example.com/api/runners/connect"
        );
        assert_eq!(
            ws_url("ws://127.0.0.1:9/", "/x").unwrap(),
            "ws://127.0.0.1:9/x"
        );
        for bad in ["ops.example.com", "ftp://ops", "http://", ""] {
            assert!(ws_url(bad, "/x").is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_enrolment_file_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("ferry-runner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Enrolment::load(&dir).is_none(), "nothing stored yet");
        Enrolment {
            host_id: "h-1".into(),
            key: "k-1".into(),
        }
        .save(&dir)
        .unwrap();
        let back = Enrolment::load(&dir).expect("stored");
        assert_eq!((back.host_id.as_str(), back.key.as_str()), ("h-1", "k-1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Enrolment::path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the key must not be world-readable");
        }
        std::fs::remove_dir_all(&dir).unwrap();
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
