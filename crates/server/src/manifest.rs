//! Application intent -> deterministic Compose document. No I/O or interpolation.
use crate::error::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Manifest {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub services: Vec<Service>,
    #[serde(default)]
    pub strategy: Option<Strategy>,
    #[serde(default)]
    pub overrides: Overrides,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<BuildArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ConfigFile>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default = "one")]
    pub replicas: u32,
    #[serde(default)]
    pub ports: Vec<Port>,
    #[serde(default)]
    pub env: Vec<Environment>,
    #[serde(default)]
    pub volumes: Vec<Volume>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildArtifact {
    pub source: String,
    pub pipeline: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConfigFile {
    pub path: String,
    pub from_template: String,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    #[serde(default)]
    pub readiness: Option<Probe>,
    #[serde(default)]
    pub liveness: Option<Probe>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Probe {
    pub http_get: HttpGet,
    #[serde(default = "probe_period")]
    pub period_seconds: u32,
    #[serde(default = "one")]
    pub timeout_seconds: u32,
    #[serde(default = "probe_failures")]
    pub failure_threshold: u32,
}
fn probe_period() -> u32 {
    10
}
fn probe_failures() -> u32 {
    3
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpGet {
    pub path: String,
    pub port: Value,
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn probe_command(service: &Service, probe: &Probe) -> Result<String> {
    let port = probe
        .http_get
        .port
        .as_u64()
        .filter(|p| (1..=65535).contains(p))
        .or_else(|| {
            let name = probe.http_get.port.as_str()?;
            service
                .ports
                .iter()
                .find(|p| p.name.as_deref() == Some(name))
                .map(|p| u64::from(p.container))
        })
        .ok_or_else(|| bad("健康检查端口必须是1到65535的端口号或已声明的端口名称。"))?;
    if !probe.http_get.path.starts_with('/')
        || probe.http_get.path.starts_with("//")
        || probe.http_get.path.len() > 2048
        || probe
            .http_get
            .path
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || probe.http_get.path.contains('#')
        || !(1..=3600).contains(&probe.period_seconds)
        || !(1..=600).contains(&probe.timeout_seconds)
        || !(1..=100).contains(&probe.failure_threshold)
    {
        return Err(bad("健康检查路径或周期/超时/失败阈值无效。"));
    }
    let url = shell_quote(&format!("http://127.0.0.1:{port}{}", probe.http_get.path));
    Ok(format!(
        "if command -v curl >/dev/null 2>&1; then curl --noproxy '*' -fsS --max-time {} -- {url} >/dev/null; else http_proxy= https_proxy= HTTP_PROXY= HTTPS_PROXY= wget -q -T {} -O /dev/null {url}; fi",
        probe.timeout_seconds, probe.timeout_seconds
    ))
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    #[serde(default)]
    pub requests: BTreeMap<String, Value>,
    #[serde(default)]
    pub limits: BTreeMap<String, Value>,
}
fn quantity(value: &Value, cpu: bool) -> Result<u64> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(bad("资源数量必须是数字或数量字符串。")),
    };
    let (number, multiplier) = if cpu {
        match text.strip_suffix('m') {
            Some(n) => (n, 1_000_000u64),
            None => (text.as_str(), 1_000_000_000),
        }
    } else {
        let units = [
            ("Ki", 1024u64),
            ("Mi", 1024u64.pow(2)),
            ("Gi", 1024u64.pow(3)),
            ("Ti", 1024u64.pow(4)),
            ("K", 1000),
            ("M", 1000u64.pow(2)),
            ("G", 1000u64.pow(3)),
            ("T", 1000u64.pow(4)),
        ];
        units
            .iter()
            .find_map(|(unit, n)| text.strip_suffix(unit).map(|v| (v, *n)))
            .unwrap_or((text.as_str(), 1))
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
    {
        return Err(bad(
            "资源数量必须为正数，CPU可使用m，内存可使用Ki/Mi/Gi/Ti或K/M/G/T。",
        ));
    }
    let scale = 10u128.pow(fraction.len() as u32);
    let numerator = whole
        .parse::<u128>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .and_then(|n| fraction.parse::<u128>().unwrap_or(0).checked_add(n));
    let scaled = numerator
        .and_then(|n| n.checked_mul(u128::from(multiplier)))
        .filter(|n| *n > 0 && n % scale == 0)
        .and_then(|n| u64::try_from(n / scale).ok());
    scaled.ok_or_else(|| bad("资源数量过大、为零或精度超出支持范围。"))
}
fn resource_values(values: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for (name, value) in values {
        match name.as_str() {
            "cpu" => {
                let nanos = quantity(value, true)?;
                out.insert(
                    "cpus".into(),
                    json!(format!(
                        "{}.{:09}",
                        nanos / 1_000_000_000,
                        nanos % 1_000_000_000
                    )),
                );
            }
            "memory" => {
                out.insert("memory".into(), json!(quantity(value, false)?.to_string()));
            }
            _ => return Err(bad("resources 仅支持 cpu 和 memory。")),
        }
    }
    Ok(out)
}
fn validate_resources(resources: &Resources) -> Result<()> {
    resource_values(&resources.requests)?;
    resource_values(&resources.limits)?;
    for name in ["cpu", "memory"] {
        if let (Some(request), Some(limit)) =
            (resources.requests.get(name), resources.limits.get(name))
        {
            if quantity(request, name == "cpu")? > quantity(limit, name == "cpu")? {
                return Err(bad("资源 requests 不能大于 limits。"));
            }
        }
    }
    Ok(())
}
fn one() -> u32 {
    1
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Environment {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_from: Option<String>,
}
impl Manifest {
    pub fn references(&self) -> BTreeMap<(String, String), String> {
        self.services
            .iter()
            .flat_map(|service| {
                service.env.iter().filter_map(|env| {
                    env.value_from.as_ref().map(|reference| {
                        (
                            (service.name.clone(), env.name.clone()),
                            reference
                                .strip_prefix("variable/")
                                .unwrap_or(reference)
                                .to_owned(),
                        )
                    })
                })
            })
            .collect()
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    #[serde(default)]
    pub name: Option<String>,
    pub container: u16,
    #[serde(default)]
    pub published: Option<u16>,
    #[serde(default = "tcp")]
    pub protocol: String,
}
fn tcp() -> String {
    "tcp".into()
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    pub name: String,
    pub path: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Strategy {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "health_timeout")]
    pub health_timeout: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_unavailable: Option<u32>,
}
fn health_timeout() -> u32 {
    120
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default)]
    pub compose: BTreeMap<String, Value>,
    #[serde(default)]
    pub docker: BTreeMap<String, Value>,
    #[serde(default)]
    pub native: BTreeMap<String, Value>,
    #[serde(default)]
    pub stack: BTreeMap<String, Value>,
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::bad(message)
}
fn identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 63
        && v.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && v.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}
fn environment_name(v: &str) -> bool {
    let mut chars = v.bytes();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
/// Deserialize directly to typed structs: duplicate keys and unknown fields are errors.
pub fn parse(input: &str) -> Result<Manifest> {
    parse_for(input, "auto")
}
pub fn parse_for(input: &str, mode: &str) -> Result<Manifest> {
    if input.len() > 64 * 1024 || input.contains('\0') {
        return Err(bad("manifest 不能超过64KB或包含空字符。"));
    }
    let m: Manifest = serde_yaml_ng::from_str(input).map_err(|e| {
        bad(format!(
            "manifest YAML/JSON 无效：{e}。字段不符合应用描述格式；请删除未实现的字段。"
        ))
    })?;
    let mode = effective_mode(&m, mode);
    if m.api_version != "ferry/v1" || m.kind != "Application" {
        return Err(bad(
            "manifest 必须使用 apiVersion: ferry/v1 和 kind: Application。",
        ));
    }
    if !identifier(&m.metadata.name) || m.services.is_empty() || m.services.len() > 50 {
        return Err(bad(
            "应用名必须是小写标识符，services 必须包含1到50个服务。",
        ));
    }
    if let Some(s) = &m.strategy {
        if s.kind != "recreate" && !(mode == "stack" && s.kind == "rolling") {
            return Err(bad(
                "Compose 不支持滚动更新；strategy.type 仅支持 recreate。",
            ));
        }
        if s.max_unavailable.is_some_and(|n| n > 100)
            || (mode != "stack" && s.max_unavailable.is_some())
        {
            return Err(bad("maxUnavailable仅用于Swarm，范围0到100。"));
        }
        if !(1..=600).contains(&s.health_timeout) {
            return Err(bad("healthTimeout 必须为1到600秒的整数。"));
        }
    }
    let mut names = BTreeSet::new();
    for s in &m.services {
        if !identifier(&s.name) || !names.insert(&s.name) {
            return Err(bad("服务名必须是唯一的小写标识符。"));
        }
        if let Some(resources) = &s.resources {
            validate_resources(resources)?;
        }
        if s.image.is_empty() == s.artifact.is_none() {
            return Err(bad("每个服务必须且只能指定image或artifact。"));
        }
        if let Some(artifact) = &s.artifact {
            if artifact.source != "pipeline" || artifact.pipeline.trim().is_empty() {
                return Err(bad("artifact必须引用构建流水线。"));
            }
            crate::artifacts::validate_paths(&[artifact.path.clone()])?;
            if artifact
                .run
                .as_ref()
                .is_some_and(|r| r.parse::<crate::snowflake::Id>().is_err())
            {
                return Err(bad("artifact.run必须是构建执行ID。"));
            }
        }
        if s.replicas == 0 || s.replicas > 100 {
            return Err(bad("replicas 必须在1到100之间。"));
        }
        let mut env = BTreeSet::new();
        for e in &s.env {
            if e.value.is_some() == e.value_from.is_some() {
                return Err(bad("环境变量必须且只能填写 value 或 valueFrom。"));
            }
            if e.value_from.as_deref().is_some_and(|reference| {
                !reference
                    .strip_prefix("variable/")
                    .is_some_and(crate::variables::valid_name)
            }) {
                return Err(bad("valueFrom 必须是 variable/变量名。"));
            }
            if !environment_name(&e.name) || !env.insert(&e.name) {
                return Err(bad("环境变量名无效或重复。"));
            }
        }
        let mut paths = BTreeSet::new();
        if s.files.len() > 20 {
            return Err(bad("每个服务最多挂载20个配置文件。"));
        }
        for file in &s.files {
            if crate::channel::check_path(&file.path).is_err()
                || file.path == "/"
                || !paths.insert(&file.path)
                || file.from_template.trim().is_empty()
                || file.from_template.chars().count() > 60
            {
                return Err(bad("配置文件路径无效、重复或模板引用无效。"));
            }
        }
        for v in &s.volumes {
            if !identifier(&v.name)
                || crate::channel::check_path(&v.path).is_err()
                || !paths.insert(&v.path)
            {
                return Err(bad("卷名无效、挂载路径不是绝对路径或重复。"));
            }
        }
    }
    fn contains_nul(value: &Value) -> bool {
        match value {
            Value::String(s) => s.contains('\0'),
            Value::Array(items) => items.iter().any(contains_nul),
            Value::Object(items) => items
                .iter()
                .any(|(key, value)| key.contains('\0') || contains_nul(value)),
            _ => false,
        }
    }
    if contains_nul(&serde_json::to_value(&m).map_err(|_| bad("manifest 无法序列化。"))?) {
        return Err(bad("manifest 字段不能包含空字符。"));
    }
    // Validating the effective document also covers changes made by overrides.
    render_for(&m, mode)?;
    Ok(m)
}

fn merge(base: &mut Value, overlay: &Value) {
    if let (Some(base), Some(overlay)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay {
            merge(base.entry(key.clone()).or_insert(Value::Null), value);
        }
    } else {
        *base = overlay.clone();
    }
}
pub(crate) fn escape(value: &mut Value) {
    match value {
        Value::String(s) => *s = s.replace('$', "$$"),
        Value::Array(items) => items.iter_mut().for_each(escape),
        Value::Object(items) => items.values_mut().for_each(escape),
        _ => {}
    }
}

/// JSON is a YAML subset accepted by Compose; sorted maps yield stable bytes.
/// Dollar signs are always literal input, never host environment substitutions.
#[cfg(test)]
pub fn render(m: &Manifest) -> Result<String> {
    render_for(m, "compose")
}
fn effective_mode<'a>(m: &Manifest, mode: &'a str) -> &'a str {
    if mode != "auto" {
        return mode;
    }
    if m.services.iter().any(|s| s.artifact.is_some()) {
        "native"
    } else if !m.overrides.stack.is_empty()
        || m.strategy
            .as_ref()
            .is_some_and(|s| s.kind == "rolling" || s.max_unavailable.is_some())
    {
        "stack"
    } else {
        "compose"
    }
}
pub fn render_for(m: &Manifest, mode: &str) -> Result<String> {
    let mode = effective_mode(m, mode);
    if mode == "native" {
        return render_native(m);
    }
    if m.services.iter().any(|s| s.artifact.is_some()) {
        return Err(bad("容器模式需要image，可执行artifact请使用native模式。"));
    }
    if !["compose", "docker", "stack"].contains(&mode) {
        return Err(bad("部署模式尚未实现。"));
    }
    if mode == "docker" && m.services.iter().any(|s| s.replicas != 1) {
        return Err(bad(
            "Docker模式每台主机的每个服务只能运行一个实例，请使用Compose或主机组。",
        ));
    }
    if mode == "docker"
        && m.services.iter().any(|s| {
            s.resources
                .as_ref()
                .is_some_and(|r| r.requests.contains_key("cpu"))
        })
    {
        return Err(bad(
            "Docker模式不提供CPU资源预留，请使用limits.cpu限制上限。",
        ));
    }

    let mut services = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    for s in &m.services {
        if mode == "stack" && !s.depends_on.is_empty() {
            return Err(bad(
                "Swarm不保证dependsOn启动顺序，请移除该约束并让服务自行重连依赖。",
            ));
        }
        let mut v = json!({"image":s.image,"deploy":{"replicas":s.replicas}});
        if mode == "stack" {
            let rolling = m.strategy.as_ref().is_some_and(|s| s.kind == "rolling");
            let unavailable = m
                .strategy
                .as_ref()
                .and_then(|s| s.max_unavailable)
                .unwrap_or(1);
            v["deploy"]["update_config"] = json!({"parallelism":if rolling {unavailable.max(1)} else {0},"order":if rolling && unavailable==0 {"start-first"} else {"stop-first"},"failure_action":"pause"});
        }
        if let Some(health) = &s.health {
            if health.readiness.is_none() && health.liveness.is_none() {
                return Err(bad("health 至少声明 readiness 或 liveness。"));
            }
            if let Some(probe) = &health.readiness {
                probe_command(s, probe)?;
            }
            if let Some(probe) = health.liveness.as_ref().or(health.readiness.as_ref()) {
                let command = if mode == "stack" && health.readiness.is_some() {
                    let readiness = probe_command(s, health.readiness.as_ref().unwrap())?;
                    let live = health
                        .liveness
                        .as_ref()
                        .map(|p| probe_command(s, p))
                        .transpose()?
                        .unwrap_or_else(|| "true".into());
                    format!(
                        "marker=/tmp/.deployment-ready-$(cat /etc/hostname); if [ ! -f \"$marker\" ]; then ({readiness}) && touch \"$marker\" || exit 1; fi; {live}"
                    )
                } else {
                    probe_command(s, probe)?
                };
                v["healthcheck"] = json!({"test":["CMD-SHELL",command],"interval":format!("{}s",probe.period_seconds),"timeout":format!("{}s",probe.timeout_seconds),"retries":probe.failure_threshold});
            }
        }
        if let Some(resources) = &s.resources {
            validate_resources(resources)?;
            let mut values = serde_json::Map::new();
            if !resources.limits.is_empty() {
                values.insert("limits".into(), json!(resource_values(&resources.limits)?));
            }
            if !resources.requests.is_empty() {
                values.insert(
                    "reservations".into(),
                    json!(resource_values(&resources.requests)?),
                );
            }
            if !values.is_empty() {
                v["deploy"]["resources"] = Value::Object(values);
            }
        }
        if let Some(command) = &s.command {
            v["command"] = json!(command);
        }
        let env: BTreeMap<_, _> = s
            .env
            .iter()
            .filter_map(|e| e.value.as_ref().map(|value| (&e.name, value)))
            .collect();
        if !env.is_empty() {
            v["environment"] = json!(env);
        }
        if !m.metadata.labels.is_empty() {
            v["labels"] = json!(m.metadata.labels);
        }
        if !s.ports.is_empty() {
            v["ports"] = Value::Array(
                s.ports
                    .iter()
                    .map(|p| {
                        let mut v = json!({"target":p.container,"protocol":p.protocol});
                        if let Some(port) = p.published {
                            v["published"] = json!(port.to_string());
                        }
                        v
                    })
                    .collect(),
            );
        }
        if !s.depends_on.is_empty() {
            v["depends_on"] = json!(s.depends_on);
        }
        if !s.volumes.is_empty() {
            v["volumes"] = Value::Array(
                s.volumes
                    .iter()
                    .map(|v| {
                        volumes.insert(v.name.clone(), json!({}));
                        json!({"type":"volume","source":v.name,"target":v.path})
                    })
                    .collect(),
            );
        }
        services.insert(s.name.clone(), v);
    }
    let mut document = json!({"services":services});
    if !volumes.is_empty() {
        document["volumes"] = json!(volumes);
    }
    merge(
        &mut document,
        &json!(if mode == "docker" {
            &m.overrides.docker
        } else if mode == "stack" {
            &m.overrides.stack
        } else {
            &m.overrides.compose
        }),
    );
    validate_compose(&document, mode)?;
    for config in document["services"].as_object_mut().unwrap().values_mut() {
        if let Some(resources) = config["deploy"]
            .get_mut("resources")
            .and_then(Value::as_object_mut)
        {
            for values in resources.values_mut() {
                let input = values
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| {
                        (
                            if key == "cpus" {
                                "cpu".to_owned()
                            } else {
                                key.clone()
                            },
                            value.clone(),
                        )
                    })
                    .collect();
                *values = json!(resource_values(&input)?);
            }
        }
    }
    for ((service, env), _) in m.references() {
        if !document["services"][&service].is_object()
            || document["services"][&service]["environment"]
                .get(&env)
                .is_some()
        {
            return Err(bad(
                "Compose overrides 不得删除引用变量的服务或覆盖 valueFrom 环境项。",
            ));
        }
    }
    let mut references = BTreeMap::<String, BTreeMap<String, String>>::new();
    for ((service, env), name) in m.references() {
        references
            .entry(service)
            .or_default()
            .insert(env, format!("variable/{name}"));
    }
    if !references.is_empty() {
        document["x-variable-references"] = json!(references);
    }
    let configs: BTreeMap<_, _> = m
        .services
        .iter()
        .filter(|s| !s.files.is_empty())
        .map(|s| (&s.name, &s.files))
        .collect();
    if !configs.is_empty() {
        document["x-config-references"] = json!(configs);
    }
    if mode == "docker" {
        for service in document["services"].as_object().unwrap().values() {
            if service["deploy"]["replicas"].as_u64().unwrap_or(1) != 1 {
                return Err(bad("Docker模式每个服务只支持一个实例。"));
            }
            if service["deploy"]["resources"]["reservations"]
                .get("cpus")
                .is_some()
            {
                return Err(bad("Docker模式不支持CPU资源预留。"));
            }
        }
    }
    if mode == "compose" {
        escape(&mut document);
    }
    serde_json::to_string_pretty(&document)
        .map(|s| s + "\n")
        .map_err(|_| bad("无法序列化 Compose 产物。"))
}

