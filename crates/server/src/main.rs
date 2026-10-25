mod assets;
mod auth;
mod channel;
mod credentials;
mod db;
mod deployments;
mod entities;
mod error;
mod hosts;
mod lock;
mod manifest;
mod migration;
mod pipeline_runs;
mod pipelines;
mod remote;
mod repositories;
mod resource;
mod runner;
mod snowflake;
mod ssh;
mod state;
mod vault;
mod worker;
mod workspace;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use fs2::FileExt;
use sea_orm::{ConnectOptions, Database};
use sea_orm_migration::MigratorTrait;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::trace::TraceLayer;

#[derive(Parser)]
#[command(version, about = "Ferry 运维平台服务")]
struct Config {
    #[arg(long, env = "FERRY_DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    #[arg(long, env = "FERRY_DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    #[arg(long, env = "FERRY_BIND", default_value = "127.0.0.1:4290")]
    bind: SocketAddr,
    /// Docker/web deployment: no desktop launcher owns this process.
    #[arg(long, env = "FERRY_SERVER_MODE", default_value_t = false)]
    server_mode: bool,
    #[arg(long, env = "FERRY_SECURE_COOKIE", default_value_t = false)]
    secure_cookie: bool,
    /// Owned desktop sidecar: closing the private stdin pipe drains work and exits.
    #[arg(long, hide = true)]
    desktop_control: bool,
    /// Hostnames browsers may reach this server on. localhost is always allowed.
    #[arg(long, env = "FERRY_ALLOWED_HOSTS", value_delimiter = ',')]
    allowed_hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferry_server=info,tower_http=info".into()),
        )
        .init();
    let mut cfg = Config::parse();
    // Compose substitutions are empty strings when unset.
    cfg.database_url = cfg.database_url.filter(|v| !v.is_empty());

    std::fs::create_dir_all(&cfg.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cfg.data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cfg.data_dir.join("server.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("该数据目录已有服务运行，不能重复启动。"))?;

    let instance_id = instance_id(&cfg.data_dir)?;
    let db = match &cfg.database_url {
        Some(url) => Database::connect(ConnectOptions::new(url.clone())).await?,
        None => {
            let path = cfg.data_dir.canonicalize()?.join("ferry.sqlite");
            let mut options = ConnectOptions::new("sqlite://ferry");
            options.sqlx_logging(false).map_sqlx_sqlite_opts(move |o| {
                o.filename(&path)
                    .create_if_missing(true)
                    .journal_mode(sea_orm::sqlx::sqlite::SqliteJournalMode::Wal)
                    .foreign_keys(true)
            });
            Database::connect(options).await?
        }
    };
    migration::Migrator::up(&db, None).await?;

    let vault = vault::Vault::open(&cfg.data_dir, cfg.server_mode)?;
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    let worker = Arc::new(
        worker::Worker::allocate(&db, &instance_id)
            .await
            .map_err(|error| anyhow::anyhow!("无法分配安全的 worker id，拒绝启动: {error}"))?,
    );
    let (heartbeat_stop, heartbeat_rx) = tokio::sync::watch::channel(false);
    let heartbeat_worker = worker.clone();
    let heartbeat_db = db.clone();
    let heartbeat_task = tokio::spawn(async move {
        heartbeat_worker.run(&heartbeat_db, heartbeat_rx).await;
    });

    let s: state::AppState = Arc::new(state::State {
        db,
        data_dir: cfg.data_dir.clone(),
        instance_id,
        server_mode: cfg.server_mode,
        secure_cookie: cfg.secure_cookie,
        bind: cfg.bind,
        vault,
        ids: worker.generator.clone(),
    });

    let (drain, drain_rx) = tokio::sync::watch::channel(false);
    let (pipeline_stop, pipeline_rx) = tokio::sync::watch::channel(false);
    let pipeline_task = tokio::spawn(pipeline_runs::scheduler(
        s.clone(),
        pipeline_rx,
        drain_rx.clone(),
    ));

    let (deployment_stop, deployment_rx) = tokio::sync::watch::channel(false);
    let deployment_task = tokio::spawn(deployments::scheduler(s.clone(), deployment_rx, drain_rx));

    let mut hosts = cfg.allowed_hosts;
    hosts.extend(["localhost".into(), "127.0.0.1".into(), "[::1]".into()]);
    let hosts = Arc::new(hosts);

    // Workspace resources all sit under /api/workspaces/{workspace}/..., which
    // is what makes the Scope extractor the only way to obtain a workspace id.
    let workspace_api = Router::new()
        .route(
            "/{workspace}/deployment-revisions/{id}",
            get(deployments::revision_get),
        )
        .route(
            "/{workspace}/deployments",
            get(deployments::list).post(deployments::create),
        )
        .route(
            "/{workspace}/deployments/{id}",
            get(deployments::get)
                .patch(deployments::update)
                .delete(deployments::remove),
        )
        .route(
            "/{workspace}/deployment-revisions",
            get(deployments::revision_list),
        )
        .route(
            "/{workspace}/deployments/{id}/revisions",
            get(deployments::revisions_for_deployment),
        )
        .route(
            "/{workspace}/deployments/{id}/actions/render",
            post(deployments::render),
        )
        .route(
            "/{workspace}/deployments/{id}/actions/preview",
            post(deployments::preview),
        )
        .route(
            "/{workspace}/deployments/{id}/actions/apply",
            post(deployments::apply),
        )
        .route(
            "/{workspace}/deployments/{id}/actions/rollback",
            post(deployments::rollback),
        )
        .route(
            "/{workspace}/deployments/{id}/actions/acknowledge",
            post(deployments::acknowledge),
        )
        .route(
            "/{workspace}/pipelines/{id}/actions/run",
            post(pipeline_runs::trigger),
        )
        .route("/{workspace}/pipeline-runs", get(pipeline_runs::list))
        .route("/{workspace}/pipeline-runs/{id}", get(pipeline_runs::get))
        .route(
            "/{workspace}/pipeline-runs/{id}/logs",
            get(pipeline_runs::log_list),
        )
        .route(
            "/{workspace}/pipeline-runs/{id}/actions/cancel",
            post(pipeline_runs::cancel),
        )
        .route("/{workspace}/dashboard", get(hosts::dashboard))
        .route("/{workspace}/roles", get(workspace::roles))
        .route("/{workspace}/activity", get(hosts::list_activity))
        .route(
            "/{workspace}/host-groups",
            get(hosts::list_groups).post(hosts::create_group),
        )
        .route(
            "/{workspace}/host-groups/{id}",
            get(hosts::get_group)
                .patch(hosts::update_group)
                .delete(hosts::delete_group),
        )
        .route(
            "/{workspace}/hosts",
            get(hosts::list_hosts).post(hosts::create_host),
        )
        .route(
            "/{workspace}/hosts/{id}",
            get(hosts::get_host)
                .patch(hosts::update_host)
                .delete(hosts::delete_host),
        )
        .route(
            "/{workspace}/hosts/actions/bulk-delete",
            post(hosts::bulk_delete_hosts),
        )
        .route(
            "/{workspace}/hosts/{id}/actions/test-connection",
            post(remote::test_connection),
        )
        .route("/{workspace}/hosts/{id}/terminal", get(remote::terminal))
        .route(
            "/{workspace}/hosts/{id}/files",
            get(remote::list_dir).delete(remote::delete_file),
        )
        .route(
            "/{workspace}/hosts/{id}/files/content",
            // The global JSON limit must not apply here: an upload is streamed
            // straight to the host and is never held in memory, so capping it
            // at the body limit would cap every file at 256 KiB.
            get(remote::download)
                .put(remote::upload)
                .layer(DefaultBodyLimit::disable()),
        )
        .route("/{workspace}/runner-tokens", post(runner::create_token))
        .route(
            "/{workspace}/pipelines",
            get(pipelines::list).post(pipelines::create),
        )
        .route(
            "/{workspace}/pipelines/{id}",
            get(pipelines::get)
                .patch(pipelines::update)
                .delete(pipelines::remove),
        )
        .route(
            "/{workspace}/repositories",
            get(repositories::list).post(repositories::create),
        )
        .route(
            "/{workspace}/repositories/{id}",
            get(repositories::get)
                .patch(repositories::update)
                .delete(repositories::remove),
        )
        .route(
            "/{workspace}/credentials",
            get(credentials::list).post(credentials::create),
        )
        .route(
            "/{workspace}/credentials/{id}",
            get(credentials::get)
                .patch(credentials::update)
                .delete(credentials::remove),
        )
        .route(
            "/{workspace}/tags",
            get(hosts::list_tags).post(hosts::create_tag),
        )
        .route(
            "/{workspace}/tags/{id}",
            get(hosts::get_tag)
                .patch(hosts::update_tag)
                .delete(hosts::delete_tag),
        );

    let draining = drain.subscribe();
    let app = Router::new()
        .route("/api/status", get(status))
        // A runner is not a browser: it proves itself with the protocol's Hello
        // frame, so these two sit outside the session cookie.
        .route("/api/runners/connect", get(runner::connect))
        .route("/api/runners/sessions/{session}", get(runner::session))
        .route("/api/permissions", get(workspace::permission_catalogue))
        .route("/api/users", post(auth::register))
        .route("/api/sessions", post(auth::login).delete(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/workspaces", post(workspace::create))
        .route("/api/workspaces/actions/join", post(workspace::join))
        .nest("/api/workspaces", workspace_api)
        .fallback(assets::serve)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let hosts = hosts.clone();
            let draining = draining.clone();
            async move {
                // Runner transport must remain available until all steps and cleanup finish.
                if *draining.borrow() && !req.uri().path().starts_with("/api/runners/") {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "服务正在安全关闭，请稍后重试。",
                    )
                        .into_response();
                }
                guard(req, next, &hosts).await
            }
        }))
        .layer(TraceLayer::new_for_http())
        .with_state(s.clone());

    tracing::info!(address=%listener.local_addr()?, "Ferry 服务已启动");
    let (http_stop, http_rx) = tokio::sync::oneshot::channel();
    let mut http_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = http_rx.await;
        })
        .await
    });
    let early_result = tokio::select! {
        result = &mut http_task => Some(result),
        _ = shutdown() => {
            let _ = deployment_stop.send(true);
            let _ = pipeline_stop.send(true);
            None
        }
        _ = desktop_shutdown(cfg.desktop_control) => {
            tracing::info!("桌面服务正在等待执行中的任务完成");
            None
        }
    };
    let _ = drain.send(true);
    deployment_task.await?;
    pipeline_task.await?;
    let _ = http_stop.send(());
    let served = match early_result {
        Some(result) => result?,
        None => http_task.await?,
    };
    let _ = heartbeat_stop.send(true);
    let heartbeat_result = heartbeat_task.await;
    let released = worker.release(&s.db).await;
    let closed = s.db.clone().close().await;
    served?;
    heartbeat_result?;
    released?;
    closed?;
    tracing::info!("服务已安全关闭");
    drop(lock);
    Ok(())
}

