//! Reads a marketplace index straight from a Git repository: one shallow fetch
//! of the branch, no hosting API and therefore no API rate limit.
use crate::error::{ApiError, Result};
use crate::repositories::analysis::{Authorization, git_command as command};
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// A clone or fetch can take far longer than one API request.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(120);

fn failed(key: &'static str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new(key),
    )
}

/// Branch, tag or commit names are passed as refspecs; refuse anything Git
/// could read as an option, a second refspec or a ref expression.
pub(crate) fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 255
        && !branch.starts_with(['-', '/'])
        && !branch.ends_with(['/', '.'])
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

/// Username a token is presented with over HTTPS for well-known hosts.
fn token_user(url: &str) -> &'static str {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if host == "github.com" {
        "x-access-token"
    } else if host.contains("gitlab") {
        "oauth2"
    } else {
        "git"
    }
}

async fn git(
    dir: &std::path::Path,
    auth: &Authorization,
    args: &[&str],
    limit: usize,
) -> Result<Vec<u8>> {
    let mut cmd = command(dir, auth);
    // Tests read a local fixture repository; production allows only network transports.
    #[cfg(test)]
    cmd.args(["-c", "protocol.file.allow=always"]);
    let mut child = cmd.args(args).spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            failed("marketplace.gitUnavailable")
        } else {
            failed("marketplace.gitFailed")
        }
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("marketplace.gitFailed"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| failed("marketplace.gitFailed"))?;
    let read = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_reader = (&mut stdout).take(limit as u64 + 1);
        let mut err_reader = (&mut stderr).take(64 * 1024);
        let (a, b, status) = tokio::join!(
            out_reader.read_to_end(&mut out),
            err_reader.read_to_end(&mut err),
            child.wait()
        );
        a.and(b).map_err(|_| failed("marketplace.gitFailed"))?;
        let status = status.map_err(|_| failed("marketplace.gitFailed"))?;
        if out.len() > limit {
            return Err(failed("marketplace.indexTooLarge"));
        }
        if !status.success() {
            return Err(failed("marketplace.gitFailed"));
        }
        Ok(out)
    };
    tokio::time::timeout(TIMEOUT, read)
        .await
        .map_err(|_| failed("marketplace.gitTimeout"))?
}

/// Fetches `branch` at depth 1 into a private temporary repository and reads
/// `path` from it. Credentials travel only through the helper's environment.
pub(crate) async fn read_index(
    url: &str,
    branch: &str,
    path: &str,
    token: Option<&str>,
    limit: usize,
) -> Result<String> {
    if !valid_branch(branch) {
        return Err(failed("marketplace.sourceFieldsInvalid"));
    }
    let auth = Authorization {
        username: token
            .map(|_| token_user(url).to_owned())
            .unwrap_or_default(),
        secret: token.unwrap_or_default().to_owned(),
        key: None,
    };
    let dir = tempfile::tempdir().map_err(|_| failed("marketplace.gitFailed"))?;
    git(
        dir.path(),
        &auth,
        &["init", "--bare", "--quiet", "--template="],
        4096,
    )
    .await?;
    // A promisor remote lets `show` fetch just the index blob: commits and trees
    // only, never the repository's files (seconds instead of a full checkout).
    git(
        dir.path(),
        &auth,
        &["remote", "add", "origin", "--", url],
        4096,
    )
    .await?;
    // A branch, tag or commit; the remote resolves short names.
    let refspec = format!("+{branch}:refs/ferry/source");
    git(
        dir.path(),
        &auth,
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "--filter=blob:none",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "origin",
            &refspec,
        ],
        64 * 1024,
    )
    .await?;
    let object = format!("refs/ferry/source:{path}");
    let bytes = git(dir.path(), &auth, &["show", &object], limit).await?;
    String::from_utf8(bytes).map_err(|_| failed("marketplace.utf8Required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_cannot_smuggle_options_or_refspecs() {
        for ok in ["main", "release/0.5", "feature-x"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in [
            "",
            "-x",
            "--upload-pack=x",
            "a:b",
            "a b",
            "a..b",
            "a@{1}",
            "/x",
            "x/",
            "x.",
        ] {
            assert!(!valid_branch(bad), "{bad}");
        }
    }

    /// A real local repository: the index is read from the requested branch only.
    #[tokio::test]
    async fn reads_the_index_from_a_branch_without_any_hosting_api() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let sh = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .current_dir(&repo)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "{args:?}");
        };
        std::fs::create_dir_all(repo.join("marketplace")).unwrap();
        sh(&["init", "--quiet", "-b", "main"]);
        std::fs::write(repo.join("marketplace/index.json"), r#"{"v":"main"}"#).unwrap();
        sh(&["add", "."]);
        sh(&["commit", "--quiet", "-m", "main"]);
        sh(&["tag", "v1"]);
        sh(&["checkout", "--quiet", "-b", "other"]);
        std::fs::write(repo.join("marketplace/index.json"), r#"{"v":"other"}"#).unwrap();
        sh(&["commit", "--quiet", "-am", "other"]);
        let url = reqwest::Url::from_directory_path(&repo)
            .unwrap()
            .to_string();
        let text = read_index(&url, "main", "marketplace/index.json", None, 1024)
            .await
            .unwrap();
        assert_eq!(text, r#"{"v":"main"}"#);
        let text = read_index(&url, "other", "marketplace/index.json", None, 1024)
            .await
            .unwrap();
        assert_eq!(text, r#"{"v":"other"}"#);
        let text = read_index(&url, "v1", "marketplace/index.json", None, 1024)
            .await
            .unwrap();
        assert_eq!(text, r#"{"v":"main"}"#, "tags resolve too");
        assert!(
            read_index(&url, "missing", "marketplace/index.json", None, 1024)
                .await
                .is_err()
        );
        assert!(
            read_index(&url, "main", "nope.json", None, 1024)
                .await
                .is_err()
        );
        assert!(
            read_index(&url, "main", "marketplace/index.json", None, 4)
                .await
                .is_err()
        );
    }

    /// Opt-in: FERRY_GIT_LIVE_URL=https://github.com/<org>/<repo>.git FERRY_GIT_LIVE_PATH=marketplace/index.json
    #[tokio::test]
    #[ignore = "network: reads a public repository over Git"]
    async fn live_public_repository_over_git() {
        let url = std::env::var("FERRY_GIT_LIVE_URL").expect("FERRY_GIT_LIVE_URL");
        let path = std::env::var("FERRY_GIT_LIVE_PATH").expect("FERRY_GIT_LIVE_PATH");
        let text = read_index(
            &url,
            "main",
            &path,
            None,
            crate::marketplace::index::MAX_INDEX_BYTES,
        )
        .await
        .unwrap();
        let packages = crate::marketplace::index::parse_bundle(
            &text,
            &crate::marketplace::ferry_version(),
            "en-US",
        )
        .unwrap();
        println!(
            "LIVE GIT: {} bytes, {} packages",
            text.len(),
            packages.len()
        );
        assert!(!packages.is_empty());
    }
}
