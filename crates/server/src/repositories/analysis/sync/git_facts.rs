//! Native, read-only contribution facts. Fetching and cache locking belong to the caller.
use crate::error::{ApiError, Result};
use ferry_protocol::contribution::{ANALYSIS_VERSION, Artifact, Commit, SCHEMA_VERSION};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

const OUTPUT_LIMIT: usize = 256 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("repository.collector.artifactFormatInvalid"),
    )
}

async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| invalid())?;
    if bytes.len() > limit {
        return Err(invalid());
    }
    Ok(bytes)
}

async fn git(repository: &Path, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut command = tokio::process::Command::new("git");
    // Strip all inherited Git controls, including injected config and executable paths.
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "--no-replace-objects",
            "--no-pager",
            "-c",
            "core.quotePath=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.allow=never",
            "-C",
        ])
        .arg(repository)
        .args(args)
        .kill_on_drop(true)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| invalid())?;
    let stdout = child.stdout.take().ok_or_else(invalid)?;
    let stderr = child.stderr.take().ok_or_else(invalid)?;
    let stdin = child.stdin.take();
    let operation = async {
        let write = async move {
            if let (Some(mut writer), Some(bytes)) = (stdin, input) {
                writer.write_all(&bytes).await.map_err(|_| invalid())?;
            }
            Ok::<_, ApiError>(())
        };
        let ((), output, _, status) = tokio::try_join!(
            write,
            read_bounded(stdout, OUTPUT_LIMIT),
            read_bounded(stderr, 64 * 1024),
            async { child.wait().await.map_err(|_| invalid()) }
        )?;
        if !status.success() {
            return Err(invalid());
        }
        Ok(output)
    };
    tokio::time::timeout(COMMAND_TIMEOUT, operation)
        .await
        .map_err(|_| invalid())?
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn identity(bytes: &[u8]) -> Result<(String, String, i64)> {
    let end = bytes.iter().rposition(|b| *b == b'>').ok_or_else(invalid)?;
    let start = bytes[..end]
        .iter()
        .rposition(|b| *b == b'<')
        .ok_or_else(invalid)?;
    if start == 0 || bytes[start - 1] != b' ' {
        return Err(invalid());
    }
    let suffix = std::str::from_utf8(&bytes[end + 1..]).map_err(|_| invalid())?;
    let fields: Vec<_> = suffix.split_whitespace().collect();
    if fields.len() != 2
        || fields[1].len() != 5
        || !matches!(fields[1].as_bytes()[0], b'+' | b'-')
        || !fields[1].as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid());
    }
    let date = fields[0]
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(1000))
        .ok_or_else(invalid)?;
    Ok((
        text(&bytes[..start - 1]),
        text(&bytes[start + 1..end]),
        date,
    ))
}

fn metadata(sha: &str, raw: &[u8]) -> Result<(Commit, bool)> {
    let split = raw
        .windows(2)
        .position(|s| s == b"\n\n")
        .ok_or_else(invalid)?;
    let mut commit = Commit {
        sha: sha.to_owned(),
        message: text(&raw[split + 2..]),
        ..Commit::default()
    };
    let mut author = false;
    let mut committer = false;
    let mut reencode = false;
    for line in raw[..split].split(|b| *b == b'\n') {
        if let Some(value) = line.strip_prefix(b"parent ") {
            let parent = text(value);
            if !ferry_protocol::contribution::valid_sha(&parent) {
                return Err(invalid());
            }
            commit.parents.push(parent);
        } else if let Some(value) = line.strip_prefix(b"author ") {
            (commit.author_name, commit.author_email, commit.author_date) = identity(value)?;
            author = true;
        } else if let Some(value) = line.strip_prefix(b"committer ") {
            commit.committer_date = identity(value)?.2;
            committer = true;
        } else if let Some(value) = line.strip_prefix(b"encoding ") {
            reencode =
                !value.eq_ignore_ascii_case(b"utf-8") && !value.eq_ignore_ascii_case(b"utf8");
        }
    }
    if !author || !committer {
        return Err(invalid());
    }
    Ok((commit, reencode))
}

fn count_extension(languages: &mut BTreeMap<String, i64>, path: &[u8]) -> Result<()> {
    let name = path.rsplit(|b| *b == b'/').next().unwrap_or_default();
    let extension = name
        .iter()
        .rposition(|b| *b == b'.')
        .map(|n| text(&name[n..]))
        .unwrap_or_default();
    let count = languages.entry(extension).or_default();
    *count = count.checked_add(1).ok_or_else(invalid)?;
    Ok(())
}

async fn analyze(repository: &Path, commit: &mut Commit) -> Result<()> {
    for (index, parent) in commit.parents.iter().enumerate() {
        let rows = git(
            repository,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--numstat",
                "-z",
                parent,
                &commit.sha,
                "--",
            ],
            None,
        )
        .await?;
        for row in rows.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let mut fields = row.splitn(3, |b| *b == b'\t');
            let mut number = || -> Result<i64> {
                let value = fields.next().ok_or_else(invalid)?;
                if value == b"-" {
                    return Ok(0);
                }
                let count = std::str::from_utf8(value)
                    .map_err(|_| invalid())?
                    .parse::<i64>()
                    .map_err(|_| invalid())?;
                if count < 0 {
                    return Err(invalid());
                }
                Ok(count)
            };
            commit.additions = commit
                .additions
                .checked_add(number()?)
                .ok_or_else(invalid)?;
            commit.deletions = commit
                .deletions
                .checked_add(number()?)
                .ok_or_else(invalid)?;
            let path = fields.next().ok_or_else(invalid)?;
            if index == 0 {
                count_extension(&mut commit.language_stats, path)?;
            }
        }
    }
    if commit.parents.is_empty() {
        let rows = git(
            repository,
            &["ls-tree", "-r", "-z", &commit.sha, "--"],
            None,
        )
        .await?;
        for row in rows.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let split = row.iter().position(|b| *b == b'\t').ok_or_else(invalid)?;
            if row[..split].split(|b| *b == b' ').nth(1) == Some(b"blob".as_slice()) {
                count_extension(&mut commit.language_stats, &row[split + 1..])?;
            }
        }
    }
    if !commit.valid() {
        return Err(invalid());
    }
    Ok(())
}

