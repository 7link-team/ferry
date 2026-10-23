//! Ferry runner: one command enrolls a host, then holds an outbound control
//! connection so the server can reach it without an inbound SSH route.
//!
//! The control connection carries no bulk data. For a terminal, an upload or a
//! download the server sends `OpenSession` and the runner dials a second,
//! dedicated connection for it.
//
// ponytail: one connection per session instead of a multiplexer. An ops host
// runs a handful of concurrent sessions; move to yamux only if that stops
// being true.

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Ferry 运维代理")]
struct Config {
    /// Server base URL, e.g. https://ops.example.com
    #[arg(long, env = "FERRY_SERVER")]
    server: String,
    /// Enrollment token from the host page, used once on the first connection.
    #[arg(long, env = "FERRY_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Where the runner keeps its host id and key after enrollment.
    #[arg(long, env = "FERRY_STATE_DIR", default_value = "/var/lib/ferry-runner")]
    state_dir: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferry_runner=info".into()),
        )
        .init();
    let cfg = Config::parse();
    tracing::info!(server = %cfg.server, protocol = ferry_protocol::VERSION, "runner 未实现，仅校验配置");
    anyhow::ensure!(
        cfg.token.is_some() || cfg.state_dir.join("runner.json").exists(),
        "首次运行需要 --token，或已有 {} 中的注册信息。",
        cfg.state_dir.display()
    );
    Ok(())
}
