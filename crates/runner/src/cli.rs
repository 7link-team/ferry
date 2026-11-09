//! Foreground command-line configuration, shared with managed service startup.

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Ferry 运维代理")]
pub(super) struct Config {
    #[arg(skip)]
    pub(super) managed: bool,
    #[arg(skip)]
    pub(super) installation_scope: Option<String>,
    #[arg(skip)]
    pub(super) confirmation: Option<tokio::sync::watch::Receiver<bool>>,
    #[arg(skip)]
    pub(super) failed_version: Option<String>,
    /// Server base URL, e.g. https://ops.example.com
    #[arg(long, env = "FERRY_SERVER")]
    pub(super) server: String,
    /// Enrollment token from the host page, used once on the first connection.
    #[arg(long, env = "FERRY_TOKEN", hide_env_values = true)]
    pub(super) token: Option<String>,
    /// Where the runner keeps its host id and key after enrollment.
    #[arg(long, env = "FERRY_STATE_DIR", default_value = "/var/lib/ferry-runner")]
    pub(super) state_dir: PathBuf,
}
