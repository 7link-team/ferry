use sea_orm::DatabaseConnection;
use std::{path::PathBuf, sync::Arc};

#[allow(dead_code)]
pub struct State {
    pub ids: Arc<crate::snowflake::Generator>,
    pub db: DatabaseConnection,
    pub data_dir: PathBuf,
    pub instance_id: String,
    /// True in Docker/web mode: no desktop launcher owns this process.
    pub server_mode: bool,
    pub secure_cookie: bool,
    pub bind: std::net::SocketAddr,
    pub vault: crate::vault::Vault,
    pub cluster: Option<Arc<crate::cluster::Cluster>>,
    pub host_monitor: crate::host_monitor::Store,
}
pub type AppState = Arc<State>;

impl State {
    pub fn next_id(&self) -> crate::error::Result<crate::snowflake::Id> {
        self.ids.next_id().map_err(|error| {
            tracing::error!(%error, "ID generation failed");
            crate::error::ApiError(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "暂时无法发号，请稍后重试。".into(),
            )
        })
    }
}
