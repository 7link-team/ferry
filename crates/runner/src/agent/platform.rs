//! A per-execution loopback bridge. Its capability never authenticates against the platform API.
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ferry_protocol::agent::Event;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;
struct PendingCall {
    id: String,
    pending: Pending,
}
impl Drop for PendingCall {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
#[derive(Clone)]
struct Context {
    token: String,
    events: mpsc::Sender<Event>,
    pending: Pending,
    active: Arc<AtomicBool>,
}
pub(super) struct Bridge {
    pub url: String,
    pub token: String,
    pending: Pending,
    active: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Bridge {
    pub async fn start(events: mpsc::Sender<Event>) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/call", listener.local_addr()?);
        let token = uuid::Uuid::new_v4().to_string();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let active = Arc::new(AtomicBool::new(true));
        let context = Context {
            token: token.clone(),
            events,
            pending: pending.clone(),
            active: active.clone(),
        };
        let app = Router::new()
            .route("/call", post(call))
            .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
            .with_state(context);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            url,
            token,
            pending,
            active,
            task,
        })
    }
    pub fn resolve(&self, id: &str, result: Value) {
        if let Some(sender) = self.pending.lock().unwrap().remove(id) {
            let _ = sender.send(result);
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
        self.task.abort();
    }
}
async fn call(
    State(context): State<Context>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> std::result::Result<Json<Value>, StatusCode> {
    if !context.active.load(Ordering::SeqCst)
        || headers.get("authorization").and_then(|v| v.to_str().ok())
            != Some(&format!("Bearer {}", context.token))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let name = value["name"]
        .as_str()
        .filter(|v| ["ferry_discover", "ferry_request"].contains(v))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = context.pending.lock().unwrap();
        if pending.len() >= 16 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        pending.insert(id.clone(), tx);
    }
    let _pending = PendingCall {
        id: id.clone(),
        pending: context.pending.clone(),
    };
    let event = Event::PlatformCall {
        request_id: id.clone(),
        name: name.into(),
        arguments: value["arguments"].clone(),
    };
    let result = async {
        context
            .events
            .send(event)
            .await
            .map_err(|_| StatusCode::GONE)?;
        tokio::time::timeout(std::time::Duration::from_secs(3600), rx)
            .await
            .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
            .map(Json)
            .map_err(|_| StatusCode::GONE)
    }
    .await;
    context.pending.lock().unwrap().remove(&id);
    result
}

pub(crate) fn tools() -> Value {
    json!([
      {
        "name": "ferry_discover",
        "description": "Discover this workspace's actual platform API operations and the current user/Agent permissions. Use before platform questions about hosts, repositories, deployments, workflows or other business data. Never substitute local files for platform records.",
        "inputSchema": {
          "type": "object",
          "properties": {
            "offset": {
              "type": "integer",
              "minimum": 0,
              "description": "Pagination offset from next_offset."
            },
            "search": {
              "type": "string",
              "description": "Optional API resource or operation filter, e.g. nodes, code-connections, deployments, pipelines, workflows."
            }
          },
          "additionalProperties": false
        }
      },
      {
        "name": "ferry_request",
        "description": "Invoke an existing platform API in this conversation's workspace. Every call is reauthorized using the user's and Agent role's current permissions. GET lists return total counts; writes wait for a user confirmation. Use ferry_discover for method/path. Do not invent data when an API fails.",
        "inputSchema": {
          "type": "object",
          "properties": {
            "method": {
              "type": "string",
              "enum": [
                "GET",
                "POST",
                "PUT",
                "PATCH",
                "DELETE"
              ]
            },
            "path": {
              "type": "string",
              "description": "Workspace-relative path including query, e.g. /pipelines?page_size=1 or /nodes/123. Never include /api/workspaces or another workspace ID."
            },
            "body": {
              "description": "JSON request body defined by the existing API."
            },
            "transfer": {
              "type": "object",
              "description": "Binary upload instead of body. Up to 1 MiB base64. Raw file upload: data_base64. Multipart upload: also set filename and optional field (default file). Downloads return data_base64 or text. Responses are limited to 256 KiB.",
              "properties": {
                "data_base64": {
                  "type": "string"
                },
                "filename": {
                  "type": "string"
                },
                "field": {
                  "type": "string"
                }
              },
              "required": [
                "data_base64"
              ],
              "additionalProperties": false
            }
          },
          "required": [
            "method",
            "path"
          ],
          "additionalProperties": false
        }
      }
    ])
}

