//! Wire contract between the Ferry server and a Ferry runner.
//!
//! Shared by both binaries so a protocol change cannot compile on one side
//! only. No HTTP, database or filesystem dependencies belong here.
//!
//! Two kinds of connection, both WebSocket, both dialled by the runner:
//!
//! * the **control** connection (`/api/runners/connect`) — one per runner, held
//!   open, carrying `Hello`/`Control`/`Report` as JSON text frames and no bulk
//!   data at all;
//! * a **session** connection (`/api/runners/sessions/{session}`) — one per
//!   terminal, transfer or command, opened on demand after the server sends
//!   `Control::OpenSession`. Text frames are `Session` control, binary frames
//!   are payload bytes, unwrapped.
//
// ponytail: one connection per session instead of a multiplexer. No stream
// ids, no flow control, no head-of-line blocking — an ops host runs a handful
// of concurrent sessions. Revisit only if that stops being true.

pub mod release;

use serde::{Deserialize, Serialize};

/// Protocol revision. Either side refuses a peer that does not match.
pub const VERSION: u32 = 1;

/// How a runner proves who it is. Enrolment and reconnection are separate
/// cases on purpose: a stolen enrolment token must not be usable as a key, and
/// a key must name the host it belongs to so the server can check it against
/// that host's hash alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Auth {
    /// First connection: a single-use, TTL-limited token from the host page.
    Enroll { token: String },
    /// Every connection after that: the long-lived key from `Welcome`.
    Key { host_id: String, key: String },
}

/// Runner -> server, first frame on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub auth: Auth,
    pub hostname: String,
    #[serde(default)]
    pub ip_address: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    #[serde(default)]
    pub managed_updates: bool,
    #[serde(default)]
    pub supports_monitor: bool,
}

/// Runner -> server, first frame on a session connection. The session id is in
/// the URL; this frame is what binds the connection to the runner that was
/// asked to open it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHello {
    pub version: u32,
    pub host_id: String,
    pub key: String,
}

/// Server -> runner on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// Connection accepted. `key` is present only on enrolment; the runner
    /// stores it and reconnects with it from then on.
    Welcome {
        host_id: String,
        key: Option<String>,
        heartbeat_seconds: u64,
        metrics_seconds: u64,
    },
    /// Open a dedicated session connection back to the server.
    OpenSession { session: String, kind: SessionKind },
    /// Signed catalog; only sent after complete host tasks have drained.
    Update {
        request: String,
        version: String,
        manifest: String,
        signature: String,
    },
    /// Server-side liveness probe; the runner answers `Report::Pong`.
    Ping,
    /// Refused for good: wrong protocol version, bad token, unknown key. The
    /// runner reports this and stops rather than retrying forever.
    Rejected { reason: String },
}

/// Runner -> server on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Report {
    Pong,
    UpdateHealth {
        version: String,
        failed_version: Option<String>,
    },
    UpdateFailed {
        request: String,
        error: String,
    },
    /// Periodic host facts for the dashboard. Doubles as the runner's own
    /// heartbeat: silence in this direction is what makes the server hang up.
    Metrics {
        load1: f64,
        #[serde(default)]
        cpu_percent: f64,
        memory_used: u64,
        memory_total: u64,
        disk_used: u64,
        disk_total: u64,
    },
    /// The runner could not dial the session it was asked for, so the server
    /// can fail the waiting request instead of timing out.
    SessionFailed {
        session: String,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkUsage {
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostMonitor {
    pub cpu_percent: f64,
    pub load1: f64,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disks: Vec<DiskUsage>,
    pub networks: Vec<NetworkUsage>,
}

/// What a dedicated session connection carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// Interactive pty for xterm.js.
    Terminal,
    /// Server -> runner file write.
    Upload,
    /// Runner -> server file read.
    Download,
    /// One-shot command with captured output.
    Exec,
    /// Directory listing and deletion: metadata only, no bulk data.
    Files,
    Monitor,
}

/// One directory entry. Mirrors the server's `channel::DirEntry`, which cannot
/// live here because it belongs to the channel abstraction rather than the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    /// "file" | "directory" | "symlink"
    pub kind: String,
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
}