async fn status(
    axum::extract::State(s): axum::extract::State<state::AppState>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": ferry_protocol::VERSION,
        "instance_id": s.instance_id,
        "server_mode": s.server_mode,
    }))
}

/// A stable identity for this installation, so runners and backups can tell
/// one deployment's data from another's.
fn instance_id(dir: &std::path::Path) -> anyhow::Result<String> {
    let path = dir.join("instance-id");
    match std::fs::read_to_string(&path) {
        Ok(value) => Ok(uuid::Uuid::parse_str(value.trim())?.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            use std::io::Write;
            let value = uuid::Uuid::new_v4().to_string();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(value.as_bytes())?;
            file.sync_all()?;
            Ok(value)
        }
        Err(error) => Err(error.into()),
    }
}

async fn desktop_shutdown(enabled: bool) {
    if !enabled {
        std::future::pending::<()>().await;
    }
    // A dedicated thread avoids an uncancellable Tokio stdin read delaying runtime exit.
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut byte = [0u8; 1];
        while matches!(std::io::stdin().read(&mut byte), Ok(1..)) {}
        let _ = tx.send(());
    });
    let _ = rx.await;
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install termination handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

/// Request-shape defences that belong in one place rather than per handler:
/// a Host allowlist, a same-origin check, a header no cross-site form can set,
/// and the response headers.
async fn guard(req: Request, next: Next, allowed: &[String]) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let hostname = host
        .parse::<axum::http::uri::Authority>()
        .map(|a| a.host().to_string())
        .unwrap_or_default();
    if !allowed.iter().any(|h| h.eq_ignore_ascii_case(&hostname)) {
        return (StatusCode::FORBIDDEN, "此主机地址未在允许列表中。").into_response();
    }
    let mutating = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    // A cross-site form can issue a POST but cannot add a custom header, and
    // cannot read the response. Cheaper and harder to get wrong than tokens.
    if mutating
        && req
            .headers()
            .get("x-ferry-request")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return (StatusCode::FORBIDDEN, "缺少操作校验标识。").into_response();
    }
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && origin != format!("http://{host}")
        && origin != format!("https://{host}")
    {
        return (StatusCode::FORBIDDEN, "不允许此来源发起操作。").into_response();
    }
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "same-origin".parse().unwrap());
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
            .parse()
            .unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

/// Picks the way to reach a host. The only place that chooses, so a third kind
/// of connection is one more arm here and no change to any caller.
pub async fn channel_open(
    s: &state::AppState,
    host: &remote::Host,
) -> error::Result<Box<dyn channel::HostChannel>> {
    match host.connection.as_str() {
        "ssh" => ssh::open(s, host).await,
        "runner" => runner::open(s, host).await,
        _ => Err(error::ApiError::bad("未知的接入方式。")),
    }
}
