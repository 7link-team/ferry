//! Native wire adaptation, independent of sockets and process ownership.
//! Codex: https://developers.openai.com/codex/app-server/
use anyhow::{Result, bail, ensure};
use ferry_protocol::agent::{Event, Input, Outcome, Permission, Request, Runtime, Usage};
use serde_json::{Value, json};
use std::collections::HashMap;
mod acp;
mod pi;

#[derive(Default)]
pub(super) struct Step {
    pub writes: Vec<Value>,
    pub events: Vec<Event>,
    pub done: Option<Outcome>,
}

struct Pending {
    native_id: Value,
    input: Value,
}

pub(super) struct Protocol {
    pub(super) mcp_server: Option<Value>,
    request: Request,
    session: Option<String>,
    turn: Option<String>,
    ready: bool,
    pending: HashMap<String, Pending>,
    streamed: HashMap<String, String>,
    usage: Option<Usage>,
    /// A new invocation gets a distinct token namespace, even if native ids repeat.
    scope: String,
    next_request: u64,
    acp_tools: HashMap<String, String>,
    acp_model: Option<String>,
    dsh_policy_sessions: std::collections::HashSet<String>,
    dsh_outcome: Option<Outcome>,
}

impl Protocol {
    pub fn new(request: Request, scope: String) -> Self {
        Self {
            mcp_server: None,
            request,
            session: None,
            turn: None,
            ready: false,
            pending: HashMap::new(),
            streamed: HashMap::new(),
            usage: None,
            scope,
            next_request: 0,
            acp_tools: HashMap::new(),
            acp_model: None,
            dsh_policy_sessions: std::collections::HashSet::new(),
            dsh_outcome: None,
        }
    }

