//! Windows hosts run the server's POSIX job scripts through Git for Windows'
//! `sh.exe`; Git is required there anyway to check out repositories. Each
//! command is written to a script file first, so it never depends on cmd.exe
//! quoting rules. Paths are native and use forward slashes (`C:/Users/...`),
//! which Git's sh, the runner's file API and the Actions worker all accept.

use crate::{
    channel::{ByteStream, DirEntry, FileMeta, HostChannel, Output, Pty, PtySize},
    error::{ApiError, Result},
};
use ferry_protocol::HostMonitor;
use std::time::Duration;

const SETUP: Duration = Duration::from_secs(15);

pub struct GitShell {
    inner: Box<dyn HostChannel>,
    /// `C:\Program Files\Git\bin\sh.exe`, quoted for cmd.exe.
    sh: String,
    /// The account's temporary directory, `C:/...`.
    temp: String,
    /// The account the runner executes as, for the private directory's ACL.
    sid: String,
}

/// `C:/Program Files/Git/mingw64/libexec/git-core` -> `C:/Program Files/Git/bin/sh.exe`.
fn sh_from_exec_path(exec_path: &str) -> Option<String> {
    let path = exec_path.trim().replace('\\', "/");
    let (prefix, _) = path.rsplit_once("/libexec/git-core")?;
    let (root, _) = prefix.trim_end_matches('/').rsplit_once('/')?;
    Some(format!("{root}/bin/sh.exe"))
}

/// The second field of `whoami /user /fo csv /nh`: `"host\user","S-1-5-..."`.
fn sid_from_whoami(text: &str) -> Option<String> {
    let sid = text.trim().rsplit(',').next()?.trim().trim_matches('"');
    (sid.starts_with("S-1-") && sid[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .then(|| sid.to_owned())
}

impl GitShell {
    pub async fn open(inner: Box<dyn HostChannel>) -> Result<Self> {
        let missing = || {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("windows.gitRequired"),
            )
        };
        // Runners before alpha.32 escaped quotes so cmd.exe saw \"…\"; every
        // command below quotes a path, so check the runner passes them intact.
        let echoed = inner.exec("echo \"ferry\"", SETUP).await?;
        if echoed.stdout.trim() != "\"ferry\"" {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("windows.runnerTooOld"),
            ));
        }
        let found = inner.exec("git --exec-path", SETUP).await?;
        if found.code != 0 {
            return Err(missing());
        }
        let sh = sh_from_exec_path(&found.stdout).ok_or_else(missing)?;
        let sh = sh.replace('/', "\\");
        let exists = inner
            .exec(
                &format!("if exist \"{sh}\" (echo yes) else (echo no)"),
                SETUP,
            )
            .await?;
        if exists.stdout.trim() != "yes" {
            return Err(missing());
        }
        let temp = inner.exec("echo %TEMP%", SETUP).await?;
        let temp = temp.stdout.trim().replace('\\', "/");
        if temp.len() < 3 || temp.contains('%') {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("windows.tempUnknown"),
            ));
        }
        // System tools by full path: Git's optional Unix tools on PATH
        // provide a `whoami` that rejects Windows switches.
        let user = inner
            .exec(
                "\"%SystemRoot%\\System32\\whoami.exe\" /user /fo csv /nh",
                SETUP,
            )
            .await?;
        let sid = sid_from_whoami(&user.stdout).ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("windows.accountUnknown"),
            )
        })?;
        Ok(Self {
            inner,
            sh: format!("\"{sh}\""),
            temp: temp.trim_end_matches('/').to_owned(),
            sid,
        })
    }

    /// A new directory only this account and SYSTEM can open.
    pub async fn private_dir(&self, prefix: &str) -> Result<String> {
        let path = format!("{}/{prefix}-{}", self.temp, uuid::Uuid::new_v4());
        let native = path.replace('/', "\\");
        let made = self
            .inner
            .exec(
                &format!(
                    "mkdir \"{native}\" && \"%SystemRoot%\\System32\\icacls.exe\" \"{native}\" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F /grant:r *{}:(OI)(CI)F",
                    self.sid
                ),
                SETUP,
            )
            .await?;
        if made.code != 0 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("windows.tempCreateFailed"),
            ));
        }
        Ok(path)
    }

    async fn script(&self, command: &str) -> Result<String> {
        let path = format!("{}/ferry-{}.sh", self.temp, uuid::Uuid::new_v4());
        let body = bytes::Bytes::from(format!("{command}\n"));
        self.inner
            .write_file(&path, 0o600, Box::pin(futures::stream::iter([Ok(body)])))
            .await?;
        Ok(path)
    }

    async fn forget(&self, script: &str) {
        let _ = self.inner.delete_file(script).await;
    }
}

#[async_trait::async_trait]
impl HostChannel for GitShell {
    async fn monitor(&self) -> Result<HostMonitor> {
        self.inner.monitor().await
    }

    async fn exec(&self, command: &str, timeout: Duration) -> Result<Output> {
        let script = self.script(command).await?;
        let output = self
            .inner
            .exec(&format!("{} \"{script}\"", self.sh), timeout)
            .await;
        self.forget(&script).await;
        output
    }

    async fn exec_stream(
        &self,
        command: &str,
        timeout: Duration,
        output: tokio::sync::mpsc::Sender<(String, Vec<u8>)>,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<i32> {
        let script = self.script(command).await?;
        let code = self
            .inner
            .exec_stream(
                &format!("{} \"{script}\"", self.sh),
                timeout,
                output,
                cancel,
            )
            .await;
        self.forget(&script).await;
        code
    }

    async fn open_pty(&self, size: PtySize) -> Result<Box<dyn Pty>> {
        self.inner.open_pty(size).await
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        self.inner.list_dir(path).await
    }

    async fn create_file(&self, path: &str) -> Result<()> {
        self.inner.create_file(path).await
    }

    async fn create_dir(&self, path: &str) -> Result<()> {
        self.inner.create_dir(path).await
    }

    async fn read_file(&self, path: &str) -> Result<(FileMeta, ByteStream)> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &str, mode: u32, body: ByteStream) -> Result<()> {
        self.inner.write_file(path, mode, body).await
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
        self.inner.delete_file(path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_git_sh_from_the_exec_path() {
        assert_eq!(
            sh_from_exec_path("C:/Program Files/Git/mingw64/libexec/git-core\r\n").as_deref(),
            Some("C:/Program Files/Git/bin/sh.exe")
        );
        assert_eq!(
            sh_from_exec_path(r"D:\Tools\Git\clangarm64\libexec\git-core").as_deref(),
            Some("D:/Tools/Git/bin/sh.exe")
        );
        assert_eq!(sh_from_exec_path("/usr/lib/git-core"), None);
    }

    #[test]
    fn reads_only_a_well_formed_sid() {
        assert_eq!(
            sid_from_whoami("\"host\\person\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(),
            Some("S-1-5-21-1-2-3-1001")
        );
        assert_eq!(
            sid_from_whoami("\"nt authority\\system\",\"S-1-5-18\"").as_deref(),
            Some("S-1-5-18")
        );
        assert_eq!(sid_from_whoami("\"x\",\"S-1-5-18 & del *\""), None);
    }
}
