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
