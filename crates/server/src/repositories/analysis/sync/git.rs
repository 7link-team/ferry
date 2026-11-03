//! Server-native Git completion, using the saved connection's authorization.
use super::*;
use fs2::FileExt;
use std::{io::Write, path::Path as FilePath, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

fn failure(key: &'static str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new(key),
    )
}

fn git_error() -> ApiError {
    failure("repository.collector.gitFailed")
}

async fn bounded(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(65537)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| git_error())?;
    if bytes.len() > 65536 {
        return Err(git_error());
    }
    Ok(bytes)
}

struct Authorization {
    username: String,
    secret: String,
    key: Option<tempfile::NamedTempFile>,
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn command(repository: &FilePath, auth: &Authorization) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("GIT_") || name.starts_with("SSH_") {
            command.env_remove(key);
        }
    }
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .args(["--no-pager", "-c"])
        .arg(format!("core.hooksPath={null}"))
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "credential.helper=",
            "-c",
            "core.askPass=",
            "-c",
            "credential.useHttpPath=true",
            "-c",
            "http.followRedirects=false",
            "-c",
            "http.sslVerify=true",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.http.allow=always",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "protocol.git.allow=always",
            "-c",
            "fetch.recurseSubmodules=false",
            "-c",
            "gc.auto=0",
            "-C",
        ])
        .arg(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(key) = &auth.key {
        command.env("GIT_SSH_COMMAND", format!(
            "ssh -F {} -o BatchMode=yes -o StrictHostKeyChecking=yes -o IdentitiesOnly=yes -o IdentityAgent=none -i {}",
            quote(null), quote(&key.path().to_string_lossy())
        ));
        command.env("GIT_SSH_VARIANT", "ssh");
    } else if !auth.secret.is_empty() {
        // The helper's argv contains references only. Values travel in its environment
        // and protocol pipes, and are never persisted in Git config or remote URLs.
        command.args(["-c", "credential.helper=!f() { cat >/dev/null; [ \"$1\" = get ] || exit 0; printf 'username=%s\\npassword=%s\\n\\n' \"$__FERRY_GIT_USER\" \"$__FERRY_GIT_SECRET\"; }; f"])
            .env("__FERRY_GIT_USER", &auth.username)
            .env("__FERRY_GIT_SECRET", &auth.secret);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

async fn run(repository: &FilePath, auth: &Authorization, args: &[&str]) -> Result<()> {
    let mut child = command(repository, auth)
        .args(args)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                failure("repository.collector.gitUnavailable")
            } else {
                git_error()
            }
        })?;
    let stdout = child.stdout.take().ok_or_else(git_error)?;
    let stderr = child.stderr.take().ok_or_else(git_error)?;
    let operation = async {
        let (_, _, status) = tokio::try_join!(bounded(stdout), bounded(stderr), async {
            child.wait().await.map_err(|_| git_error())
        })?;
        if !status.success() {
            return Err(git_error());
        }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(900), operation)
        .await
        .map_err(|_| git_error())?
}

