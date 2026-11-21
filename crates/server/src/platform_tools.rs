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
        {
            return Err(ApiError::forbidden());
        }
        if let Some(transfer) = &self.transfer {
            if !self.body.is_null()
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

pub(crate) async fn dispatch(s: &AppState, scope: Scope, mut call: Call) -> Result<Value> {
    call.validate()?;
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    // The editor consumes the full catalog, but MCP must stay below its bounded
    // response size. Preserve explicit pagination while making an unfiltered
    // discovery call safe as well.
    if call.method == "GET" && call.path.split('?').next() == Some("/workflow-node-catalog") {
        let query = call
            .path
            .split_once('?')
            .map(|(_, query)| query)
            .unwrap_or("");
        let has_page_size = query.split('&').any(|field| {
            percent_encoding::percent_decode_str(field.split('=').next().unwrap_or(""))
                .decode_utf8_lossy()
                == "page_size"
        });
        if !has_page_size {
            call.path.push_str(if call.path.contains('?') {
                "&page_size=5"
            } else {
                "?page_size=5"
            });
        }
    }
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
    if !ferry_protocol::resources::allows(policy.tool_request_bytes, body.len()) {
        return Ok(
            json!({"status":413,"ok":false,"error":"The request exceeds the administrator-configured tool request limit."}),
        );
    }
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
    let error_key = response
        .extensions()
        .get::<crate::i18n::Message>()
        .map(|message| message.key.clone());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let bytes = match to_bytes(
        response.into_body(),
        ferry_protocol::resources::byte_limit(policy.tool_response_bytes),
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(_) => {
            let reason = if call.read_only() {
                "This was a read-only request; no write was performed. Request a smaller page, a specific node type, or a smaller file."
            } else {
                "A write may already have completed; read its state before retrying. Request a smaller response."
            };
            let error = format!(
                "The response exceeds the configured tool limit of {} bytes. {reason}",
                policy.tool_response_bytes.unwrap_or_default()
            );
            return Ok(
                json!({"status":413,"ok":false,"error":error,"guidance":call_guidance(&call,413)}),
            );
        }
    };
    let mut body = serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| {
        if content_type.starts_with("text/") { json!({"text":String::from_utf8_lossy(&bytes)}) }
        else { json!({"content_type":content_type,"data_base64":STANDARD.encode(&bytes),"size":bytes.len()}) }
    });
    redact(&mut body);
    let mut result = json!({"status":status.as_u16(),"ok":status.is_success(),"data":body});
    if !status.is_success() {
        result["guidance"] = json!(if error_key.as_deref() == Some("storage.unavailable") {
            STORAGE_GUIDANCE
        } else {
            call_guidance(&call, status.as_u16())
        });
    }
    Ok(result)
}

const STORAGE_GUIDANCE: &str = "File storage failed. Check File storage health in Platform settings and ask an administrator to inspect the server diagnostics. Do not change the workflow definition, model or Runner to work around it. Do not blindly retry while storage is unhealthy. Before retrying a write after storage recovers, read the resource and its executions to check whether it already completed.";

fn call_guidance(call: &Call, status: u16) -> &'static str {
    let path = call.path.split('?').next().unwrap_or("");
    let definition_write = (call.method == "POST" && path == "/workflows")
        || (call.method == "PATCH"
            && path
                .strip_prefix("/workflows/")
                .is_some_and(|id| !id.is_empty() && !id.contains('/')));
    if matches!(status, 400 | 422) && definition_write {
        return "The workflow was rejected; this request created no workflow or revision. GET /workflow-definition-format for the exact Schema and executable requests.create_minimal example. definition must be a JSON string whose object has engine=\"ferry-graph\", schema=1, nodes and connections (not edges). Inspect the failing field, correct it using the Schema/node contracts, and request a new approval for the corrected write. Never describe this HTTP rejection as approval pending.";
    }
    if status == 413 && path == "/workflow-node-catalog" {
        return "Read a bounded catalog page with GET /workflow-node-catalog?page_size=5&offset=0, or use ?type=exact.node.type / ?search=keyword. Follow next_offset until null; both nodes and builtin_nodes contain contracts. Do not repeat the same oversized query. The limit is response bytes, not the model context window.";
    }
    if matches!(status, 400 | 422)
        && call.method == "POST"
        && path.starts_with("/workflows/")
        && (path.ends_with("/executions") || path.ends_with("/publications"))
    {
        return "The draft was rejected before execution/publication. GET /workflow-definition-format and inspect execution_support, then read the workflow's current definition and version. An error at /settings identifies an unsupported execution policy, not a missing Runner. Correct only optional settings whose removal preserves the user's requirements; otherwise explain the missing capability and request a decision. Retry the corrected draft before publishing or scheduling. Never claim a report or notification was produced by this rejected request.";
    }
    guidance(status)
}

