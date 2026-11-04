mod accounts;
mod assets;
mod cluster;
mod credentials;
mod cron_jobs;
mod db;
mod deployments;
mod entities;
mod error;
mod hosts;
mod i18n;
mod lock;
mod maintenance;
mod marketplace;
mod migration;
mod notifications;
mod pipelines;
mod repositories;
mod resources;
mod snowflake;
mod state;
mod templates;
mod variables;
mod vault;
mod worker;

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
    /// Optional shared NATS JetStream cluster transport.
    #[arg(long, env = "FERRY_NATS_URL", hide_env_values = true)]
    nats_url: Option<String>,
    #[arg(long, env = "FERRY_CLUSTER_NAMESPACE", default_value = "ferry")]
    cluster_namespace: String,
    #[arg(long, env = "FERRY_NATS_REPLICAS", default_value_t = 1)]
    nats_replicas: usize,
    /// Cluster-wide concurrent repository analysis workers (use the same value on every replica).
    #[arg(
        long,
        env = "FERRY_REPOSITORY_ANALYSIS_CONCURRENCY",
        default_value = "1"
    )]
    repository_analysis_concurrency: std::num::NonZeroUsize,
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
        Some(url) => Database::connect(db::utc_options(ConnectOptions::new(url.clone()))).await?,
        None => {
            let path = cfg.data_dir.canonicalize()?.join("ferry.sqlite");
            let mut options = ConnectOptions::new("sqlite://ferry");
            options.sqlx_logging(false).map_sqlx_sqlite_opts(move |o| {
                o.filename(&path)
                    .create_if_missing(true)
                    .journal_mode(sea_orm::sqlx::sqlite::SqliteJournalMode::Wal)
                    .foreign_keys(true)
            });
            Database::connect(db::utc_options(options)).await?
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

    let cluster = match cfg.nats_url.as_deref().filter(|url| !url.is_empty()) {
        Some(url) => {
            anyhow::ensure!(
                cfg.database_url.is_some(),
                "cluster mode requires a shared database URL"
            );
            anyhow::ensure!(
                std::env::var("FERRY_MASTER_KEY").is_ok_and(|key| !key.is_empty()),
                "cluster mode requires a shared FERRY_MASTER_KEY"
            );
            Some(
                cluster::Cluster::connect(
                    url,
                    &cfg.cluster_namespace,
                    cfg.nats_replicas,
                    db.clone(),
                )
                .await?,
            )
        }
        None => None,
    };
    let s: state::AppState = Arc::new(state::State {
        host_monitor: Default::default(),
        cluster,
        db,
        data_dir: cfg.data_dir.clone(),
        instance_id,
        server_mode: cfg.server_mode,
        secure_cookie: cfg.secure_cookie,
        bind: cfg.bind,
        vault,
        ids: worker.generator.clone(),
    });

    hosts::runner::recover_updates(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;
    accounts::platform::prepare(&s).await?;
    maintenance::prepare(&s).await?;
    accounts::workspace::prepare_permissions(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;
    repositories::auth::recover(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;
    repositories::analysis::recover(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;
    repositories::analysis::insights::recover(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;
    marketplace::recover(&s)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e.1))?;

    let (drain, drain_rx) = tokio::sync::watch::channel(false);
    let (cluster_stop, cluster_rx) = tokio::sync::watch::channel(false);
    let cluster_task = s
        .cluster
        .as_ref()
        .map(|cluster| tokio::spawn(cluster.clone().serve(s.clone(), cluster_rx)));
    let (pipeline_stop, pipeline_rx) = tokio::sync::watch::channel(false);
    let pipeline_task = tokio::spawn(pipelines::runs::scheduler(
        s.clone(),
        pipeline_rx,
        drain_rx.clone(),
    ));

    let (deployment_stop, deployment_rx) = tokio::sync::watch::channel(false);
    let deployment_task = tokio::spawn(deployments::scheduler(
        s.clone(),
        deployment_rx,
        drain_rx.clone(),
    ));
    let (cron_stop, cron_rx) = tokio::sync::watch::channel(false);
    let cron_task = tokio::spawn(cron_jobs::scheduler(s.clone(), cron_rx, drain_rx.clone()));
    let runner_auto_update_task = tokio::spawn(hosts::runner::automatic_updates(
        s.clone(),
        drain_rx.clone(),
    ));
    let host_monitor_task = tokio::spawn(hosts::monitor::collect(s.clone(), drain_rx.clone()));
    let repository_monitor_task = tokio::spawn(repositories::monitor::scheduler(
        s.clone(),
        cfg.repository_analysis_concurrency,
        drain_rx.clone(),
    ));
    let repository_analysis_task = tokio::spawn(repositories::analysis::scheduler(
        s.clone(),
        cfg.repository_analysis_concurrency,
        drain_rx.clone(),
    ));
    let report_state = s.clone();
    let mut report_stop = drain_rx.clone();
    let reports_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = report_stop.changed() => break,
                _ = interval.tick() => {
                    if let Err(error) = repositories::reports::tick(&report_state).await {
                        tracing::warn!(error = %error.1, "Insight report generation failed");
                    }
                }
            }
        }
    });
    let recovery_state = s.clone();
    let mut recovery_stop = drain_rx.clone();
    let recovery_task = tokio::spawn(async move {
        if recovery_state.cluster.is_none() {
            return;
        }
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = recovery_stop.changed() => break,
                _ = tick.tick() => {
                    tokio::select! {
                        _ = recovery_stop.changed() => break,
                        result = hosts::runner::recover_updates(&recovery_state) => {
                            if let Err(error) = result {
                                tracing::warn!(error=%error.1,"Cannot recover orphaned Runner updates");
                            }
                        }
                    }
                }
            }
        }
    });
    let alerts_task = tokio::spawn(notifications::alerts::scheduler(s.clone(), drain_rx));

    let mut hosts = cfg.allowed_hosts;
    hosts.extend(["localhost".into(), "127.0.0.1".into(), "[::1]".into()]);
    let hosts = Arc::new(hosts);

    // Workspace resources all sit under /api/workspaces/{workspace}/..., which
    // is what makes the Scope extractor the only way to obtain a workspace id.
    let workspace_api = Router::new()
        .merge(repositories::reports::routes())
        .route(
            "/{workspace}/code-insights",
            get(repositories::analysis::insights::list)
                .post(repositories::analysis::insights::create),
        )
        .route(
            "/{workspace}/code-insights/{id}",
            get(repositories::analysis::insights::get)
                .patch(repositories::analysis::insights::update)
                .delete(repositories::analysis::insights::delete),
        )
        .route(
            "/{workspace}/code-insights/{id}/sync",
            post(repositories::analysis::insights::retry),
        )
        .route(
            "/{workspace}/code-insights/{id}/reports/{kind}",
            get(repositories::analysis::insight_report),
        )
        .route(
            "/{workspace}/code-connections/{id}/analysis",
            get(repositories::analysis::get).patch(repositories::analysis::update),
        )
        .route(
            "/{workspace}/code-connection-analyses/{id}",
            get(repositories::analysis::flat_get),
        )
        .route(
            "/{workspace}/code-connections/{id}/analysis-runs",
            post(repositories::analysis::retry),
        )
        .route(
            "/{workspace}/code-contributions/{kind}",
            get(repositories::analysis::report),
        )
        .route(
            "/{workspace}/repository-providers/{provider}/connections",
            post(repositories::auth::connect_token),
        )
        .route(
            "/{workspace}/repository-providers/{provider}/repositories",
            get(repositories::auth::browse_repositories),
        )
        .route(
            "/{workspace}/code-connections",
            get(repositories::auth::list_connections).post(repositories::auth::create_connection),
        )
        .route(
            "/{workspace}/code-connections/{id}",
            get(repositories::auth::get_connection)
                .patch(repositories::auth::update_connection)
                .delete(repositories::auth::delete_connection),
        )
        .route(
            "/{workspace}/code-connections/{id}/repositories",
            get(repositories::auth::connection_repositories),
        )
        .route(
            "/{workspace}/code-connection-monitors/{id}",
            get(repositories::monitor::get).patch(repositories::monitor::update),
        )
        .route(
            "/{workspace}/code-connections/{id}/events",
            get(repositories::monitor::history),
        )
        .route(
            "/{workspace}/repository-events",
            get(repositories::monitor::workspace_history),
        )
        .route(
            "/{workspace}/repository-events/{id}",
            get(repositories::monitor::event),
        )
        .route(
            "/{workspace}/code-connections/{id}/branches",
            get(repositories::auth::connection_branches),
        )
        .route(
            "/{workspace}/repository-providers",
            get(repositories::auth::list),
        )
        .route(
            "/{workspace}/repository-providers/{provider}/authorizations",
            post(repositories::auth::start),
        )
        .route(
            "/{workspace}/marketplace-sources",
            get(marketplace::list_sources).post(marketplace::create_source),
        )
        .route(
            "/{workspace}/marketplace-sources/{id}",
            get(marketplace::get_source)
                .patch(marketplace::update_source)
                .delete(marketplace::delete_source),
        )
        .route(
            "/{workspace}/marketplace-sources/{id}/synchronizations",
            post(marketplace::sync),
        )
        .route("/{workspace}/marketplace", get(marketplace::list))
        .route("/{workspace}/marketplace/{id}", get(marketplace::get))
        .route(
            "/{workspace}/marketplace/{id}/installations",
            post(marketplace::install),
        )
        .route(
            "/{workspace}/alert-rules",
            get(notifications::alerts::list).post(notifications::alerts::create),
        )
        .route(
            "/{workspace}/alert-rules/{id}",
            get(notifications::alerts::get)
                .patch(notifications::alerts::update)
                .delete(notifications::alerts::delete),
        )
        .route(
            "/{workspace}/alert-events",
            get(notifications::alerts::history),
        )
        .route(
            "/{workspace}/alert-events/{id}",
            get(notifications::alerts::detail),
        )
        .route(
            "/{workspace}/cron-jobs",
            get(cron_jobs::list).post(cron_jobs::create),
        )
        .route(
            "/{workspace}/cron-jobs/{id}",
            get(cron_jobs::get)
                .patch(cron_jobs::update)
                .delete(cron_jobs::delete),
        )
        .route("/{workspace}/cron-jobs/{id}/runs", post(cron_jobs::run_now))
        .route("/{workspace}/cron-jobs/{id}/runs", get(cron_jobs::history))
        .route("/{workspace}/cron-runs", get(cron_jobs::list_runs))
        .route("/{workspace}/cron-runs/{id}", get(cron_jobs::get_run))
        .route(
            "/{workspace}/cron-runs/{id}/acknowledgements",
            post(cron_jobs::acknowledge),
        )
        .route("/{workspace}/preferences", get(maintenance::preferences))
        .route(
            "/{workspace}/settings",
            get(maintenance::list_settings).post(maintenance::create_setting),
        )
        .route(
            "/{workspace}/settings/{id}",
            get(maintenance::get_setting)
                .patch(maintenance::update_setting)
                .delete(maintenance::delete_setting),
        )
        .route(
            "/{workspace}/dictionaries",
            get(maintenance::list_dictionaries).post(maintenance::create_dictionary),
        )
        .route(
            "/{workspace}/dictionaries/{id}",
            get(maintenance::get_dictionary)
                .patch(maintenance::update_dictionary)
                .delete(maintenance::delete_dictionary),
        )
        .route(
            "/{workspace}/variables",
            get(variables::list).post(variables::create),
        )
        .route(
            "/{workspace}/variables/{id}",
            get(variables::get)
                .patch(variables::update)
                .delete(variables::remove),
        )
        .route(
            "/{workspace}/variables/{id}/disclosures",
            post(variables::reveal),
        )
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
            "/{workspace}/deployments/{id}/renderings",
            post(deployments::render),
        )
        .route(
            "/{workspace}/deployments/{id}/previews",
            post(deployments::preview),
        )
        .route(
            "/{workspace}/deployments/{id}/executions",
            post(deployments::apply),
        )
        .route(
            "/{workspace}/deployments/{id}/rollbacks",
            post(deployments::rollback),
        )
        .route(
            "/{workspace}/deployments/{id}/acknowledgements",
            post(deployments::acknowledge),
        )
        .route(
            "/{workspace}/pipelines/{id}/runs",
            post(pipelines::runs::trigger),
        )
        .route(
            "/{workspace}/config-templates",
            get(templates::configs::list).post(templates::configs::create),
        )
        .route(
            "/{workspace}/config-templates/{id}",
            get(templates::configs::get)
                .patch(templates::configs::update)
                .delete(templates::configs::delete),
        )
        .route(
            "/{workspace}/config-templates/{id}/renderings",
            post(templates::configs::render),
        )
        .route(
            "/{workspace}/pipeline-artifacts",
            get(pipelines::artifacts::list),
        )
        .route(
            "/{workspace}/pipeline-artifacts/{id}/content",
            get(pipelines::artifacts::download),
        )
        .route(
            "/{workspace}/pipeline-groups",
            get(pipelines::contexts::groups::list).post(pipelines::contexts::groups::create),
        )
        .route(
            "/{workspace}/pipeline-groups/ordering",
            axum::routing::patch(pipelines::contexts::reorder_groups),
        )
        .route(
            "/{workspace}/pipeline-groups/{id}",
            get(pipelines::contexts::groups::get)
                .patch(pipelines::contexts::groups::update)
                .delete(pipelines::contexts::groups::remove),
        )
        .route(
            "/{workspace}/pipeline-environments",
            get(pipelines::contexts::environments::list)
                .post(pipelines::contexts::environments::create),
        )
        .route(
            "/{workspace}/pipeline-environments/{id}",
            get(pipelines::contexts::environments::get)
                .patch(pipelines::contexts::environments::update)
                .delete(pipelines::contexts::environments::remove),
        )
        .route("/{workspace}/pipeline-runs", get(pipelines::runs::list))
        .route("/{workspace}/pipeline-runs/{id}", get(pipelines::runs::get))
        .route(
            "/{workspace}/pipeline-runs/{id}/logs",
            get(pipelines::runs::log_list),
        )
        .route(
            "/{workspace}/pipeline-runs/{id}/cancellations",
            post(pipelines::runs::cancel),
        )
        .route(
            "/{workspace}/pipeline-runs/{id}/jobs/{key}/approvals",
            post(pipelines::runs::approve),
        )
        .route(
            "/{workspace}/app-templates",
            get(templates::apps::list).post(templates::apps::create),
        )
        .route(
            "/{workspace}/app-templates/{id}",
            get(templates::apps::get)
                .patch(templates::apps::update)
                .delete(templates::apps::delete),
        )
        .route(
            "/{workspace}/app-templates/{id}/renderings",
            post(templates::apps::render),
        )
        .route(
            "/{workspace}/pipeline-templates",
            get(templates::pipelines::list).post(templates::pipelines::create),
        )
        .route(
            "/{workspace}/pipeline-templates/{id}",
            get(templates::pipelines::get)
                .patch(templates::pipelines::update)
                .delete(templates::pipelines::delete),
        )
        .route(
            "/{workspace}/pipeline-templates/{id}/renderings",
            post(templates::pipelines::render),
        )
        .route("/{workspace}/dashboard", get(hosts::dashboard))
        .route(
            "/{workspace}/roles",
            get(accounts::workspace_admin::roles).post(accounts::workspace_admin::create_role),
        )
        .route(
            "/{workspace}/roles/{id}",
            get(accounts::workspace_admin::get_role)
                .patch(accounts::workspace_admin::update_role)
                .delete(accounts::workspace_admin::delete_role),
        )
        .route(
            "/{workspace}/members",
            get(accounts::workspace_admin::members),
        )
        .route(
            "/{workspace}/members/invitation",
            get(accounts::workspace_admin::invitation)
                .post(accounts::workspace_admin::rotate_invitation),
        )
        .route(
            "/{workspace}/members/role-options",
            get(accounts::workspace_admin::role_options),
        )
        .route(
            "/{workspace}/members/{id}",
            get(accounts::workspace_admin::get_member)
                .patch(accounts::workspace_admin::update_member)
                .delete(accounts::workspace_admin::delete_member),
        )
        .route("/{workspace}/activity", get(hosts::list_activity))
        .route(
            "/{workspace}/node-groups",
            get(hosts::list_groups).post(hosts::create_group),
        )
        .route(
            "/{workspace}/node-groups/ordering",
            axum::routing::patch(hosts::reorder_groups),
        )
        .route(
            "/{workspace}/node-groups/{id}",
            get(hosts::get_group)
                .patch(hosts::update_group)
                .delete(hosts::delete_group),
        )
        .route(
            "/{workspace}/nodes",
            get(hosts::list_hosts)
                .post(hosts::create_host)
                .delete(hosts::bulk_delete_hosts),
        )
        .route(
            "/{workspace}/nodes/{id}",
            get(hosts::get_host)
                .patch(hosts::update_host)
                .delete(hosts::delete_host),
        )
        .route(
            "/{workspace}/nodes/{id}/connection-tests",
            post(hosts::remote::test_connection),
        )
        .route("/{workspace}/nodes/{id}/metrics", get(hosts::monitor::read))
        .route(
            "/{workspace}/nodes/{id}/file-systems",
            get(hosts::remote::file_systems),
        )
        .route(
            "/{workspace}/nodes/{id}/terminal-sessions",
            get(hosts::remote::terminal),
        )
        .route(
            "/{workspace}/nodes/{id}/files",
            get(hosts::remote::list_dir)
                .post(hosts::remote::create_entry)
                .delete(hosts::remote::delete_file),
        )
        .route(
            "/{workspace}/nodes/{id}/files/content",
            // The global JSON limit must not apply here: an upload is streamed
            // straight to the host and is never held in memory, so capping it
            // at the body limit would cap every file at 256 KiB.
            get(hosts::remote::download)
                .put(hosts::remote::upload)
                .layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/{workspace}/runner-tokens",
            post(hosts::runner::create_token),
        )
        .route(
            "/{workspace}/runner-tokens/{id}",
            get(hosts::runner::token_status),
        )
        .route(
            "/{workspace}/nodes/{id}/connection-credentials",
            get(hosts::runner::connection_command).post(hosts::runner::reset_connection_command),
        )
        .route(
            "/{workspace}/nodes/{id}/runner-updates",
            get(hosts::runner::update_status).post(hosts::runner::start_update),
        )
        .route(
            "/{workspace}/pipelines",
            get(pipelines::list)
                .post(pipelines::create)
                .layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route(
            "/{workspace}/pipelines/{id}",
            get(pipelines::get)
                .patch(pipelines::update)
                .delete(pipelines::remove)
                .layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
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

    let workspace_api = workspace_api
        .merge(resources::transfers::routes())
        .merge(maintenance::files::routes())
        .fallback(|| async { error::ApiError::missing() });

    let draining = drain.subscribe();
    let app = Router::new()
        .route("/api/status", get(status))
        // A runner is not a browser: it proves itself with the protocol's Hello
        // frame, so these two sit outside the session cookie.
        .route("/api/runners/connect", get(hosts::runner::connect))
        .route("/api/runners/install.sh", get(hosts::runner::install::shell))
        .route("/api/runners/install.ps1", get(hosts::runner::install::powershell))
        .route("/api/runners/install/{target}", get(hosts::runner::install::artifact))
        .route("/api/runners/sessions/{session}", get(hosts::runner::session))
        .route("/api/permissions", get(accounts::workspace::permission_catalogue))
        .route(
            "/api/setup",
            get(accounts::platform::status).post(accounts::platform::initialize),
        )
        .route("/api/platform/users", get(accounts::platform::list_users))
        .route(
            "/api/platform/users/{id}",
            get(accounts::platform::get_user).patch(accounts::platform::update_user),
        )
        .route(
            "/api/platform/workspaces",
            get(accounts::platform::list_workspaces).post(accounts::platform::create_workspace),
        )
        .route(
            "/api/platform/workspaces/{id}",
            get(accounts::platform::get_workspace).patch(accounts::platform::update_workspace),
        )
        .route(
            "/api/platform/settings/1",
            get(accounts::platform::get_settings).patch(accounts::platform::update_settings),
        )
        .route("/api/users", post(accounts::auth::register))
        .route("/api/sessions", post(accounts::auth::login).delete(accounts::auth::logout))
        .route("/api/me", get(accounts::auth::me))
        .route(
            "/api/profile/1",
            get(accounts::auth::profile).patch(accounts::auth::update_profile),
        )
        .route("/api/repository-providers/callback",get(repositories::auth::complete))
        .route("/api/repository-events/{id}",post(repositories::monitor::receive).layer(DefaultBodyLimit::max(1024 * 1024)))
        .route("/api/workspaces", post(accounts::workspace::create))
        .route("/api/workspace-memberships", post(accounts::workspace::join))
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
                        i18n::message("server.draining"),
                    )
                        .into_response();
                }
                guard(req, next, &hosts).await
            }
        }))
        .layer(middleware::from_fn(i18n::middleware))
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request| tracing::debug_span!("http", method=%request.method(), path=%request.uri().path())))
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
            let _ = cron_stop.send(true);
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
    alerts_task.await?;
    host_monitor_task.await?;
    runner_auto_update_task.await?;
    repository_monitor_task.await?;
    repository_analysis_task.await?;
    reports_task.await?;
    recovery_task.await?;
    cron_task.await?;
    deployment_task.await?;
    pipeline_task.await?;
    let _ = cluster_stop.send(true);
    if let Some(task) = cluster_task {
        task.await?
            .map_err(|error| anyhow::anyhow!("{}", error.1))?;
    }
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
        return (StatusCode::FORBIDDEN, i18n::message("guard.host")).into_response();
    }
    let mutating = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    // A cross-site form can issue a POST but cannot add a custom header, and
    // cannot read the response. Cheaper and harder to get wrong than tokens.
    let provider_event = req.method() == axum::http::Method::POST
        && req
            .uri()
            .path()
            .strip_prefix("/api/repository-events/")
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
    if mutating
        && !provider_event
        && req
            .headers()
            .get("x-ferry-request")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return (StatusCode::FORBIDDEN, i18n::message("guard.request")).into_response();
    }
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && origin != format!("http://{host}")
        && origin != format!("https://{host}")
    {
        return (StatusCode::FORBIDDEN, i18n::message("guard.origin")).into_response();
    }
    let oauth_callback = req.uri().path() == "/api/repository-providers/callback";
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(
        header::REFERRER_POLICY,
        if oauth_callback {
            "no-referrer"
        } else {
            "same-origin"
        }
        .parse()
        .unwrap(),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
            .parse()
            .unwrap(),
    );
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert("no-store".parse().unwrap());
    response
}

/// Picks the way to reach a host. The only place that chooses, so a third kind
/// of connection is one more arm here and no change to any caller.
pub async fn channel_open(
    s: &state::AppState,
    host: &hosts::remote::Host,
) -> error::Result<Box<dyn hosts::channel::HostChannel>> {
    match host.connection.as_str() {
        "ssh" => hosts::ssh::open(s, host).await,
        "runner" => hosts::runner::open(s, host).await,
        _ => Err(error::ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            i18n::Message::new("host.connectionUnknown"),
        )),
    }
}