pub(super) async fn collect(s: &AppState, claim: &super::Claim) -> Result<()> {
    let repository = repositories::Entity::find_by_id(claim.source.repository_id)
        .filter(repositories::Column::WorkspaceId.eq(claim.source.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let scope = Scope::for_actor_in(
        &s.db,
        claim.connection.workspace_id.into(),
        claim.connection.user_id.into(),
    )
    .await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    let (_, credential) = crate::repositories::auth::resolve_saved_connection(
        s,
        &s.db,
        &scope,
        claim.connection.id.into(),
        &repository.url,
    )
    .await?;
    // Reuse the provider's refresh lock and vault rather than duplicating OAuth.
    crate::repositories::auth::ensure_fresh(s, scope.workspace_id, credential).await?;
    let (kind, username, secret) = if i64::from(credential) == 0 {
        (String::new(), String::new(), String::new())
    } else {
        crate::credentials::unseal(s, &scope.workspace_id, &credential).await?
    };
    if username.contains(['\r', '\n', '\0'])
        || (kind != "ssh-key" && secret.contains(['\r', '\n', '\0']))
    {
        return Err(git_error());
    }
    let key = if kind == "ssh-key" {
        // tempfile creates a private 0600 file on Unix. The handle keeps it alive
        // across the child process and deletes it on cancellation as well.
        let mut key = tempfile::NamedTempFile::new().map_err(|_| git_error())?;
        key.write_all(secret.as_bytes()).map_err(|_| git_error())?;
        key.flush().map_err(|_| git_error())?;
        Some(key)
    } else {
        None
    };
    let auth = Authorization {
        username: if kind == "token" && claim.connection.provider == "gitlab" {
            "oauth2".into()
        } else if kind == "token" && claim.connection.provider == "github" {
            "x-access-token".into()
        } else if username.is_empty() {
            "git".into()
        } else {
            username
        },
        secret: if key.is_some() { String::new() } else { secret },
        key,
    };
    let cache = s.data_dir.join("repository-analysis-git");
    tokio::fs::create_dir_all(&cache)
        .await
        .map_err(|_| git_error())?;
    // Rotating an access token must not discard fetched objects or duplicate
    // the cache. Authorization is resolved afresh and publication is fenced.
    let name = claim.source.id.to_string();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cache.join(format!("{name}.lock")))
        .map_err(|_| git_error())?;
    lock.try_lock_exclusive()
        .map_err(|_| failure("repository.collector.gitBusy"))?;
    // The OS lock also fences a cancelled worker's cache from a successor. The
    // database lease remains the authority for importing facts and publishing refs.
    let bare = cache.join(format!("{name}.git"));
    tokio::fs::create_dir_all(&bare)
        .await
        .map_err(|_| git_error())?;
    set_phase(s, claim, "git_fetch").await?;
    run(&bare, &auth, &["init", "--bare", "--template="]).await?;
    run(
        &bare,
        &auth,
        &[
            "fetch",
            "--quiet",
            "--force",
            "--prune",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--",
            &repository.url,
            "+refs/heads/*:refs/heads/*",
        ],
    )
    .await?;
    let known: BTreeSet<String> = commits::Entity::find()
        .filter(commits::Column::RepositoryId.eq(claim.source.repository_id))
        .filter(commits::Column::AnalysisVersion.eq(ANALYSIS_VERSION))
        .filter(if claim.source.coverage_start == 0 {
            sea_orm::Condition::all().add(commits::Column::Complete.eq(1_i64))
        } else {
            // Out-of-range immutable metadata already proves ancestry. Reuse
            // it too; expanding history later will require its full statistics.
            sea_orm::Condition::any()
                .add(commits::Column::Complete.eq(1_i64))
                .add(commits::Column::CommitterDate.lt(claim.source.coverage_start))
        })
        .all(&s.db)
        .await?
        .into_iter()
        .map(|row| row.sha)
        .collect();
    set_phase(s, claim, "git_analyze").await?;
    let artifact = super::git_facts::collect(
        &bare,
        claim.source.id.to_string(),
        claim.job.generation,
        claim.source.remote_path.clone(),
        claim.source.coverage_start,
        &known,
    )
    .await?;
    super::pipeline::import(s, claim, artifact).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Set;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    #[ignore = "requires public GitHub network access"]
    async fn public_git_repository_publishes_without_pipeline_or_runner() {
        use sea_orm::PaginatorTrait;
        let (_directory, state) = super::super::tests::fixture().await;
        connections::Entity::update_many()
            .col_expr(connections::Column::Provider, Expr::value("git"))
            .col_expr(connections::Column::CredentialId, Expr::value(0_i64))
            .filter(connections::Column::Id.eq(4))
            .exec(&state.db)
            .await
            .unwrap();
        repositories::Entity::update_many()
            .col_expr(
                repositories::Column::Url,
                Expr::value("https://github.com/octocat/Hello-World.git"),
            )
            .filter(repositories::Column::Id.eq(6))
            .exec(&state.db)
            .await
            .unwrap();
        let mut previous = 0;
        for _ in 0..2 {
            let claim = super::super::tests::claimed(&state).await;
            super::super::run_claim(&state, claim).await.unwrap();
            let job = jobs::Entity::find_by_id(7)
                .one(&state.db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(job.state, "complete");
            let count = commits::Entity::find().count(&state.db).await.unwrap();
            assert!(count > 0);
            if previous > 0 {
                assert_eq!(count, previous, "incremental runs must not duplicate facts");
            }
            previous = count;
            assert_eq!(
                crate::entities::pipeline_runs::Entity::find()
                    .count(&state.db)
                    .await
                    .unwrap(),
                0
            );
            dirty(&state, &state.db, 7, 1).await.unwrap();
        }
    }

    #[tokio::test]
    async fn credential_helper_uses_pipes_and_does_not_persist_the_secret() {
        let directory = tempfile::tempdir().unwrap();
        let auth = Authorization {
            username: "account with spaces".into(),
            secret: "private-token-'-$-value".into(),
            key: None,
        };
        let mut configured = command(directory.path(), &auth);
        assert!(
            configured
                .as_std()
                .get_args()
                .all(|arg| !arg.to_string_lossy().contains(&auth.secret))
        );
        let mut child = configured
            .args(["credential", "fill"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=repository.invalid\npath=team/repo.git\n\n")
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(output.status.success());
        let supplied = String::from_utf8(output.stdout).unwrap();
        assert!(
            supplied
                .lines()
                .any(|line| line == format!("username={}", auth.username))
        );
        assert!(
            supplied
                .lines()
                .any(|line| line == format!("password={}", auth.secret))
        );
        assert!(!directory.path().join(".git-credentials").exists());
        assert!(!directory.path().join("config").exists());
    }

    #[tokio::test]
    async fn fetch_cannot_use_local_file_or_external_helper_transports() {
        let directory = tempfile::tempdir().unwrap();
        let auth = Authorization {
            username: String::new(),
            secret: String::new(),
            key: None,
        };
        run(directory.path(), &auth, &["init", "--bare", "--template="])
            .await
            .unwrap();
        for remote in [
            "file:///never-read-this-repository",
            "ext::echo unexpected-helper",
        ] {
            assert!(
                run(directory.path(), &auth, &["ls-remote", "--", remote])
                    .await
                    .is_err()
            );
        }
        let config = tokio::fs::read_to_string(directory.path().join("config"))
            .await
            .unwrap();
        assert!(!config.contains("helper"));
    }

    #[tokio::test]
    async fn authenticated_http_fetch_tracks_all_heads_and_prunes_deleted_heads() {
        use axum::{
            Router,
            body::Body,
            http::{HeaderMap, Response, StatusCode},
            routing::get,
        };
        use base64::Engine;
        let origin = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let auth = Authorization {
            username: "test-user".into(),
            secret: "private-test-token".into(),
            key: None,
        };
        run(
            origin.path(),
            &auth,
            &["init", "--initial-branch=main", "--template="],
        )
        .await
        .unwrap();
        run(
            origin.path(),
            &auth,
            &[
                "-c",
                "user.name=Tester",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        )
        .await
        .unwrap();
        run(origin.path(), &auth, &["branch", "feature"])
            .await
            .unwrap();
        run(origin.path(), &auth, &["update-server-info"])
            .await
            .unwrap();
        let git_dir = origin.path().join(".git");
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", auth.username, auth.secret))
        );
        let app = Router::new().route(
            "/{*path}",
            get(
                move |axum::extract::Path(path): axum::extract::Path<String>,
                      headers: HeaderMap| {
                    let git_dir = git_dir.clone();
                    let expected = expected.clone();
                    async move {
                        if headers.get("authorization").and_then(|v| v.to_str().ok())
                            != Some(expected.as_str())
                        {
                            return Response::builder()
                                .status(StatusCode::UNAUTHORIZED)
                                .header("www-authenticate", "Basic realm=repository")
                                .body(Body::empty())
                                .unwrap();
                        }
                        if path.split('/').any(|part| part == "..") {
                            return Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(Body::empty())
                                .unwrap();
                        }
                        match tokio::fs::read(git_dir.join(path)).await {
                            Ok(bytes) => Response::new(Body::from(bytes)),
                            Err(_) => Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Body::empty())
                                .unwrap(),
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        run(cache.path(), &auth, &["init", "--bare", "--template="])
            .await
            .unwrap();
        let fetch = [
            "fetch",
            "--quiet",
            "--force",
            "--prune",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            &url,
            "+refs/heads/*:refs/heads/*",
        ];
        run(cache.path(), &auth, &fetch).await.unwrap();
        let first = super::super::git_facts::collect(
            cache.path(),
            "test".into(),
            1,
            "repo".into(),
            0,
            &BTreeSet::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            first.refs.keys().cloned().collect::<Vec<_>>(),
            ["feature", "main"]
        );
        run(origin.path(), &auth, &["branch", "-D", "feature"])
            .await
            .unwrap();
        run(origin.path(), &auth, &["update-server-info"])
            .await
            .unwrap();
        run(cache.path(), &auth, &fetch).await.unwrap();
        let second = super::super::git_facts::collect(
            cache.path(),
            "test".into(),
            2,
            "repo".into(),
            0,
            &BTreeSet::new(),
        )
        .await
        .unwrap();
        assert_eq!(second.refs.keys().cloned().collect::<Vec<_>>(), ["main"]);
        // Exercise the durable worker, authorization, vault, cache, importer and
        // publication together. The fixture role has no pipelines.run permission.
        use sea_orm::PaginatorTrait;
        let (_database, state) = super::super::tests::fixture().await;
        crate::entities::credentials::Entity::insert(crate::entities::credentials::ActiveModel {
            id: Set(5),
            workspace_id: Set(1),
            name: Set("Git fixture".into()),
            kind: Set("token".into()),
            scope: Set("build".into()),
            username: Set(auth.username.clone()),
            secret: Set(state
                .vault
                .seal(&auth.secret, &crate::vault::aad("1", "credentials", "5"))
                .unwrap()),
            description: Set(String::new()),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&state.db)
        .await
        .unwrap();
        connections::Entity::update_many()
            .col_expr(connections::Column::Provider, Expr::value("git"))
            .filter(connections::Column::Id.eq(4))
            .exec(&state.db)
            .await
            .unwrap();
        repositories::Entity::update_many()
            .col_expr(repositories::Column::Url, Expr::value(url.clone()))
            .filter(repositories::Column::Id.eq(6))
            .exec(&state.db)
            .await
            .unwrap();
        for _ in 0..2 {
            let claim = super::super::tests::claimed(&state).await;
            super::super::run_claim(&state, claim).await.unwrap();
            let job = jobs::Entity::find_by_id(7)
                .one(&state.db)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(job.state, "complete");
            assert_eq!(job.generation, job.completed_generation);
            assert_eq!(commits::Entity::find().count(&state.db).await.unwrap(), 1);
            assert!(
                sources::Entity::find_by_id(7)
                    .one(&state.db)
                    .await
                    .unwrap()
                    .unwrap()
                    .synced_at
                    > 0
            );
            dirty(&state, &state.db, 7, 1).await.unwrap();
        }
        server.abort();
    }
}
