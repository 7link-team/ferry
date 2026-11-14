//! ACP v1 transport, including DSH's resume/config-option variant.
//! https://agentclientprotocol.com/protocol/overview
use super::*;
use anyhow::Context;

impl Protocol {
    pub(super) fn acp(&mut self, value: Value) -> Result<Step> {
        let mut step = Step::default();
        let params = &value["params"];
        if let Some(method) = value["method"].as_str() {
            if self.request.runtime == Runtime::Dsh && method == "agent_runtime/permissions" {
                ensure!(params["preset"] == "read-only", "unsafe DSH permissions");
                let id = params["sessionId"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .context("missing DSH policy session")?;
                if !self.ready {
                    ensure!(
                        self.dsh_policy_sessions.len() < 256,
                        "too many DSH policy sessions"
                    );
                    self.dsh_policy_sessions.insert(id.to_owned());
                }
                return Ok(step);
            }
            if self.request.runtime == Runtime::Dsh && method == "agent_runtime/turn_end" {
                // The policy plugin observes descendant Agents too. Their endings
                // must neither complete nor fail the admitted root conversation.
                if self.turn.is_none() || params["sessionId"].as_str() != self.session.as_deref() {
                    return Ok(step);
                }
                ensure!(self.dsh_outcome.is_none(), "duplicate DSH ending");
                self.dsh_outcome = Some(match params["reason"].as_str() {
                    Some("completed") => Outcome::Completed,
                    Some("interrupted") => Outcome::Cancelled,
                    _ => Outcome::Failed,
                });
                return Ok(step);
            }
            // ACP reserves "_" methods for vendor extensions (Grok's _x.ai/*).
            // Notifications carry no turn data; requests get "method not found".
            if method.starts_with('_') {
                if !value["id"].is_null() {
                    step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],
                        "error":{"code":-32601,"message":"Method not found"}}));
                }
                return Ok(step);
            }
            // load may replay native history before its response. It is already
            // durable in Ferry and must not become a new assistant reply.
            if method == "session/update" && self.turn.is_none() {
                return Ok(step);
            }
            ensure!(
                self.ready && self.session.is_some(),
                "ACP event before session admission"
            );
            ensure!(
                params["sessionId"].as_str() == self.session.as_deref(),
                "ACP session mismatch"
            );
            match method {
                "session/request_permission" => {
                    ensure!(!value["id"].is_null(), "ACP permission id missing");
                    // DSH sends only toolCallId here. The preceding tool_call
                    // update supplies its name; never infer it from raw inputs.
                    let tool = params["toolCall"]["title"]
                        .as_str()
                        .or_else(|| {
                            params["toolCall"]["toolCallId"]
                                .as_str()
                                .and_then(|id| self.acp_tools.get(id).map(String::as_str))
                        })
                        .unwrap_or("tool")
                        .to_owned();
                    let platform = self.request.platform_tools && {
                        [
                            "ferry_discover",
                            "ferry_request",
                            "mcp__ferry__ferry_discover",
                            "mcp__ferry__ferry_request",
                        ]
                        .contains(&tool.as_str())
                    };
                    if platform {
                        // Approve only this transport call. The server independently authorizes
                        // its operation and obtains the user's confirmation before any write.
                        let once = params["options"]
                            .as_array()
                            .and_then(|options| options.iter().find(|o| o["kind"] == "allow_once"))
                            .and_then(|o| o["optionId"].as_str())
                            .ok_or_else(|| anyhow::anyhow!("ACP requires per-call approval"))?;
                        step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"result":{"outcome":{"outcome":"selected","optionId":once}}}));
                        return Ok(step);
                    }
                    if self.request.permission == Permission::ReadOnly {
                        step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"result":{"outcome":{"outcome":"cancelled"}}}));
                    } else {
                        let once = params["options"]
                            .as_array()
                            .and_then(|options| {
                                options.iter().find(|option| option["kind"] == "allow_once")
                            })
                            .and_then(|option| option["optionId"].as_str())
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| anyhow::anyhow!("ACP requires per-call approval"))?;
                        let input = json!({"tool_call":params["toolCall"],"allow_once":once});
                        return self.approval(value["id"].clone(), &tool, input);
                    }
                }
                "session/update" => {
                    let update = &params["update"];
                    match update["sessionUpdate"].as_str() {
                        Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                            ensure!(self.turn.is_some(), "ACP output before prompt");
                            step.events.push(self.text(
                                "assistant",
                                update["content"]["text"].as_str().unwrap_or(""),
                                false,
                            ));
                        }
                        Some("tool_call" | "tool_call_update") => {
                            if self.request.runtime == Runtime::Dsh {
                                if let Some(id) = update["toolCallId"].as_str() {
                                    if matches!(
                                        update["status"].as_str(),
                                        Some("completed" | "failed")
                                    ) {
                                        self.acp_tools.remove(id);
                                    } else if let Some(title) = update["title"].as_str() {
                                        ensure!(
                                            self.acp_tools.len() < 256
                                                || self.acp_tools.contains_key(id),
                                            "too many active ACP tools"
                                        );
                                        self.acp_tools.insert(id.to_owned(), title.to_owned());
                                    }
                                }
                            }
                            step.events.push(Event::Tool {
                                item_id: update["toolCallId"].as_str().unwrap_or("").into(),
                                name: update["title"].as_str().unwrap_or("tool").into(),
                                state: match update["status"].as_str() {
                                    Some("completed") => "finished",
                                    Some("failed") => "failed",
                                    _ => "running",
                                }
                                .into(),
                            });
                        }
                        _ => {}
                    }
                }
                _ if !value["id"].is_null() => bail!("unsupported ACP client capability"),
                _ => {}
            }
            return Ok(step);
        }
        ensure!(
            value.get("error").is_none(),
            "ACP rejected request; check native CLI login and capabilities"
        );
        let result = &value["result"];
        match value["id"].as_u64() {
            Some(0) if !self.ready => {
                ensure!(result["protocolVersion"] == 1, "unsupported ACP version");
                if self.request.runtime == Runtime::Dsh {
                    ensure!(
                        result["agentInfo"]["name"] == "deepseek-harness-acp",
                        "unexpected DSH agent"
                    );
                }
                let mut params = json!({"cwd":self.request.cwd,"mcpServers":[]});
                if self.request.runtime == Runtime::Dsh {
                    params["cwd"] = json!(super::super::native_path(std::path::Path::new(
                        &self.request.cwd
                    )));
                }
                if let Some(mcp) = &self.mcp_server {
                    params["mcpServers"] = json!([mcp]);
                }
                let method = if let Some(session) = &self.request.native_session_id {
                    params["sessionId"] = json!(session);
                    if self.request.runtime == Runtime::Dsh {
                        ensure!(
                            result["agentCapabilities"]["sessionCapabilities"]["resume"]
                                .is_object(),
                            "CLI cannot resume DSH sessions"
                        );
                        "session/resume"
                    } else {
                        ensure!(
                            result["agentCapabilities"]["loadSession"] == true,
                            "CLI cannot resume ACP sessions"
                        );
                        "session/load"
                    }
                } else {
                    "session/new"
                };
                step.writes
                    .push(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}));
            }
            Some(1) if !self.ready => {
                if let (Some(expected), Some(actual)) = (
                    &self.request.native_session_id,
                    result["sessionId"].as_str(),
                ) {
                    ensure!(expected == actual, "ACP resumed a different session");
                }
                let id = self
                    .request
                    .native_session_id
                    .clone()
                    .or_else(|| result["sessionId"].as_str().map(str::to_owned))
                    .ok_or_else(|| anyhow::anyhow!("ACP session missing"))?;
                if self.request.runtime == Runtime::Dsh {
                    ensure!(
                        self.dsh_policy_sessions.remove(&id),
                        "DSH permission policy not applied"
                    );
                }
                step.events.push(self.bind(&id)?);
                self.ready = true;
                if self.request.runtime == Runtime::Dsh {
                    if let Some(model) = &self.request.model {
                        let selected = dsh_model(&result["configOptions"], model)?;
                        step.writes.push(
                            json!({"jsonrpc":"2.0","id":3,"method":"session/set_config_option",
                            "params":{"sessionId":id,"configId":"model","value":selected}}),
                        );
                        self.acp_model = Some(selected);
                    } else {
                        self.acp_prompt(&mut step);
                    }
                    return Ok(step);
                }
                if self.request.runtime == Runtime::Grok {
                    // Grok has no ACP modes. Started without --always-approve it
                    // asks through session/request_permission; it has no read-only mode.
                    ensure!(
                        self.request.permission != Permission::ReadOnly,
                        "Grok Build has no read-only mode"
                    );
                    if let Some(model) = &self.request.model {
                        step.writes.push(json!({"jsonrpc":"2.0","id":3,"method":"session/set_model","params":{"sessionId":id,"modelId":model}}));
                    } else {
                        self.acp_prompt(&mut step);
                    }
                    return Ok(step);
                }
                // Set an explicit mode. Never inherit native yolo/auto-edit settings.
                let target = if self.request.permission == Permission::ReadOnly {
                    &["plan"][..]
                } else if self.request.runtime == Runtime::Opencode {
                    // OpenCode's build mode asks first: the Runner forces every
                    // tool permission to "ask" through OPENCODE_CONFIG_CONTENT.
                    &["build"][..]
                } else {
                    &["default", "ask"][..]
                };
                // Legacy `modes` or the newer `configOptions` select with id "mode".
                let legacy = result["modes"]["availableModes"]
                    .as_array()
                    .and_then(|modes| {
                        modes
                            .iter()
                            .find_map(|mode| mode["id"].as_str().filter(|id| target.contains(id)))
                    });
                let option = || {
                    result["configOptions"]
                        .as_array()?
                        .iter()
                        .find(|option| option["id"] == "mode")?["options"]
                        .as_array()?
                        .iter()
                        .find_map(|o| o["value"].as_str().filter(|v| target.contains(v)))
                };
                if let Some(mode) = legacy {
                    step.writes.push(json!({"jsonrpc":"2.0","id":2,"method":"session/set_mode","params":{"sessionId":id,"modeId":mode}}));
                } else if let Some(mode) = option() {
                    step.writes.push(
                        json!({"jsonrpc":"2.0","id":2,"method":"session/set_config_option",
                        "params":{"sessionId":id,"configId":"mode","value":mode}}),
                    );
                } else {
                    bail!("CLI does not expose the requested safe ACP mode");
                }
            }
            Some(2)
                if self.ready && self.turn.is_none() && self.request.runtime != Runtime::Dsh =>
            {
                if let Some(model) = &self.request.model {
                    step.writes.push(json!({"jsonrpc":"2.0","id":3,"method":"session/set_model","params":{"sessionId":self.session,"modelId":model}}));
                } else {
                    self.acp_prompt(&mut step);
                }
            }
            Some(3) if self.ready && self.turn.is_none() => {
                if let Some(expected) = &self.acp_model {
                    let actual = result["configOptions"]
                        .as_array()
                        .and_then(|options| options.iter().find(|option| option["id"] == "model"))
                        .and_then(|option| option["currentValue"].as_str());
                    ensure!(
                        actual == Some(expected.as_str()),
                        "DSH selected a different model"
                    );
                }
                self.acp_prompt(&mut step);
            }
            Some(4) if self.turn.is_some() => {
                ensure!(
                    self.pending.is_empty(),
                    "ACP completed with unresolved approvals"
                );
                step.done = Some(match result["stopReason"].as_str() {
                    Some("end_turn") if self.request.runtime == Runtime::Dsh => self
                        .dsh_outcome
                        .take()
                        .context("DSH turn outcome missing")?,
                    Some("end_turn") => Outcome::Completed,
                    Some("cancelled") => Outcome::Cancelled,
                    _ => Outcome::Failed,
                });
            }
            _ => bail!("unexpected ACP response"),
        }
        Ok(step)
    }

    fn acp_prompt(&mut self, step: &mut Step) {
        self.turn = Some("prompt".into());
        step.writes.push(json!({"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":self.session,"prompt":[{"type":"text","text":self.request.prompt}]}}));
    }
}

/// Select only a value advertised by this session. Human-readable model IDs
/// must be unique across providers; otherwise require provider/model.
fn dsh_model(options: &Value, requested: &str) -> Result<String> {
    let model = options
        .as_array()
        .and_then(|options| options.iter().find(|option| option["id"] == "model"))
        .context("DSH has no model selection")?;
    let groups = model["options"]
        .as_array()
        .context("DSH model catalog missing")?;
    let mut matches = Vec::new();
    for group in groups {
        let entries = group["options"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(std::slice::from_ref(group));
        for option in entries {
            let Some(value) = option["value"].as_str() else {
                continue;
            };
            let route = serde_json::from_str::<Vec<String>>(value).ok();
            if value == requested
                || route.as_ref().is_some_and(|route| {
                    route.len() == 2
                        && (route[1] == requested
                            || format!("{}/{}", route[0], route[1]) == requested)
                })
            {
                if !matches.iter().any(|item| item == value) {
                    matches.push(value.to_owned());
                }
            }
        }
    }
    ensure!(matches.len() == 1, "DSH model is unavailable or ambiguous");
    Ok(matches.remove(0))
}
