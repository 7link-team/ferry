//! Pure Git polling reads advertised refs on a selected Runner, never a checkout.
use super::{Config, connections, polling};
use crate::{
    channel::HostChannel,
    entities::hosts,
    error::{ApiError, Result},
    pipeline_runs::{credential_environment, quote},
    snowflake::Id,
    state::AppState,
    workspace::Scope,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::{mpsc, watch};

const SETUP: Duration = Duration::from_secs(15);
const QUERY: Duration = Duration::from_secs(120);
const MAX_OUTPUT: usize = 16 * 1024 * 1024;

struct CancelOnDrop(watch::Sender<bool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

fn failed() -> ApiError {
    // Neither command errors nor Git stderr may expose credential-bearing URLs.
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("repository.monitor.gitCheckFailed"),
    )
}

pub(super) async fn snapshot(
    s: &AppState,
    connection: &connections::Model,
    config: &Config,
) -> Result<polling::Snapshot> {
    if connection.provider != "git" || connection.state != "connected" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.gitConnectionRequired"),
        ));
    }
    if config.all_repositories || config.repositories.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.gitRepositoriesRequired"),
        ));
    }
    let scope =
        Scope::for_actor(s, connection.workspace_id.into(), connection.user_id.into()).await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    scope.require("hosts.terminal")?;
    let runner: Id = config.runner_id.parse().map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.gitRunnerRequired"),
        )
    })?;
    let host = hosts::Entity::find_by_id(i64::from(runner))
        .filter(hosts::Column::WorkspaceId.eq(connection.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if host.connection != "runner" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.monitor.gitRunnerOnly"),
        ));
    }
    for address in &config.repositories {
        let (provider, credential) = crate::repository_auth::resolve_connection(
            s,
            &s.db,
            &scope,
            connection.id.into(),
            address,
        )
        .await?;
        if provider != "git" || i64::from(credential) != connection.credential_id {
            return Err(failed());
        }
    }
    let credential = if connection.credential_id == 0 {
        None
    } else {
        crate::repository_auth::ensure_fresh(
            s,
            connection.workspace_id.into(),
            connection.credential_id.into(),
        )
        .await?;
        Some(
            crate::credentials::unseal(
                s,
                &connection.workspace_id.into(),
                &connection.credential_id.into(),
            )
            .await?,
        )
    };
    let ch = crate::remote::open(s, &connection.workspace_id.into(), &runner)
        .await
        .map_err(|_| failed())?;
    let windows = host.os == "windows";
    let addresses = config.repositories.clone();
    let (cancel, stopped) = watch::channel(false);
    let _cancel_on_drop = CancelOnDrop(cancel);
    // A dropped polling/lease future must not drop the credential cleanup future.
    // The owned task finishes its bounded Git command and cleans up independently.
    tokio::spawn(async move {
        let (ch, private): (Box<dyn HostChannel>, String) = if windows {
            let shell = crate::windows_shell::GitShell::open(ch)
                .await
                .map_err(|_| failed())?;
            let private = shell
                .private_dir("git-monitor")
                .await
                .map_err(|_| failed())?;
            (Box::new(shell), private)
        } else {
            let private = format!("/tmp/git-monitor-{}", uuid::Uuid::new_v4());
            let made = ch
                .exec(&format!("umask 077; mkdir {}", quote(&private)), SETUP)
                .await
                .map_err(|_| failed())?;
            if made.code != 0 {
                return Err(failed());
            }
            (ch, private)
        };
        let result = read_refs(
            ch.as_ref(),
            &private,
            credential.as_ref(),
            &addresses,
            stopped,
        )
        .await;
        let cleaned = ch
            .exec(&format!("rm -rf -- {}", quote(&private)), SETUP)
            .await
            .map_err(|_| failed())?;
        if cleaned.code != 0 {
            return Err(failed());
        }
        result
    })
    .await
    .map_err(|_| failed())?
}

async fn read_refs(
    ch: &dyn HostChannel,
    private: &str,
    credential: Option<&(String, String, String)>,
    addresses: &[String],
    stopped: watch::Receiver<bool>,
) -> Result<polling::Snapshot> {
    let mut snapshot = BTreeMap::new();
    for address in addresses {
        if *stopped.borrow() {
            return Err(failed());
        }
        let environment = environment(ch, private, credential).await?;
        if *stopped.borrow() {
            return Err(failed());
        }
        let output = capture(ch, &script(private, &environment, address), stopped.clone()).await?;
        snapshot.insert(address.clone(), parse(&output)?);
    }
    Ok(snapshot)
}

async fn environment(
    ch: &dyn HostChannel,
    private: &str,
    credential: Option<&(String, String, String)>,
) -> Result<String> {
    let mut environment = credential_environment(ch, private, credential)
        .await
        .map_err(|_| failed())?;
    if !credential.is_some_and(|(kind, _, _)| kind != "ssh-key") {
        // The shared helper only resets credential helpers for password auth.
        environment.push_str(
            "unset GIT_ASKPASS SSH_ASKPASS\nexport GIT_CONFIG_COUNT=2\nexport GIT_CONFIG_KEY_0=credential.helper\nexport GIT_CONFIG_VALUE_0=''\nexport GIT_CONFIG_KEY_1=core.askPass\nexport GIT_CONFIG_VALUE_1=''\n",
        );
    }
    Ok(environment)
}

