//! Agent calls reuse the normal workspace HTTP handlers; only their identity is delegated.
use crate::{
    accounts::workspace::Scope,
    entities::{ai_agents, definition_revisions, workspace_roles},
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Extension,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_service::Service;

#[derive(Clone)]
pub(crate) struct DelegatedScope(pub Scope);

pub(crate) fn current_agent() -> Option<Id> {
    CURRENT_AGENT.try_with(|id| *id).ok()
}

/// Restore the invoking Agent's ceiling when a durable operation resumes later.
pub(crate) async fn delegated_future<T>(
    s: &AppState,
    scope: Scope,
    agent_id: Id,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let agent = ai_agents::Entity::find_by_id(agent_id)
        .filter(ai_agents::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let scope = authorized_scope(s, scope, &agent).await?;
    CURRENT_AGENT
        .scope(agent_id, CURRENT_SCOPE.scope(scope, future))
        .await
}

tokio::task_local! {
    /// A handler that refreshes the invoking account must retain its delegated ceiling.
    static CURRENT_SCOPE: Scope;
    static CURRENT_AGENT: Id;
}

pub(crate) async fn restrict(conn: &impl ConnectionTrait, mut scope: Scope) -> Result<Scope> {
    if let Ok(ceiling) = CURRENT_SCOPE.try_with(Clone::clone) {
        if scope.workspace_id != ceiling.workspace_id {
            return Err(ApiError::forbidden());
        }
        // Other accounts are looked up for ownership-transfer validation, never impersonation.
        if scope.user.id == ceiling.user.id {
            scope
                .permissions
                .retain(|p| ceiling.permissions.contains(p));
            scope.user.is_superadmin = false;
            if let Ok(agent_id) = CURRENT_AGENT.try_with(|id| *id) {
                let agent = ai_agents::Entity::find_by_id(agent_id)
                    .one(conn)
                    .await?
                    .ok_or_else(ApiError::forbidden)?;
                scope = intersect_agent(conn, scope, &agent).await?;
            }
        }
    }
    Ok(scope)
}

pub(crate) async fn role_id(conn: &impl ConnectionTrait, agent: &ai_agents::Model) -> Result<Id> {
    let Some(revision) = definition_revisions::Entity::find_by_id(agent.revision_id)
        .filter(definition_revisions::Column::WorkspaceId.eq(agent.workspace_id))
        .filter(definition_revisions::Column::SubjectKind.eq("agent"))
        .filter(definition_revisions::Column::SubjectId.eq(agent.id))
        .one(conn)
        .await?
    else {
        return Ok(Id::default());
    };
    let value: Value =
        serde_json::from_str(&revision.definition).map_err(|_| ApiError::forbidden())?;
    Id::reference(value["platform_role_id"].as_str().unwrap_or("0"))
}

pub(crate) async fn authorized_scope(
    s: &AppState,
    scope: Scope,
    agent: &ai_agents::Model,
) -> Result<Scope> {
    intersect_agent(&s.db, scope, agent).await
}

async fn intersect_agent(
    conn: &impl ConnectionTrait,
    mut scope: Scope,
    agent: &ai_agents::Model,
) -> Result<Scope> {
    if agent.workspace_id != i64::from(scope.workspace_id) || agent.state != "enabled" {
        return Err(ApiError::forbidden());
    }
    let id = role_id(conn, agent).await?;
    if i64::from(id) == 0 {
        return Err(ApiError::forbidden());
    }
    let role = workspace_roles::Entity::find_by_id(id)
        .filter(workspace_roles::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::forbidden)?;
    let grants: Vec<String> =
        serde_json::from_str(&role.permissions).map_err(|_| ApiError::forbidden())?;
    scope.permissions.retain(|p| grants.contains(p));
    // Being invoked by a platform administrator does not make an Agent a superadmin.
    scope.user.is_superadmin = false;
    Ok(scope)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub transfer: Option<Transfer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transfer {
    /// Base64 keeps binary bytes out of JSON string encoding and filesystem paths.
    pub data_base64: String,
    /// Omit for raw uploads. Multipart endpoints receive one file field.
    pub filename: Option<String>,
    #[serde(default)]
    pub field: Option<String>,
}
impl Call {
    pub fn read_only(&self) -> bool {
        self.method == "GET"
    }
    pub fn validate(&self) -> Result<()> {
        if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&self.method.as_str())
            || !self.path.starts_with('/')
            || self.path.starts_with("//")
            || self.path.len() > 4096
            || self.path.contains(['#', '\\'])
            || self
                .path
                .split('?')
                .next()
                .unwrap_or("")
                .split('/')
                .any(|v| v == "." || v == "..")
            || self
                .path
                .split('?')
                .next()
                .unwrap_or("")
                .to_lowercase()
                .contains("%2f")
            || self
                .path
                .split('?')
                .next()
                .unwrap_or("")
                .to_lowercase()
                .contains("%5c")
            || self.path.starts_with("/api/")
            || self.path.chars().any(char::is_control)
            || serde_json::to_vec(&self.body)
                .map_err(|_| ApiError::forbidden())?
                .len()
                > 1024 * 1024
        {
            return Err(ApiError::forbidden());
        }
        if let Some(transfer) = &self.transfer {
            if !self.body.is_null()
                || transfer.data_base64.len() > 1024 * 1024
                || transfer
                    .filename
                    .iter()
                    .chain(transfer.field.iter())
                    .any(|s| {
                        s.len() > 255
                            || s.contains(['\r', '\n', '"', '\\'])
                            || s.chars().any(char::is_control)
                    })
            {
                return Err(ApiError::forbidden());
            }
        }
        Ok(())
    }
}

pub(crate) fn redact(value: &mut Value) {
    fn secret(key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        [
            "password",
            "secret",
            "token",
            "private_key",
            "api_key",
            "authorization",
            "cookie",
        ]
        .iter()
        .any(|s| key == *s || key.ends_with(&format!("_{s}")))
    }
    fn collect(value: &Value, secrets: &mut Vec<String>) {
        match value {
            Value::Object(values) => {
                for (key, value) in values {
                    if secret(key) {
                        if let Some(text) = value.as_str().filter(|s| !s.is_empty()) {
                            secrets.push(text.to_owned());
                        }
                    } else {
                        collect(value, secrets);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, secrets);
                }
            }
            _ => {}
        }
    }
    fn replace(value: &mut Value, secrets: &[String]) {
        match value {
            Value::Object(values) => {
                for (key, value) in values {
                    if secret(key) {
                        *value = json!("[redacted]");
                    } else {
                        replace(value, secrets);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    replace(value, secrets);
                }
            }
            Value::String(text) => {
                for secret in secrets {
                    *text = text.replace(secret, "[redacted]");
                }
            }
            _ => {}
        }
    }
    let mut secrets = Vec::new();
    collect(value, &mut secrets);
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    replace(value, &secrets);
}

pub(crate) async fn dispatch_as_agent(
    s: &AppState,
    scope: Scope,
    call: Call,
    agent_id: Id,
) -> Result<Value> {
    CURRENT_AGENT
        .scope(agent_id, dispatch(s, scope, call))
        .await
}

pub(crate) async fn dispatch(s: &AppState, scope: Scope, call: Call) -> Result<Value> {
    call.validate()?;
    let uri = format!(
        "/{workspace}{path}",
        workspace = scope.workspace_id,
        path = call.path
    );
    let (content_type, body) = if let Some(transfer) = &call.transfer {
        let bytes = STANDARD
            .decode(&transfer.data_base64)
            .map_err(|_| ApiError::forbidden())?;
        if let Some(filename) = &transfer.filename {
            let boundary = format!("platform-{}", uuid::Uuid::new_v4());
            let field = transfer.field.as_deref().unwrap_or("file");
            let mut body=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend(bytes);
            body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
            (format!("multipart/form-data; boundary={boundary}"), body)
        } else {
            ("application/octet-stream".into(), bytes)
        }
    } else {
        (
            "application/json".into(),
            if call.body.is_null() {
                Vec::new()
            } else {
                serde_json::to_vec(&call.body).map_err(|_| ApiError::forbidden())?
            },
        )
    };
    let request = Request::builder()
        .method(
            call.method
                .parse::<Method>()
                .map_err(|_| ApiError::forbidden())?,
        )
        .uri(uri)
        .header("content-type", content_type)
        .header("x-ferry-request", "1")
        .body(Body::from(body))
        .map_err(|_| ApiError::forbidden())?;
    let mut router = crate::workspace_routes()
        .layer(Extension(DelegatedScope(scope.clone())))
        .with_state(s.clone());
    let response = CURRENT_SCOPE
        .scope(scope, router.call(request))
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let bytes = match to_bytes(response.into_body(), 256 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(
                json!({"status":413,"ok":false,"error":"The response exceeds the tool's 256 KiB limit. Reduce page_size or select a smaller file. A write may already have completed; inspect its state before retrying."}),
            );
        }
    };
    let mut body = serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| {
        if content_type.starts_with("text/") { json!({"text":String::from_utf8_lossy(&bytes)}) }
        else { json!({"content_type":content_type,"data_base64":STANDARD.encode(&bytes),"size":bytes.len()}) }
    });
    redact(&mut body);
    Ok(json!({"status":status.as_u16(),"ok":status.is_success(),"data":body}))
}

pub(crate) fn catalogue(scope: &Scope, search: &str, offset: usize) -> Value {
    let routes: Vec<Value> = serde_json::from_str(include_str!("platform_tools_catalog.json"))
        .expect("generated platform catalogue");
    let search = search.to_ascii_lowercase();
    let mut permissions = scope.permissions.iter().cloned().collect::<Vec<_>>();
    permissions.sort();
    let rows = routes
        .into_iter()
        .filter(|row| {
            row["permission"]
                .as_str()
                .is_none_or(|p| scope.permissions.contains(p))
        })
        .filter(|row| {
            row["permission_checks"].as_array().is_none_or(|checks| {
                checks.is_empty()
                    || checks
                        .iter()
                        .any(|p| p.as_str().is_some_and(|p| scope.permissions.contains(p)))
            })
        })
        .filter(|row| {
            search.is_empty()
                || row["path"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&search)
                || row["handler"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&search)
        })
        .collect::<Vec<_>>();
    let total = rows.len();
    json!({"workspace_id":scope.workspace_id,"permissions":permissions,"total":total,"next_offset":(offset+20<total).then_some(offset+20),
        "operations":rows.into_iter().skip(offset).take(20).collect::<Vec<_>>(),
        "ui_routes":{
            "workflow":format!("/workspaces/{}/workflows/{{id}}",scope.workspace_id),
            "workflow_execution":format!("/workspaces/{}/workflows/{{workflow_id}}?execution={{id}}",scope.workspace_id),
            "instructions":"Use these UI routes for user-facing links, prefixed with the user's current UI locale, e.g. /zh-CN. API /api URLs return JSON and must not be labeled as detail pages. Get workflow_id from the execution detail response. Keep authenticated file preview/download URLs unchanged."
        },
        "instructions":"Paths are relative to this conversation's workspace. Call ferry_request using the documented method/path/body. The normal API enforces each operation's permissions, ownership and validation. GET list responses contain total; never count only the first page. Writes require a user confirmation and fresh authorization. A 403 is a permission denial, not an empty list. Never infer platform records by scanning the Runner filesystem."})
}

#[cfg(test)]
pub(crate) mod tests;
