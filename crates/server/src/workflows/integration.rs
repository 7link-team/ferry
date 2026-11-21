//! Declarative SaaS operations. Only trusted, versioned manifests choose origins,
//! headers and methods; graph parameters can never choose where secrets are sent.
use super::graph;
use crate::{
    accounts::workspace::Scope,
    db,
    definitions::access::{self, Operation},
    entities::{credentials, workflow_executions, workflow_integration_calls as calls},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use ferry_protocol::sandbox::Item;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, sea_query::Expr};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

fn error(key: &'static str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new(key))
}
fn invalid() -> ApiError {
    error("workflow.integrationInvalid")
}
fn unknown() -> ApiError {
    error("workflow.integrationUnknown")
}
fn remote() -> ApiError {
    error("workflow.integrationRemote")
}
/// The application answered with an error status: keep the integration's
/// "rejected" category (a known failure) and carry the specific reason.
fn remote_from(error: ApiError) -> ApiError {
    match error.2 {
        Some(reason) => ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.integrationRemoteReason").with_message("reason", reason),
        ),
        None => remote(),
    }
}
/// An application rejection: the request was refused, so nothing is in an unknown state.
pub(super) fn is_remote(key: &str) -> bool {
    matches!(
        key,
        "workflow.integrationRemote"
            | "workflow.integrationRemoteStatus"
            | "workflow.integrationRemoteReason"
    )
}
/// A non-2xx status without a host (recorded responses); live calls use upstream::status.
fn remote_status(status: u16) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.integrationRemoteStatus").with("status", status.to_string()),
    )
}

fn catalog() -> &'static Value {
    static CATALOG: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut bundled: Value = serde_json::from_str(include_str!(
            "../../../../src/shared/workflow-contracts/workflow-integration-nodes.json"
        ))
        .expect("bundled integration catalog");
        bundled["nodes"]
            .as_array_mut()
            .expect("bundled nodes")
            .extend(super::extensions::catalog().nodes.iter().cloned());
        bundled
    })
}

pub(super) fn supports(node: &Value) -> bool {
    node["type"] == "host.command"
        || node["type"] == "notification.send"
        || graph::manifest(node)
            .is_some_and(|spec| spec["http"].is_object() || spec["git"].is_object())
}

fn client() -> Result<reqwest::Client> {
    client_with_policy(&Default::default())
}
fn client_with_policy(
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<reqwest::Client> {
    let mut client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30));
    if let Some(seconds) = policy.task_timeout_seconds {
        client = client.timeout(std::time::Duration::from_secs(seconds));
    }
    client
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("workflow-integration/1")
        .build()
        .map_err(|_| invalid())
}

fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(value.to_string()),
        _ => Err(invalid()),
    }
}

/// Encode each path variable as a single segment, including namespaced GitLab IDs.
fn request(
    client: &reqwest::Client,
    http: &Value,
    params: &Value,
    secret: &str,
) -> Result<reqwest::RequestBuilder> {
    request_with_connection_origin(client, http, params, secret, false)
}

fn request_with_connection_origin(
    client: &reqwest::Client,
    http: &Value,
    params: &Value,
    secret: &str,
    connection_origin: bool,
) -> Result<reqwest::RequestBuilder> {
    let origin = http["origin"].as_str().ok_or_else(invalid)?;
    let mut url = reqwest::Url::parse(origin).map_err(|_| invalid())?;
    if (url.scheme() != "https"
        && !(url.scheme() == "http"
            && (connection_origin || (cfg!(test) && url.host_str() == Some("127.0.0.1")))))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(invalid());
    }
    let template = http["path"]
        .as_str()
        .filter(|path| path.starts_with('/') && !path.starts_with("//"))
        .ok_or_else(invalid)?;
    let mut path = String::new();
    for segment in template.trim_start_matches('/').split('/') {
        path.push('/');
        if let Some(key) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let value = scalar(params.get(key).ok_or_else(invalid)?)?;
            if value.is_empty() || matches!(value.as_str(), "." | "..") {
                return Err(invalid());
            }
            path.push_str(&utf8_percent_encode(&value, NON_ALPHANUMERIC).to_string());
        } else {
            path.push_str(segment);
        }
    }
    url.set_path(&path);
    if let Some(query) = http["query"].as_object() {
        for (name, field) in query {
            let field = field.as_str().ok_or_else(invalid)?;
            if let Some(value) = params.get(field).filter(|v| !v.is_null() && **v != "") {
                url.query_pairs_mut().append_pair(name, &scalar(value)?);
            }
        }
    }
    let method =
        reqwest::Method::from_bytes(http["method"].as_str().ok_or_else(invalid)?.as_bytes())
            .map_err(|_| invalid())?;
    if !matches!(
        method,
        reqwest::Method::GET
            | reqwest::Method::POST
            | reqwest::Method::PUT
            | reqwest::Method::PATCH
            | reqwest::Method::DELETE
    ) {
        return Err(invalid());
    }
    let mut builder = client.request(method, url);
    if let Some(headers) = http["headers"].as_object() {
        for (name, value) in headers {
            builder = builder.header(name, value.as_str().ok_or_else(invalid)?);
        }
    }
    if secret.is_empty() && http["auth"]["optional"] != true {
        return Err(invalid());
    }
    if !secret.is_empty() {
        let mut token = reqwest::header::HeaderValue::from_str(&format!(
            "{}{}",
            http["auth"]["prefix"].as_str().ok_or_else(invalid)?,
            secret
        ))
        .map_err(|_| invalid())?;
        token.set_sensitive(true);
        builder = builder.header(http["auth"]["header"].as_str().ok_or_else(invalid)?, token);
    }
    if let Some(fields) = http["body"].as_object().filter(|fields| !fields.is_empty()) {
        let mut body = Map::new();
        for (name, field) in fields {
            if let Some(value) = params.get(field.as_str().ok_or_else(invalid)?) {
                body.insert(name.clone(), value.clone());
            }
        }
        builder = builder.json(&body);
    }
    Ok(builder)
}

