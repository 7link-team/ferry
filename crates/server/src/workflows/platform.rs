//! Read-only platform data (`ferry.request`). Each request reuses the in-process
//! platform tool gateway under the execution identity, rechecked immediately
//! before it is sent. The workspace always comes from the execution; graph
//! parameters only fill the path placeholders and query of a catalogue GET.
use crate::{
    entities::workflow_executions,
    error::{ApiError, Result},
    i18n::Message,
    platform_tools::{self, Call},
    state::AppState,
};
use axum::http::StatusCode;
use ferry_protocol::sandbox::Item;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};

const MAX_PAGES: u64 = 50;
const MAX_PATH_BYTES: usize = 4096;
const MAX_MESSAGE_CHARS: usize = 500;

pub(super) fn supports(spec: &Value) -> bool {
    spec["type"] == "ferry.request"
}

fn invalid(key: &'static str, path: String) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new(key).with("path", path),
    )
}

pub(super) fn too_large() -> ApiError {
    ApiError::localized(
        StatusCode::PAYLOAD_TOO_LARGE,
        Message::new("workflow.platformTooLarge"),
    )
}

/// One URL segment: never empty, a separator or a relative step.
fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.contains(['/', '\\'])
        && !value.contains("..")
        && !value.chars().any(char::is_control)
}

fn placeholders(operation: &str) -> impl Iterator<Item = &str> {
    operation
        .split('/')
        .filter_map(|part| part.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
}

/// Save-time and resolved-parameter checks. Expression values are checked
/// again as literals by `path` immediately before the request.
pub(super) fn validate(params: &Value, base: &str) -> Result<()> {
    let operation = params["operation"].as_str().unwrap_or_default();
    if !platform_tools::read_operation(operation) {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            Message::new("workflow.platformOperation")
                .with("path", format!("{base}/params/operation"))
                .with("operation", operation.chars().take(512).collect::<String>()),
        ));
    }
    let values = params["path_params"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let names: Vec<_> = placeholders(operation).collect();
    if let Some(name) = values.keys().find(|key| !names.contains(&key.as_str())) {
        return Err(path_param(base, name));
    }
    for name in names {
        let valid = values
            .get(name)
            .and_then(Value::as_str)
            .is_some_and(|value| {
                // Expressions are resolved per item before execution.
                value.starts_with('=') || segment(value)
            });
        if !valid {
            return Err(path_param(base, name));
        }
    }
    Ok(())
}

fn path_param(base: &str, name: &str) -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.platformPathParam")
            .with(
                "path",
                format!(
                    "{base}/params/path_params/{}",
                    name.replace('~', "~0").replace('/', "~1")
                ),
            )
            .with("name", name.chars().take(128).collect::<String>()),
    )
}

/// Fill a catalogue template; every value is encoded as exactly one segment.
pub(super) fn path(operation: &str, values: &Value) -> Result<String> {
    operation
        .split('/')
        .map(
            |part| match part.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(name) => values[name]
                    .as_str()
                    .filter(|value| segment(value))
                    .map(|value| utf8_percent_encode(value, NON_ALPHANUMERIC).to_string())
                    .ok_or_else(|| path_param("", name)),
                None => Ok(part.to_owned()),
            },
        )
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

pub(super) fn url(path: &str, query: &Map<String, Value>) -> Result<String> {
    let encode = |text: &str| utf8_percent_encode(text, NON_ALPHANUMERIC).to_string();
    let pairs = query
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(text) => text.clone(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                _ => return Err(invalid("workflow.platformQuery", "/params/query".into())),
            };
            if key.is_empty() || key.chars().any(char::is_control) {
                return Err(invalid("workflow.platformQuery", "/params/query".into()));
            }
            Ok(format!("{}={}", encode(key), encode(&value)))
        })
        .collect::<Result<Vec<_>>>()?;
    let url = if pairs.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", pairs.join("&"))
    };
    if url.len() > MAX_PATH_BYTES {
        return Err(invalid("workflow.platformQuery", "/params/query".into()));
    }
    Ok(url)
}

