//! ACP v1 transport shared by Gemini CLI, Qwen Code and OpenCode.
//! https://agentclientprotocol.com/protocol/overview
use super::*;

impl Protocol {
    pub(super) fn acp(&mut self, value: Value) -> Result<Step> {
        let mut step = Step::default();
        let params = &value["params"];
        if let Some(method) = value["method"].as_str() {
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
                    let platform = self.request.platform_tools
                        && params["toolCall"]["title"].as_str().is_some_and(|name| {
                            [
                                "ferry_discover",
                                "ferry_request",
                                "mcp__ferry__ferry_discover",
                                "mcp__ferry__ferry_request",
                            ]
                            .contains(&name)
                        });
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
                        let tool = params["toolCall"]["title"].as_str().unwrap_or("tool");
                        let input = json!({"tool_call":params["toolCall"],"allow_once":once});
                        return self.approval(value["id"].clone(), tool, input);
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
                let mut params = json!({"cwd":self.request.cwd,"mcpServers":[]});
                if let Some(mcp) = &self.mcp_server {
                    params["mcpServers"] = json!([mcp]);
                }
                let method = if let Some(session) = &self.request.native_session_id {
                    ensure!(
                        result["agentCapabilities"]["loadSession"] == true,
                        "CLI cannot resume ACP sessions"
                    );
                    params["sessionId"] = json!(session);
                    "session/load"
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
                step.events.push(self.bind(&id)?);
                self.ready = true;
                // Set an explicit mode. Never inherit native yolo/auto-edit settings.
                let target = if self.request.permission == Permission::ReadOnly {
                    &["plan"][..]
                } else {
                    &["default", "ask"][..]
                };
                let mode = result["modes"]["availableModes"]
                    .as_array()
                    .and_then(|modes| {
                        modes
                            .iter()
                            .find_map(|mode| mode["id"].as_str().filter(|id| target.contains(id)))
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("CLI does not expose the requested safe ACP mode")
                    })?;
                step.writes.push(json!({"jsonrpc":"2.0","id":2,"method":"session/set_mode","params":{"sessionId":id,"modeId":mode}}));
            }
            Some(2) if self.ready && self.turn.is_none() => {
                if let Some(model) = &self.request.model {
                    step.writes.push(json!({"jsonrpc":"2.0","id":3,"method":"session/set_model","params":{"sessionId":self.session,"modelId":model}}));
                } else {
                    self.acp_prompt(&mut step);
                }
            }
            Some(3) if self.ready && self.turn.is_none() => self.acp_prompt(&mut step),
            Some(4) if self.turn.is_some() => {
                ensure!(
                    self.pending.is_empty(),
                    "ACP completed with unresolved approvals"
                );
                step.done = Some(match result["stopReason"].as_str() {
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
