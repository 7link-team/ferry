//! Wire contract between the Ferry server and a Ferry runner.
//!
//! Shared by both binaries so a protocol change cannot compile on one side
//! only. No HTTP, database or filesystem dependencies belong here.

use serde::{Deserialize, Serialize};

/// Protocol revision. The server refuses a runner that does not match.
pub const VERSION: u32 = 1;

/// Runner -> server, first frame on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    /// Enrollment token (first connection) or the runner's stored key.
    pub token: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
}

/// Server -> runner on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// Enrollment accepted; the runner stores `key` and reconnects with it.
    Welcome {
        host_id: String,
        key: String,
        heartbeat_seconds: u64,
    },
    /// Open a dedicated session connection back to the server.
    OpenSession {
        session: String,
        kind: SessionKind,
    },
    Ping,
    Shutdown {
        reason: String,
    },
}

/// Runner -> server on the control connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Report {
    Pong,
    /// Periodic host facts for the dashboard.
    Metrics {
        load1: f64,
        memory_used: u64,
        memory_total: u64,
        disk_used: u64,
        disk_total: u64,
    },
    SessionFailed {
        session: String,
        error: String,
    },
}

/// What a dedicated session connection carries. One connection per session
/// keeps the control channel free of bulk data and needs no multiplexer.
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
}

/// Framing inside a session connection. Control frames are JSON text frames;
/// payload bytes travel as binary frames, unwrapped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Session {
    TerminalStart {
        shell: Option<String>,
        cols: u16,
        rows: u16,
    },
    TerminalResize {
        cols: u16,
        rows: u16,
    },
    FileStart {
        path: String,
        mode: Option<u32>,
    },
    FileMeta {
        size: u64,
        mode: u32,
        modified_at: i64,
    },
    ExecStart {
        command: String,
        timeout_seconds: u64,
    },
    ExecDone {
        code: i32,
    },
    Eof,
    Failed {
        error: String,
    },
}