pub(super) async fn collect(
    repository: &Path,
    task_id: String,
    generation: i64,
    remote_path: String,
    coverage_start: i64,
    known: &BTreeSet<String>,
) -> Result<Artifact> {
    if git(repository, &["rev-parse", "--is-shallow-repository"], None).await? != b"false\n" {
        return Err(invalid());
    }
    let mut refs = BTreeMap::new();
    let rows = git(
        repository,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)",
            "refs/heads/",
            "refs/remotes/",
        ],
        None,
    )
    .await?;
    for row in rows.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
        let fields: Vec<_> = row.split(|b| *b == 0).collect();
        if fields.len() != 3 {
            return Err(invalid());
        }
        if !fields[2].is_empty() {
            continue;
        }
        let reference = text(fields[0]);
        let name = if let Some(name) = reference.strip_prefix("refs/heads/") {
            name
        } else {
            reference
                .strip_prefix("refs/remotes/")
                .and_then(|r| r.split_once('/'))
                .ok_or_else(invalid)?
                .1
        };
        let sha = text(fields[1]);
        if !ferry_protocol::contribution::valid_sha(&sha) {
            return Err(invalid());
        }
        if let Some(previous) = refs.insert(name.to_owned(), sha.clone()) {
            if previous != sha {
                return Err(invalid());
            }
        }
    }
    let mut reachable = BTreeMap::new();
    let mut ref_commits = BTreeMap::new();
    for (name, target) in &refs {
        let mut members = Vec::new();
        // Never prune by date: parent clocks may run forward or backward.
        let graph = git(repository, &["rev-list", "--timestamp", target, "--"], None).await?;
        for row in graph.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
            let row = std::str::from_utf8(row).map_err(|_| invalid())?;
            let (seconds, sha) = row.split_once(' ').ok_or_else(invalid)?;
            if !ferry_protocol::contribution::valid_sha(sha) {
                return Err(invalid());
            }
            let millis = seconds
                .parse::<i128>()
                .ok()
                .and_then(|n| n.checked_mul(1000));
            let timestamp = match millis
                .and_then(|n| i64::try_from(n).ok())
                .filter(|n| *n != 0)
            {
                Some(timestamp) => timestamp,
                None => {
                    metadata(
                        sha,
                        &git(repository, &["cat-file", "commit", sha], None).await?,
                    )?
                    .0
                    .committer_date
                }
            };
            reachable.insert(sha.to_owned(), timestamp);
            if coverage_start == 0 || timestamp >= coverage_start {
                members.push(sha.to_owned());
            }
        }
        members.sort();
        ref_commits.insert(name.clone(), members);
    }
    let pending: Vec<_> = reachable
        .keys()
        .filter(|sha| !known.contains(*sha))
        .collect();
    let mut commits = Vec::with_capacity(pending.len());
    if !pending.is_empty() {
        let mut input = Vec::new();
        for sha in &pending {
            input.extend_from_slice(sha.as_bytes());
            input.push(b'\n');
        }
        let batch = git(repository, &["cat-file", "--batch"], Some(input)).await?;
        let mut rest = batch.as_slice();
        for sha in pending {
            let end = rest.iter().position(|b| *b == b'\n').ok_or_else(invalid)?;
            let header = std::str::from_utf8(&rest[..end]).map_err(|_| invalid())?;
            let fields: Vec<_> = header.split(' ').collect();
            if fields.len() != 3 || fields[0] != sha || fields[1] != "commit" {
                return Err(invalid());
            }
            let length = fields[2].parse::<usize>().map_err(|_| invalid())?;
            rest = &rest[end + 1..];
            let raw = rest.get(..length).ok_or_else(invalid)?;
            let (mut commit, reencode) = metadata(sha, raw)?;
            if rest.get(length) != Some(&b'\n') {
                return Err(invalid());
            }
            rest = &rest[length + 1..];
            if reencode {
                // Git's own iconv support avoids a second encoding implementation.
                let encoded = git(
                    repository,
                    &[
                        "-c",
                        "i18n.logOutputEncoding=UTF-8",
                        "log",
                        "-1",
                        "--no-show-signature",
                        "--format=format:%an%x00%ae%x00%B",
                        sha,
                        "--",
                    ],
                    None,
                )
                .await?;
                let fields: Vec<_> = encoded.splitn(3, |b| *b == 0).collect();
                if fields.len() != 3 {
                    return Err(invalid());
                }
                commit.author_name = text(fields[0]);
                commit.author_email = text(fields[1]);
                commit.message = text(fields[2]);
            }
            commit.complete = coverage_start == 0 || reachable[sha] >= coverage_start;
            if commit.complete {
                analyze(repository, &mut commit).await?;
            }
            commits.push(commit);
        }
        if !rest.is_empty() {
            return Err(invalid());
        }
    }
    Ok(Artifact {
        schema_version: SCHEMA_VERSION,
        analysis_version: ANALYSIS_VERSION,
        task_id,
        generation,
        repository: remote_path,
        coverage_start,
        refs,
        ref_commits,
        commits,
        complete: true,
    })
}

#[cfg(test)]
#[path = "git_facts_tests.rs"]
mod tests;
