//! Local sandbox pipe contract. Contains data only, never credentials, database
//! configuration, executable paths or a request to load host code/modules.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: u8 = 1;
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EVALUATIONS: usize = 20_000;
pub const MAX_TIMEOUT_MS: u64 = 5_000;
pub const MIN_MEMORY_BYTES: usize = 1024 * 1024;
pub const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub version: u8,
    pub timeout_ms: u64,
    pub memory_bytes: usize,
    pub deterministic: bool,
    /// Parse every expression without executing tenant code or producing values.
    #[serde(default)]
    pub syntax_only: bool,
    /// One authorized execution-data snapshot per batch, never repeated per item.
    #[serde(default)]
    pub data: Option<InputData>,
    /// Shared frozen plain-data context. Per-evaluation contexts must be empty.
    #[serde(default)]
    pub context: Option<Value>,
    pub evaluations: Vec<Evaluation>,
    /// Versioned, data-only extension invocation; mutually exclusive with expressions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<Program>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Js,
    Lua,
    Wasm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    pub abi: u8,
    pub language: Language,
    /// JS/Lua function body, or base64-encoded WASM module. No file/module paths.
    pub source: String,
}
impl Code {
    pub fn valid(&self) -> bool {
        self.abi == 1
            && !self.source.trim().is_empty()
            && self.source.len()
                <= match self.language {
                    Language::Wasm => 1024 * 1024,
                    _ => 64 * 1024,
                }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub code: Code,
    pub inputs: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub json: serde_json::Map<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub binary: BTreeMap<String, Asset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paired: Vec<Paired>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub asset: String,
    pub mime: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paired {
    pub node: String,
    pub iteration_hash: String,
    pub index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeOutput {
    pub node: String,
    pub name: String,
    pub iteration_hash: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputData {
    pub items: Vec<Item>,
    /// Ordered, authorized output batches supplied by the graph executor.
    pub upstream: Vec<NodeOutput>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evaluation {
    /// Raw JS, or a Ferry parameter beginning with '=' when template is true.
    pub expression: String,
    #[serde(default)]
    pub template: bool,
    /// Internal conditional operands may be missing; ordinary results stay strict JSON.
    #[serde(default)]
    pub capture_presence: bool,
    #[serde(default)]
    pub item_index: Option<usize>,
    pub context: Value,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Ok {
        version: u8,
        values: Vec<Value>,
    },
    Error {
        version: u8,
        code: ErrorCode,
        index: Option<usize>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Isolation,
    Timeout,
    Memory,
    Syntax,
    Evaluation,
    AmbiguousItem,
    MissingItem,
    ResultLimit,
    Internal,
}
impl Reply {
    pub fn error(code: ErrorCode, index: Option<usize>) -> Self {
        Self::Error {
            version: VERSION,
            code,
            index,
        }
    }
}
impl Batch {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && (1..=MAX_TIMEOUT_MS).contains(&self.timeout_ms)
            && (MIN_MEMORY_BYTES..=MAX_MEMORY_BYTES).contains(&self.memory_bytes)
            && self.evaluations.len() <= MAX_EVALUATIONS
            && self.program.as_ref().is_none_or(|program| {
                program.code.valid()
                    && program.inputs.len() <= MAX_EVALUATIONS
                    && program.inputs.iter().all(Value::is_object)
                    && self.evaluations.is_empty()
                    && self.data.is_none()
                    && self.context.is_none()
                    && self.deterministic
            })
            && self.data.as_ref().is_none_or(InputData::valid)
            && self.context.as_ref().is_none_or(valid_context)
            && self.evaluations.iter().all(|evaluation| {
                !evaluation.expression.is_empty()
                    && evaluation.expression.len() <= 64 * 1024
                    && evaluation.item_index.is_none_or(|index| {
                        self.data
                            .as_ref()
                            .is_some_and(|data| index < data.items.len())
                    })
                    && valid_context(&evaluation.context)
                    && (self.context.is_none()
                        || evaluation
                            .context
                            .as_object()
                            .is_some_and(|context| context.is_empty()))
            })
    }
}

fn valid_context(value: &Value) -> bool {
    value.as_object().is_some_and(|context| {
        context.keys().all(|key| {
            [
                "$json",
                "$binary",
                "$input",
                "$node",
                "$execution",
                "$workflow",
                "$vars",
                "$now",
                "$today",
            ]
            .contains(&key.as_str())
        })
    })
}

impl InputData {
    pub fn valid(&self) -> bool {
        fn node(value: &str) -> bool {
            (1..=64).contains(&value.len())
                && value.as_bytes()[0].is_ascii_lowercase()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }
        fn hash(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }
        fn item(item: &Item) -> bool {
            item.paired
                .iter()
                .all(|pair| node(&pair.node) && hash(&pair.iteration_hash))
                && item.binary.values().all(|asset| {
                    asset.asset.strip_prefix("sha256:").is_some_and(hash) && !asset.mime.is_empty()
                })
        }
        let mut identities = BTreeSet::new();
        let mut names = BTreeMap::new();
        self.items.iter().all(item)
            && self.upstream.iter().all(|output| {
                node(&output.node)
                    && hash(&output.iteration_hash)
                    && !output.name.is_empty()
                    && identities.insert((&output.node, &output.iteration_hash))
                    && names
                        .insert(&output.node, &output.name)
                        .is_none_or(|name| name == &output.name)
                    && output.items.iter().all(item)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shared_context_never_accepts_host_namespaces_or_per_item_overrides() {
        let mut value = json!({"version":1,"timeout_ms":100,"memory_bytes":1048576,"deterministic":true,
            "context":{"$vars":{"value":2}},"evaluations":[{"expression":"$vars.value","context":{}}]});
        assert!(
            serde_json::from_value::<Batch>(value.clone())
                .unwrap()
                .valid()
        );
        value["evaluations"][0]["context"] = json!({"$vars":{"value":999}});
        assert!(
            !serde_json::from_value::<Batch>(value.clone())
                .unwrap()
                .valid()
        );
        value["evaluations"][0]["context"] = json!({});
        value["context"] = json!({"$env":{"secret":"PRIVATE"}});
        assert!(
            !serde_json::from_value::<Batch>(value.clone())
                .unwrap()
                .valid()
        );
        value["context"] = json!("wrong shape");
        assert!(!serde_json::from_value::<Batch>(value).unwrap().valid());
    }
    #[test]
    fn shared_input_rejects_invalid_assets_duplicate_identity_and_out_of_range_selection() {
        let value = json!({"version":1,"timeout_ms":100,"memory_bytes":1048576,"deterministic":true,
            "data":{"items":[{"json":{}}],"upstream":[{"node":"source","name":"Source","iteration_hash":"a".repeat(64),"items":[]}]},
            "evaluations":[{"expression":"$input.item","item_index":0,"context":{}}]});
        let mut request: Batch = serde_json::from_value(value.clone()).unwrap();
        assert!(request.valid());
        request.evaluations[0].item_index = Some(1);
        assert!(!request.valid());
        request.evaluations[0].item_index = Some(0);
        request.data = None;
        assert!(!request.valid());
        let mut bad = value.clone();
        bad["data"]["upstream"]
            .as_array_mut()
            .unwrap()
            .push(value["data"]["upstream"][0].clone());
        assert!(!serde_json::from_value::<Batch>(bad).unwrap().valid());
        let mut bad = value.clone();
        bad["data"]["items"][0]["binary"] =
            json!({"file":{"asset":"/private/path","mime":"text/plain","name":"x","size":1}});
        assert!(!serde_json::from_value::<Batch>(bad).unwrap().valid());
        let mut bad = value.clone();
        bad["data"]["items"][0]["json"] = json!("wrong type");
        assert!(serde_json::from_value::<Batch>(bad).is_err());
        let mut bad = value;
        bad["data"]["database_url"] = json!("PRIVATE");
        assert!(serde_json::from_value::<Batch>(bad).is_err());
    }

    #[test]
    fn protocol_rejects_unknown_options_secret_contexts_and_out_of_budget_requests() {
        let good = json!({"version":1,"timeout_ms":100,"memory_bytes":1048576,"deterministic":true,
            "evaluations":[{"expression":"$json.value","context":{"$json":{"value":1}}}]});
        let mut request: Batch = serde_json::from_value(good.clone()).unwrap();
        assert!(request.valid());
        request.evaluations[0].context = json!({"$env":{"key":"private"}});
        assert!(!request.valid());
        for (key, value) in [
            ("version", json!(2)),
            ("timeout_ms", json!(0)),
            ("timeout_ms", json!(5001)),
            ("memory_bytes", json!(1048575)),
            ("memory_bytes", json!(67108865)),
        ] {
            let mut value2 = good.clone();
            value2[key] = value;
            assert!(!serde_json::from_value::<Batch>(value2).unwrap().valid());
        }
        let mut extra = good;
        extra["database_url"] = json!("private");
        assert!(serde_json::from_value::<Batch>(extra).is_err());
    }
}
