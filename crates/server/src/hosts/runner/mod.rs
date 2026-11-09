//! The reverse-connection agent, server side: enrolment, the registry of live
//! runners, and `RunnerChannel`.
//!
//! A runner dials out, so a host behind NAT needs no inbound route and the
//! server never holds its credentials — the host holds its own key. Three
//! endpoints make that work:
//!
//! * `POST /api/workspaces/{workspace}/runner-tokens` — an operator mints a
//!   single-use, TTL-limited enrolment token for their workspace;
//! * `GET /api/runners/connect` — the runner's long-lived control connection;
//! * `GET /api/runners/sessions/{session}` — one dedicated connection per
//!   terminal, transfer or command, dialled on request.
//!
//! Workspace isolation rests on one fact rather than on a check in every
//! handler: a runner authenticates as a single `host_id`, that host row belongs
//! to exactly one workspace, and `remote::open` only ever reaches a host it
//! already loaded through the `Scope`-filtered query. An enrolment token is
//! tied to its workspace, so enrolling cannot produce a host anywhere else,
//! and a key only matches the hash on its own host's row.
pub(crate) mod install;
pub(crate) mod reservations;

mod enrollment;
#[cfg(test)]
mod test_support;
mod transport;
mod updates;

pub(crate) use enrollment::bootstrap_token;
pub use enrollment::{
    connection_command, create_token, fingerprint, reset_connection_command, secret, token_status,
};
use enrollment::{enrol, reported_address, verify_key};
pub(crate) use transport::cluster_open;
pub use transport::{connect, open, session};
pub(crate) use updates::older_version;
use updates::stored_update_failure;
pub use updates::{UpdateBody, start_update, update_status};
pub(crate) use updates::{automatic_updates, recover_updates};

use crate::cluster::Socket;
use crate::snowflake::Id;

use crate::error::{Json, Path};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{hosts, runner_tokens},
    error::{ApiError, Result},
    hosts::channel::{ByteStream, DirEntry, FileMeta, HostChannel, Output, Pty, PtySize},
    hosts::remote::Host,
    state::AppState,
};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use ferry_protocol as wire;
use futures::StreamExt;
use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

// --------------------------------------------------------------- 在线登记

/// A live control connection. Dropping the last handle closes the sender,
/// which is how an evicted connection's own loop learns it was replaced.
struct Live {
    owner: Option<crate::cluster::Owner>,
    owner_revision: std::sync::atomic::AtomicU64,
    key_hash: String,
    control: mpsc::Sender<wire::Control>,
    disconnect: watch::Sender<bool>,
    managed: bool,
    installation_scope: Option<String>,
    supports_monitor: bool,
    supports_agent: bool,
    supports_agent_probe: bool,
    version: String,
    ready: std::sync::atomic::AtomicBool,
    failed_version: Mutex<Option<String>>,
    update_error: Mutex<Option<(String, String)>>,
}

// ponytail: process-global, so this is one server process's view of its own
// runners. Running several replicas behind a load balancer needs the session
// hand-off to cross processes — route runners and browsers to the same replica,
// or add a shared broker, before scaling out.
static LIVE: LazyLock<Mutex<HashMap<String, Arc<Live>>>> = LazyLock::new(Default::default);

fn live() -> MutexGuard<'static, HashMap<String, Arc<Live>>> {
    LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
