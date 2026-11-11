//! Ferry runner: one command enrols a host, then holds an outbound control
//! connection so the server can reach it without an inbound SSH route.
//!
//! The control connection carries no bulk data. For a terminal, a transfer or
//! a command the server sends `OpenSession` and the runner dials a second,
//! dedicated connection for it.

mod agent;
mod cli;
mod enrollment;
mod exec_stream;
#[cfg(windows)]
mod process_job;
mod runtime;
mod service;
mod session;
mod transport;
mod update;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("ferry-mcp" | "platform-mcp")
    ) {
        return tokio::runtime::Runtime::new()?.block_on(agent::platform::stdio());
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferry_runner=info".into()),
        )
        .init();
    if service::dispatch()? {
        return Ok(());
    }
    runtime::foreground(cli::Config::parse())
}