/// What an Agent should do about a failed call, so it fixes and retries
/// instead of giving up (or blindly repeating a write).
fn guidance(status: u16) -> &'static str {
    match status {
        400 | 422 => {
            "The request was invalid; nothing was changed. The error names the field (a JSON path and the reason): fix exactly that and send the request again."
        }
        401 | 403 => {
            "Not permitted for this user and Agent role; nothing was changed. Name the missing permission to the user instead of retrying."
        }
        404 => "Not found: check the path and ids (list the resource first), then retry.",
        409 => {
            "The resource changed or is busy: read it again (for the current version) and retry once."
        }
        413 => "Too large: send less (smaller page_size or fewer fields).",
        424 => {
            "A setting it depends on failed its check; the error says which. Tell the user what to fix."
        }
        429 | 500..=599 => {
            "A temporary platform problem: wait briefly and retry once. For a write, first read the resource to check it did not already happen."
        }
        _ => "The platform refused the call; the error explains why.",
    }
}

fn routes() -> &'static [Value] {
    static ROUTES: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    ROUTES.get_or_init(|| {
        serde_json::from_str(include_str!("platform_tools_catalog.json"))
            .expect("generated platform catalogue")
    })
}

/// Workflow platform-data nodes may call exactly the catalogue's GET templates.
pub(crate) fn read_operation(path: &str) -> bool {
    routes()
        .iter()
        .any(|row| row["method"] == "GET" && row["path"] == path)
}

/// A concrete request path (query ignored) matches a catalogue GET template;
/// each placeholder stands for exactly one non-empty segment.
pub(crate) fn read_path(path: &str) -> bool {
    let concrete: Vec<_> = path.split('?').next().unwrap_or("").split('/').collect();
    routes().iter().any(|row| {
        row["method"] == "GET"
            && row["path"].as_str().is_some_and(|template| {
                let template: Vec<_> = template.split('/').collect();
                template.len() == concrete.len()
                    && template.iter().zip(&concrete).all(|(part, value)| {
                        if part.starts_with('{') && part.ends_with('}') {
                            !value.is_empty()
                        } else {
                            part == value
                        }
                    })
            })
    })
}

/// The tool result returned to an Agent when the platform refuses a call.
pub(crate) fn denial(error: &ApiError) -> Value {
    let status = error.0.as_u16();
    let guidance = if error
        .2
        .as_ref()
        .is_some_and(|message| message.key == "storage.unavailable")
    {
        STORAGE_GUIDANCE
    } else if matches!(status, 401 | 403) {
        "The platform denied this call. No Agent role means no platform access. Check this workspace's current user and Agent role grants. Do not substitute local filesystem searches or invent counts."
    } else {
        guidance(status)
    };
    json!({"status":status,"ok":false,"error":error.1,"guidance":guidance})
}

pub(crate) fn catalogue(scope: &Scope, search: &str, offset: usize) -> Value {
    listing(scope, search, offset, false)
}

/// The catalogue as seen by an unattended workflow Agent: GET operations only.
pub(crate) fn read_catalogue(scope: &Scope, search: &str, offset: usize) -> Value {
    let mut value = listing(scope, search, offset, true);
    value["instructions"] = json!(
        "Paths are relative to this workflow execution's workspace. This is an unattended, read-only workflow step: only the GET operations listed here are available through ferry_request; any other method or path is refused with 403 and nothing is executed. GET list responses contain total; never count only the first page. A 403 is a permission denial, not an empty list. Never infer platform records by scanning the Runner filesystem."
    );
    value
}

