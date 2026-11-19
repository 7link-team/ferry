//! Agent execution contract. Native session ids belong to one host/user/workdir;
//! the server must verify that binding before issuing a resumed request.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Native lines can echo JSON tool results as escaped MCP text. Keep room for
/// that envelope without allocating a whole turn's output in one message.
pub const MAX_NATIVE_LINE_BYTES: usize = 32 * 1024 * 1024;
/// Both peers account cumulative output with the same budget.
pub const MAX_TURN_OUTPUT_BYTES: usize = 1024 * 1024 * 1024;
/// Normalized events leave framing headroom under the cluster's 16 MiB cap.
pub const MAX_EVENT_BYTES: usize = 12 * 1024 * 1024;
/// Includes JSON-RPC/HTTP envelopes around the API's bounded request body.
pub const MAX_PLATFORM_CALL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Codex,
    Claude,
    Gemini,
    Qwen,
    Opencode,
    Pi,
    Dsh,
    /// Kimi Code (`@moonshot-ai/kimi-code`), not the legacy Python kimi-cli.
    Kimi,
    Grok,
    Copilot,
    Codebuddy,
}
impl Runtime {
    /// Upstream wire formats this CLI can be pointed at directly. Empty means the CLI
    /// only uses its own login/configuration; Ferry never converts formats.
    pub fn provider_formats(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["anthropic"],
            Self::Codex => &["openai_responses"],
            Self::Gemini => &["gemini"],
            Self::Qwen => &["openai_chat"],
            Self::Grok => &["openai_responses", "openai_chat"],
            Self::Opencode | Self::Pi | Self::Kimi => {
                &["openai_chat", "openai_responses", "anthropic", "gemini"]
            }
            Self::Dsh | Self::Copilot | Self::Codebuddy => &[],
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            "qwen" => Some(Self::Qwen),
            "opencode" => Some(Self::Opencode),
            "pi" => Some(Self::Pi),
            "dsh" => Some(Self::Dsh),
            "kimi" => Some(Self::Kimi),
            "grok" => Some(Self::Grok),
            "copilot" => Some(Self::Copilot),
            "codebuddy" => Some(Self::Codebuddy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    /// This Runner can transport per-turn platform tool calls and results.
    #[serde(default)]
    pub platform_tools: bool,
    pub runtime: Runtime,
    pub execution_user: String,
    #[serde(default)]
    pub execution_identity: String,
    pub executable: Option<String>,
    pub version: Option<String>,
    /// Model identifiers from this execution user's native CLI catalog only.
    #[serde(default)]
    pub models: Vec<String>,
    /// logged_in / logged_out / unknown; presence never proves authentication.
    pub authentication: String,
    pub directory_exists: bool,
    /// missing / failed / none. Native diagnostics never cross the wire.
    pub error: Option<String>,
    #[serde(default)]
    pub canonical_directory: Option<String>,
    /// This Runner can run a turn that writes only inside its working directory
    /// (`Request::write_workdir`). Absent on older Runners.
    #[serde(default)]
    pub write_workdir: bool,
    /// File-production tools this execution user has (python modules, office
    /// tools, ffmpeg, CLI skills...), as short descriptions. Never installed by Ferry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub execution_user: String,
    pub execution_identity: String,
    pub canonical_directory: String,
    pub executable: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadOnly,
    OnRequest,
}

/// A custom upstream for one execution. The Runner writes it into an isolated
/// configuration home; the key travels only in the child's environment.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    /// Server-issued model profile id; selects the isolated configuration home.
    pub key: String,
    /// One of `Runtime::provider_formats`.
    pub format: String,
    pub base_url: String,
    pub api_key: String,
    /// Catalog written into CLIs that need declared models.
    #[serde(default)]
    pub models: Vec<String>,
    /// Per-CLI options (model roles, reasoning effort, context window, headers...).
    #[serde(default)]
    pub options: Value,
}
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("key", &self.key)
            .field("format", &self.format)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub binding: Option<Binding>,
    pub runtime: Runtime,
    pub cwd: String,
    pub prompt: String,
    /// None inherits the native runtime's configured model, without a fallback.
    pub model: Option<String>,
    pub native_session_id: Option<String>,
    pub permission: Permission,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub platform_tools: bool,
    /// None keeps the CLI's own login and configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// IANA time zone the model requires for this execution; overrides the
    /// runner's setting. Absent for older servers and models without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Unattended file production: with `Permission::ReadOnly`, the CLI may
    /// write only inside `cwd` (OS sandbox for Codex, path-scoped edit tools for
    /// Claude) and is never asked for approval. Sent only to Runners whose probe
    /// reports `write_workdir`; other runtimes are refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub write_workdir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    PlatformResult {
        request_id: String,
        result: Value,
    },
    Cancel,
    /// The token is scoped to this invocation; no session-wide grants.
    Decision {
        request_id: String,
        allow: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    PlatformCall {
        request_id: String,
        name: String,
        arguments: Value,
    },
    PlatformResult {
        request_id: String,
        result: Value,
        /// What was asked (tool, method, path; never a body), for a readable
        /// execution history. Server-created only, so older peers never see it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<Value>,
    },
    Session {
        native_session_id: String,
    },
    /// Deltas append; a final replacement is authoritative for this item only.
    Text {
        item_id: String,
        text: String,
        replace: bool,
    },
    Tool {
        item_id: String,
        name: String,
        state: String,
        /// Native tool arguments, when supplied by the runtime.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
    },
    Approval {
        request_id: String,
        tool: String,
        input: Value,
    },
    /// Written to the native process; this is not a tool execution receipt.
    DecisionSent {
        request_id: String,
        allowed: bool,
    },
    Finished {
        outcome: Outcome,
        usage: Option<Usage>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_arguments_are_optional_and_round_trip_when_present() {
        let old = json!({"type":"tool","item_id":"t","name":"Read","state":"running"});
        let parsed: Event = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(&parsed, Event::Tool { input: None, .. }));
        assert_eq!(serde_json::to_value(parsed).unwrap(), old);
        let mut with_input = old;
        with_input["input"] = json!({"path":"src/main.rs"});
        let parsed: Event = serde_json::from_value(with_input.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), with_input);
    }

    #[test]
    fn working_directory_writes_and_file_tools_are_backward_compatible() {
        let request = json!({"runtime":"codex","cwd":"/w","prompt":"p","model":null,
            "native_session_id":null,"permission":"read_only","timeout_seconds":60});
        // Older servers send no flag; older Runners never see it unless it is set.
        let parsed: Request = serde_json::from_value(request.clone()).unwrap();
        assert!(!parsed.write_workdir);
        let sent = serde_json::to_value(&parsed).unwrap();
        assert!(sent.get("write_workdir").is_none());
        let mut files = parsed.clone();
        files.write_workdir = true;
        assert_eq!(serde_json::to_value(&files).unwrap()["write_workdir"], true);
        // Older Probes have neither field; empty tools are not sent.
        let probe = json!({"runtime":"claude","execution_user":"u","executable":null,
            "version":null,"authentication":"unknown","directory_exists":true,"error":null});
        let parsed: Probe = serde_json::from_value(probe).unwrap();
        assert!(!parsed.write_workdir && parsed.file_tools.is_empty());
        assert!(
            serde_json::to_value(&parsed)
                .unwrap()
                .get("file_tools")
                .is_none()
        );
    }
}