fn render_native(m: &Manifest) -> Result<String> {
    let mut services = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    for service in &m.services {
        if service.artifact.is_none() || !service.image.is_empty() || service.replicas != 1 {
            return Err(bad(
                "native模式每个服务只支持一个可执行产物实例；不能使用镜像或多副本。",
            ));
        }
        if service
            .ports
            .iter()
            .any(|p| p.published.is_some_and(|port| port != p.container))
        {
            return Err(bad(
                "native模式不提供端口映射，请让程序直接监听声明的端口。",
            ));
        }
        if service
            .resources
            .as_ref()
            .is_some_and(|r| !r.requests.is_empty())
        {
            return Err(bad("native模式支持资源limits，不提供资源requests预留。"));
        }
        if let Some(health) = &service.health {
            for probe in [&health.readiness, &health.liveness].into_iter().flatten() {
                probe_command(service, probe)?;
            }
        }
        let environment: BTreeMap<_, _> = service
            .env
            .iter()
            .filter_map(|e| e.value.as_ref().map(|v| (&e.name, v)))
            .collect();
        let mounts: Vec<_> = service
            .volumes
            .iter()
            .map(|v| {
                volumes.insert(v.name.clone(), json!({}));
                json!({"type":"volume","source":v.name,"target":v.path})
            })
            .collect();
        if let Some(cpu) = service.resources.as_ref().and_then(|r| r.limits.get("cpu")) {
            if quantity(cpu, true)? % 100_000 != 0 {
                return Err(bad("native CPU限制精度最多为0.0001核（0.01%）。"));
            }
        }
        let limits = service
            .resources
            .as_ref()
            .map(|r| resource_values(&r.limits))
            .transpose()?
            .unwrap_or_default();
        services.insert(service.name.clone(),json!({"artifact":service.artifact,"command":service.command.clone().unwrap_or_default(),"environment":environment,"volumes":mounts,"depends_on":service.depends_on,"limits":limits}));
    }
    let mut remaining: BTreeSet<_> = services.keys().cloned().collect();
    for service in &m.services {
        if service
            .depends_on
            .iter()
            .any(|name| !services.contains_key(name))
        {
            return Err(bad("依赖服务不存在。"));
        }
    }
    while !remaining.is_empty() {
        let ready: Vec<_> = m
            .services
            .iter()
            .filter(|s| {
                remaining.contains(&s.name) && s.depends_on.iter().all(|d| !remaining.contains(d))
            })
            .map(|s| s.name.clone())
            .collect();
        if ready.is_empty() {
            return Err(bad("服务依赖包含循环。"));
        }
        for name in ready {
            remaining.remove(&name);
        }
    }
    if m.overrides.native.keys().any(|key| key != "units") {
        return Err(bad("native overrides仅支持按服务名覆盖units完整文本。"));
    }
    if let Some(units) = m.overrides.native.get("units") {
        let units = units
            .as_object()
            .ok_or_else(|| bad("native.units必须是服务名到unit文本的映射。"))?;
        if units
            .iter()
            .any(|(name, value)| !services.contains_key(name) || !value.is_string())
        {
            return Err(bad("native unit覆盖必须引用已有服务，内容为字符串。"));
        }
    }
    serde_json::to_string_pretty(
        &json!({"services":services,"volumes":volumes,"x-native-overrides":m.overrides.native}),
    )
    .map(|s| s + "\n")
    .map_err(|_| bad("无法渲染native产物。"))
}
pub(crate) fn native_probes(m: &Manifest) -> Result<Vec<(String, String, u32)>> {
    m.services
        .iter()
        .flat_map(|s| {
            s.health.iter().flat_map(move |h| {
                [&h.readiness, &h.liveness]
                    .into_iter()
                    .flatten()
                    .map(move |p| (s, p))
            })
        })
        .map(|(s, p)| Ok((s.name.clone(), probe_command(s, p)?, p.period_seconds)))
        .collect()
}
/// Readiness checks run after reconciliation, separately from ongoing liveness.
pub fn readiness(m: &Manifest) -> Result<Vec<(String, String, u32)>> {
    m.services
        .iter()
        .filter_map(|s| {
            s.health
                .as_ref()
                .and_then(|h| h.readiness.as_ref())
                .map(|p| (s, p))
        })
        .map(|(s, p)| Ok((s.name.clone(), probe_command(s, p)?, p.period_seconds)))
        .collect()
}
fn validate_compose(doc: &Value, mode: &str) -> Result<()> {
    let root = doc
        .as_object()
        .ok_or_else(|| bad("Compose 产物必须是对象。"))?;
    if root
        .keys()
        .any(|k| !["services", "volumes"].contains(&k.as_str()))
    {
        return Err(bad("首批 Compose overrides 顶层仅支持 services、volumes。"));
    }
    let services = doc["services"]
        .as_object()
        .filter(|s| !s.is_empty() && s.len() <= 50)
        .ok_or_else(|| bad("Compose services 必须包含1到50个服务。"))?;
    let mut published = BTreeSet::new();
    let mut dependencies = BTreeMap::<String, Vec<String>>::new();
    if let Some(volumes) = doc.get("volumes") {
        let volumes = volumes
            .as_object()
            .ok_or_else(|| bad("Compose volumes 必须是对象。"))?;
        for (name, config) in volumes {
            if !identifier(name) || config.as_object().is_none_or(|v| !v.is_empty()) {
                return Err(bad(
                    "首批仅支持由 Compose 管理的命名卷，不支持 external 或自定义驱动。",
                ));
            }
        }
    }
    for (name, config) in services {
        let service = config
            .as_object()
            .ok_or_else(|| bad("Compose 服务配置必须是对象。"))?;
        if !identifier(name) {
            return Err(bad("Compose 服务名无效。"));
        }
        for key in service.keys() {
            if ![
                "image",
                "command",
                "deploy",
                "environment",
                "ports",
                "volumes",
                "depends_on",
                "labels",
                "logging",
                "restart",
                "healthcheck",
            ]
            .contains(&key.as_str())
            {
                return Err(bad(format!(
                    "首批 Compose overrides 不支持服务字段 {key}。"
                )));
            }
        }
        if let Some(check) = service.get("healthcheck") {
            let check = check
                .as_object()
                .ok_or_else(|| bad("healthcheck 必须是对象。"))?;
            if check.keys().any(|k| {
                ![
                    "test",
                    "interval",
                    "timeout",
                    "retries",
                    "start_period",
                    "start_interval",
                    "disable",
                ]
                .contains(&k.as_str())
            }) {
                return Err(bad("healthcheck 字段无效。"));
            }
            if let Some(test) = check.get("test") {
                let test = test
                    .as_array()
                    .ok_or_else(|| bad("healthcheck.test 必须为字符串数组。"))?;
                if test.is_empty()
                    || !test.iter().all(Value::is_string)
                    || !["CMD", "CMD-SHELL", "NONE"].contains(&test[0].as_str().unwrap_or(""))
                {
                    return Err(bad("healthcheck.test 必须使用 CMD、CMD-SHELL 或 NONE。"));
                }
            }
            for key in ["interval", "timeout", "start_period", "start_interval"] {
                if check.get(key).is_some_and(|v| {
                    !v.as_str().is_some_and(|v| {
                        !v.is_empty()
                            && v.bytes()
                                .all(|c| c.is_ascii_digit() || b".nsumh".contains(&c))
                    })
                }) {
                    return Err(bad("healthcheck 时间必须是有效的时长字符串。"));
                }
            }
            if check
                .get("retries")
                .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=100).contains(&n)))
                || check.get("disable").is_some_and(|v| !v.is_boolean())
            {
                return Err(bad("healthcheck 阈值或开关无效。"));
            }
        }
        if mode == "stack"
            && (service.contains_key("depends_on") || service.contains_key("restart"))
        {
            return Err(bad(
                "Swarm不支持depends_on/restart字段，请使用服务重连和Swarm重启策略。",
            ));
        }
        let image = config["image"].as_str().unwrap_or("");
        if image.is_empty()
            || image.starts_with('-')
            || image
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '$')
        {
            return Err(bad("每个服务必须指定有效镜像引用，镜像不支持环境插值。"));
        }
        let replicas = if let Some(deploy) = service.get("deploy") {
            let d = deploy
                .as_object()
                .ok_or_else(|| bad("deploy 必须是对象。"))?;
            if d.keys().any(|k| {
                k != "replicas" && k != "resources" && !(mode == "stack" && k == "update_config")
            }) {
                return Err(bad(
                    "Compose deploy 仅支持 replicas、resources；不支持滚动更新。",
                ));
            }
            if let Some(update) = d.get("update_config") {
                let update = update
                    .as_object()
                    .ok_or_else(|| bad("update_config必须是对象。"))?;
                if update
                    .keys()
                    .any(|k| !["parallelism", "order", "failure_action"].contains(&k.as_str()))
                    || update
                        .get("parallelism")
                        .is_some_and(|v| !v.as_u64().is_some_and(|n| n <= 100))
                    || update.get("order").is_some_and(|v| {
                        ![Some("start-first"), Some("stop-first")].contains(&v.as_str())
                    })
                    || update.get("failure_action").is_some_and(|v| v != "pause")
                {
                    return Err(bad(
                        "Swarm更新配置仅支持parallelism(0–100)、order(start-first/stop-first)、failure_action(pause)。",
                    ));
                }
            }
            if let Some(resources) = d.get("resources") {
                let resources = resources
                    .as_object()
                    .ok_or_else(|| bad("deploy.resources 必须为对象。"))?;
                let mut normalized = Resources::default();
                for (kind, values) in resources {
                    let target = match kind.as_str() {
                        "limits" => &mut normalized.limits,
                        "reservations" => &mut normalized.requests,
                        _ => return Err(bad("资源字段仅支持 limits/reservations。")),
                    };
                    for (name, value) in values
                        .as_object()
                        .ok_or_else(|| bad("资源配置必须为对象。"))?
                    {
                        let name = match name.as_str() {
                            "cpus" => "cpu",
                            "memory" => "memory",
                            _ => return Err(bad("Compose资源仅支持cpus/memory。")),
                        };
                        target.insert(name.into(), value.clone());
                    }
                }
                validate_resources(&normalized)?;
            }
            match d.get("replicas") {
                Some(v) => v
                    .as_u64()
                    .filter(|v| (1..=100).contains(v))
                    .ok_or_else(|| bad("replicas 必须在1到100之间。"))?,
                None => 1,
            }
        } else {
            1
        };
        if let Some(command) = service.get("command") {
            if command
                .as_array()
                .is_none_or(|v| v.iter().any(|x| !x.is_string()))
            {
                return Err(bad("command 必须是字符串数组。"));
            }
        }
        for field in ["environment", "labels"] {
            if let Some(value) = service.get(field) {
                let entries = value
                    .as_object()
                    .ok_or_else(|| bad(format!("{field} 必须是字符串映射。")))?;
                if entries.iter().any(|(k, v)| {
                    !v.is_string() || (field == "environment" && !environment_name(k))
                }) {
                    return Err(bad(format!(
                        "{field} 必须是有效名称和字符串值，不支持变量引用。"
                    )));
                }
            }
        }
        if let Some(ports) = service.get("ports") {
            for p in ports.as_array().ok_or_else(|| bad("ports 必须是数组。"))? {
                let p = p
                    .as_object()
                    .ok_or_else(|| bad("ports 必须使用长格式对象。"))?;
                if p.keys()
                    .any(|k| !["target", "published", "protocol"].contains(&k.as_str()))
                {
                    return Err(bad("端口字段无效。"));
                }
                if !p
                    .get("target")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| (1..=65535).contains(&v))
                {
                    return Err(bad("容器端口必须在1到65535之间。"));
                }
                let protocol = p.get("protocol").and_then(Value::as_str).unwrap_or("tcp");
                if !["tcp", "udp"].contains(&protocol) {
                    return Err(bad("端口协议仅支持 tcp/udp。"));
                }
                if let Some(port) = p.get("published") {
                    let port = port
                        .as_str()
                        .and_then(|s| s.parse::<u16>().ok())
                        .filter(|n| *n > 0)
                        .ok_or_else(|| bad("published 必须是1到65535的端口字符串。"))?;
                    if (mode != "stack" && replicas > 1)
                        || !published.insert((port, protocol.to_owned()))
                    {
                        return Err(bad("固定宿主端口不能重复，也不能用于多副本服务。"));
                    }
                }
            }
        }
        let mut deps = Vec::new();
        if let Some(value) = service.get("depends_on") {
            for d in value
                .as_array()
                .ok_or_else(|| bad("depends_on 必须是服务名数组。"))?
            {
                let d = d
                    .as_str()
                    .ok_or_else(|| bad("depends_on 必须是服务名数组。"))?;
                if !services.contains_key(d) || deps.contains(&d.to_owned()) {
                    return Err(bad("依赖服务不存在或重复。"));
                }
                deps.push(d.to_owned());
            }
        }
        dependencies.insert(name.clone(), deps);
        if let Some(value) = service.get("volumes") {
            let mut paths = BTreeSet::new();
            for v in value
                .as_array()
                .ok_or_else(|| bad("volumes 必须是数组。"))?
            {
                let v = v.as_object().ok_or_else(|| bad("卷必须使用长格式对象。"))?;
                if v.keys()
                    .any(|k| !["type", "source", "target"].contains(&k.as_str()))
                    || v.get("type").and_then(Value::as_str) != Some("volume")
                {
                    return Err(bad("首批仅支持命名卷。"));
                }
                let source = v.get("source").and_then(Value::as_str).unwrap_or("");
                let target = v.get("target").and_then(Value::as_str).unwrap_or("");
                if doc["volumes"].get(source).is_none()
                    || crate::channel::check_path(target).is_err()
                    || !paths.insert(target)
                {
                    return Err(bad("命名卷未声明或挂载路径无效/重复。"));
                }
            }
        }
        if let Some(value) = service.get("restart") {
            if !value
                .as_str()
                .is_some_and(|v| ["no", "always", "on-failure", "unless-stopped"].contains(&v))
            {
                return Err(bad("restart 策略无效。"));
            }
        }
        if let Some(value) = service.get("logging") {
            let l = value
                .as_object()
                .ok_or_else(|| bad("logging 必须是对象。"))?;
            if l.keys()
                .any(|k| !["driver", "options"].contains(&k.as_str()))
                || l.get("driver").is_some_and(|v| !v.is_string())
                || l.get("options").is_some_and(|v| {
                    v.as_object()
                        .is_none_or(|m| m.values().any(|v| !v.is_string()))
                })
            {
                return Err(bad("logging driver/options 无效。"));
            }
        }
    }
    // Kahn's algorithm: no recursion even for adversarial input.
    while !dependencies.is_empty() {
        let ready: Vec<_> = dependencies
            .iter()
            .filter(|(_, d)| d.is_empty())
            .map(|(n, _)| n.clone())
            .collect();
        if ready.is_empty() {
            return Err(bad("服务依赖包含循环。"));
        }
        for n in &ready {
            dependencies.remove(n);
        }
        for d in dependencies.values_mut() {
            d.retain(|n| !ready.contains(n));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Value {
        json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"demo"},"services":[{"name":"web","image":"nginx:alpine","ports":[{"container":80,"published":8080}]}]})
    }
    fn fails(v: &Value, part: &str) {
        let e = parse(&v.to_string()).unwrap_err();
        assert!(e.1.contains(part), "{e}");
    }
    #[test]
    fn yaml_json_same_bytes_and_dollars_remain_literal() {
        let mut v = sample();
        v["services"][0]["env"] =
            json!([{"name":"MESSAGE","value":"${RENDER_TEST_VALUE} $literal $$"}]);
        let a = render(&parse(&v.to_string()).unwrap()).unwrap();
        let b = render(&parse(&serde_yaml_ng::to_string(&v).unwrap()).unwrap()).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("$${RENDER_TEST_VALUE} $$literal $$$$"));
        assert_eq!(a, render(&parse(&v.to_string()).unwrap()).unwrap());
    }
    #[test]
    fn capabilities_fail_instead_of_dropping_fields() {
        for (key, value) in [("artifact", json!({})), ("health", json!({}))] {
            let mut v = sample();
            v["services"][0][key] = value;
            fails(&v, key);
        }
        let mut v = sample();
        v["services"][0]["env"] = json!([{"name":"SECRET","valueFrom":"template/password"}]);
        fails(&v, "valueFrom");
        let mut v = sample();
        v["strategy"] = json!({"type":"rolling"});
        assert!(parse_for(&v.to_string(), "compose").is_err());
        assert!(parse_for(&v.to_string(), "stack").is_ok());
    }
    #[test]
    fn variable_references_are_explicit_and_overrides_cannot_replace_them() {
        let mut v = sample();
        v["services"][0]["env"] = json!([{"name":"SECRET","valueFrom":"variable/db-url"}]);
        let m = parse(&v.to_string()).unwrap();
        assert_eq!(m.references().values().next().unwrap(), "db-url");
        let rendered: Value = serde_json::from_str(&render(&m).unwrap()).unwrap();
        let service = m.services[0].name.clone();
        assert_eq!(
            rendered["x-variable-references"][&service]["SECRET"],
            "variable/db-url"
        );
        assert!(
            rendered["services"][&service]["environment"]
                .get("SECRET")
                .is_none()
        );
        v["overrides"] =
            json!({"compose":{"services":{service:{"environment":{"SECRET":"bypass"}}}}});
        fails(&v, "valueFrom");
        v["overrides"] = json!({});
        v["services"][0]["env"][0]["value"] = json!("literal");
        fails(&v, "只能");
        v["services"][0]["env"] = json!([{"name":"SECRET"}]);
        fails(&v, "只能");
    }
    #[test]
    fn resources_preserve_limits_and_validate_effective_overrides() {
        let mut v = sample();
        v["services"][0]["resources"] = json!({"requests":{"cpu":"100m","memory":"16Mi"},"limits":{"cpu":"0.5","memory":"64Mi"}});
        let rendered: Value =
            serde_json::from_str(&render(&parse(&v.to_string()).unwrap()).unwrap()).unwrap();
        assert_eq!(
            rendered["services"]["web"]["deploy"]["resources"]["limits"],
            json!({"cpus":"0.500000000","memory":"67108864"})
        );
        v["services"][0]["resources"]["requests"]["cpu"] = json!("1");
        fails(&v, "requests");
        v["services"][0]["resources"]["requests"]["cpu"] = json!("100m");
        v["overrides"] = json!({"compose":{"services":{"web":{"deploy":{"resources":{"limits":{"memory":0}}}}}}});
        fails(&v, "资源数量");
        for value in [
            json!("-1"),
            json!("NaN"),
            json!("0"),
            json!("999999999999999999999Gi"),
            json!(true),
        ] {
            assert!(quantity(&value, false).is_err());
        }
        assert_eq!(quantity(&json!("128Mi"), false).unwrap(), 134217728);
        assert_eq!(quantity(&json!("100m"), true).unwrap(), 100000000);
    }
    #[test]
    fn duplicates_and_dependency_cycles_rejected() {
        let mut v = sample();
        let s = v["services"][0].clone();
        v["services"].as_array_mut().unwrap().push(s);
        fails(&v, "唯一");
        let mut v = sample();
        v["services"][0]["dependsOn"] = json!(["absent"]);
        fails(&v, "不存在");
        v["services"][0]["dependsOn"] = json!(["web"]);
        fails(&v, "循环");
        let mut v = sample();
        v["services"][0]["env"] = json!([{"name":"A","value":"x"},{"name":"A","value":"y"}]);
        fails(&v, "重复");
    }
    #[test]
    fn escaped_nul_is_rejected_after_parsing() {
        let mut v = sample();
        v["services"][0]["command"] = json!(["echo", "a\0b"]);
        fails(&v, "空字符");
    }
    #[test]
    fn overrides_merge_replace_and_revalidate() {
        let mut v = sample();
        v["overrides"] = json!({"compose":{"services":{"web":{"command":["echo","hello"],"logging":{"driver":"json-file","options":{"max-size":"10m"}}}}}});
        let out: Value =
            serde_json::from_str(&render(&parse(&v.to_string()).unwrap()).unwrap()).unwrap();
        assert_eq!(out["services"]["web"]["image"], "nginx:alpine");
        assert_eq!(out["services"]["web"]["command"], json!(["echo", "hello"]));
        v["overrides"]["compose"]["services"]["web"]["deploy"] = json!({"replicas":2});
        fails(&v, "多副本");
        v["overrides"]["compose"]["services"]["web"]["deploy"] = json!({"update_config":{}});
        fails(&v, "滚动");
        let mut v = sample();
        v["overrides"] = json!({"compose":{"services":{"web":{"env_file":".env"}}}});
        fails(&v, "env_file");
    }
    #[test]
    fn ports_volumes_and_types() {
        let mut v = sample();
        v["services"][0]["ports"][0]["container"] = json!(0);
        fails(&v, "端口");
        let mut v = sample();
        v["services"][0]["volumes"] = json!([{"name":"data","path":"../data"}]);
        fails(&v, "挂载");
        v["services"][0]["volumes"] = json!([{"name":"data","path":"/data"}]);
        assert!(parse(&v.to_string()).is_ok());
        let mut v = sample();
        v["services"][0]["image"] = json!("${IMAGE}");
        fails(&v, "插值");
        for input in [
            "null",
            "[]",
            "apiVersion: ferry/v1\napiVersion: other",
            "---\n{}\n---\n{}",
        ] {
            assert!(parse(input).is_err());
        }
    }
    #[test]
    #[ignore = "requires Docker Engine and Compose; run explicitly with --ignored"]
    fn compose_cli_accepts_rendered_document_and_preserves_literal_values() {
        let mut v = sample();
        v["services"][0]["image"] = json!("debian:bookworm-slim");
        v["services"][0]["env"] =
            json!([{"name":"MESSAGE","value":"${RENDER_TEST_VALUE} $literal $$"}]);
        v["services"][0]["volumes"] = json!([{"name":"data","path":"/data"}]);
        let rendered = render(&parse(&v.to_string()).unwrap()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compose.json");
        std::fs::write(&path, &rendered).unwrap();
        let project = format!("render-test-{}", std::process::id());
        let compose = |args: &[&str]| {
            std::process::Command::new("docker")
                .args(["compose", "--project-name", &project, "-f"])
                .arg(&path)
                .args(args)
                .env("RENDER_TEST_VALUE", "must-not-interpolate")
                .output()
                .expect("Docker Compose is required")
        };
        let output = compose(&["config", "--format", "json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config: Value = serde_json::from_slice(&output.stdout).unwrap();
        // `config` re-escapes dollars so its output is itself a reusable Compose file.
        assert_eq!(config["services"]["web"]["ports"][0]["published"], "8080");
        assert_eq!(config["volumes"]["data"]["name"], format!("{project}_data"));
        let run = compose(&["run", "--rm", "--no-deps", "web", "printenv", "MESSAGE"]);
        let cleanup = compose(&["down", "--volumes"]);
        assert!(
            cleanup.status.success(),
            "{}",
            String::from_utf8_lossy(&cleanup.stderr)
        );
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8(run.stdout).unwrap().trim(),
            "${RENDER_TEST_VALUE} $literal $$"
        );
    }
    #[test]
    fn native_requires_an_artifact_and_rejects_container_only_capabilities() {
        let mut v = sample();
        v["services"][0].as_object_mut().unwrap().remove("image");
        v["services"][0]["artifact"] =
            json!({"source":"pipeline","pipeline":"build","path":"dist/server"});
        v["services"][0]["ports"] = json!([]);
        let m = parse_for(&v.to_string(), "native").unwrap();
        assert!(render_for(&m, "native").is_ok());
        assert!(render_for(&m, "docker").is_err());
        assert!(render_for(&m, "compose").is_err());
        v["services"][0]["replicas"] = json!(2);
        assert!(parse_for(&v.to_string(), "native").is_err());
        v["services"][0]["replicas"] = json!(1);
        v["services"][0]["image"] = json!("busybox:1.36");
        assert!(parse_for(&v.to_string(), "native").is_err());
        v["services"][0].as_object_mut().unwrap().remove("image");
        v["services"][0]["artifact"]["path"] = json!("../server");
        assert!(parse_for(&v.to_string(), "native").is_err());
    }
}
