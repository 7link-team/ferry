//! The launcher is trusted; only its OS-confined child executes tenant JS.
mod engine;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

use ferry_protocol::sandbox::{Batch, ErrorCode, Reply};
use std::io::{Read, Write};

fn read_request() -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn run() -> Reply {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let inner = args.as_slice() == ["--inner"];
    let bytes = match read_request() {
        Ok(bytes) => bytes,
        Err(_) => return Reply::error(ErrorCode::InvalidRequest, None),
    };
    let mut request: Batch = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(_) => return Reply::error(ErrorCode::InvalidRequest, None),
    };
    request.timeout_ms = request
        .resource_policy
        .sandbox_timeout_ms
        .or(request.timeout_ms);
    request.memory_bytes = request
        .resource_policy
        .sandbox_memory_bytes
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .or(request.memory_bytes);
    if !request.valid()
        || !ferry_protocol::resources::allows(
            request.resource_policy.workflow_packet_bytes,
            bytes.len(),
        )
    {
        return Reply::error(ErrorCode::InvalidRequest, None);
    }
    #[cfg(target_os = "linux")]
    if args.len() == 2 && args[0] == "--linux-worker" {
        return match args[1].parse::<i32>() {
            Ok(parent) if parent > 1 => linux::worker(request, parent),
            _ => Reply::error(ErrorCode::Isolation, None),
        };
    }
    if inner {
        #[cfg(windows)]
        if !windows::confined() {
            return Reply::error(ErrorCode::Isolation, None);
        }
        #[cfg(not(windows))]
        return Reply::error(ErrorCode::Isolation, None);
        #[cfg(windows)]
        return engine::evaluate(request);
    }
    if !args.is_empty() {
        return Reply::error(ErrorCode::InvalidRequest, None);
    }
    #[cfg(windows)]
    {
        windows::evaluate(
            &bytes,
            request.timeout_ms,
            request
                .resource_policy
                .sandbox_process_memory_bytes
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX)),
            request.resource_policy.workflow_packet_bytes,
        )
    }
    #[cfg(target_os = "linux")]
    {
        linux::evaluate(
            &bytes,
            request.timeout_ms,
            request.resource_policy.workflow_packet_bytes,
        )
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Reply::error(ErrorCode::Isolation, None)
    }
}
fn main() {
    #[cfg(target_os = "linux")]
    if let Some(reply) = linux::diagnostic(&std::env::args().skip(1).collect::<Vec<_>>()) {
        let _ = std::io::stdout().write_all(&serde_json::to_vec(&reply).unwrap());
        return;
    }
    #[cfg(windows)]
    if let Some(reply) = windows::diagnostic(&std::env::args().skip(1).collect::<Vec<_>>()) {
        let _ = std::io::stdout().write_all(&serde_json::to_vec(&reply).unwrap());
        return;
    }
    let reply = run();
    let bytes = serde_json::to_vec(&reply).unwrap_or_default();
    let _ = std::io::stdout().write_all(&bytes);
}
