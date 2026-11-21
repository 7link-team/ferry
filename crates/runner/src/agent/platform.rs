//! A per-execution loopback bridge. Its capability never authenticates against the platform API.
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ferry_protocol::{
    agent::Event,
    resources::{ResourcePolicy, allows, byte_limit},
};
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
    policy: ResourcePolicy,
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
    #[cfg(test)]
    pub async fn start(events: mpsc::Sender<Event>) -> Result<Self> {
        Self::start_with_policy(events, ResourcePolicy::default()).await
    }
    pub async fn start_with_policy(
        events: mpsc::Sender<Event>,
        policy: ResourcePolicy,
    ) -> Result<Self> {
        let body_limit = policy.tool_request_bytes;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/call", listener.local_addr()?);
        let token = uuid::Uuid::new_v4().to_string();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let active = Arc::new(AtomicBool::new(true));
        let context = Context {
            token: token.clone(),
            policy,
            events,
            pending: pending.clone(),
            active: active.clone(),
        };
        let app = Router::new()
            .route("/call", post(call))
            .layer(match body_limit {
                Some(limit) => axum::extract::DefaultBodyLimit::max(byte_limit(Some(limit))),
                None => axum::extract::DefaultBodyLimit::disable(),
            })
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
        .filter(|v| ["ferry_discover", "ferry_request", "ferry_ask_user"].contains(v))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = context.pending.lock().unwrap();
        if !allows(
            context.policy.agent_tool_concurrency,
            pending.len().saturating_add(1),
        ) {
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
        let value = tokio::select! {
            value = rx => value.map_err(|_| StatusCode::GONE)?,
            _ = crate::transport::deadline(context.policy.task_timeout_seconds) => return Err(StatusCode::GATEWAY_TIMEOUT),
        };
        if !allows(context.policy.tool_response_bytes, serde_json::to_vec(&value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.len()) {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        Ok(Json(value))
    }
    .await;
    context.pending.lock().unwrap().remove(&id);
    result
}

pub(crate) fn tools() -> Value {
    ferry_protocol::agent::platform_tools()
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
    let policy = std::env::var("FERRY_RESOURCE_POLICY")
        .ok()
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    serve_with_policy(
        tokio::io::stdin(),
        tokio::io::stdout(),
        client,
        url,
        token,
        policy,
    )
    .await
}

/// One MCP session over a byte stream; generic so tests can drive it in memory.
#[cfg(test)]
pub(super) async fn serve<R, W>(
    input: R,
    output: W,
    client: reqwest::Client,
    url: String,
    token: String,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    serve_with_policy(input, output, client, url, token, ResourcePolicy::default()).await
}

async fn serve_with_policy<R, W>(
    input: R,
    mut output: W,
    client: reqwest::Client,
    url: String,
    token: String,
    policy: ResourcePolicy,
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
            .take(byte_limit(policy.tool_request_bytes).saturating_add(1) as u64)
            .read_until(b'\n', &mut buffer)
            .await?;
        if size == 0 {
            break;
        }
        ensure!(
            allows(policy.tool_request_bytes, buffer.len()) && buffer.last() == Some(&b'\n'),
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
            let policy = policy.clone();
            let key = id.to_string();
            let params = request["params"].clone();
            let mut pending_calls = calls.lock().unwrap();
            let task = tasks.spawn(async move {
                let result = tool_call(&client, &url, &token, &params, &policy).await;
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
    policy: &ResourcePolicy,
) -> std::result::Result<Value, String> {
    match client.post(url).bearer_auth(token).json(params).send().await {
        Ok(mut response) if response.status().is_success() => {
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| "The platform returned an invalid result.".to_owned())? {
                if !allows(policy.tool_response_bytes, bytes.len().saturating_add(chunk.len())) { return Err("The platform result exceeds the configured limit.".into()); }
                bytes.extend_from_slice(&chunk);
            }
            match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Ok(
                json!({"content":[{"type":"text","text":value.to_string()}],"isError":value["ok"]==false}),
            ),
            Err(_) => Err("The platform returned an invalid result.".into()),
            }
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
  for (const name of ['ferry_discover','ferry_request','ferry_ask_user']) {
    pi.registerTool({name,label:name,description:'Query or operate on the current platform workspace using its live role permissions. Never infer platform data from local files.',
      parameters:name==='ferry_discover'?Type.Object({search:Type.Optional(Type.String()),offset:Type.Optional(Type.Integer({minimum:0}))}):name==='ferry_ask_user'?Type.Object({title:Type.String(),question:Type.String(),reason:Type.Optional(Type.String()),options:Type.Optional(Type.Array(Type.Object({id:Type.String(),label:Type.String(),description:Type.Optional(Type.String())}))),recommended_option_id:Type.Optional(Type.String())}):Type.Object({method:Type.Union(['GET','POST','PUT','PATCH','DELETE'].map(value=>Type.Literal(value))),path:Type.String(),body:Type.Optional(Type.Any()),transfer:Type.Optional(Type.Object({data_base64:Type.String(),filename:Type.Optional(Type.String()),field:Type.Optional(Type.String())}))}),
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
    async fn bridge_defaults_allow_large_calls_and_explicit_body_limits_apply() {
        for limited in [false, true] {
            let (events, mut receiver) = mpsc::channel(1);
            let policy = ResourcePolicy {
                tool_request_bytes: limited.then_some(1024),
                ..Default::default()
            };
            let bridge = Bridge::start_with_policy(events, policy).await.unwrap();
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let (url, token) = (bridge.url.clone(), bridge.token.clone());
            let request = tokio::spawn(async move {
                client.post(url).bearer_auth(token).json(&json!({"name":"ferry_request", "arguments":{"data":"x".repeat(if limited { 2048 } else { 2 * 1024 * 1024 + 1 })}})).send().await.unwrap()
            });
            if limited {
                assert_eq!(
                    request.await.unwrap().status(),
                    StatusCode::PAYLOAD_TOO_LARGE
                );
                assert!(receiver.try_recv().is_err());
            } else {
                let Some(Event::PlatformCall {
                    request_id,
                    arguments,
                    ..
                }) = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
                    .await
                    .unwrap()
                else {
                    panic!("large call must reach platform")
                };
                assert!(arguments["data"].as_str().unwrap().len() > 2 * 1024 * 1024);
                bridge.resolve(
                    &request_id,
                    json!({"ok":true,"data":"x".repeat(2 * 1024 * 1024 + 1)}),
                );
                let response = request.await.unwrap().json::<Value>().await.unwrap();
                assert!(response["data"].as_str().unwrap().len() > 2 * 1024 * 1024);
            }
        }
    }

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
                &ResourcePolicy::default(),
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