fn listing(scope: &Scope, search: &str, offset: usize, read_only: bool) -> Value {
    let routes = routes().to_vec();
    let search = search.to_ascii_lowercase();
    let mut permissions = scope.permissions.iter().cloned().collect::<Vec<_>>();
    permissions.sort();
    let rows = routes
        .into_iter()
        .filter(|row| !read_only || row["method"] == "GET")
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
                || row["documentation"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&search)
        })
        .collect::<Vec<_>>();
    let total = rows.len();
    json!({"workspace_id":scope.workspace_id,"permissions":permissions,"total":total,"next_offset":(offset+20<total).then_some(offset+20),
        "operations":rows.into_iter().skip(offset).take(20).collect::<Vec<_>>(),
        "usage":{
            "invoke":{"tool":"ferry_request","arguments":{"method":"GET","path":"<discovered workspace-relative path>","body":"<JSON object for a write; omit for reads>"}},
            "path":"Use a discovered operation. Replace {id} placeholders with real IDs returned by this workspace's APIs; URL-encode each substituted path/query value. Do not include /api, /workspaces/{workspace}, a hostname, or an arbitrary workspace_id.",
            "query":"GET operation.query lists supported keys and sort options. Append them to path, e.g. ?page=1&page_size=20. Only send keys declared for that operation. Page lists use page/page_size; discovery and node catalogs use offset/next_offset. An absent query description is not permission to invent filters.",
            "body":"For Json<T>, body is a JSON object matching definitions and documentation, not a serialized copy of a Rust struct. IDs are decimal strings; integer/bool values are JSON numbers/booleans. Option<T> fields can be omitted; serde defaults are server-owned. Respect declared enum/rename rules. Opaque fields such as workflow definition require their dedicated Schema, not guessed structure.",
            "response":{"status":"Actual HTTP status; 2xx means the request was accepted, not that a queued execution finished.","ok":"Request success boolean; false is an error, never an empty list.","data":"The real API result. Lists include items/total; inspect all required pages.","error":"Failure reason; may also be inside data.error.","guidance":"Concrete recovery instructions. Read these before retrying."},
            "writes":if read_only {"This unattended tool is read-only; no write or approval is available."} else {"Each non-GET request is reauthorized and follows the human-configured MCP approval mode. Required approvals permit an attempt; the platform_result proves the outcome. A failed request is not pending approval. Never replay an uncertain write without checking whether it already happened."},
            "recovery":"Correct validation errors from the actual Schema/contracts. For permission or configuration blockers, explain the exact cause and available remedies. Ask the user when a product choice or access change is needed; allow custom instructions, pause or abandon. Do not repeatedly send an unchanged failing request or claim completion when work remains.",
            "workflow_start":{"tool":"ferry_request","arguments":{"method":"GET","path":"/workflow-definition-format"}},
            "large_results":"The MCP gateway bounds response bytes independently of model context size. Read smaller pages or exact node contracts; use file download/preview links for files instead of loading their entire contents."
        },
        "ui_routes":{
            "workflow":format!("/workspaces/{}/workflows/{{id}}",scope.workspace_id),
            "workflow_execution":format!("/workspaces/{}/workflows/{{workflow_id}}?execution={{id}}",scope.workspace_id),
            "instructions":"Use these UI routes for user-facing links, prefixed with the user's current UI locale, e.g. /zh-CN. API /api URLs return JSON and must not be labeled as detail pages. Get workflow_id from the execution detail response. Keep authenticated file preview/download URLs unchanged."
        },
        "instructions":"Paths are relative to this conversation's workspace. Call ferry_request using the documented method/path/body. The normal API enforces each operation's permissions, ownership and validation. GET list responses contain total; never count only the first page. Writes follow the human-configured MCP approval mode and fresh authorization. A 403 is a permission denial, not an empty list. Never infer platform records by scanning the Runner filesystem."})
}

#[cfg(test)]
pub(crate) mod tests;