/// Gateway results become data or a localized failure; nothing is reported as success
/// unless the original handler returned 2xx.
pub(super) fn interpret(response: Value) -> Result<Value> {
    let status = response["status"].as_u64().unwrap_or(500);
    if status == 413 {
        return Err(too_large());
    }
    if response["ok"] == true && (200..300).contains(&status) {
        return Ok(response["data"].clone());
    }
    let data = &response["data"];
    let message = data["error"]
        .as_str()
        .or_else(|| data["message"].as_str())
        .or_else(|| data["text"].as_str())
        .unwrap_or_default()
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect::<String>();
    let code = u16::try_from(status)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .filter(|code| code.is_client_error() || code.is_server_error())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    Err(ApiError::localized(
        code,
        Message::new("workflow.platformRequest")
            .with("status", status.to_string())
            .with("message", message),
    ))
}

/// Arrays and `{items:[...]}` lists become one item per element; anything else one item.
pub(super) fn values(data: Value, split: bool) -> Vec<Map<String, Value>> {
    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(object) => object,
            value => Map::from_iter([("value".to_owned(), value)]),
        }
    }
    if split {
        if let Value::Array(rows) = data {
            return rows.into_iter().map(object).collect();
        }
        if let Some(Value::Array(rows)) = data.get("items") {
            return rows.iter().cloned().map(object).collect();
        }
    }
    vec![object(data)]
}

async fn request(s: &AppState, run: &workflow_executions::Model, url: String) -> Result<Value> {
    let mut scope = super::integration::execution_scope(s, run).await?;
    // A platform administrator's global role is never delegated into workflow reads.
    scope.user.is_superadmin = false;
    let response = platform_tools::dispatch(
        s,
        scope,
        Call {
            method: "GET".into(),
            path: url,
            ..Default::default()
        },
    )
    .await?;
    interpret(response)
}

/// Read `page` = 1.. using the API's `items`/`total` envelope. Needing more than
/// `max_pages` fails instead of returning a truncated list.
async fn pages(
    s: &AppState,
    run: &workflow_executions::Model,
    path: &str,
    query: &Map<String, Value>,
    max_pages: u64,
) -> Result<Value> {
    let mut rows = Vec::new();
    for page in 1..=max_pages {
        let mut query = query.clone();
        query.insert("page".into(), json!(page));
        let data = request(s, run, url(path, &query)?).await?;
        let (Some(items), Some(total)) = (data["items"].as_array(), data["total"].as_u64()) else {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new("workflow.platformPagination"),
            ));
        };
        let done = items.is_empty() || rows.len() + items.len() >= total as usize;
        rows.extend(items.iter().cloned());
        if done {
            return Ok(json!({"items":rows,"total":total}));
        }
    }
    Err(ApiError::localized(
        StatusCode::BAD_REQUEST,
        Message::new("workflow.platformPageLimit").with("max_pages", max_pages.to_string()),
    ))
}

pub(super) async fn execute(
    s: &AppState,
    run: &workflow_executions::Model,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    if items.len() != params.len() {
        return Err(invalid("workflow.graphSchema", "/params".into()));
    }
    let mut output = Vec::new();
    let mut bytes = 0;
    for (item, params) in items.iter().zip(params) {
        // The catalogue may have changed since the definition was saved.
        validate(params, "")?;
        let path = path(
            params["operation"].as_str().unwrap_or_default(),
            &params["path_params"],
        )?;
        let query = params["query"].as_object().cloned().unwrap_or_default();
        let data = if params["all_pages"] == true {
            let max_pages = super::graph::port_index(&params["max_pages"])
                .filter(|pages| (1..=MAX_PAGES).contains(pages))
                .ok_or_else(|| invalid("workflow.graphSchema", "/params/max_pages".into()))?;
            pages(s, run, &path, &query, max_pages).await?
        } else {
            request(s, run, url(&path, &query)?).await?
        };
        for json in values(data, params["split_items"] != false) {
            let mut result = item.clone();
            result.json = json;
            super::transform::append(&mut output, &mut bytes, result).map_err(|_| too_large())?;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
