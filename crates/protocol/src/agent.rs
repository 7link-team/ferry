//! Agent execution contract. Native session ids belong to one host/user/workdir;
//! the server must verify that binding before issuing a resumed request.
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
impl Runtime {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            "qwen" => Some(Self::Qwen),
            "opencode" => Some(Self::Opencode),
            "pi" => Some(Self::Pi),
            "dsh" => Some(Self::Dsh),
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