/// Internal stdio MCP command used by native CLI clients. Logs belong on stderr only.
pub(crate) async fn stdio() -> Result<()> {
    let url = std::env::var("FERRY_PLATFORM_BRIDGE_URL")?;
    let token = std::env::var("FERRY_PLATFORM_BRIDGE_TOKEN")?;
    let parsed = reqwest::Url::parse(&url)?;
    ensure!(
        parsed.scheme() == "http"
            && parsed.host_str() == Some("127.0.0.1")
            && parsed.path() == "/call"
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "invalid execution bridge"
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    loop {
        let mut buffer = Vec::new();
        use tokio::io::AsyncReadExt;
        let size = (&mut input)
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut buffer)
            .await?;
        if size == 0 {
            break;
        }
        ensure!(
            buffer.len() <= 1024 * 1024 && buffer.last() == Some(&b'\n'),
            "MCP input exceeds limit"
        );
        let request: Value = serde_json::from_slice(&buffer)?;
        if request.get("id").is_none() {
            continue;
        }
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => {
                let requested = request["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = if ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2024-11-05"
                };
                Ok(
                    json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"ferry","version":env!("CARGO_PKG_VERSION")}}),
                )
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools":tools()})),
            "tools/call" => match client
                .post(&url)
                .bearer_auth(&token)
                .json(&request["params"])
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    match response.json::<Value>().await {
                        Ok(value) => Ok(
                            json!({"content":[{"type":"text","text":value.to_string()}],"isError":value["ok"]==false}),
                        ),
                        Err(_) => Err("The platform returned an invalid result."),
                    }
                }
                _ => Err(
                    "The execution bridge is unavailable; this call was not confirmed as successful. Do not replay a write automatically.",
                ),
            },
            _ => Err("Unsupported MCP method"),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            Err(message) => {
                json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":message}})
            }
        };
        output.write_all(response.to_string().as_bytes()).await?;
        output.write_all(b"\n").await?;
        output.flush().await?;
    }
    Ok(())
}

// Pi exposes tools through its extension API, rather than a native MCP client.
// This adapter only transports calls; it contains no platform business logic or credentials.
pub(super) const PI_EXTENSION: &str = r#"
import { Type } from '@sinclair/typebox';
export default function(pi) {
  for (const name of ['ferry_discover','ferry_request']) {
    pi.registerTool({name,label:name,description:'Query or operate on the current platform workspace using its live role permissions. Never infer platform data from local files.',
      parameters:name==='ferry_discover'?Type.Object({search:Type.Optional(Type.String()),offset:Type.Optional(Type.Integer({minimum:0}))}):Type.Object({method:Type.Union(['GET','POST','PUT','PATCH','DELETE'].map(value=>Type.Literal(value))),path:Type.String(),body:Type.Optional(Type.Any()),transfer:Type.Optional(Type.Object({data_base64:Type.String(),filename:Type.Optional(Type.String()),field:Type.Optional(Type.String())}))}),
      async execute(_id,args,signal) {
        const response=await fetch(process.env.FERRY_PLATFORM_BRIDGE_URL,{method:'POST',headers:{'content-type':'application/json',authorization:'Bearer '+process.env.FERRY_PLATFORM_BRIDGE_TOKEN},body:JSON.stringify({name,arguments:args}),signal});
        if (!response.ok) throw new Error('Platform execution bridge failed: '+response.status);
        return {content:[{type:'text',text:JSON.stringify(await response.json())}],details:{}};
      }
    });
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bridge_authentication_result_binding_and_shutdown() {
        let (tx, mut rx) = mpsc::channel(4);
        let bridge = Bridge::start(tx).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            client
                .post(&bridge.url)
                .json(&json!({"name":"ferry_discover","arguments":{}}))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let url = bridge.url.clone();
        let token = bridge.token.clone();
        let task = tokio::spawn(async move {
            client.post(url).bearer_auth(token).json(&json!({"name":"ferry_request","arguments":{"method":"GET","path":"/pipelines"}})).send().await.unwrap().json::<Value>().await.unwrap()
        });
        let Event::PlatformCall {
            request_id,
            name,
            arguments,
        } = rx.recv().await.unwrap()
        else {
            panic!("expected platform call")
        };
        assert_eq!(name, "ferry_request");
        assert_eq!(arguments["path"], "/pipelines");
        bridge.resolve("another-call", json!({"total":999}));
        assert!(!task.is_finished());
        bridge.resolve(&request_id, json!({"total":2}));
        assert_eq!(task.await.unwrap()["total"], 2);
        let url = bridge.url.clone();
        drop(bridge);
        tokio::task::yield_now().await;
        assert!(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(url)
                .send()
                .await
                .is_err()
        );
    }
}
