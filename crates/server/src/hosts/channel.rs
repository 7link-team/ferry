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
use ferry_protocol::HostMonitor;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::{pin::Pin, time::Duration};

tokio::task_local! { static EXECUTION_TIMEZONE: Option<String>; }

pub(crate) fn execution_timezone() -> Option<Option<String>> {
    EXECUTION_TIMEZONE.try_with(Clone::clone).ok()
}

pub(crate) fn with_timezone<T>(
    timezone: Option<String>,
    work: impl std::future::Future<Output = T>,
) -> impl std::future::Future<Output = T> {
    // Allocate before entering the task-local scope; workflow executor futures
    // otherwise inflate each scope/poll frame on Windows' default worker stack.
    EXECUTION_TIMEZONE.scope(timezone, Box::pin(work))
}

pub(crate) fn timezone_unsupported(zone: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::CONFLICT,
        crate::i18n::Message::new("channel.timezoneUnsupported").with("zone", zone),
    )
}

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
    /// Captured on this connection only, never on the host or a shared Runner.
    fn set_timezone(&mut self, timezone: Option<String>) -> Result<()> {
        if let Some(zone) = timezone {
            return Err(timezone_unsupported(&zone));
        }
        Ok(())
    }

    async fn agent(
        &self,
        _request: ferry_protocol::agent::Request,
        _events: tokio::sync::mpsc::Sender<ferry_protocol::agent::Event>,
        _inputs: tokio::sync::mpsc::Receiver<ferry_protocol::agent::Input>,
        _cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.agentUpgrade"),
        ))
    }
    async fn agent_probe(
        &self,
        _runtime: ferry_protocol::agent::Runtime,
        _cwd: &str,
        _binding: Option<&ferry_protocol::agent::Binding>,
    ) -> Result<ferry_protocol::agent::Probe> {
        Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("agents.probeUnsupported"),
        ))
    }
    /// A byte pipe to a port on the host or its private network, for preview holes.
    async fn forward(&self, _address: &str, _port: u16) -> Result<crate::cluster::Socket> {
        Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("holes.connectionUnsupported"),
        ))
    }
    async fn monitor(&self) -> Result<HostMonitor>;
    async fn agent_workspace(
        &self,
        _runtime: ferry_protocol::agent::Runtime,
        _key: &str,
    ) -> Result<ferry_protocol::agent::Probe> {
        Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.agentUpgrade"),
        ))
    }
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

    async fn create_file(&self, path: &str) -> Result<()>;

    async fn create_dir(&self, path: &str) -> Result<()>;

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
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("channel.pathRequired"),
        ));
    }
    if ["\\\\?\\", "\\\\.\\", "//?/", "//./"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("channel.devicePathUnsupported"),
        ));
    }
    let drive = path.as_bytes().get(0).is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(1) == Some(&b':')
        && matches!(path.as_bytes().get(2), Some(b'/' | b'\\'));
    let unc = (path.starts_with("\\\\") || path.starts_with("//"))
        && path[2..]
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .count()
            >= 2;
    let original = path;
    let trimmed = path.trim_end_matches(if drive || unc {
        &['/', '\\'][..]
    } else {
        &['/'][..]
    });
    let path = if trimmed.is_empty() { "/" } else { trimmed };
    if !path.starts_with('/') && !drive && !unc {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("channel.absolutePathRequired"),
        ));
    }
    if path
        .split(if drive || unc {
            &['/', '\\'][..]
        } else {
            &['/'][..]
        })
        .any(|part| part == "..")
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("channel.parentPathForbidden"),
        ));
    }
    if path.contains('\0') || path.len() > 4096 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("channel.pathInvalid"),
        ));
    }
    // A bare drive prefix is relative on Windows; retain its root separator.
    if drive && path.len() == 2 {
        return Ok(&original[..3]);
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

    #[test]
    fn windows_paths_keep_drive_roots_and_reject_relative_or_parent_paths() {
        for path in [
            r"C:\",
            "D:/",
            "D:/Users/Full Stack",
            r"C:\Users\Full Stack",
            r"\\server\share\folder",
            "//server/share/folder",
        ] {
            assert_eq!(check_path(path).unwrap(), path);
        }
        for path in [
            "C:",
            "C:relative",
            r"C:\Users\..\Windows",
            "D:/a/../b",
            r"D:/a\..\b",
            r"\\server\share\..",
            "//server/share/..",
            r"\\?\C:\",
            "//?/C:/",
            r"\\.\PhysicalDrive0",
        ] {
            assert!(check_path(path).is_err(), "{path}");
        }
    }
}

#[cfg(test)]
mod timezone_tests {
    use super::*;
    use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, sea_query::Expr};
    #[tokio::test]
    async fn workspace_process_timezone_is_connection_scoped_and_frozen_override_keeps_legacy() {
        let (_dir, s) = crate::agents::tests::setup().await;
        for (workspace, zone) in [(1_i64, "Asia/Kuala_Lumpur"), (2_i64, "America/New_York")] {
            crate::entities::workspaces::Entity::update_many()
                .col_expr(
                    crate::entities::workspaces::Column::Timezone,
                    Expr::value(Some(zone)),
                )
                .filter(crate::entities::workspaces::Column::Id.eq(workspace))
                .exec(&s.db)
                .await
                .unwrap();
            let id = s.next_id().unwrap();
            let row = crate::hosts::fake::host_row(workspace, id.into(), "timezone", "online");
            crate::entities::hosts::Entity::insert(row.into_active_model())
                .exec(&s.db)
                .await
                .unwrap();
            let fake = crate::hosts::fake::register(id.into());
            let current = crate::hosts::remote::open(&s, &workspace.into(), &id)
                .await
                .unwrap();
            let frozen = with_timezone(
                Some("Pacific/Chatham".into()),
                crate::hosts::remote::open(&s, &workspace.into(), &id),
            )
            .await
            .unwrap();
            let legacy =
                with_timezone(None, crate::hosts::remote::open(&s, &workspace.into(), &id))
                    .await
                    .unwrap();
            // Capture is per channel; opening another one never mutates an existing channel.
            current.exec("date", Duration::from_secs(1)).await.unwrap();
            frozen.exec("date", Duration::from_secs(1)).await.unwrap();
            legacy.exec("date", Duration::from_secs(1)).await.unwrap();
            assert_eq!(
                *fake.process_timezones.lock().unwrap(),
                vec![Some(zone.into()), Some("Pacific/Chatham".into()), None]
            );
            assert!(execution_timezone().is_none());
        }
    }
}
