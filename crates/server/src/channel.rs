//! The one way to reach a host.
//!
//! Everything that touches a remote machine — the terminal, file transfer,
//! deployment steps, pipeline steps, scheduled jobs — goes through
//! `HostChannel` and nothing else. Nobody calls ssh directly and nobody speaks
//! the runner protocol outside `runner.rs`. Adding a third way to reach a host
//! means writing one more implementation and extending `open`, and changes no
//! caller (§9.1 of the design doc).
//!
//! Streaming is part of the contract, not an optimisation: a file transfer
//! must not grow the server's memory with the size of the file, so reads and
//! writes are byte streams rather than buffers.

use crate::error::{ApiError, Result};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::{pin::Pin, time::Duration};

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    /// Terminals send nonsense sizes while a pane is being dragged, and a zero
    /// row count makes some shells emit nothing at all.
    pub fn sane(self) -> Self {
        Self {
            cols: self.cols.clamp(2, 500),
            rows: self.rows.clamp(1, 300),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize)]
pub struct DirEntry {
    pub name: String,
    /// "file" | "directory" | "symlink"
    pub kind: String,
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
}

#[derive(Debug, Serialize)]
pub struct FileMeta {
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
}

/// A live interactive terminal. Dropping it closes the remote session.
#[async_trait::async_trait]
pub trait Pty: Send {
    async fn write(&mut self, data: &[u8]) -> Result<()>;
    /// Returns `None` once the remote side has closed.
    async fn read(&mut self) -> Result<Option<Bytes>>;
    async fn resize(&mut self, size: PtySize) -> Result<()>;
}

#[async_trait::async_trait]
pub trait HostChannel: Send + Sync {
    /// One-shot command with captured output. Used by scheduled jobs and
    /// deployment steps, never for anything interactive.
    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output>;

    /// Streams raw chunks while running. Cancellation must terminate owned processes.
    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32>;

    async fn open_pty(&self, size: PtySize) -> Result<Box<dyn Pty>>;

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>>;

    /// Streamed download. The server never buffers the whole file.
    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)>;

    /// Streamed upload.
    async fn write_file(&self, path: &str, mode: u32, body: ByteStream) -> Result<()>;

    async fn delete_file(&self, path: &str) -> Result<()>;
}

/// Rejects paths that are not absolute, or that try to climb out of wherever
/// the operator thinks they are. The remote side would resolve `..` itself, so
/// this is about the request meaning what it appears to mean.
pub fn check_path(path: &str) -> Result<&str> {
    // An absent `?path=` must not quietly become "list the root"; only a path
    // that is genuinely all slashes collapses to "/".
    if path.is_empty() {
        return Err(ApiError::bad("请填写路径。"));
    }
    let trimmed = path.trim_end_matches('/');
    let path = if trimmed.is_empty() { "/" } else { trimmed };
    if !path.starts_with('/') {
        return Err(ApiError::bad("路径必须以 / 开头。"));
    }
    if path.split('/').any(|part| part == "..") {
        return Err(ApiError::bad("路径不能包含 ..，请填写完整路径。"));
    }
    if path.contains('\0') || path.len() > 4096 {
        return Err(ApiError::bad("路径无效。"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_sizes_are_clamped_to_something_a_shell_can_use() {
        let tiny = PtySize { cols: 0, rows: 0 }.sane();
        assert!(tiny.cols >= 2 && tiny.rows >= 1);
        let huge = PtySize {
            cols: u16::MAX,
            rows: u16::MAX,
        }
        .sane();
        assert!(huge.cols <= 500 && huge.rows <= 300);
        let normal = PtySize {
            cols: 120,
            rows: 40,
        }
        .sane();
        assert_eq!((normal.cols, normal.rows), (120, 40));
    }

    #[test]
    fn paths_must_be_absolute_and_free_of_parent_links() {
        assert_eq!(check_path("/etc/nginx").unwrap(), "/etc/nginx");
        assert_eq!(check_path("/").unwrap(), "/");
        assert_eq!(check_path("/var/log/").unwrap(), "/var/log");
        for bad in [
            "etc/passwd",
            "../etc/passwd",
            "/var/../etc/passwd",
            "/a/../../b",
            "",
            "relative",
        ] {
            assert!(check_path(bad).is_err(), "{bad} should be refused");
        }
        assert!(check_path("/a\0b").is_err());
        assert!(check_path(&format!("/{}", "x".repeat(5000))).is_err());
    }

    #[test]
    fn a_filename_containing_dots_is_still_fine() {
        // Only a whole `..` segment is a parent link.
        assert_eq!(check_path("/etc/..config").unwrap(), "/etc/..config");
        assert_eq!(check_path("/a/b..c/d").unwrap(), "/a/b..c/d");
    }
}
