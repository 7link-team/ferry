//! Declarative SaaS operations. Only trusted, versioned manifests choose origins,
//! headers and methods; graph parameters can never choose where secrets are sent.
use super::graph;
use crate::{
    accounts::workspace::Scope,
    db,
    definitions::access::{self, Operation},
    entities::{credentials, workflow_executions, workflow_integration_calls as calls},
    error::{ApiError, Json, Path, Result},
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
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
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

async fn response(http: &Value, mut response: reqwest::Response, secret: &str) -> Result<Value> {
    // Never log/persist upstream error text, URLs or credentials.
    if !response.status().is_success() {
        return Err(remote());
    }
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unknown())? {
        if chunk.len() > (1024 * 1024_usize).saturating_sub(bytes.len()) {
            return Err(unknown());
        }
        bytes.extend_from_slice(&chunk);
    }
    response_bytes(http, status, &bytes, secret)
}

fn response_bytes(http: &Value, status: u16, bytes: &[u8], secret: &str) -> Result<Value> {
    if !(200..300).contains(&status) {
        return Err(remote());
    }
    if bytes.len() > 1024 * 1024 {
        return Err(unknown());
    }
    if !secret.is_empty()
        && bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    {
        return Err(unknown());
    }
    let value: Value = if bytes.is_empty() {
        json!({"status_code":status})
    } else {
        serde_json::from_slice(&bytes).map_err(|_| unknown())?
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
    let mut config = format!(
        "silent\nshow-error\nmax-time = 30\nmax-filesize = 1048576\nurl = {}\nrequest = {}\n",
        config_quote(request.url().as_str())?,
        config_quote(request.method().as_str())?
    );
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
    let mut result = response_bytes(
        http,
        status.trim().parse().map_err(|_| unknown())?,
        body.as_bytes(),
        secret,
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

pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    node_run: i64,
    node: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    if node["type"] == "notification.send" {
        return execute_notification(s, run, node_run, items, params).await;
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
    let client = client()?;
    if params.len() != items.len() {
        return Err(invalid());
    }
    let mut output = Vec::new();
    for (index, (item, params)) in items.iter().zip(params).enumerate() {
        // Recheck the actor, resource and lifecycle immediately before every request.
        let scope =
            Scope::for_actor(s, Id::from(run.workspace_id), Id::from(run.triggered_by)).await?;
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
            match builder.ok_or_else(invalid)?.send().await {
                Ok(reply) => response(&http, reply, &secret).await,
                Err(_) => Err(unknown()),
            }
        };
        let (state, sealed) = match &result {
            Ok(value) => (
                "succeeded",
                Some(
                    s.vault
                        .seal(&value.to_string(), &aad(run.workspace_id, id))?,
                ),
            ),
            Err(error)
                if error
                    .2
                    .as_ref()
                    .is_some_and(|m| m.key == "workflow.integrationRemote") =>
            {
                ("failed", None)
            }
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
        let scope = Scope::for_actor(s, run.workspace_id.into(), run.triggered_by.into()).await?;
        scope.require("workflows.run")?;
        let workflow = super::scoped(&s.db, scope.workspace_id, run.workflow_id.into()).await?;
        access::require(&s.db, &scope, &workflow, Operation::Run).await?;
        let current = workflow_executions::Entity::find_by_id(run.id)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if workflow.lifecycle != "active" || current.state != "running" {
            return Err(ApiError::forbidden());
        }
        let channel_id = Id::reference(params["channel_id"].as_str().ok_or_else(invalid)?)?;
        let mut body = params.clone();
        body.as_object_mut()
            .ok_or_else(invalid)?
            .remove("channel_id");
        if body["template_id"] == "" {
            body.as_object_mut().unwrap().remove("template_id");
        }
        body["client_id"] = json!(format!("workflow:{}:{node_run}:{index}", run.id));
        let body = serde_json::from_value(body).map_err(|_| invalid())?;
        let result = crate::notifications::center::send_message(
            s,
            &scope,
            channel_id,
            body,
            "workflow",
            &run.id.to_string(),
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
}
async fn node_catalog(scope: Scope, Path(_): Path<String>) -> Result<Json<Value>> {
    // Scope verifies active membership. Only public declarations are returned;
    // tokens, workspace data and saved workflow definitions are never included.
    let _ = scope;
    Ok(Json(
        json!({"nodes":catalog()["nodes"],"packages":super::extensions::catalog().packages}),
    ))
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
    let reply = request(&client()?, &spec["http"], &params, &secret)?
        .send()
        .await
        .map_err(|_| unknown())?;
    Ok(Json(response(&spec["http"], reply, &secret).await?))
}

#[cfg(test)]
mod tests;
