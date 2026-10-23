use sea_orm::DatabaseConnection;
use std::{path::PathBuf, sync::Arc};

#[allow(dead_code)]
pub struct State {
    pub db: DatabaseConnection,
    pub data_dir: PathBuf,
    pub instance_id: String,
    /// True in Docker/web mode: no desktop launcher owns this process.
    pub server_mode: bool,
    pub secure_cookie: bool,
    pub bind: std::net::SocketAddr,
}
pub type AppState = Arc<State>;

impl State {
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }
}
