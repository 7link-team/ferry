mod assets;
// Bedrock the business modules build on; unused until they land.
#[allow(dead_code)]
mod db;
#[allow(dead_code)]
mod error;
mod migration;
mod state;

use axum::{Router, extract::DefaultBodyLimit, routing::get};
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

    let s: state::AppState = Arc::new(state::State {
        db,
        data_dir: cfg.data_dir.clone(),
        instance_id,
        server_mode: cfg.server_mode,
        secure_cookie: cfg.secure_cookie,
        bind: cfg.bind,
    });

    let app = Router::new()
        .route("/api/status", get(status))
        .fallback(assets::serve)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(s.clone());

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(address=%listener.local_addr()?, "Ferry 服务已启动");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown())
        .await?;
    s.db.clone().close().await?;
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
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
            file.write_all(value.as_bytes())?;
            file.sync_all()?;
            Ok(value)
        }
        Err(error) => Err(error.into()),
    }
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