fn script(private: &str, environment: &str, address: &str) -> String {
    // Git runs without a local repository, inherited config, or arbitrary remote
    // helpers. The URL has already passed the connection's protocol validation.
    format!(
        "umask 077\ntrap {} EXIT\ntrap 'exit 1' HUP INT TERM\nunset GIT_CONFIG_PARAMETERS GIT_CONFIG_COUNT GIT_DIR GIT_WORK_TREE\nexport GIT_CONFIG_NOSYSTEM=1\nexport GIT_CONFIG_GLOBAL=/dev/null\nexport GIT_ALLOW_PROTOCOL=http:https:ssh:git\n{environment}cd {} || exit 1\ngit ls-remote --heads --tags --refs -- {}\n",
        quote(&format!(
            "rm -f -- {} {} {} {}",
            quote(&format!("{private}/key")),
            quote(&format!("{private}/username")),
            quote(&format!("{private}/password")),
            quote(&format!("{private}/credential-helper")),
        )),
        quote(private),
        quote(address),
    )
}

async fn capture(
    ch: &dyn HostChannel,
    script: &str,
    mut owner: watch::Receiver<bool>,
) -> Result<Vec<u8>> {
    let (output, mut receiver) = mpsc::channel(16);
    let (cancel, stop) = watch::channel(*owner.borrow());
    let exec = ch.exec_stream(script, QUERY, output, stop);
    let collect = async {
        let mut bytes = Vec::new();
        let mut oversized = false;
        let mut cancelled = *owner.borrow();
        loop {
            let next = tokio::select! {
                next = receiver.recv() => next,
                _ = owner.changed(), if !cancelled => {
                    cancelled = true;
                    let _ = cancel.send(true);
                    continue;
                }
            };
            let Some((stream, chunk)) = next else { break };
            if stream == "stdout" && !oversized {
                if chunk.len() > MAX_OUTPUT.saturating_sub(bytes.len()) {
                    oversized = true;
                    let _ = cancel.send(true);
                    bytes.clear();
                } else {
                    bytes.extend_from_slice(&chunk);
                }
            }
        }
        if oversized || cancelled {
            Err(failed())
        } else {
            Ok(bytes)
        }
    };
    let (status, bytes) = tokio::join!(exec, collect);
    if status.map_err(|_| failed())? != 0 {
        return Err(failed());
    }
    bytes
}

fn parse(bytes: &[u8]) -> Result<polling::Repository> {
    let text = std::str::from_utf8(bytes).map_err(|_| failed())?;
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(failed());
    }
    let mut result = polling::Repository::default();
    for line in text.lines() {
        let (sha, reference) = line.split_once('\t').ok_or_else(failed)?;
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(failed());
        }
        let (map, name) = if let Some(name) = reference.strip_prefix("refs/heads/") {
            (&mut result.branches, name)
        } else if let Some(name) = reference.strip_prefix("refs/tags/") {
            (&mut result.tags, name)
        } else {
            return Err(failed());
        };
        if !valid_ref(name) || map.insert(name.into(), sha.to_ascii_lowercase()).is_some() {
            return Err(failed());
        }
    }
    Ok(result)
}

fn valid_ref(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .bytes()
            .any(|b| b <= b' ' || b == 127 || b"~^:?*[\\".contains(&b))
        && name
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_snapshot_owner_signals_cleanup_task() {
        let (cancel, stopped) = watch::channel(false);
        let guard = CancelOnDrop(cancel);
        assert!(!*stopped.borrow());
        drop(guard);
        assert!(*stopped.borrow());
    }

    #[test]
    fn reads_heads_and_unpeeled_tags_with_both_object_formats() {
        let sha1 = "a".repeat(40);
        let sha256 = "B".repeat(64);
        let refs =
            parse(format!("{sha1}\trefs/heads/feature/a\n{sha256}\trefs/tags/v1\n").as_bytes())
                .unwrap();
        assert_eq!(refs.branches["feature/a"], sha1);
        assert_eq!(refs.tags["v1"], sha256.to_ascii_lowercase());
        assert!(parse(b"").unwrap().branches.is_empty());
    }

    #[test]
    fn rejects_incomplete_malformed_and_duplicate_refs() {
        let sha = "a".repeat(40);
        for text in [
            format!("{sha}\trefs/heads/main"),
            format!("{sha}\trefs/heads/main\n{sha}\trefs/heads/main\n"),
            format!("{sha}\tHEAD\n"),
            format!("{sha}\trefs/tags/v1^{{}}\n"),
            format!("{sha}\trefs/heads/a..b\n"),
            format!("{sha}\trefs/heads/a.lock\n"),
            format!("{sha}\trefs/heads/a\tb\n"),
            "z".repeat(40) + "\trefs/heads/main\n",
            "a".repeat(39) + "\trefs/heads/main\n",
            "\n".into(),
        ] {
            assert!(parse(text.as_bytes()).is_err(), "{text:?}");
        }
        assert!(parse(&[255, b'\n']).is_err());
    }

    #[test]
    fn script_quotes_urls_and_installs_cleanup_before_git() {
        let text = script(
            "/tmp/private dir",
            "export GIT_TERMINAL_PROMPT=0\n",
            "https://host/a'$(echo bad).git",
        );
        assert!(text.contains(
            "git ls-remote --heads --tags --refs -- 'https://host/a'\\''$(echo bad).git'"
        ));
        assert!(text.find("trap ").unwrap() < text.find("git ls-remote").unwrap());
        assert!(text.contains("GIT_CONFIG_GLOBAL=/dev/null"));
        assert!(text.contains("GIT_ALLOW_PROTOCOL=http:https:ssh:git"));
        assert!(!text.contains("git clone"));
        assert!(!text.contains("git fetch"));
    }
}
