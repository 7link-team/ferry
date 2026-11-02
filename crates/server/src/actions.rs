//! GitHub Actions document validation and immutable execution snapshots.
//! act evaluates expressions and runs steps; this module scopes host bindings.
use crate::{
    entities::{host_groups, hosts},
    error::{ApiError, Result},
    pipelines::{Definition, Environment, Step},
    snowflake::Id,
    workspace::Scope,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const ENGINE: &str = "github-actions";
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub job_id: String,
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_ref: String,
    #[serde(default = "host_image")]
    pub image: String,
}
fn host_image() -> String {
    "host".into()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutBinding {
    pub job_id: String,
    pub step_id: String,
    pub connection_id: String,
    pub repository_url: String,
    #[serde(default, rename = "ref")]
    pub git_ref: String,
    #[serde(default)]
    pub path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedCheckout {
    pub binding: CheckoutBinding,
    pub credential_id: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub engine: String,
    pub workflow: String,
    #[serde(default)]
    pub executor_bindings: Vec<Binding>,
    #[serde(default)]
    pub checkout_bindings: Vec<CheckoutBinding>,
    #[serde(default)]
    pub graph_layout: Value,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub key: String,
    pub name: String,
    pub needs: Vec<String>,
    pub labels: Vec<String>,
    pub steps: Vec<JobStep>,
    pub host_id: i64,
    pub image: String,
    #[serde(default)]
    pub manual: bool,
}
impl Job {
    pub(crate) fn reusable(&self) -> bool {
        !self.manual && self.steps.is_empty()
    }
}
impl Serialize for Job {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Job", 9)?;
        state.serialize_field("key", &self.key)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("needs", &self.needs)?;
        state.serialize_field("labels", &self.labels)?;
        state.serialize_field("steps", &self.steps)?;
        state.serialize_field("host_id", &self.host_id)?;
        state.serialize_field("image", &self.image)?;
        state.serialize_field("manual", &self.manual)?;
        state.serialize_field("reusable", &self.reusable())?;
        state.end()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStep {
    pub key: String,
    pub name: String,
    pub index: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub engine: String,
    pub document: Document,
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub checkouts: Vec<ResolvedCheckout>,
}

pub fn document(input: &str) -> Option<Document> {
    let value: Value = serde_json::from_str(input).ok()?;
    if value.get("engine")?.as_str()? != ENGINE {
        return None;
    }
    serde_json::from_value(value).ok()
}
pub fn snapshot(input: &str) -> Option<Snapshot> {
    serde_json::from_str::<Snapshot>(input)
        .ok()
        .filter(|s| s.engine == ENGINE)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContextReferences {
    pub(crate) names: BTreeSet<String>,
    pub(crate) deliver_all: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct References {
    pub(crate) secrets: ContextReferences,
    pub(crate) vars: ContextReferences,
}
#[derive(Debug, PartialEq)]
enum ExpressionToken {
    Name(String),
    String(String),
    Symbol(char),
}

/// Return the closing delimiter's offset, ignoring delimiters inside quoted strings.
pub(crate) fn expression_end(text: &str, start: usize) -> Result<usize> {
    let mut chars = text[start..].char_indices().peekable();
    let mut quoted = false;
    let mut nesting = Vec::new();
    while let Some((offset, ch)) = chars.next() {
        if ch == '\'' {
            if quoted && chars.peek().is_some_and(|(_, c)| *c == '\'') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if !quoted {
            match ch {
                '(' | '[' => nesting.push(ch),
                ')' | ']' => {
                    if nesting.pop() != Some(if ch == ')' { '(' } else { '[' }) {
                        return Err(ApiError::bad("Actions 表达式括号不匹配。"));
                    }
                }
                '}' if nesting.is_empty() && chars.peek().is_some_and(|(_, c)| *c == '}') => {
                    return Ok(start + offset);
                }
                _ => {}
            }
        }
    }
    Err(ApiError::bad("Actions 表达式或字符串未闭合。"))
}

fn expression_tokens(expression: &str) -> Result<Vec<ExpressionToken>> {
    let mut chars = expression.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '\'' {
            let mut value = String::new();
            loop {
                match chars.next() {
                    Some('\'') if chars.peek() == Some(&'\'') => {
                        chars.next();
                        value.push('\'');
                    }
                    Some('\'') => break,
                    Some(ch) => value.push(ch),
                    None => return Err(ApiError::bad("Actions 表达式字符串未闭合。")),
                }
            }
            tokens.push(ExpressionToken::String(value));
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let mut name = ch.to_string();
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                name.push(chars.next().unwrap());
            }
            tokens.push(ExpressionToken::Name(name));
        } else {
            tokens.push(ExpressionToken::Symbol(ch));
        }
    }
    Ok(tokens)
}

fn expression_references(expression: &str, references: &mut References) -> Result<()> {
    let tokens = expression_tokens(expression)?;
    for (index, token) in tokens.iter().enumerate() {
        let ExpressionToken::Name(name) = token else {
            continue;
        };
        if index > 0 && tokens[index - 1] == ExpressionToken::Symbol('.') {
            continue;
        }
        let context = if name.eq_ignore_ascii_case("secrets") {
            &mut references.secrets
        } else if name.eq_ignore_ascii_case("vars") {
            &mut references.vars
        } else {
            continue;
        };
        let name = match tokens.get(index + 1) {
            Some(ExpressionToken::Symbol('.')) => match tokens.get(index + 2) {
                Some(ExpressionToken::Name(name)) => Some(name),
                _ => None,
            },
            Some(ExpressionToken::Symbol('[')) => {
                match (tokens.get(index + 2), tokens.get(index + 3)) {
                    (Some(ExpressionToken::String(name)), Some(ExpressionToken::Symbol(']'))) => {
                        Some(name)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(name) = name {
            context.names.insert(name.clone());
        } else {
            context.deliver_all = true;
        }
    }
    Ok(())
}

fn discover_references(value: &Value) -> Result<References> {
    fn visit(value: &Value, implicit: bool, references: &mut References) -> Result<()> {
        match value {
            Value::String(text) => {
                if implicit && !text.contains("${{") {
                    expression_references(text, references)?;
                } else {
                    let mut offset = 0;
                    while let Some(relative) = text[offset..].find("${{") {
                        let start = offset + relative + 3;
                        let end = expression_end(text, start)?;
                        expression_references(&text[start..end], references)?;
                        offset = end + 2;
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, false, references)?;
                }
            }
            Value::Object(items) => {
                for (key, item) in items {
                    visit(item, key == "if", references)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut references = References::default();
    references.secrets.deliver_all =
        value
            .get("jobs")
            .and_then(Value::as_object)
            .is_some_and(|jobs| {
                jobs.values().any(|job| {
                    job.get("uses").is_some()
                        && job.get("secrets").and_then(Value::as_str) == Some("inherit")
                })
            });
    visit(value, false, &mut references)?;
    Ok(references)
}

/// Runtime snapshot discovery, including dynamic/whole contexts. `parse` returns
/// only static references for the existing definition summary; S2 must use this
/// result with `variables::snapshot_for_actions` to prepare worker contexts.
pub(crate) fn references(document: &Document) -> Result<References> {
    let value = serde_yaml_ng::from_str(&document.workflow)
        .map_err(|e| ApiError::bad(format!("Actions YAML 格式错误：{e}。")))?;
    discover_references(&value)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Concurrency {
    pub(crate) group: String,
    pub(crate) cancel_in_progress: Value,
}
fn concurrency_value(value: &Value) -> Option<Concurrency> {
    Some(Concurrency {
        group: value
            .as_str()
            .or_else(|| value.get("group")?.as_str())?
            .into(),
        cancel_in_progress: value
            .get("cancel-in-progress")
            .cloned()
            .unwrap_or(json!(false)),
    })
}
pub(crate) fn concurrency(document: &Document) -> Option<Concurrency> {
    let value: Value = serde_yaml_ng::from_str(&document.workflow).ok()?;
    concurrency_value(value.get("concurrency")?)
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JobPolicies {
    pub(crate) concurrency: Option<Value>,
    pub(crate) environment: Option<Value>,
    pub(crate) reusable_workflow: Option<String>,
    /// The job's `if`, as written.
    pub(crate) condition: Option<String>,
}
pub(crate) fn job_policies(document: &Document, key: &str) -> Option<JobPolicies> {
    let value: Value = serde_yaml_ng::from_str(&document.workflow).ok()?;
    let job = value.get("jobs")?.get(key)?;
    Some(JobPolicies {
        concurrency: job.get("concurrency").cloned(),
        environment: job.get("environment").cloned(),
        reusable_workflow: job.get("uses").and_then(Value::as_str).map(str::to_owned),
        condition: job.get("if").map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }),
    })
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InputSpec {
    pub(crate) name: String,
    pub(crate) r#type: String,
    pub(crate) required: bool,
    pub(crate) default: Option<Value>,
    pub(crate) options: Vec<String>,
    pub(crate) description: String,
}
pub(crate) fn dispatch_inputs(document: &Document) -> Vec<InputSpec> {
    let Ok(value) = serde_yaml_ng::from_str::<Value>(&document.workflow) else {
        return vec![];
    };
    let Some(inputs) = value
        .pointer("/on/workflow_dispatch/inputs")
        .and_then(Value::as_object)
    else {
        return vec![];
    };
    inputs
        .iter()
        .map(|(name, input)| InputSpec {
            name: name.clone(),
            r#type: input
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("string")
                .into(),
            required: input
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            default: input.get("default").cloned(),
            options: input
                .get("options")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            description: input
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        })
        .collect()
}
fn string_list(value: Option<&Value>, noun: &str) -> Result<Vec<String>> {
    match value {
        None => Ok(vec![]),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| ApiError::bad(format!("{noun}必须是字符串或字符串数组。")))
            })
            .collect(),
        _ => Err(ApiError::bad(format!("{noun}必须是字符串或字符串数组。"))),
    }
}
pub fn parse(document: &Document) -> Result<(Vec<Job>, Vec<Environment>)> {
    if document.engine != ENGINE || document.workflow.contains('\0') {
        return Err(ApiError::bad("Actions YAML 无效或包含空字符。"));
    }
    let value: Value = serde_yaml_ng::from_str(&document.workflow)
        .map_err(|e| ApiError::bad(format!("Actions YAML 格式错误：{e}。")))?;
    let jobs = value
        .get("jobs")
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| ApiError::bad("Actions 工作流至少需要一个任务。"))?;
    let mut index = 0i64;
    let mut result = Vec::new();
    for (key, raw) in jobs {
        let manual = match raw.get("ferry-approval") {
            None => false,
            Some(Value::Bool(true)) => true,
            _ => {
                return Err(ApiError::bad(format!(
                    "任务 {key} 的 ferry-approval 必须为 true。"
                )));
            }
        };
        if manual
            && (raw.get("steps").is_some()
                || raw.get("uses").is_some()
                || raw.get("runs-on").is_some())
        {
            return Err(ApiError::bad(format!(
                "人工卡点 {key} 不能配置执行步骤或主机。"
            )));
        }
        if manual
            && raw.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .any(|field| !matches!(field.as_str(), "name" | "needs" | "ferry-approval"))
            })
        {
            return Err(ApiError::bad(format!(
                "人工卡点 {key} 仅支持名称和前置任务。"
            )));
        }
        if !key
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ApiError::bad("Actions 任务标识无效。"));
        }
        if let Some(uses) = raw.get("uses") {
            let valid = uses.as_str().is_some_and(|uses| {
                !uses.contains(['\0', '\r', '\n'])
                    && !uses.contains("${{")
                    && if let Some(path) = uses.strip_prefix("./") {
                        !path.is_empty()
                            && !path.contains(['@', '\\'])
                            && !path.split('/').any(|s| s.is_empty() || s == "..")
                    } else {
                        uses.rsplit_once('@').is_some_and(|(path, reference)| {
                            let parts: Vec<_> = path.split('/').collect();
                            parts.len() >= 3
                                && parts.iter().all(|p| !p.is_empty() && *p != "..")
                                && !reference.trim().is_empty()
                                && !path.contains(['@', ':', '\\'])
                        })
                    }
            });
            if !valid || raw.get("steps").is_some() || raw.get("runs-on").is_some() {
                return Err(ApiError::bad(
                    "复用工作流必须使用 ./本地路径或 owner/repo/path@ref，且不能配置 steps 或 runs-on。",
                ));
            }
        }
        let container = document
            .executor_bindings
            .iter()
            .find(|binding| binding.job_id == *key)
            .map(|binding| binding.image != "host")
            .unwrap_or_else(|| raw.get("container").is_some());
        let mut step_ids = BTreeSet::new();
        if raw.get("services").is_some() && !container {
            return Err(ApiError::bad(
                "services 需要容器执行，请把该任务绑定到容器镜像",
            ));
        }
        // runs-on is descriptive; the executor binding always selects the actual host/image,
        // including when the labels or runner group contain expressions.
        let labels = match raw.get("runs-on") {
            Some(Value::Object(runner)) => {
                let mut labels = string_list(runner.get("labels"), "runs-on.labels")?;
                if let Some(group) = runner.get("group") {
                    labels.extend(string_list(Some(group), "runs-on.group")?);
                }
                labels
            }
            other => string_list(other, "runs-on")?,
        };
        if labels.is_empty() && !manual && raw.get("uses").is_none() {
            return Err(ApiError::bad(format!("任务 {key} 缺少 runs-on。")));
        }
        let needs = string_list(raw.get("needs"), "needs")?;
        let steps = match raw.get("steps") {
            Some(Value::Array(steps)) if !steps.is_empty() => steps
                .iter()
                .enumerate()
                .map(|(n, step)| {
                    if step.get("run").is_some() == step.get("uses").is_some() {
                        return Err(ApiError::bad(format!(
                            "任务 {key} 第 {} 步必须且只能填写 run 或 uses。",
                            n + 1
                        )));
                    }
                    let command = step
                        .get("run")
                        .or_else(|| step.get("uses"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty() && !value.contains('\0'))
                        .ok_or_else(|| ApiError::bad("run 或 uses 必须是非空字符串。"))?;
                    if step.get("uses").is_some() {
                        if command.starts_with("docker://") && !container {
                            return Err(ApiError::bad(
                                "Docker Action 需要在任务配置中选择容器执行。",
                            ));
                        }
                    }
                    if let Some(id) = step.get("id") {
                        let id =
                            id.as_str()
                                .filter(|id| {
                                    id.bytes().next().is_some_and(|byte| {
                                        byte.is_ascii_alphabetic() || byte == b'_'
                                    }) && id.bytes().all(|byte| {
                                        byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
                                    })
                                })
                                .ok_or_else(|| ApiError::bad("步骤 id 无效。"))?;
                        if !step_ids.insert(id.to_owned()) {
                            return Err(ApiError::bad("同一任务的步骤 id 不能重复。"));
                        }
                    }
                    if let Some(timeout) = step.get("timeout-minutes") {
                        let expression =
                            timeout.as_str().is_some_and(|value| value.contains("${{"));
                        let number = timeout
                            .as_u64()
                            .or_else(|| timeout.as_str().and_then(|value| value.parse().ok()));
                        if !expression && !number.is_some_and(|value| value > 0) {
                            return Err(ApiError::bad(
                                "步骤 timeout-minutes 必须为正整数分钟或 Actions 表达式。",
                            ));
                        }
                    }
                    let name = step
                        .get("name")
                        .or(step.get("uses"))
                        .and_then(Value::as_str)
                        .unwrap_or("执行命令")
                        .to_owned();
                    let item = JobStep {
                        key: step
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| n.to_string()),
                        name,
                        index,
                    };
                    index += 1;
                    Ok(item)
                })
                .collect::<Result<Vec<_>>>()?,
            None if manual || raw.get("uses").is_some() => vec![],
            _ => {
                return Err(ApiError::bad(format!("任务 {key} 至少需要一个执行步骤。")));
            }
        };
        result.push(Job {
            key: key.clone(),
            name: raw
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .into(),
            needs,
            labels,
            steps,
            host_id: 0,
            image: "host".into(),
            manual,
        });
    }
    fn visit(
        key: &str,
        jobs: &[Job],
        done: &mut BTreeSet<String>,
        path: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(key) {
            return Ok(());
        }
        if !path.insert(key.into()) {
            return Err(ApiError::bad("任务依赖存在循环。"));
        }
        let job = jobs
            .iter()
            .find(|j| j.key == key)
            .ok_or_else(|| ApiError::bad(format!("依赖任务 {key} 不存在。")))?;
        for dep in &job.needs {
            visit(dep, jobs, done, path)?;
        }
        path.remove(key);
        done.insert(key.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for job in &result {
        visit(&job.key, &result, &mut done, &mut BTreeSet::new())?;
    }
    let mut bindings = BTreeSet::new();
    for binding in &document.executor_bindings {
        if !jobs.contains_key(&binding.job_id) || !bindings.insert(&binding.job_id) {
            return Err(ApiError::bad("执行器绑定包含未知或重复的任务。"));
        }
        if binding.image.trim().is_empty()
            || binding.image.len() > 255
            || binding.image.contains('\0')
        {
            return Err(ApiError::bad("请填写有效的执行镜像。"));
        }
    }
    let mut checkout_ids = BTreeSet::new();
    for binding in &document.checkout_bindings {
        let step = jobs
            .get(&binding.job_id)
            .and_then(|job| job.get("steps"))
            .and_then(Value::as_array)
            .and_then(|steps| {
                steps.iter().find(|step| {
                    step.get("id").and_then(Value::as_str) == Some(binding.step_id.as_str())
                })
            });
        if !step
            .and_then(|step| step.get("uses"))
            .and_then(Value::as_str)
            .is_some_and(|action| action.starts_with("actions/checkout@"))
            || !checkout_ids.insert((&binding.job_id, &binding.step_id))
        {
            return Err(ApiError::bad("代码源绑定必须引用唯一的 checkout 步骤 id。"));
        }
        if let Some(options) = step.and_then(|step| step.get("with")) {
            let options = options
                .as_object()
                .ok_or_else(|| ApiError::bad("绑定代码源步骤的 with 必须是对象。"))?;
            for (key, value) in options {
                let expected = match key.as_str() {
                    "repository" => &binding.repository_url,
                    "ref" => &binding.git_ref,
                    "path" => &binding.path,
                    _ => {
                        return Err(ApiError::bad(format!(
                            "绑定代码源步骤暂不支持 checkout 参数 {key}，请移除该参数或使用未绑定的原生 Action。"
                        )));
                    }
                };
                if value.as_str() != Some(expected.as_str()) {
                    return Err(ApiError::bad(format!(
                        "checkout 参数 {key} 与代码源节点配置不一致。"
                    )));
                }
            }
        }
        if Id::reference(&binding.connection_id)? == Id::from(0)
            || binding.repository_url.trim().is_empty()
            || [&binding.repository_url, &binding.git_ref, &binding.path]
                .iter()
                .any(|value| value.contains("${{"))
            || binding.git_ref.len() > 255
            || binding.git_ref.contains(['\0', '\n', '\r'])
            || binding.path.contains(['\0', '\n', '\r', '\\'])
            || binding.path.starts_with('/')
            || binding.path.split('/').any(|part| part == "..")
        {
            return Err(ApiError::bad("代码源连接、引用或工作区路径无效。"));
        }
    }
    let references = discover_references(&value)?;
    let mut variables = Vec::new();
    for (namespace, references) in [("secrets", references.secrets), ("vars", references.vars)] {
        for name in references.names {
            variables.push(Environment {
                name: format!("{namespace}:{name}"),
                value: None,
                value_from: Some(format!("variable/{name}")),
            });
        }
    }
    Ok((result, variables))
}

pub fn summary(document: &Document) -> Result<Definition> {
    let (jobs, env) = parse(document)?;
    Ok(Definition {
        workdir: "/tmp/actions-{{run}}".into(),
        env,
        steps: jobs
            .into_iter()
            .flat_map(|job| {
                job.steps.into_iter().map(move |step| Step {
                    name: format!("{} / {}", job.name, step.name)
                        .chars()
                        .take(255)
                        .collect(),
                    run: String::new(),
                    checkout: false,
                    timeout_seconds: None,
                })
            })
            .collect(),
        artifacts: vec![],
    })
}
pub async fn host(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    kind: &str,
    reference: Id,
) -> Result<i64> {
    let query = hosts::Entity::find().filter(hosts::Column::WorkspaceId.eq(workspace));
    let row = match kind {
        "host" => {
            query
                .filter(hosts::Column::Id.eq(reference))
                .one(conn)
                .await?
        }
        "host-group" => {
            if host_groups::Entity::find_by_id(i64::from(reference))
                .filter(host_groups::Column::WorkspaceId.eq(workspace))
                .one(conn)
                .await?
                .is_none()
            {
                return Err(ApiError::bad("执行主机组不存在于当前工作空间。"));
            }
            query
                .filter(hosts::Column::GroupId.eq(reference))
                .filter(hosts::Column::State.eq("online"))
                .order_by_asc(hosts::Column::Id)
                .one(conn)
                .await?
        }
        _ => return Err(ApiError::bad("执行器类型无效。")),
    }
    .ok_or_else(|| ApiError::bad("当前工作空间没有可用的执行主机。"))?;
    Ok(row.id)
}
pub async fn check_bindings(
    s: &crate::state::AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    document: &Document,
) -> Result<()> {
    parse(document)?;
    for binding in &document.checkout_bindings {
        crate::repository_auth::resolve_connection(
            s,
            conn,
            scope,
            Id::reference(&binding.connection_id)?,
            &binding.repository_url,
        )
        .await?;
    }
    for binding in &document.executor_bindings {
        if !binding.target_ref.is_empty() {
            let reference = Id::reference(&binding.target_ref)?;
            if binding.target_kind == "host-group" {
                if host_groups::Entity::find_by_id(i64::from(reference))
                    .filter(host_groups::Column::WorkspaceId.eq(scope.workspace_id))
                    .one(conn)
                    .await?
                    .is_none()
                {
                    return Err(ApiError::bad("执行主机组不存在于当前工作空间。"));
                }
            } else {
                host(conn, scope.workspace_id, &binding.target_kind, reference).await?;
            }
        }
    }
    Ok(())
}
pub async fn resolve(
    s: &crate::state::AppState,
    conn: &impl sea_orm::ConnectionTrait,
    scope: &Scope,
    document: Document,
    default_host: i64,
) -> Result<Snapshot> {
    let (mut jobs, _) = parse(&document)?;
    for job in &mut jobs {
        if job.manual {
            continue;
        }
        job.host_id = default_host;
        if let Some(binding) = document
            .executor_bindings
            .iter()
            .find(|b| b.job_id == job.key)
        {
            job.image = binding.image.clone();
            if !binding.target_ref.is_empty() {
                job.host_id = host(
                    conn,
                    scope.workspace_id,
                    &binding.target_kind,
                    Id::reference(&binding.target_ref)?,
                )
                .await?;
            }
        }
    }
    if jobs.iter().any(|job| !job.manual && job.host_id == 0) {
        return Err(ApiError::bad(
            "每个任务需要执行器绑定或所选环境的执行主机。",
        ));
    }
    let mut checkouts = Vec::new();
    for binding in &document.checkout_bindings {
        let (_, credential_id) = crate::repository_auth::resolve_saved_connection(
            s,
            conn,
            scope,
            Id::reference(&binding.connection_id)?,
            &binding.repository_url,
        )
        .await?;
        checkouts.push(ResolvedCheckout {
            binding: binding.clone(),
            credential_id: credential_id.into(),
        });
    }
    Ok(Snapshot {
        checkouts,
        engine: ENGINE.into(),
        document,
        jobs,
    })
}
pub fn display(row: &mut Value, definition: &str) {
    if let Some(document) = document(definition) {
        row["definition_format"] = json!(ENGINE);
        row["definition"] = json!(document.workflow);
        row["executor_bindings"] = json!(document.executor_bindings);
        row["checkout_bindings"] = json!(document.checkout_bindings);
        row["graph_layout"] = document.graph_layout;
    } else {
        row["definition_format"] = json!("legacy");
        row["executor_bindings"] = json!([]);
        row["checkout_bindings"] = json!([]);
        row["graph_layout"] = json!({});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(workflow: &str) -> Document {
        Document {
            engine: ENGINE.into(),
            workflow: workflow.into(),
            executor_bindings: vec![],
            checkout_bindings: vec![],
            graph_layout: json!({}),
        }
    }
    #[test]
    fn manual_approval_has_no_executor_or_steps() {
        let workflow = "jobs:\n  gate:\n    name: 发布确认\n    ferry-approval: true\n  deploy:\n    needs: gate\n    runs-on: self-hosted\n    steps:\n      - run: echo deploy\n";
        let (jobs, _) = parse(&doc(workflow)).unwrap();
        let gate = jobs.iter().find(|job| job.key == "gate").unwrap();
        let deploy = jobs.iter().find(|job| job.key == "deploy").unwrap();
        assert!(gate.manual);
        assert!(gate.steps.is_empty());
        assert_eq!(gate.host_id, 0);
        assert_eq!(deploy.needs, vec!["gate"]);
        for invalid in [
            workflow.replace("ferry-approval: true", "ferry-approval: false"),
            workflow.replace(
                "ferry-approval: true",
                "ferry-approval: true\n    runs-on: self-hosted",
            ),
            workflow.replace(
                "ferry-approval: true",
                "ferry-approval: true\n    steps: [{run: echo bad}]",
            ),
        ] {
            assert!(parse(&doc(&invalid)).is_err());
        }
    }
    #[test]
    fn rejects_unexecutable_actions_before_enqueue() {
        for workflow in [
            "jobs: {123bad: {runs-on: self-hosted, steps: [{run: echo ok}]}}",
            "jobs: {build: {uses: 'owner/repo/path'}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{run: echo ok, timeout-minutes: 0}]}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{run: \"bad\\0command\"}]}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{uses: 'docker://alpine:3.21'}]}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{id: same, run: one}, {id: same, run: two}]}}",
        ] {
            assert!(parse(&doc(workflow)).is_err(), "accepted {workflow}");
        }
    }

    #[test]
    fn checkout_bindings_require_stable_ids_and_workspace_relative_paths() {
        let mut document = doc(
            "jobs: {build: {runs-on: self-hosted, steps: [{id: source, uses: 'actions/checkout@v4'}]}}",
        );
        let binding = CheckoutBinding {
            job_id: "build".into(),
            step_id: "source".into(),
            connection_id: "1".into(),
            repository_url: "https://example.invalid/project.git".into(),
            git_ref: "main".into(),
            path: "src".into(),
        };
        document.checkout_bindings = vec![binding.clone()];
        assert!(parse(&document).is_ok());
        for path in [
            "../outside",
            "/outside",
            "src/../../outside",
            "${{ github.workspace }}",
        ] {
            document.checkout_bindings[0].path = path.into();
            assert!(parse(&document).is_err(), "accepted {path}");
        }
        document.checkout_bindings = vec![binding.clone(), binding.clone()];
        assert!(parse(&document).is_err());
        document.checkout_bindings = vec![binding];
        document.checkout_bindings[0].step_id = "missing".into();
        assert!(parse(&document).is_err());
    }

    #[test]
    fn bound_checkout_rejects_unsupported_or_conflicting_action_inputs() {
        let mut document = doc(
            "jobs: {build: {runs-on: self-hosted, steps: [{id: source, uses: 'actions/checkout@v4', with: {ref: main}}]}}",
        );
        document.checkout_bindings = vec![CheckoutBinding {
            job_id: "build".into(),
            step_id: "source".into(),
            connection_id: "1".into(),
            repository_url: "https://example.invalid/project.git".into(),
            git_ref: "main".into(),
            path: "".into(),
        }];
        assert!(parse(&document).is_ok());
        document.workflow = document.workflow.replace("ref: main", "ref: other");
        assert!(parse(&document).is_err());
        document.workflow = document
            .workflow
            .replace("ref: other", "submodules: recursive");
        assert!(parse(&document).is_err());
        document.checkout_bindings.clear();
        assert!(parse(&document).is_ok());
    }

    #[test]
    fn rejects_cycles_unknown_bindings_and_policies() {
        let base =
            "jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo hello\n";
        assert!(parse(&doc(base)).is_ok());
        assert!(
            parse(&doc(
                &base.replace("    runs-on:", "    needs: build\n    runs-on:")
            ))
            .is_err()
        );
        assert!(parse(&doc(&format!("concurrency: production\n{base}"))).is_ok());
        let mut document = doc(base);
        document.executor_bindings.push(Binding {
            job_id: "missing".into(),
            image: "host".into(),
            ..Default::default()
        });
        assert!(parse(&document).is_err());
    }
    #[test]
    fn preserves_original_yaml_and_layout_for_api() {
        let mut document = doc("# original comment\njobs: {}\n");
        document.graph_layout = json!({"build":{"x":17,"y":29}});
        let mut api = json!({});
        display(&mut api, &serde_json::to_string(&document).unwrap());
        assert_eq!(api["definition"], document.workflow);
        assert_eq!(api["graph_layout"], document.graph_layout);
        assert_eq!(api["definition_format"], ENGINE);
    }
    #[test]
    fn static_secret_references_and_dynamic_contexts_are_discovered() {
        let workflow = "jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo '${{ secrets.TOKEN }}'\n";
        let (_, env) = parse(&doc(workflow)).unwrap();
        assert_eq!(env[0].value_from.as_deref(), Some("variable/TOKEN"));
        let (_, env) = parse(&doc(&workflow.replace("secrets.TOKEN", "secrets['TOKEN']"))).unwrap();
        assert_eq!(env.len(), 1);
        let dynamic = doc(&workflow.replace("secrets.TOKEN", "secrets[matrix.key]"));
        assert!(parse(&dynamic).is_ok());
        let discovered = references(&dynamic).unwrap();
        assert!(discovered.secrets.deliver_all);
        assert!(discovered.secrets.names.is_empty());
        assert!(!discovered.vars.deliver_all);
    }

    #[test]
    fn compatibility_caps_are_removed_but_positive_timeouts_are_required() {
        let key = "j".repeat(200);
        let mut workflow = format!(
            "# {}\njobs:\n  {key}:\n    runs-on: self-hosted\n    steps:\n",
            "x".repeat(140 * 1024)
        );
        for _ in 0..60 {
            workflow.push_str("      - run: echo ok\n        timeout-minutes: 2000\n");
        }
        for n in 0..60 {
            workflow.push_str(&format!(
                "  job{n}: {{runs-on: self-hosted, steps: [{{run: echo ok}}]}}\n"
            ));
        }
        let (jobs, _) = parse(&doc(&workflow)).unwrap();
        assert_eq!(jobs.len(), 61);
        assert_eq!(jobs.iter().find(|j| j.key == key).unwrap().steps.len(), 60);
        for timeout in ["0", "-1", "1.5", "false", "invalid"] {
            assert!(
                parse(&doc(&workflow.replace("2000", timeout))).is_err(),
                "{timeout}"
            );
        }
        assert!(parse(&doc(&workflow.replace("2000", "'${{ inputs.timeout }}'"))).is_ok());
        assert!(parse(&doc("jobs: [malformed")).is_err());
        assert!(parse(&doc(&workflow.replace("echo ok", "echo \0"))).is_err());
    }

    #[test]
    fn reusable_jobs_keep_graph_binding_and_snapshot_metadata() {
        let workflow = "jobs:\n  local:\n    uses: ./.github/workflows/shared.yml\n    with: {flag: true}\n    secrets: inherit\n  remote:\n    needs: local\n    uses: owner/repo/.github/workflows/shared.yml@feature/ref\n    secrets: {token: '${{ secrets.TOKEN }}'}\n";
        let mut document = doc(workflow);
        document.executor_bindings.push(Binding {
            job_id: "remote".into(),
            image: "alpine".into(),
            ..Default::default()
        });
        let (jobs, _) = parse(&document).unwrap();
        assert!(jobs.iter().all(Job::reusable));
        assert!(jobs.iter().all(|j| j.steps.is_empty()));
        assert_eq!(jobs[1].needs, ["local"]);
        let snapshot = Snapshot {
            engine: ENGINE.into(),
            document: document.clone(),
            jobs,
            checkouts: vec![],
        };
        let serialized = serde_json::to_string(&snapshot).unwrap();
        let value: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(value["jobs"][0]["reusable"], true);
        assert!(super::snapshot(&serialized).unwrap().jobs[0].reusable());
        assert!(references(&document).unwrap().secrets.deliver_all);
        assert_eq!(
            job_policies(&document, "remote")
                .unwrap()
                .reusable_workflow
                .as_deref(),
            Some("owner/repo/.github/workflows/shared.yml@feature/ref")
        );
        for invalid in [
            "owner/repo/path",
            "repo@v1",
            "owner//path@v1",
            "owner/repo/path@",
            "https://example/repo/path@v1",
            "./../escape.yml",
        ] {
            assert!(
                parse(&doc(&format!("jobs: {{call: {{uses: '{invalid}'}}}}"))).is_err(),
                "{invalid}"
            );
        }
        for field in ["steps: [{run: echo bad}]", "runs-on: self-hosted"] {
            assert!(
                parse(&doc(&format!(
                    "jobs: {{call: {{uses: './shared.yml', {field}}}}}"
                )))
                .is_err()
            );
        }
        assert!(parse(&doc(&workflow.replace("needs: local", "needs: remote"))).is_err());
        assert!(parse(&doc(&workflow.replace("needs: local", "needs: missing"))).is_err());
        document
            .executor_bindings
            .push(document.executor_bindings[0].clone());
        assert!(parse(&document).is_err());
    }

    #[test]
    fn runner_objects_dynamic_labels_and_services_respect_bindings() {
        let mut document = doc(
            "jobs: {build: {runs-on: {group: '${{ vars.GROUP }}', labels: [self-hosted, '${{ matrix.os }}']}, services: {db: {image: postgres}}, steps: [{uses: 'actions/cache@v4'}]}}",
        );
        assert_eq!(
            parse(&document).unwrap_err().1,
            "services 需要容器执行，请把该任务绑定到容器镜像"
        );
        document.executor_bindings.push(Binding {
            job_id: "build".into(),
            image: "ubuntu".into(),
            ..Default::default()
        });
        let (jobs, _) = parse(&document).unwrap();
        assert_eq!(
            jobs[0].labels,
            ["self-hosted", "${{ matrix.os }}", "${{ vars.GROUP }}"]
        );
        document.executor_bindings[0].image = "host".into();
        document.workflow = document
            .workflow
            .replace("services:", "container: ubuntu, services:");
        assert!(parse(&document).is_err());
        document.executor_bindings.clear();
        assert!(parse(&document).is_ok());
    }

    #[test]
    fn tokenizer_handles_quotes_implicit_conditions_and_nested_indexes() {
        let value = json!({
            "if": "contains('it''s }} secrets.FAKE', vars.FLAG) && secrets['TOKEN'] != ''",
            "run": "${{ format('}} vars.FAKE {{', secrets.REAL) }} ${{ secrets[format('{0}', vars[inputs.key])] }}",
            "env": {"ALL": "${{ toJSON(vars) }}", "NOT_ROOT": "${{ inputs.secrets.FAKE }}", "ESCAPED": "${{ secrets['it''s'] }}"}
        });
        let found = discover_references(&value).unwrap();
        assert_eq!(
            found.secrets.names,
            BTreeSet::from(["TOKEN".into(), "REAL".into(), "it's".into()])
        );
        assert_eq!(found.vars.names, BTreeSet::from(["FLAG".into()]));
        assert!(found.secrets.deliver_all);
        assert!(found.vars.deliver_all);
        for invalid in [
            "${{ secrets['TOKEN' }}",
            "${{ 'unclosed }}",
            "${{ secrets.TOKEN",
        ] {
            assert!(discover_references(&json!(invalid)).is_err(), "{invalid}");
        }
        assert!(
            !discover_references(&json!({"secrets":"inherit"}))
                .unwrap()
                .secrets
                .deliver_all
        );
    }

    #[test]
    fn exposes_raw_policies_and_typed_dispatch_input_defaults() {
        let document = doc(r#"
on:
  workflow_dispatch:
    inputs:
      flag: {type: boolean, required: true, default: false, description: Enable}
      count: {type: number, default: 0}
      target: {type: choice, options: [staging, production], default: staging}
      text: {description: Optional}
      env: {type: environment}
concurrency:
  group: ${{ github.ref }}
  cancel-in-progress: ${{ inputs.flag }}
jobs:
  build:
    runs-on: ${{ inputs.target }}
    concurrency: {group: '${{ vars.GROUP }}', cancel-in-progress: true}
    environment: {name: '${{ inputs.env }}', url: '${{ steps.publish.outputs.url }}'}
    steps: [{run: echo ok}]
"#);
        assert!(parse(&document).is_ok());
        assert_eq!(
            concurrency(&document),
            Some(Concurrency {
                group: "${{ github.ref }}".into(),
                cancel_in_progress: json!("${{ inputs.flag }}")
            })
        );
        let policies = job_policies(&document, "build").unwrap();
        assert_eq!(policies.concurrency.unwrap()["cancel-in-progress"], true);
        assert_eq!(policies.environment.unwrap()["name"], "${{ inputs.env }}");
        let inputs = dispatch_inputs(&document);
        assert_eq!(inputs.len(), 5);
        let flag = inputs.iter().find(|i| i.name == "flag").unwrap();
        assert_eq!(flag.r#type, "boolean");
        assert!(flag.required);
        assert_eq!(flag.default, Some(json!(false)));
        assert_eq!(flag.description, "Enable");
        assert_eq!(
            inputs.iter().find(|i| i.name == "count").unwrap().default,
            Some(json!(0))
        );
        assert_eq!(
            inputs.iter().find(|i| i.name == "target").unwrap().options,
            ["staging", "production"]
        );
        let text = inputs.iter().find(|i| i.name == "text").unwrap();
        assert_eq!(text.r#type, "string");
        assert!(!text.required);
        assert_eq!(text.default, None);
        assert!(dispatch_inputs(&doc("on: workflow_dispatch")).is_empty());
        assert_eq!(
            concurrency(&doc("concurrency: deploy")),
            Some(Concurrency {
                group: "deploy".into(),
                cancel_in_progress: json!(false)
            })
        );
        assert!(concurrency(&doc("jobs: {}")).is_none());
        assert!(job_policies(&document, "missing").is_none());
    }
}

/// Binding metadata must participate in the same delete protections as the
/// pipeline's default target; active snapshots keep their resolved hosts too.
pub async fn ensure_target_unreferenced(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    kind: &str,
    id: Id,
) -> Result<()> {
    use crate::entities::{pipeline_runs, pipelines};
    for pipeline in pipelines::Entity::find()
        .filter(pipelines::Column::WorkspaceId.eq(workspace))
        .all(conn)
        .await?
    {
        if let Some(document) = document(&pipeline.definition)
            && document.executor_bindings.iter().any(|binding| {
                binding.target_kind == kind
                    && Id::reference(&binding.target_ref).is_ok_and(|reference| reference == id)
            })
        {
            return Err(ApiError::conflict(
                "执行器仍被 Actions 任务引用，请先修改任务绑定。",
            ));
        }
    }
    if kind == "host" {
        for run in pipeline_runs::Entity::find()
            .filter(pipeline_runs::Column::WorkspaceId.eq(workspace))
            .filter(pipeline_runs::Column::State.is_in(["queued", "running", "cancelling"]))
            .all(conn)
            .await?
        {
            if let Some(snapshot) = snapshot(&run.definition)
                && snapshot.jobs.iter().any(|job| job.host_id == i64::from(id))
            {
                return Err(ApiError::conflict("执行器仍有 Actions 任务尚未结束。"));
            }
        }
    }
    Ok(())
}