pub(super) fn validate_declaration(http: &Value, params: &Value) -> anyhow::Result<()> {
    request(
        &client().map_err(|_| anyhow::anyhow!("HTTP client unavailable"))?,
        http,
        params,
        "extension-validation",
    )
    .and_then(|builder| builder.build().map_err(|_| invalid()))
    .map_err(|_| anyhow::anyhow!("invalid HTTP declaration"))?;
    Ok(())
}

async fn response(http: &Value, response: reqwest::Response, secret: &str) -> Result<Value> {
    response_with_policy(http, response, secret, &Default::default()).await
}
async fn response_with_policy(
    http: &Value,
    mut response: reqwest::Response,
    secret: &str,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<Value> {
    // Never log/persist upstream error text, URLs or credentials: host and status only.
    let host = response.url().host_str().unwrap_or("?").to_owned();
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(remote_from(crate::upstream::status(
            "integration",
            &host,
            status,
            false,
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
        if !ferry_protocol::resources::allows(
            policy.workflow_packet_bytes,
            bytes.len().saturating_add(chunk.len()),
        ) {
            tracing::warn!(host, "integration response exceeds configured limit");
            return Err(error("workflow.integrationTooLarge"));
        }
        bytes.extend_from_slice(&chunk);
    }
    response_bytes_with_policy(http, status, &bytes, secret, policy)
}

fn response_bytes(http: &Value, status: u16, bytes: &[u8], secret: &str) -> Result<Value> {
    response_bytes_with_policy(http, status, bytes, secret, &Default::default())
}
fn response_bytes_with_policy(
    http: &Value,
    status: u16,
    bytes: &[u8],
    secret: &str,
    policy: &ferry_protocol::resources::ResourcePolicy,
) -> Result<Value> {
    if !(200..300).contains(&status) {
        return Err(remote_status(status));
    }
    if !ferry_protocol::resources::allows(policy.workflow_packet_bytes, bytes.len()) {
        return Err(error("workflow.integrationTooLarge"));
    }
    if !secret.is_empty()
        && bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    {
        tracing::warn!(
            status,
            "integration response echoed its credential; result rejected"
        );
        return Err(error("workflow.integrationSecretEcho"));
    }
    let value: Value = if bytes.is_empty() {
        json!({"status_code":status})
    } else {
        serde_json::from_slice(bytes).map_err(|_| {
            tracing::warn!(status, "integration response is not JSON");
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("workflow.integrationNotJson").with("status", status.to_string()),
            )
        })?
    };
    if let Some(pointer) = http["success"]["pointer"].as_str() {
        if value.pointer(pointer) != Some(&http["success"]["equals"]) {
            return Err(remote());
        }
    }
    // Arrays stay intact so provider pagination/cursors are never silently dropped.
    Ok(if value.is_object() {
        value
    } else {
        json!({"data":value})
    })
}

/// Curl runs on the selected host. Config goes over a private file/stdin,
/// redirects stay disabled and provider errors/secret echoes stay suppressed.
async fn host_request(
    s: &AppState,
    run: &workflow_executions::Model,
    node: &Value,
    http: &Value,
    builder: reqwest::RequestBuilder,
    secret: &str,
) -> Result<Value> {
    let request = builder.build().map_err(|_| invalid())?;
    let config_quote = |value: &str| -> Result<String> {
        if value.contains(['\n', '\r', '\0']) {
            return Err(invalid());
        }
        Ok(format!(
            "\"{}\"",
            value.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    };
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let mut config = format!(
        "silent\nshow-error\nconnect-timeout = 30\nurl = {}\nrequest = {}\n",
        config_quote(request.url().as_str())?,
        config_quote(request.method().as_str())?
    );
    if let Some(seconds) = policy.task_timeout_seconds {
        config.push_str(&format!("max-time = {seconds}\n"));
    }
    if let Some(bytes) = policy.workflow_packet_bytes {
        config.push_str(&format!("max-filesize = {bytes}\n"));
    }
    for (name, value) in request.headers() {
        config.push_str(&format!(
            "header = {}\n",
            config_quote(&format!(
                "{name}: {}",
                value.to_str().map_err(|_| invalid())?
            ))?
        ));
    }
    if let Some(body) = request.body().and_then(reqwest::Body::as_bytes) {
        config.push_str(&format!(
            "data = {}\n",
            config_quote(std::str::from_utf8(body).map_err(|_| invalid())?)?
        ));
    }
    let delimiter = format!("FERRY_{}", uuid::Uuid::new_v4().simple());
    let script = format!(
        "#!/bin/sh\nset -eu\ncurl --config - --write-out '\\n%{{http_code}}' <<'{delimiter}'\n{config}{delimiter}\n"
    );
    let output = super::host::execute(s, run, node, &script, false).await?;
    let (body, status) = output["stdout"]
        .as_str()
        .ok_or_else(unknown)?
        .rsplit_once('\n')
        .ok_or_else(unknown)?;
    let mut result = response_bytes_with_policy(
        http,
        status.trim().parse().map_err(|_| unknown())?,
        body.as_bytes(),
        secret,
        &policy,
    )?;
    result["execution_host"] = output["execution_host"].clone();
    Ok(result)
}

async fn credential(
    s: &AppState,
    scope: &Scope,
    http: &Value,
    params: &Value,
) -> Result<(Id, String)> {
    if params["credential_id"] == "0" && http["auth"]["optional"] == true {
        return Ok((Id::from(0_i64), String::new()));
    }
    scope.require("credentials.use")?;
    let id: Id = params["credential_id"]
        .as_str()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let row = credentials::Entity::find_by_id(id)
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(invalid)?;
    if row.scope != "integration"
        || row.kind != "token"
        || row.secret.is_empty()
        || Some(row.username.as_str()) != http["auth"]["profile"].as_str()
    {
        return Err(invalid());
    }
    let (_, _, secret) = crate::credentials::unseal(s, &scope.workspace_id, &id).await?;
    if secret.is_empty() {
        return Err(invalid());
    }
    Ok((id, secret))
}

fn aad(workspace: i64, call: i64) -> String {
    format!("workflow:{workspace}:integration:{call}:result")
}

async fn reserve_call(
    db: &sea_orm::DatabaseConnection,
    call: calls::Model,
) -> Result<Option<calls::Model>> {
    let query = || {
        calls::Entity::find()
            .filter(calls::Column::WorkspaceId.eq(call.workspace_id))
            .filter(calls::Column::ExecutionId.eq(call.execution_id))
            .filter(calls::Column::NodeRunId.eq(call.node_run_id))
            .filter(calls::Column::ItemIndex.eq(call.item_index))
    };
    if let Some(existing) = query().one(db).await? {
        if existing.request_hash != call.request_hash || existing.state != "succeeded" {
            return Err(unknown());
        }
        return Ok(Some(existing));
    }
    if let Err(error) = calls::Entity::insert(call.clone().into_active_model())
        .exec(db)
        .await
    {
        if query().one(db).await?.is_some() {
            return Err(unknown());
        }
        return Err(error.into());
    }
    Ok(None)
}

fn connection_http(
    http: &Value,
    params: &Value,
    access: &crate::repositories::auth::RepositoryApiAccess,
) -> Result<(Value, Value)> {
    let profile = http["auth"]["profile"].as_str().ok_or_else(invalid)?;
    if !matches!(profile, "github" | "gitlab" | "codeup") {
        return Err(invalid());
    }
    let mut http = http.clone();
    let mut params = params.clone();
    let path = http["path"].as_str().ok_or_else(invalid)?.to_owned();
    let base = access.api.path().trim_end_matches('/');
    let suffix = if profile == "gitlab" {
        path.strip_prefix("/api/v4").ok_or_else(invalid)?
    } else {
        &path
    };
    // The API base comes exclusively from the scoped, server-owned connection.
    http["origin"] = json!(access.api.origin().ascii_serialization());
    http["path"] = json!(format!("{base}{suffix}"));
    if profile == "codeup" {
        if access.organization.is_empty() {
            http["path"] = json!(
                http["path"]
                    .as_str()
                    .ok_or_else(invalid)?
                    .replace("/organizations/{organization_id}", "")
            );
            params
                .as_object_mut()
                .ok_or_else(invalid)?
                .remove("organization_id");
        } else {
            params["organization_id"] = json!(access.organization);
        }
    }
    if profile == "gitlab" && !access.personal {
        http["auth"]["header"] = json!("Authorization");
        http["auth"]["prefix"] = json!("Bearer ");
    }
    Ok((http, params))
}

async fn resolve_http(
    s: &AppState,
    scope: &Scope,
    http: &Value,
    params: &Value,
) -> Result<(Value, Value, Id, String)> {
    if let Some(id) = params.get("connection_id") {
        let id = id
            .as_str()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let profile = http["auth"]["profile"].as_str().ok_or_else(invalid)?;
        if !matches!(profile, "github" | "gitlab" | "codeup") {
            return Err(invalid());
        }
        let access =
            crate::repositories::auth::repository_api_access(s, scope, id, profile).await?;
        let (http, params) = connection_http(http, params, &access)?;
        Ok((http, params, access.credential_id, access.token))
    } else {
        let (id, secret) = credential(s, scope, http, params).await?;
        Ok((http.clone(), params.clone(), id, secret))
    }
}

/// Validate references in the same workspace transaction as the definition save.
/// OAuth refresh and secret access remain execution-time operations.
pub(super) async fn validate_connections(
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    definition: &str,
) -> Result<()> {
    let graph: Value = serde_json::from_str(definition).map_err(|_| invalid())?;
    let mut layers = vec![&graph];
    while let Some(graph) = layers.pop() {
        for node in graph["nodes"].as_array().into_iter().flatten() {
            if node["type"] == "core.loop" {
                layers.push(&node["params"]["body"]);
            }
            let Some(id) = node["params"].get("connection_id") else {
                continue;
            };
            let Some(spec) = graph::manifest(node) else {
                continue;
            };
            let api = spec["http"]["auth"]["profile"]
                .as_str()
                .filter(|profile| matches!(*profile, "github" | "gitlab" | "codeup"));
            if api.is_none() && !spec["git"].is_object() {
                continue;
            }
            scope.require("repositories.read")?;
            scope.require("credentials.use")?;
            let id = id.as_str().ok_or_else(invalid)?;
            // Expressions are resolved and authorized again immediately before execution.
            if id.starts_with('=') {
                continue;
            }
            let id: Id = id.parse().map_err(|_| invalid())?;
            let row = crate::repositories::auth::owned_connection(conn, scope, id).await?;
            if spec["git"]["provider"]
                .as_str()
                .is_some_and(|provider| row.provider != provider)
                || !matches!(row.state.as_str(), "connected" | "refreshing")
                || api.is_some_and(|provider| row.provider != provider || row.credential_id == 0)
            {
                return Err(invalid());
            }
            if row.credential_id != 0 {
                credentials::Entity::find_by_id(row.credential_id)
                    .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
                    .one(conn)
                    .await?
                    .ok_or_else(invalid)?;
            }
        }
    }
    Ok(())
}

async fn git_credential(
    s: &AppState,
    scope: &Scope,
    params: &Value,
    provider: Option<&str>,
) -> Result<(Id, Option<(String, String, String)>)> {
    if provider.is_some()
        && (params.get("connection_id").is_none() || params["auth_mode"] == "anonymous")
    {
        return Err(invalid());
    }
    if params["auth_mode"] == "anonymous"
        && (params.get("connection_id").is_some()
            || !graph::anonymous_git_url(params["url"].as_str().unwrap_or_default()))
    {
        return Err(invalid());
    }
    let Some(connection) = params.get("connection_id") else {
        return Ok((0.into(), None));
    };
    scope.require("repositories.read")?;
    scope.require("credentials.use")?;
    let id = connection
        .as_str()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if let Some(provider) = provider {
        let row = crate::repositories::auth::owned_connection(&s.db, scope, id).await?;
        if row.provider != provider {
            return Err(invalid());
        }
    }
    let url = params["url"].as_str().ok_or_else(invalid)?;
    let (_, credential) =
        crate::repositories::auth::resolve_connection(s, &s.db, scope, id, url).await?;
    if credential == 0.into() {
        return Ok((credential, None));
    }
    Ok((
        credential,
        Some(crate::credentials::unseal(s, &scope.workspace_id, &credential).await?),
    ))
}

/// Expand a saved repository selection before sending any host operation.
/// Each clone retains the existing per-item durable call gate and authorization.
fn clone_inputs(items: &[Item], params: &[Value]) -> Result<(Vec<Item>, Vec<Value>)> {
    if items.len() != params.len() {
        return Err(invalid());
    }
    let mut expanded_items = Vec::new();
    let mut expanded_params = Vec::new();
    let mut directories = std::collections::HashSet::new();
    for (item, params) in items.iter().zip(params) {
        let Some(repositories) = params.get("repositories") else {
            expanded_items.push(item.clone());
            expanded_params.push(params.clone());
            continue;
        };
        let repositories = repositories
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or_else(invalid)?;
        if repositories.len() > 500
            || expanded_params.len() + repositories.len() > 500
            || params.get("connection_id").is_none()
            || params["auth_mode"] == "anonymous"
        {
            return Err(invalid());
        }
        let root = params["directory"].as_str().ok_or_else(invalid)?;
        crate::hosts::channel::check_path(root)?;
        let mut urls = std::collections::HashSet::new();
        for (index, repository) in repositories.iter().enumerate() {
            let url = repository["url"].as_str().ok_or_else(invalid)?;
            if !urls.insert(url) {
                return Err(invalid());
            }
            let name = repository["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(invalid)?;
            let slug: String = name
                .chars()
                .take(80)
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let directory = format!(
                "{}/{:03}-{slug}",
                root.trim_end_matches(['/', '\\']),
                index + 1
            );
            if !directories.insert(directory.replace('\\', "/").to_lowercase()) {
                return Err(invalid());
            }
            let mut next = params.clone();
            next.as_object_mut()
                .ok_or_else(invalid)?
                .remove("repositories");
            next["url"] = json!(url);
            next["directory"] = json!(directory);
            super::host::git_script("clone", &next)?;
            expanded_items.push(item.clone());
            expanded_params.push(next);
        }
    }
    Ok((expanded_items, expanded_params))
}

/// The execution identity, workflow access and run lifecycle as of this moment.
pub(super) async fn execution_scope(
    s: &AppState,
    run: &workflow_executions::Model,
) -> Result<Scope> {
    let scope = Scope::for_actor(s, Id::from(run.workspace_id), Id::from(run.triggered_by)).await?;
    scope.require("workflows.run")?;
    let workflow = super::scoped(&s.db, scope.workspace_id, Id::from(run.workflow_id)).await?;
    access::require(&s.db, &scope, &workflow, Operation::Run).await?;
    let current = workflow_executions::Entity::find_by_id(run.id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if workflow.lifecycle != "active" || current.state != "running" {
        return Err(ApiError::forbidden());
    }
    Ok(scope)
}

pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    node: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    if node["type"] == "notification.send" {
        // SMTP/provider futures must not inflate every Git/HTTP integration task.
        return Box::pin(execute_notification(s, run, node_run, items, params)).await;
    }
    let manifest = graph::manifest(node).ok_or_else(invalid)?;
    let http = &manifest["http"];
    let git = manifest["git"]["operation"].as_str();
    let command = node["type"] == "host.command";
    // Commands run once per batch; preserve input lineage without rerunning a build per item.
    let command_items = [Item {
        json: Map::new(),
        binary: Default::default(),
        paired: items
            .iter()
            .flat_map(|item| item.paired.iter().cloned())
            .collect(),
    }];
    let items = if command { &command_items[..] } else { items };
    let expanded;
    let (items, params) = if git == Some("clone") {
        expanded = clone_inputs(items, params)?;
        (expanded.0.as_slice(), expanded.1.as_slice())
    } else {
        (items, params)
    };
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let client = client_with_policy(&policy)?;
    if params.len() != items.len() {
        return Err(invalid());
    }
    let mut output = Vec::new();
    for (index, (item, params)) in items.iter().zip(params).enumerate() {
        // Recheck the actor, resource and lifecycle immediately before every request.
        let scope = execution_scope(s, run).await?;
        let (git_id, git_auth) = if git.is_some() {
            git_credential(s, &scope, params, manifest["git"]["provider"].as_str()).await?
        } else {
            (0.into(), None)
        };
        let (http, params, credential_id, secret) = if git.is_some() || command {
            (http.clone(), params.clone(), git_id, String::new())
        } else {
            resolve_http(s, &scope, http, params).await?
        };
        let builder = if git.is_some() || command {
            None
        } else {
            Some(request_with_connection_origin(
                &client,
                &http,
                &params,
                &secret,
                params.get("connection_id").is_some(),
            )?)
        };
        let canonical = crate::definitions::canonical(
            &json!({"http":http,"git":git,"params":params,"execution":node.get("settings")})
                .to_string(),
        )?;
        let hash = hex::encode(Sha256::digest(canonical.as_bytes()));
        let id: i64 = s.next_id()?.into();
        let now = db::now();
        let call = calls::Model {
            id,
            workspace_id: run.workspace_id,
            execution_id: run.id,
            node_run_id: node_run,
            item_index: index as i64,
            credential_id: credential_id.into(),
            request_hash: hash,
            state: "started".into(),
            result: None,
            created_at: now,
            created_by: run.triggered_by,
            updated_at: now,
            updated_by: run.triggered_by,
            version: 1,
        };
        // The unique attempt/item identity is the durable send gate. A crashed
        // request remains unknown and is never automatically issued again.
        if let Some(call) = reserve_call(&s.db, call).await? {
            let plain = s.vault.open_secret(
                call.result.as_deref().ok_or_else(unknown)?,
                &aad(run.workspace_id, call.id),
            )?;
            let mut result = item.clone();
            result.json = serde_json::from_str(&plain).map_err(|_| unknown())?;
            output.push(result);
            continue;
        }
        let result = if command {
            let script = super::host::command_script(&params)?;
            super::host::execute(s, run, node, &script, true)
                .await
                .map(|mut value| {
                    value["exit_code"] = json!(0);
                    value
                })
        } else if let Some(operation) = git {
            let script = super::host::git_script(operation, &params)?;
            // Authenticated transport output is redacted as a whole so a token
            // split across output chunks cannot escape into live node logs.
            super::host::execute_with_credential(
                s,
                run,
                node,
                &script,
                git_auth.is_none(),
                git_auth.as_ref(),
            )
            .await
            .map(|mut value| {
                if let Some(text) = value["stdout"].as_str() {
                    value["stdout"] = json!(crate::pipelines::runs::redact(
                        text.to_owned(),
                        git_auth.as_ref(),
                        &Default::default()
                    ));
                }
                value["operation"] = json!(operation);
                if operation == "clone" {
                    value["repository_url"] = params["url"].clone();
                    value["directory"] = params["directory"].clone();
                }
                value["lines"] = json!(
                    value["stdout"]
                        .as_str()
                        .unwrap_or_default()
                        .lines()
                        .collect::<Vec<_>>()
                );
                value
            })
        } else if super::host::requested(node) {
            host_request(s, run, node, &http, builder.ok_or_else(invalid)?, &secret).await
        } else {
            super::host::while_active(s, run, async {
                match builder.ok_or_else(invalid)?.send().await {
                    Ok(reply) => response_with_policy(&http, reply, &secret, &policy).await,
                    Err(e) => {
                        let host = e
                            .url()
                            .and_then(|url| url.host_str().map(str::to_owned))
                            .unwrap_or_else(|| "?".into());
                        // Only a request that never reached the application is a known failure.
                        Err(
                            match crate::upstream::unreachable("integration", &host, &e) {
                                Some(error) if e.is_connect() => remote_from(error),
                                _ => unknown(),
                            },
                        )
                    }
                }
            })
            .await
        };
        let (state, sealed) = match &result {
            Ok(value) => (
                "succeeded",
                Some(
                    s.vault
                        .seal(&value.to_string(), &aad(run.workspace_id, id))?,
                ),
            ),
            Err(error) if error.2.as_ref().is_some_and(|m| is_remote(&m.key)) => ("failed", None),
            Err(_) => ("unknown", None),
        };
        calls::Entity::update_many()
            .col_expr(calls::Column::State, Expr::value(state))
            .col_expr(calls::Column::Result, Expr::value(sealed))
            .col_expr(calls::Column::UpdatedAt, Expr::value(db::now()))
            .col_expr(calls::Column::Version, Expr::value(2_i64))
            .filter(calls::Column::Id.eq(id))
            .filter(calls::Column::State.eq("started"))
            .exec(&s.db)
            .await?;
        let mut result_item = item.clone();
        result_item.json = result?.as_object().ok_or_else(invalid)?.clone();
        output.push(result_item);
    }
    Ok(output)
}

async fn execute_notification(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    if items.len() != params.len() {
        return Err(invalid());
    }
    let mut output = Vec::new();
    for (index, (item, params)) in items.iter().zip(params).enumerate() {
        let scope = execution_scope(s, run).await?;
        let channel_id = Id::reference(params["channel_id"].as_str().ok_or_else(invalid)?)?;
        let mut body = params.clone();
        body.as_object_mut()
            .ok_or_else(invalid)?
            .remove("channel_id");
        if body["template_id"] == "" {
            body.as_object_mut().unwrap().remove("template_id");
        }
        let fields = body.as_object_mut().unwrap().remove("attachments");
        let files = attachments(s, run, item, fields.as_ref()).await?;
        body["client_id"] = json!(format!("workflow:{}:{node_run}:{index}", run.id));
        let body = serde_json::from_value(body).map_err(|_| invalid())?;
        let result = crate::notifications::center::send_message_with_files(
            s,
            &scope,
            channel_id,
            body,
            "workflow",
            &run.id.to_string(),
            files,
        )
        .await?;
        if result["state"] != "succeeded" {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                Message::new(if result["state"] == "unknown" {
                    "notification.unknown"
                } else {
                    "notification.sendFailed"
                }),
            ));
        }
        output.push(Item {
            json: serde_json::from_value(result).map_err(|_| invalid())?,
            binary: item.binary.clone(),
            paired: item.paired.clone(),
        });
    }
    Ok(output)
}

/// The files a notification attaches: the item's file artifacts named in
/// `fields` (binary field names; `"*"` means every file on the item), whatever
/// node produced them.
async fn attachments(
    s: &AppState,
    run: &workflow_executions::Model,
    item: &Item,
    fields: Option<&Value>,
) -> Result<Vec<crate::notifications::center::Attachment>> {
    let names: Vec<&str> = fields
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if names.is_empty() {
        return Ok(vec![]);
    }
    let mut chosen = Vec::new();
    if names.contains(&"*") {
        chosen.extend(item.binary.values());
    } else {
        for name in names {
            let file = item.binary.get(name).ok_or_else(|| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    Message::new("notification.attachmentMissing").with("field", name),
                )
            })?;
            chosen.push(file);
        }
    }
    let backend = crate::artifact_store::ArtifactStore::for_state(s).await?;
    let store = crate::content_assets::Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    let mut files = Vec::new();
    for file in chosen {
        let asset = super::media::resolve(s, run.workspace_id, run.workflow_id, file).await?;
        let bytes = store.read_bytes(&s.db, run.workspace_id, asset.id).await?;
        let link = crate::file_links::sign(
            s,
            run.workspace_id,
            &asset.sha256,
            &file.name,
            &asset.mime,
            crate::file_links::DEFAULT_TTL_DAYS,
        )
        .await?;
        files.push(crate::notifications::center::Attachment {
            name: file.name.clone(),
            mime: asset.mime.clone(),
            sha256: asset.sha256.clone(),
            bytes,
            link: link.map(|(url, _)| url),
        });
    }
    Ok(files)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    node_type: String,
    credential_id: Id,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/workflow-integration-options", post(options))
        .route("/{workspace}/workflow-node-catalog", get(node_catalog))
        .route(
            "/{workspace}/workflow-definition-format",
            get(definition_format),
        )
}
/// How to create, edit, publish, schedule and run workflows. The exact envelope
/// of a workflow definition as POST/PATCH /workflows
/// validate it, with one complete valid example, so an Agent never guesses
/// field names (e.g. `version` or `edges`).
async fn definition_format(scope: Scope, Path(_): Path<String>) -> Result<Json<Value>> {
    let _ = scope;
    Ok(Json(json!({
        "schema": super::graph::graph_schema(),
        "example": definition_example(),
        "execution_support": {
            "settings": {
                "timeout_seconds":"Omit or use 0. A workflow-wide timeout is not implemented; legacy node timeout_s is accepted but ignored; administrators may configure task_timeout_seconds.",
                "save_data":"Omit. Custom retention policies are not implemented; existing execution records remain in use.",
                "concurrency":"Omit or use scope=none. Workflow-wide concurrency policies are not implemented.",
                "error_workflow_id":"Omit or use an empty string. Error workflows are not implemented."
            },
            "node_settings":"Use on_error=stop and retry.max_tries=1 (or omit them). Other policies are not implemented. core.wait supports duration/until; core.merge supports append. Flat pins, bypass and always_output have restrictions on conditional/preview nodes; inspect exact run rejection paths before changing those semantics.",
            "placement":"Execution is per node, not per workflow. CLI nodes use an explicit node target, otherwise a compatible workflow default, otherwise their model's candidate Runner hosts. Different nodes may use different hosts. Pass data and file artifacts through node outputs; local filesystem paths are not shared across hosts.",
            "api_agents":"ai.agent.run also accepts enabled API-model Agents. The server calls that model's configured provider, without a Runner, ignoring CLI host targets. The same GET-only platform tools, live user/Agent permission checks and output_schema apply. files.enabled requires a supported CLI: use structured API output followed by report.render for PDF files. max_tokens is an explicit provider request parameter. OpenAI-compatible requests omit it when absent; Anthropic requires a positive value before execution.",
            "metering":"No per-call or monthly monetary limit is enforced. Legacy max_cost_micros and monthly_limit_micros fields are accepted for compatibility but ignored for admission. Configured prices calculate recorded costs; absent prices do not block calls. Administrator-configured provider request rate and concurrency limits apply; legacy provider qps/concurrency values are retained but ignored. Uncertain calls are recorded and never automatically resent.",
            "recovery":"A schema-valid draft can contain settings this runtime cannot execute. On a run/publication rejection, inspect the exact path and reason. Do not attribute /settings failures to Runner availability. Remove unsupported optional settings only when their semantics are not required; otherwise explain the missing capability and ask the user. Inspect the current version before PATCH, then retry the draft before publishing or scheduling."
        },
        "requests": {
            "create_minimal": {
                "method":"POST", "path":"/workflows",
                "body":{"name":"Manual workflow","definition":json!({
                    "engine":"ferry-graph","schema":1,"settings":{"timezone":"UTC"},
                    "nodes":[{"id":"start","type":"core.trigger.manual","type_version":1,
                        "name":"Start","position":[0,0],"params":{"input_schema":{"type":"object"}}}],
                    "connections":[]
                }).to_string()}
            },
            "inspect":{"method":"GET","path":"/workflows/{workflow_id}"},
            "run_draft":{"method":"POST","path":"/workflows/{workflow_id}/executions",
                "body":{"mode":"manual","revision":"draft","version":"<current workflow version: integer>","entry_node_id":"start","inputs":{}}},
            "publish":{"method":"POST","path":"/workflows/{workflow_id}/publications",
                "body":{"version":"<current workflow version: integer>","revision_id":"<draft_revision_id>","name":"v1"}},
            "schedule":{"method":"POST","path":"/cron-jobs","body":{
                "name":"Weekly workflow","scope":"build","action":"workflow","target_kind":"workflow",
                "target_ref":"<workflow_id>","entry_node_id":"start","inputs":{},
                "schedule":"0 9 * * 1","timezone":"UTC","enabled":true}},
            "run_schedule":{"method":"POST","path":"/cron-jobs/{job_id}/runs"},
            "inspect_execution":{"method":"GET","path":"/workflow-executions/{execution_id}"},
            "inspect_nodes":{"method":"GET","path":"/workflow-executions/{execution_id}/node-runs"}
        },
        "rules": [
            "All requests use ferry_request. Replace every {path_placeholder} and <value_placeholder> using actual prior API results; version must be a JSON integer, IDs are strings. create_minimal is executable as-is but is only a manual trigger: extend it with real nodes for the user's task.",
            "Inspect node contracts with GET /workflow-node-catalog?type=exact.node.type, or search=keyword. Follow next_offset using offset until null. Nodes are in nodes and builtin_nodes; total counts all matches. Never repeatedly request the unfiltered full directory after a 413.",
            "The server applies the human-configured approval mode: request requires approval for writes; assisted automatically approves declared ordinary draft/organization edits; yolo automatically approves permitted writes. No mode bypasses live permissions or validation. When required, approval grants permission to attempt the operation; only a successful API result proves it ran. A 400 is a rejected request, not a pending approval.",
            "Read the workflow again after changes, publication or a 409 before using version. Save and test the draft before publishing. A scheduled workflow must have a published revision; saving alone does not enable scheduling. Choose timezone and schedule from the user's requirements, not this UTC example.",
            "Poll run_schedule's run via GET /cron-runs/{id}. When downstream_kind is workflow-executions and downstream_id is nonzero, GET /workflow-executions/{downstream_id} identifies the actual execution; workflow_id identifies its workflow. There is no execution_id field on a cron-run. Read execution state and node output before reporting success; queued/running is not completion. Never claim a report was delivered without its actual result.",
            "Send the definition as a JSON string in the `definition` field of POST /workflows (with `name`).",
            "Top level: exactly engine=\"ferry-graph\", schema=1, nodes, connections; optional settings, layout, pin_data. No other fields.",
            "Each node: id (lower_snake, unique in its graph), type and type_version from GET /workflow-node-catalog?section=builtin (or the integration catalog), name, params valid for that type, position [x, y]. Names do not have to be unique.",
            "The schema describes the graph envelope. Node params use that node's catalog params_schema; expression_fields additionally allow strings beginning with = at those paths in saved drafts. Evaluated values must satisfy params_schema.",
            "Connections link {node, port, index} to {node, port, index}; ports are the `port` names in the node contract (usually main).",
            "Saving also validates node versions, runtime/cache/credential settings, node-specific semantics, existing endpoint nodes and ports/indexes, duplicate connections, acyclic graphs, loop nesting, pin references, timezone and administrator-configured resource limits. JSON Schema validation alone does not guarantee acceptance or permission to execute.",
            "Replace example ids (agent_id, channel_id, report source ids) with real ones from the platform.",
            "If saving fails, the error names a JSON path and the reason: fix that field and send the request again."
        ]
    })))
}

/// Manual trigger, platform data, AI analysis, PDF report, then a notification
/// with the PDF attached. Ids are placeholders that must be replaced.
pub(crate) fn definition_example() -> Value {
    let link = |from: &str, to: &str| json!({"from":{"node":from,"port":"main","index":0},"to":{"node":to,"port":"main","index":0}});
    json!({
        "engine": "ferry-graph",
        "schema": 1,
        "settings": {"timezone": "Asia/Shanghai"},
        "nodes": [
            {"id":"start","name":"Start","type":"core.trigger.manual","type_version":1,"position":[0,0],
             "params":{"input_schema":{"type":"object"}}},
            {"id":"commits","name":"Read contributors","type":"ferry.request","type_version":1,"position":[260,0],
             "params":{"operation":"/code-insights/{id}/reports/{kind}","path_params":{"id":"1","kind":"contributors"},
                       "query":{},"split_items":false,"all_pages":false,"max_pages":50}},
            {"id":"analyze","name":"Summarize contributors","type":"ai.agent.run","type_version":1,"position":[520,0],
             "params":{"agent_id":"1","prompt":"=Summarize this contributors report using only the provided data: {{ JSON.stringify($json) }}",
                       "output_schema":{"type":"object","properties":{"title":{"type":"string"},"summary":{"type":"string"}},
                       "required":["title","summary"]}}},
            {"id":"report","name":"Render PDF","type":"report.render","type_version":1,"position":[780,0],
             "params":{"format":"pdf","title":"={{ $json.title }}","subtitle":"","period":"","summary":"={{ $json.summary }}",
                       "metrics":[],"sections":[{"title":"Contributors","markdown":"={{ $json.summary }}","page_break":false}],
                       "language":"zh","file_name":"weekly-report","output_field":"file","link_ttl_days":7}},
            {"id":"notify","name":"Send report","type":"notification.send","type_version":1,"position":[1040,0],
             "params":{"channel_id":"1","subject":"","body":"={{ $json.summary }}","format":"text","to":[],"variables":{},"attachments":["file"]}}
        ],
        "connections": [link("start","commits"), link("commits","analyze"), link("analyze","report"), link("report","notify")]
    })
}
/// Node contracts for workflows. Use offset (default 0), page_size (1..20, default 5),
/// search (type/name substring), type (exact node type), and section=builtin or
/// section=integration. Follow next_offset until null; total counts all matches.
/// MCP calls are paginated automatically. Browser calls without filters retain
/// the full editor catalog. GET /workflow-definition-format supplies the graph
/// schema and a valid definition example; node params_schema supplies parameters.
async fn node_catalog(
    scope: Scope,
    Path(_): Path<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>> {
    // Scope verifies active membership. Only public declarations are returned;
    // tokens, workspace data and saved workflow definitions are never included.
    let _ = scope;
    if ["offset", "page_size", "search", "type"]
        .iter()
        .any(|key| query.contains_key(*key))
        || query
            .get("section")
            .is_some_and(|section| section == "integration")
    {
        return catalog_page(&query).map(Json);
    }
    if query.get("section").map(String::as_str) == Some("builtin") {
        return Ok(Json(json!({"builtin_nodes":builtin_nodes()})));
    }
    Ok(Json(
        // `nodes` stays the integration catalogue the editor installs; built-in
        // nodes are listed separately for clients (e.g. Agents) that need them.
        json!({"nodes":catalog()["nodes"],"builtin_nodes":builtin_nodes(),"packages":super::extensions::catalog().packages}),
    ))
}

fn catalog_page(query: &std::collections::HashMap<String, String>) -> Result<Value> {
    let offset = query
        .get("offset")
        .map(|value| value.parse::<usize>().map_err(|_| invalid()))
        .transpose()?
        .unwrap_or(0);
    let page_size = query
        .get("page_size")
        .map(|value| value.parse::<usize>().map_err(|_| invalid()))
        .transpose()?
        .unwrap_or(5);
    if !(1..=20).contains(&page_size) {
        return Err(invalid());
    }
    let section = query.get("section").map(String::as_str).unwrap_or("");
    if !["", "builtin", "integration"].contains(&section) {
        return Err(invalid());
    }
    let search = query
        .get("search")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let kind = query.get("type").map(String::as_str);
    let builtin = builtin_nodes();
    let integrations = catalog()["nodes"].as_array().ok_or_else(invalid)?;
    let matches: Vec<_> = integrations
        .iter()
        .map(|node| (false, node))
        .chain(builtin.iter().map(|node| (true, node)))
        .filter(|(is_builtin, _)| match section {
            "builtin" => *is_builtin,
            "integration" => !*is_builtin,
            _ => true,
        })
        .filter(|(_, node)| kind.is_none_or(|kind| node["type"] == kind))
        .filter(|(_, node)| {
            search.is_empty()
                || ["type", "name"].iter().any(|key| {
                    node[*key]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&search)
                })
        })
        .collect();
    let total = matches.len();
    let mut nodes = Vec::new();
    let mut builtin_nodes = Vec::new();
    let mut bytes = 0;
    let mut count = 0;
    for (is_builtin, node) in matches.into_iter().skip(offset).take(page_size) {
        let size = serde_json::to_vec(node).map_err(|_| invalid())?.len();
        // Leave room for the envelope and MCP metadata below the 256 KiB limit.
        if count > 0 && bytes + size > 192 * 1024 {
            break;
        }
        bytes += size;
        count += 1;
        if is_builtin {
            builtin_nodes.push(node.clone());
        } else {
            nodes.push(node.clone());
        }
    }
    let next = offset.saturating_add(count);
    Ok(
        json!({"nodes":nodes,"builtin_nodes":builtin_nodes,"total":total,
        "offset":offset,"page_size":page_size,"next_offset":(next<total).then_some(next)}),
    )
}

/// Bundled non-integration contracts this server can execute (triggers, data,
/// flow control, platform data, AI); planned contracts without an executor are omitted.
pub(crate) fn builtin_nodes() -> Vec<Value> {
    let integrations: std::collections::BTreeSet<String> = catalog()["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| node["type"].as_str().map(str::to_owned))
        .collect();
    super::graph::manifests()
        .filter(|spec| {
            let kind = spec["type"].as_str().unwrap_or_default();
            let probe = json!({"type": kind, "type_version": spec["type_version"].clone(), "params": spec["example_params"].clone()});
            !integrations.contains(kind) && super::run::runnable(&probe)
        })
        .map(|spec| {
            // Only nodes the executor runs are listed; their contract files may
            // still carry the draft "design_only" label, which agents read as
            // "cannot run" and then refuse to build.
            let mut spec = spec.clone();
            spec["status"] = json!("implemented");
            spec
        })
        .collect()
}
async fn options(
    State(s): State<AppState>,
    scope: Scope,
    Path(_): Path<String>,
    Json(body): Json<Options>,
) -> Result<Json<Value>> {
    scope.require("workflows.read")?;
    // Options are restricted to explicitly declared read-only lookup operations.
    let node = json!({"type":body.node_type,"type_version":1});
    let spec = graph::manifest(&node).ok_or_else(invalid)?;
    if !catalog()["nodes"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .any(|node| {
            node["params_schema"]["properties"]
                .as_object()
                .is_some_and(|fields| {
                    fields
                        .values()
                        .any(|field| field["options_source"]["type"] == body.node_type)
                })
        })
        || spec["http"]["method"] != "GET"
    {
        return Err(invalid());
    }
    let mut params = spec["example_params"].clone();
    params["credential_id"] = body.credential_id.to_string().into();
    graph::validate_resolved(&node, "/options", &params)?;
    let (_, secret) = credential(&s, &scope, &spec["http"], &params).await?;
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    let reply = request(
        &client_with_policy(&policy)?,
        &spec["http"],
        &params,
        &secret,
    )?
    .send()
    .await
    .map_err(|_| unknown())?;
    Ok(Json(
        response_with_policy(&spec["http"], reply, &secret, &policy).await?,
    ))
}

#[cfg(test)]
mod tests;
