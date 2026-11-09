//! Browser authorization for external repositories. App secrets stay server-side.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{
        credentials, platform_state, repository_connections as connections,
        repository_oauth_flows as flows,
    },
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
    vault,
};
use axum::{extract::State, http::StatusCode, response::Redirect};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, Set,
    sea_query::{Expr, ExprTrait},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

mod connection;
mod oauth;
mod provider;

pub use provider::ensure_fresh;
pub(crate) use provider::{MonitorAccess, OAuthSettings};
// Keep the existing request and response types available at the domain entry point.
pub(crate) use connection::owned_connection;
pub(crate) use provider::RepositoryApiAccess;
pub(crate) use connection::repository_api_access;
// Named connections are workspace resources. Unnamed OAuth setup sessions
// remain private to the account currently authorizing the provider.
pub(crate) fn connection_access(user: Id) -> sea_orm::Condition {
    sea_orm::Condition::any()
        .add(connections::Column::Name.ne(""))
        .add(connections::Column::UserId.eq(user))
}
#[allow(unused_imports)]
pub use connection::{BranchQuery, ConnectionBody, RepositoryPage};
pub use connection::{
    browse_repositories, connection_branches, connection_repositories, create_connection,
    delete_connection, get_connection, list_connections, resolve_connection,
    resolve_saved_connection, update_connection, validate_target,
};
use connection::{save_connection, validate_connection_name};
#[allow(unused_imports)]
pub use oauth::{Callback, StartOptions};
pub use oauth::{complete, connect_token, list, recover, start};
#[allow(unused_imports)]
pub(crate) use provider::MonitorPage;
use provider::{
    Provider, TokenConfig, authorize_request, codeup_has_more, config_fingerprint,
    connection_provider, endpoint, http, parse_endpoint, remote_repositories, repository_response,
    require_same_config, response_json, token_config, token_data,
};

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn connection_aad(workspace: Id, id: Id) -> String {
    format!("repository-refresh:{workspace}:{id}")
}
#[derive(Deserialize)]
pub struct TokenConnect {
    token: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_url: String,
    #[serde(default)]
    organization: String,
}