/// Framing inside a session connection. Control frames are JSON text frames;
/// payload bytes travel as binary frames, unwrapped.
///
/// The server opens with exactly one request frame and the runner answers it.
/// `Failed` may replace any answer, and is the only error shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Session {
    // ---- server -> runner requests
    TerminalStart {
        cols: u16,
        rows: u16,
    },
    TerminalResize {
        cols: u16,
        rows: u16,
    },
    ExecStart {
        command: String,
        timeout_seconds: u64,
    },
    /// Additive streaming command protocol; legacy ExecStart remains unchanged.
    ExecStreamStart {
        command: String,
        timeout_seconds: u64,
    },
    ExecCancel,
    ExecOutput {
        stream: String,
        data: Vec<u8>,
    },
    ExecExit {
        code: i32,
    },
    /// Download: the runner answers `FileMeta`, then binary frames, then `Eof`.
    ReadFile {
        path: String,
    },
    /// Upload: the server then sends binary frames and `Eof`; the runner
    /// answers `Done`.
    WriteFile {
        path: String,
        mode: u32,
    },
    ListDir {
        path: String,
    },
    Monitor,
    MonitorResult {
        metrics: HostMonitor,
    },
    CreateFile {
        path: String,
    },
    CreateDir {
        path: String,
    },
    DeleteFile {
        path: String,
    },

    // ---- runner -> server answers
    FileMeta {
        size: u64,
        mode: u32,
        modified_at: i64,
    },
    DirEntries {
        entries: Vec<Entry>,
    },
    /// Captured output rather than binary frames: it is bounded by the runner,
    /// and stdout and stderr have to stay apart.
    ExecDone {
        code: i32,
        stdout: String,
        stderr: String,
    },
    /// The request is finished and nothing more follows.
    Done,
    /// End of a byte stream, in whichever direction it was flowing.
    Eof,
    Failed {
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frames both binaries actually have to agree on, through the exact
    /// encoding the wire uses. A renamed field or a changed tag fails here
    /// rather than at 2am against a runner that is already deployed.
    #[test]
    fn frames_survive_a_round_trip() {
        let hello = Hello {
            version: VERSION,
            auth: Auth::Enroll {
                token: "7f3a".into(),
            },
            hostname: "web-01".into(),
            ip_address: "192.168.0.10".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            agent_version: "0.1.0".into(),
            managed_updates: false,
            supports_monitor: true,
        };
        let text = serde_json::to_string(&hello).unwrap();
        assert!(text.contains(r#""type":"enroll""#), "{text}");
        let back: Hello = serde_json::from_str(&text).unwrap();
        assert!(matches!(back.auth, Auth::Enroll { token } if token == "7f3a"));
        assert_eq!(back.ip_address, "192.168.0.10");
        assert!(back.supports_monitor);
        let mut legacy: serde_json::Value = serde_json::from_str(&text).unwrap();
        legacy.as_object_mut().unwrap().remove("ip_address");
        legacy.as_object_mut().unwrap().remove("supports_monitor");
        assert!(
            !serde_json::from_value::<Hello>(legacy.clone())
                .unwrap()
                .supports_monitor
        );
        assert_eq!(
            serde_json::from_value::<Hello>(legacy).unwrap().ip_address,
            ""
        );

        for frame in [
            Control::Welcome {
                host_id: "h1".into(),
                key: Some("k".into()),
                heartbeat_seconds: 20,
                metrics_seconds: 30,
            },
            Control::OpenSession {
                session: "a1b2".into(),
                kind: SessionKind::Terminal,
            },
            Control::Ping,
            Control::Rejected {
                reason: "版本不匹配".into(),
            },
        ] {
            let text = serde_json::to_string(&frame).unwrap();
            serde_json::from_str::<Control>(&text).unwrap();
        }

        for frame in [
            Report::Pong,
            Report::Metrics {
                load1: 0.5,
                cpu_percent: 12.5,
                memory_used: 1,
                memory_total: 2,
                disk_used: 3,
                disk_total: 4,
            },
            Report::SessionFailed {
                session: "a1b2".into(),
                error: "拨号失败".into(),
            },
        ] {
            let text = serde_json::to_string(&frame).unwrap();
            serde_json::from_str::<Report>(&text).unwrap();
        }

        for frame in [
            Session::TerminalStart {
                cols: 120,
                rows: 40,
            },
            Session::TerminalResize { cols: 80, rows: 24 },
            Session::ExecStart {
                command: "echo hi".into(),
                timeout_seconds: 15,
            },
            Session::ReadFile {
                path: "/etc/hosts".into(),
            },
            Session::WriteFile {
                path: "/tmp/x".into(),
                mode: 0o644,
            },
            Session::ListDir {
                path: "/etc".into(),
            },
            Session::DeleteFile {
                path: "/tmp/x".into(),
            },
            Session::FileMeta {
                size: 10,
                mode: 0o644,
                modified_at: 1,
            },
            Session::DirEntries {
                entries: vec![Entry {
                    name: "hosts".into(),
                    kind: "file".into(),
                    size: 10,
                    mode: 0o644,
                    modified_at: 1,
                }],
            },
            Session::ExecDone {
                code: 2,
                stdout: "out".into(),
                stderr: "err".into(),
            },
            Session::Done,
            Session::Eof,
            Session::Failed { error: "no".into() },
        ] {
            let text = serde_json::to_string(&frame).unwrap();
            serde_json::from_str::<Session>(&text).unwrap();
        }
    }

    /// An unknown frame must not deserialize into the wrong arm: that is how a
    /// version skew turns into silent misbehaviour instead of a clean refusal.
    #[test]
    fn an_unknown_frame_is_an_error_not_a_guess() {
        assert!(serde_json::from_str::<Control>(r#"{"type":"teleport"}"#).is_err());
        assert!(serde_json::from_str::<Session>(r#"{"type":"exec_done"}"#).is_err());
        assert!(serde_json::from_str::<SessionKind>(r#""vnc""#).is_err());
    }

    /// The session key frame must not be satisfiable by an enrolment token, and
    /// vice versa — the two auth paths stay distinct on the wire.
    #[test]
    fn the_two_auth_paths_do_not_overlap() {
        let enroll = serde_json::to_string(&Auth::Enroll { token: "t".into() }).unwrap();
        let key = serde_json::to_string(&Auth::Key {
            host_id: "h".into(),
            key: "k".into(),
        })
        .unwrap();
        assert!(matches!(
            serde_json::from_str::<Auth>(&enroll).unwrap(),
            Auth::Enroll { .. }
        ));
        assert!(matches!(
            serde_json::from_str::<Auth>(&key).unwrap(),
            Auth::Key { .. }
        ));
        // A token with no host_id cannot be read as a key.
        assert!(serde_json::from_str::<Auth>(r#"{"type":"key","key":"k"}"#).is_err());
    }
}
