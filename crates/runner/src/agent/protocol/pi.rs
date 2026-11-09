//! Pi RPC: get_state admits the session; agent_end completes a prompt.
//! Tools, extensions and MCP are disabled by the command builder.
use super::*;

impl Protocol {
    pub(super) fn pi(&mut self, value: Value) -> Result<Step> {
        let mut step = Step::default();
        match value["type"].as_str() {
            Some("response") => {
                ensure!(value["success"] == true, "Pi rejected RPC request");
                match value["id"].as_str() {
                    Some("initialize") if !self.ready => {
                        ensure!(
                            self.request.native_session_id.is_none()
                                || value["data"]["messageCount"]
                                    .as_u64()
                                    .is_some_and(|count| count > 0),
                            "Pi native history is missing"
                        );
                        let id = value["data"]["sessionId"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Pi session id missing"))?;
                        step.events.push(self.bind(id)?);
                        self.ready = true;
                        step.writes.push(
                            json!({"id":"prompt","type":"prompt","message":self.request.prompt}),
                        );
                    }
                    Some("prompt") if self.ready => {
                        ensure!(
                            value["data"]["disposition"] != "handled",
                            "Pi handled prompt without execution"
                        );
                    }
                    _ => bail!("unexpected Pi RPC response"),
                }
            }
            Some("message_start") if value["message"]["role"] == "assistant" => {
                ensure!(self.ready, "Pi output before initialization");
                self.next_request += 1;
                self.turn = Some(format!("message-{}", self.next_request));
            }
            Some("message_update") if value["assistantMessageEvent"]["type"] == "text_delta" => {
                let id = self
                    .turn
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Pi output before message"))?;
                step.events.push(
                    self.text(
                        &id,
                        value["assistantMessageEvent"]["delta"]
                            .as_str()
                            .unwrap_or(""),
                        false,
                    ),
                );
            }
            Some("message_end") if value["message"]["role"] == "assistant" => {
                let message = &value["message"];
                if matches!(message["stopReason"].as_str(), Some("error" | "aborted")) {
                    // Carry the last assistant outcome to the terminal agent event.
                    self.streamed.insert("pi_failed".into(), "true".into());
                } else {
                    self.streamed.remove("pi_failed");
                }
                let id = self
                    .turn
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Pi final output before message"))?;
                let text = message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|part| part["type"] == "text")
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>();
                step.events.push(self.text(&id, &text, true));
                self.usage = message["usage"]["input"]
                    .as_u64()
                    .zip(message["usage"]["output"].as_u64())
                    .map(|(input_tokens, output_tokens)| Usage {
                        input_tokens,
                        output_tokens,
                    });
            }
            Some("tool_execution_start" | "extension_ui_request") => {
                bail!("Pi tools and extensions are disabled")
            }
            Some("agent_end" | "agent_settled") => {
                ensure!(
                    self.ready && self.turn.is_some(),
                    "Pi settled without an assistant message"
                );
                step.done = Some(if value["aborted"] == true {
                    Outcome::Cancelled
                } else if self.streamed.contains_key("pi_failed") {
                    Outcome::Failed
                } else {
                    Outcome::Completed
                });
            }
            _ => {}
        }
        Ok(step)
    }
}