/// Shared platform tool contracts for native and API model clients.
pub fn platform_tools() -> Value {
    serde_json::json!([
      {
        "name": "ferry_discover",
        "description": "Discover current workspace operations and user/Agent permissions. Read usage, operation documentation, query keys, parameter definitions and recovery instructions before calling ferry_request. Follow next_offset for additional operations. For workflows, GET /workflow-definition-format supplies the exact Schema and executable examples. Never infer platform records from local files.",
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
        "description": "Invoke a discovered workspace API using its documentation and usage contract. The server rechecks current user/Agent permissions and the human-configured approval mode. Approval permits an attempt, not proof of success. Check status/ok/data/guidance; correct recoverable errors instead of repeating an unchanged failing call. Use ferry_ask_user when a user decision is needed. Never invent results.",
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
              "description": "Binary upload instead of body. Up to 1 MiB base64. Raw upload: data_base64. Multipart: add filename and optional field (default file). The response ceiling is maintained in platform settings; use paging or file download/preview paths for large data.",
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
      },
      {
        "name":"ferry_ask_user",
        "description":"Save a durable question in this conversation when a decision or missing information blocks progress. Explain the cause, feasible choices and recommendation. Suggested options are optional: users can always type any answer, pause, or abandon. After this tool returns awaiting_user, end the turn and wait; do not continue operations. The user's answer resumes the same conversation and never grants approval for a write. This is not available in unattended workflow steps.",
        "inputSchema":{
          "type":"object","additionalProperties":false,
          "properties":{
            "title":{"type":"string","minLength":1,"maxLength":120},
            "question":{"type":"string","minLength":1,"maxLength":2048},
            "reason":{"type":"string","maxLength":4096},
            "options":{"type":"array","maxItems":6,"items":{
              "type":"object","additionalProperties":false,
              "properties":{"id":{"type":"string","minLength":1,"maxLength":64},"label":{"type":"string","minLength":1,"maxLength":160},"description":{"type":"string","maxLength":2048}},
              "required":["id","label"]}},
            "recommended_option_id":{"type":"string","description":"ID of one provided option; omit when there are no suggestions."}
          },"required":["title","question"]
        }
      }
    ])
}
