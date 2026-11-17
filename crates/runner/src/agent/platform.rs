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
///
/// Requests are served concurrently: a `ferry_request` write can wait a long
/// time for the user's approval, and meanwhile the client's pings and other
/// calls must still be answered. A server that stops answering looks dead to
/// the client, which then fails the waiting call and lets the turn end with
/// the approval still undecided.
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
    serve(tokio::io::stdin(), tokio::io::stdout(), client, url, token).await
}

/// One MCP session over a byte stream; generic so tests can drive it in memory.
pub(super) async fn serve<R, W>(
    input: R,
    mut output: W,
    client: reqwest::Client,
    url: String,
    token: String,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    let (responses, mut outbox) = mpsc::channel::<Value>(64);
    let writer = tokio::spawn(async move {
        while let Some(response) = outbox.recv().await {
            if output
                .write_all(response.to_string().as_bytes())
                .await
                .is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });
    // Outstanding tool calls by JSON-RPC id, so a client cancellation stops
    // only that call (MCP: no response is sent for a cancelled request).
    let calls: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>> = Arc::default();
    let mut tasks = tokio::task::JoinSet::new();
    let mut input = BufReader::new(input);
    loop {
        let mut buffer = Vec::new();
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
        let method = request["method"].as_str().unwrap_or("").to_owned();
        if method == "notifications/cancelled" {
            let id = request["params"]["requestId"].to_string();
            if let Some(call) = calls.lock().unwrap().remove(&id) {
                call.abort();
            }
            continue;
        }
        if request.get("id").is_none() {
            continue;
        }
        let id = request["id"].clone();
        if method == "tools/call" {
            let (client, url, token, responses, calls_done) = (
                client.clone(),
                url.clone(),
                token.clone(),
                responses.clone(),
                calls.clone(),
            );
            let key = id.to_string();
            let params = request["params"].clone();
            let mut pending_calls = calls.lock().unwrap();
            let task = tasks.spawn(async move {
                let result = tool_call(&client, &url, &token, &params).await;
                calls_done.lock().unwrap().remove(&id.to_string());
                let _ = responses.send(reply(&id, result)).await;
            });
            pending_calls.insert(key, task);
            drop(pending_calls);
            // Finished tasks need no further attention.
            while tasks.try_join_next().is_some() {}
            continue;
        }
        let result = match method.as_str() {
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
            _ => Err("Unsupported MCP method".to_owned()),
        };
        if responses.send(reply(&id, result)).await.is_err() {
            break;
        }
    }
    // End of input: let outstanding calls answer, then close the writer.
    while tasks.join_next().await.is_some() {}
    drop(responses);
    let _ = writer.await;
    Ok(())
}

fn reply(id: &Value, result: std::result::Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(message) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":message}}),
    }
}

async fn tool_call(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    params: &Value,
) -> std::result::Result<Value, String> {
    match client.post(url).bearer_auth(token).json(params).send().await {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(value) => Ok(
                json!({"content":[{"type":"text","text":value.to_string()}],"isError":value["ok"]==false}),
            ),
            Err(_) => Err("The platform returned an invalid result.".into()),
        },
        Ok(response) => {
            let status = response.status();
            Err(format!("Platform execution bridge returned {status}; this call was not confirmed as successful."))
        }
        Err(_) => Err(
            "The execution bridge is unavailable; this call was not confirmed as successful. Do not replay a write automatically.".into(),
        ),
    }
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
    async fn platform_validation_errors_reach_the_cli_verbatim() {
        let (events, mut platform) = mpsc::channel(8);
        let bridge = Bridge::start(events).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = bridge.url.clone();
        let token = bridge.token.clone();
        let call = tokio::spawn(async move {
            tool_call(
                &client,
                &url,
                &token,
                &json!({"name":"ferry_request","arguments":{"method":"POST","path":"/workflows"}}),
            )
            .await
            .unwrap()
        });
        let Some(Event::PlatformCall { request_id, .. }) = platform.recv().await else {
            panic!("expected call")
        };
        let message = "$.nodes[2].parameters: missing required field model_id";
        bridge.resolve(
            &request_id,
            json!({"ok":false,"status":400,"data":{"error":message}}),
        );
        let response = call.await.unwrap();
        assert_eq!(response["isError"], true);
        let text: Value =
            serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["data"]["error"], message);
        assert_eq!(text["status"], 400);
    }

    /// Reads one JSON-RPC line with a deadline.
    async fn next_line<R: tokio::io::AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Value {
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            reader.read_line(&mut line),
        )
        .await
        .expect("a response in time")
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn pings_are_answered_while_a_write_waits_for_approval_and_cancelled_calls_stay_silent() {
        let (events, mut platform) = mpsc::channel(8);
        let bridge = Bridge::start(events).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (client_end, server_end) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_end);
        let session = tokio::spawn(serve(
            server_read,
            server_write,
            client,
            bridge.url.clone(),
            bridge.token.clone(),
        ));
        let (client_read, mut client_write) = tokio::io::split(client_end);
        let mut responses = BufReader::new(client_read);
        let mut send = |value: Value| {
            let line = format!("{value}\n");
            async move { line }
        };
        // A write that the platform holds for approval: no result yet.
        let call = send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ferry_request","arguments":{"method":"POST","path":"/workflows"}}})).await;
        client_write.write_all(call.as_bytes()).await.unwrap();
        let Some(Event::PlatformCall { request_id, .. }) = platform.recv().await else {
            panic!("platform call expected");
        };
        // The client keeps the session alive with pings; they are answered at once.
        let ping = send(json!({"jsonrpc":"2.0","id":8,"method":"ping"})).await;
        client_write.write_all(ping.as_bytes()).await.unwrap();
        assert_eq!(next_line(&mut responses).await["id"], 8);
        // The decision arrives later; the waiting call then completes.
        bridge.resolve(
            &request_id,
            json!({"status":201,"ok":true,"data":{"id":"42"}}),
        );
        let done = next_line(&mut responses).await;
        assert_eq!(done["id"], 7);
        assert_eq!(done["result"]["isError"], false);
        // A call the client cancels is never answered; later requests still are.
        let second = send(json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"ferry_request","arguments":{"method":"POST","path":"/workflows"}}})).await;
        client_write.write_all(second.as_bytes()).await.unwrap();
        assert!(matches!(
            platform.recv().await,
            Some(Event::PlatformCall { .. })
        ));
        let cancel = send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9}}),
        )
        .await;
        client_write.write_all(cancel.as_bytes()).await.unwrap();
        let list = send(json!({"jsonrpc":"2.0","id":10,"method":"tools/list"})).await;
        client_write.write_all(list.as_bytes()).await.unwrap();
        assert_eq!(next_line(&mut responses).await["id"], 10);
        client_write.shutdown().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), session)
            .await
            .expect("session ends with its input")
            .unwrap()
            .unwrap();
    }
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
