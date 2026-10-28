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
use std::collections::{BTreeMap, BTreeSet};

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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub key: String,
    pub name: String,
    pub needs: Vec<String>,
    pub labels: Vec<String>,
    pub steps: Vec<JobStep>,
    pub host_id: i64,
    pub image: String,
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
    if document.engine != ENGINE || document.workflow.len() > 128 * 1024 {
        return Err(ApiError::bad("Actions YAML 无效或超过 128KB。"));
    }
    let value: Value = serde_yaml_ng::from_str(&document.workflow)
        .map_err(|e| ApiError::bad(format!("Actions YAML 格式错误：{e}。")))?;
    let jobs = value
        .get("jobs")
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty() && m.len() <= 50)
        .ok_or_else(|| ApiError::bad("Actions 工作流需要 1 到 50 个任务。"))?;
    // These service-level policies cannot be silently ignored by an independent engine.
    if value.get("concurrency").is_some() {
        return Err(ApiError::bad(
            "当前尚不支持 Actions concurrency 策略，请移除后执行。",
        ));
    }
    let mut index = 0i64;
    let mut result = Vec::new();
    for (key, raw) in jobs {
        if !key
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || key.len() > 100
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ApiError::bad("Actions 任务标识无效。"));
        }
        if raw.get("concurrency").is_some() || raw.get("environment").is_some() {
            return Err(ApiError::bad(format!(
                "任务 {key} 的 concurrency / environment 策略尚未支持，不能静默忽略。"
            )));
        }
        if raw.get("uses").is_some() {
            return Err(ApiError::bad(
                "当前任务调度不支持复用工作流，请将任务展开为 jobs/steps。",
            ));
        }
        let container = raw.get("container").is_some()
            || document
                .executor_bindings
                .iter()
                .any(|binding| binding.job_id == *key && binding.image != "host");
        let mut step_ids = BTreeSet::new();
        let labels = string_list(raw.get("runs-on"), "runs-on")?;
        if labels.is_empty() && raw.get("uses").is_none() {
            return Err(ApiError::bad(format!("任务 {key} 缺少 runs-on。")));
        }
        if labels.iter().any(|label| label.contains("${{")) {
            return Err(ApiError::bad(
                "当前执行器绑定需要静态 runs-on 标签，暂不支持动态切换执行主机。",
            ));
        }
        let needs = string_list(raw.get("needs"), "needs")?;
        let steps = match raw.get("steps") {
            Some(Value::Array(steps)) if !steps.is_empty() && steps.len() <= 50 => steps
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
                        if command.starts_with("actions/cache@")
                            || command.starts_with("actions/cache/")
                        {
                            return Err(ApiError::bad(
                                "当前尚未提供 Actions 缓存服务，不能执行 actions/cache。",
                            ));
                        }
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
                        if !expression && !number.is_some_and(|value| (1..=1440).contains(&value)) {
                            return Err(ApiError::bad(
                                "步骤 timeout-minutes 必须为 1–1440 分钟或 Actions 表达式。",
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
            None if raw.get("uses").is_some() => vec![],
            _ => {
                return Err(ApiError::bad(format!(
                    "任务 {key} 需要 1 到 50 个执行步骤。"
                )));
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
    let mut variables = BTreeMap::new();
    fn references(value: &Value, variables: &mut BTreeMap<String, Environment>) -> Result<()> {
        match value {
            Value::String(s) => {
                for tail in s.split("${{").skip(1) {
                    let expr = tail.split("}}").next().unwrap_or("");
                    let mut rest = expr;
                    while !rest.is_empty() {
                        if rest.starts_with('\'') {
                            rest = &rest[1..];
                            loop {
                                let Some(end) = rest.find('\'') else {
                                    return Err(ApiError::bad("Actions 表达式字符串未闭合。"));
                                };
                                rest = &rest[end + 1..];
                                if rest.starts_with('\'') {
                                    rest = &rest[1..];
                                } else {
                                    break;
                                }
                            }
                            continue;
                        }
                        let length = rest
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                            .count();
                        if length == 0 {
                            rest = &rest[rest.chars().next().unwrap().len_utf8()..];
                            continue;
                        }
                        let member = expr[..expr.len() - rest.len()].trim_end().ends_with('.');
                        let token = &rest[..length];
                        rest = &rest[length..];
                        if member || !["secrets", "vars"].contains(&token) {
                            continue;
                        }
                        rest = rest.trim_start();
                        let name;
                        if let Some(tail) = rest.strip_prefix('.') {
                            rest = tail.trim_start();
                            let end = rest
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .count();
                            name = &rest[..end];
                            rest = &rest[end..];
                        } else if let Some(tail) = rest.strip_prefix('[') {
                            let tail = tail.trim_start().strip_prefix('\'').ok_or_else(|| {
                                ApiError::bad("密钥及变量引用必须使用静态名称，暂不支持动态索引。")
                            })?;
                            let end = tail
                                .find('\'')
                                .ok_or_else(|| ApiError::bad("变量引用缺少结束引号。"))?;
                            name = &tail[..end];
                            rest = tail[end + 1..]
                                .trim_start()
                                .strip_prefix(']')
                                .ok_or_else(|| ApiError::bad("变量引用缺少结束括号。"))?;
                        } else {
                            return Err(ApiError::bad("密钥及变量引用必须指定静态名称。"));
                        }
                        if !crate::variables::valid_name(name) {
                            return Err(ApiError::bad("变量名称无效。"));
                        }
                        let key = format!("{token}:{name}");
                        variables.insert(
                            key.clone(),
                            Environment {
                                name: key,
                                value: None,
                                value_from: Some(format!("variable/{name}")),
                            },
                        );
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    references(item, variables)?;
                }
            }
            Value::Object(items) => {
                for item in items.values() {
                    references(item, variables)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    references(&value, &mut variables)?;
    Ok((result, variables.into_values().collect()))
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
    if jobs.iter().any(|job| job.host_id == 0) {
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
    fn rejects_unexecutable_actions_before_enqueue() {
        for workflow in [
            "jobs: {123bad: {runs-on: self-hosted, steps: [{run: echo ok}]}}",
            "jobs: {build: {uses: './.github/workflows/shared.yml'}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{run: echo ok, timeout-minutes: 0}]}}",
            "jobs: {build: {runs-on: self-hosted, steps: [{uses: 'actions/cache@v4'}]}}",
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
        assert!(parse(&doc(&format!("concurrency: production\n{base}"))).is_err());
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
    fn static_secret_references_are_snapshotted_but_dynamic_ones_fail() {
        let workflow = "jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo '${{ secrets.TOKEN }}'\n";
        let (_, env) = parse(&doc(workflow)).unwrap();
        assert_eq!(env[0].value_from.as_deref(), Some("variable/TOKEN"));
        let (_, env) = parse(&doc(&workflow.replace("secrets.TOKEN", "secrets['TOKEN']"))).unwrap();
        assert_eq!(env.len(), 1);
        assert!(
            parse(&doc(
                &workflow.replace("secrets.TOKEN", "secrets[matrix.key]")
            ))
            .is_err()
        );
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