    pub fn initial(&self) -> Value {
        match self.request.runtime {
            Runtime::Codex => json!({"id":0,"method":"initialize","params":{
                "clientInfo":{"name":"agent_runtime","version":env!("CARGO_PKG_VERSION")}}}),
            Runtime::Claude => json!({"type":"control_request","request_id":"initialize",
                "request":{"subtype":"initialize","hooks":{}}}),
            Runtime::Gemini | Runtime::Qwen | Runtime::Opencode | Runtime::Dsh => {
                json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
                "protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},
                "clientInfo":{"name":"agent_runtime","version":env!("CARGO_PKG_VERSION")}}})
            }
            Runtime::Pi => json!({"id":"initialize","type":"get_state"}),
        }
    }

    pub fn usage(&self) -> Option<Usage> {
        self.usage.clone()
    }

    pub fn receive(&mut self, value: Value) -> Result<Step> {
        match self.request.runtime {
            Runtime::Codex => self.codex(value),
            Runtime::Claude => self.claude(value),
            Runtime::Gemini | Runtime::Qwen | Runtime::Opencode | Runtime::Dsh => self.acp(value),
            Runtime::Pi => self.pi(value),
        }
    }

    pub fn input(&mut self, input: Input) -> Result<Step> {
        let Input::Decision { request_id, allow } = input else {
            bail!("cancel is handled by the process owner")
        };
        let pending = self
            .pending
            .remove(&request_id)
            .ok_or_else(|| anyhow::anyhow!("unknown or expired approval request"))?;
        let response = match self.request.runtime {
            Runtime::Codex => {
                json!({"id":pending.native_id,"result":{"decision":if allow {"accept"} else {"decline"}}})
            }
            Runtime::Claude => json!({"type":"control_response","response":{"subtype":"success",
                "request_id":pending.native_id,"response":if allow {
                    json!({"behavior":"allow","updatedInput":pending.input})
                } else { json!({"behavior":"deny","message":"The user declined this tool request."}) }}}),
            Runtime::Gemini | Runtime::Qwen | Runtime::Opencode | Runtime::Dsh => {
                let outcome = if allow {
                    json!({"outcome":"selected","optionId":pending.input["allow_once"]})
                } else {
                    json!({"outcome":"cancelled"})
                };
                json!({"jsonrpc":"2.0","id":pending.native_id,"result":{"outcome":outcome}})
            }
            Runtime::Pi => bail!("Pi does not support tool approvals"),
        };
        Ok(Step {
            writes: vec![response],
            events: vec![Event::DecisionSent {
                request_id,
                allowed: allow,
            }],
            ..Step::default()
        })
    }

    fn approval(&mut self, native_id: Value, tool: &str, input: Value) -> Result<Step> {
        ensure!(
            self.request.permission == Permission::OnRequest,
            "runtime requested tools in read-only mode"
        );
        ensure!(self.pending.len() < 16, "too many pending approvals");
        ensure!(
            !self.pending.values().any(|p| p.native_id == native_id),
            "duplicate native approval id"
        );
        self.next_request += 1;
        let request_id = format!("{}:{}", self.scope, self.next_request);
        self.pending.insert(
            request_id.clone(),
            Pending {
                native_id,
                input: input.clone(),
            },
        );
        Ok(Step {
            events: vec![Event::Approval {
                request_id,
                tool: tool.into(),
                input,
            }],
            ..Step::default()
        })
    }

    fn bind(&mut self, id: &str) -> Result<Event> {
        ensure!(
            !id.is_empty() && id.len() <= 256,
            "missing native session id"
        );
        if let Some(expected) = &self.request.native_session_id {
            ensure!(expected == id, "runtime resumed a different native session");
        }
        if let Some(expected) = &self.session {
            ensure!(expected == id, "runtime changed native session");
        }
        self.session = Some(id.into());
        Ok(Event::Session {
            native_session_id: id.into(),
        })
    }

    fn text(&mut self, id: &str, text: &str, replace: bool) -> Event {
        Event::Text {
            item_id: id.into(),
            text: text.into(),
            replace,
        }
    }

    fn codex(&mut self, value: Value) -> Result<Step> {
        if value.get("method").is_none() {
            ensure!(
                value.get("error").is_none(),
                "Codex rejected a protocol request"
            );
            let result = &value["result"];
            return match value["id"].as_u64() {
                Some(0) if !self.ready => {
                    self.ready = true;
                    let mut params = json!({"cwd":self.request.cwd,"approvalPolicy":if self.request.permission==Permission::ReadOnly {"never"} else {"untrusted"},
                        "sandbox":if self.request.permission==Permission::ReadOnly {"read-only"} else {"workspace-write"}});
                    if let Some(model) = &self.request.model {
                        params["model"] = json!(model);
                    }
                    let method = if let Some(id) = &self.request.native_session_id {
                        params["threadId"] = json!(id);
                        "thread/resume"
                    } else {
                        "thread/start"
                    };
                    Ok(Step {
                        writes: vec![
                            json!({"method":"initialized"}),
                            json!({"id":1,"method":method,"params":params}),
                        ],
                        ..Step::default()
                    })
                }
                Some(1) if self.ready && self.session.is_none() => {
                    let event = self.bind(result["thread"]["id"].as_str().unwrap_or(""))?;
                    let sandbox = if self.request.permission == Permission::ReadOnly {
                        json!({"type":"readOnly"})
                    } else {
                        json!({"type":"workspaceWrite","writableRoots":[self.request.cwd],"networkAccess":false})
                    };
                    Ok(Step {
                        writes: vec![json!({"id":2,"method":"turn/start","params":{
                        "threadId":self.session,"input":[{"type":"text","text":self.request.prompt}],
                        "sandboxPolicy":sandbox}})],
                        events: vec![event],
                        ..Step::default()
                    })
                }
                Some(2) if self.session.is_some() => {
                    let id = result["turn"]["id"].as_str().unwrap_or("");
                    ensure!(!id.is_empty(), "missing native turn id");
                    if let Some(expected) = &self.turn {
                        ensure!(expected == id, "runtime changed native turn");
                    }
                    self.turn = Some(id.into());
                    Ok(Step::default())
                }
                _ => bail!("unexpected Codex response"),
            };
        }
        let method = value["method"].as_str().unwrap_or("");
        let params = &value["params"];
        // Only turn events are projected. Global/account/other-thread events are ignored.
        let relevant = method.starts_with("turn/")
            || method.starts_with("item/")
            || method == "thread/tokenUsage/updated";
        if relevant {
            ensure!(
                self.session
                    .as_deref()
                    .is_some_and(|id| params["threadId"].as_str() == Some(id)),
                "event belongs to a different native session"
            );
            if let Some(id) = params["turnId"].as_str() {
                // Resuming emits the previous turn's usage before the new turn
                // starts. It is historical telemetry, not this request's usage.
                if method == "thread/tokenUsage/updated" && self.turn.as_deref() != Some(id) {
                    return Ok(Step::default());
                }
                ensure!(
                    self.turn.as_deref() == Some(id),
                    "event {method} belongs to native turn {id}, expected {:?}",
                    self.turn
                );
            }
        }
        if let Some(id) = value.get("id") {
            ensure!(id.is_string() || id.is_i64(), "invalid native request id");
            ensure!(self.turn.is_some(), "approval arrived before turn start");
            ensure!(
                matches!(
                    method,
                    "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
                ),
                "unsupported native input or permission request"
            );
            if let Some(decisions) = params["availableDecisions"].as_array() {
                ensure!(
                    decisions.iter().any(|v| v.as_str() == Some("accept")),
                    "runtime cannot grant this item only"
                );
            }
            return self.approval(id.clone(), method, params.clone());
        }
        let mut step = Step::default();
        match method {
            "turn/started" => {
                let id = params["turn"]["id"].as_str().unwrap_or("");
                ensure!(!id.is_empty(), "missing native turn id");
                if let Some(expected) = &self.turn {
                    ensure!(expected == id, "runtime changed native turn");
                }
                self.turn = Some(id.into());
            }
            "item/agentMessage/delta" => {
                let id = params["itemId"].as_str().unwrap_or("");
                let delta = params["delta"].as_str().unwrap_or("");
                ensure!(!id.is_empty(), "missing native item id");
                self.streamed.entry(id.into()).or_default().push_str(delta);
                step.events.push(self.text(id, delta, false));
            }
            "item/started" | "item/completed" => {
                let item = &params["item"];
                let id = item["id"].as_str().unwrap_or("");
                if item["type"] == "agentMessage" && method == "item/completed" {
                    let text = item["text"].as_str().unwrap_or("");
                    if self.streamed.remove(id).as_deref() != Some(text) {
                        step.events.push(self.text(id, text, true));
                    }
                } else if matches!(
                    item["type"].as_str(),
                    Some("commandExecution" | "fileChange" | "mcpToolCall")
                ) {
                    step.events.push(Event::Tool {
                        item_id: id.into(),
                        name: item["type"].as_str().unwrap().into(),
                        state: if method == "item/started" {
                            "running"
                        } else {
                            "finished"
                        }
                        .into(),
                    });
                }
            }
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"]["last"];
                self.usage = usage["inputTokens"]
                    .as_u64()
                    .zip(usage["outputTokens"].as_u64())
                    .map(|(input_tokens, output_tokens)| Usage {
                        input_tokens,
                        output_tokens,
                    });
            }
            "turn/completed" => {
                ensure!(
                    self.turn.as_deref() == params["turn"]["id"].as_str(),
                    "completion belongs to a different turn"
                );
                ensure!(
                    self.pending.is_empty(),
                    "runtime completed with unresolved approvals"
                );
                step.done = Some(match params["turn"]["status"].as_str() {
                    Some("completed") => Outcome::Completed,
                    Some("interrupted") => Outcome::Cancelled,
                    _ => Outcome::Failed,
                });
            }
            "error" if params["willRetry"] != true => step.done = Some(Outcome::Failed),
            _ => {}
        }
        Ok(step)
    }

    fn claude(&mut self, value: Value) -> Result<Step> {
        let mut step = Step::default();
        if let Some(id) = value["session_id"].as_str().filter(|id| !id.is_empty()) {
            let first = self.session.is_none();
            let event = self.bind(id)?;
            if first {
                step.events.push(event);
            }
        }
        match value["type"].as_str() {
            Some("control_response") => {
                ensure!(
                    !self.ready
                        && value["response"]["request_id"] == "initialize"
                        && value["response"]["subtype"] == "success",
                    "Claude initialization failed"
                );
                self.ready = true;
                step.writes.push(json!({"type":"user","message":{"role":"user","content":self.request.prompt},"parent_tool_use_id":null,"session_id":self.request.native_session_id.clone().unwrap_or_default()}));
            }
            Some("control_request") => {
                let request = &value["request"];
                ensure!(
                    self.ready && self.session.is_some() && request["subtype"] == "can_use_tool",
                    "unsupported Claude control request"
                );
                let id = value["request_id"].as_str().unwrap_or("");
                let tool = request["tool_name"].as_str().unwrap_or("");
                ensure!(
                    !id.is_empty() && !tool.is_empty() && request["input"].is_object(),
                    "invalid Claude tool request"
                );
                // AskUserQuestion needs a structured answer, not a boolean permission.
                ensure!(
                    tool != "AskUserQuestion",
                    "structured user input is not supported yet"
                );
                return self.approval(json!(id), tool, request["input"].clone());
            }
            Some("assistant") => {
                ensure!(
                    self.ready && self.session.is_some(),
                    "assistant output before initialization"
                );
                if let Some(content) = value["message"]["content"].as_array() {
                    let id = value["message"]["id"].as_str().unwrap_or("assistant");
                    let text = content
                        .iter()
                        .filter(|v| v["type"] == "text")
                        .filter_map(|v| v["text"].as_str())
                        .collect::<String>();
                    if !text.is_empty() {
                        step.events.push(self.text(id, &text, true));
                    }
                    for item in content.iter().filter(|v| v["type"] == "tool_use") {
                        step.events.push(Event::Tool {
                            item_id: item["id"].as_str().unwrap_or("").into(),
                            name: item["name"].as_str().unwrap_or("").into(),
                            state: "running".into(),
                        });
                    }
                }
            }
            Some("result") => {
                ensure!(
                    self.ready && self.session.is_some() && self.pending.is_empty(),
                    "Claude completed without valid session or with unresolved approval"
                );
                self.usage = value["usage"]["input_tokens"]
                    .as_u64()
                    .zip(value["usage"]["output_tokens"].as_u64())
                    .map(|(input_tokens, output_tokens)| Usage {
                        input_tokens,
                        output_tokens,
                    });
                step.done = Some(
                    if value["subtype"] == "success" && value["is_error"] == false {
                        Outcome::Completed
                    } else {
                        Outcome::Failed
                    },
                );
            }
            _ => {}
        }
        Ok(step)
    }
}
