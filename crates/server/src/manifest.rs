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
    pub image: String,
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
fn one() -> u32 {
    1
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub name: String,
    pub value: String,
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
}
fn health_timeout() -> u32 {
    120
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default)]
    pub compose: BTreeMap<String, Value>,
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
    if input.len() > 64 * 1024 || input.contains('\0') {
        return Err(bad("manifest 不能超过64KB或包含空字符。"));
    }
    let m: Manifest = serde_yaml_ng::from_str(input).map_err(|e| bad(format!(
        "manifest YAML/JSON 无效：{e}。首批不支持 artifact、valueFrom、files、health、resources；请删除未实现的字段。")))?;
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
        if s.kind != "recreate" {
            return Err(bad(
                "Compose 不支持滚动更新；strategy.type 仅支持 recreate。",
            ));
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
        if s.replicas == 0 || s.replicas > 100 {
            return Err(bad("replicas 必须在1到100之间。"));
        }
        let mut env = BTreeSet::new();
        for e in &s.env {
            if !environment_name(&e.name) || !env.insert(&e.name) {
                return Err(bad("环境变量名无效或重复。"));
            }
        }
        let mut paths = BTreeSet::new();
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
    render(&m)?;
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
fn escape(value: &mut Value) {
    match value {
        Value::String(s) => *s = s.replace('$', "$$"),
        Value::Array(items) => items.iter_mut().for_each(escape),
        Value::Object(items) => items.values_mut().for_each(escape),
        _ => {}
    }
}

/// JSON is a YAML subset accepted by Compose; sorted maps yield stable bytes.
/// Dollar signs are always literal input, never host environment substitutions.
pub fn render(m: &Manifest) -> Result<String> {
    let mut services = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    for s in &m.services {
        let mut v = json!({"image":s.image,"deploy":{"replicas":s.replicas}});
        if let Some(command) = &s.command {
            v["command"] = json!(command);
        }
        let env: BTreeMap<_, _> = s.env.iter().map(|e| (&e.name, &e.value)).collect();
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
    merge(&mut document, &json!(m.overrides.compose));
    validate_compose(&document)?;
    escape(&mut document);
    serde_json::to_string_pretty(&document)
        .map(|s| s + "\n")
        .map_err(|_| bad("无法序列化 Compose 产物。"))
}

fn validate_compose(doc: &Value) -> Result<()> {
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
            ]
            .contains(&key.as_str())
            {
                return Err(bad(format!(
                    "首批 Compose overrides 不支持服务字段 {key}。"
                )));
            }
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
            if d.keys().any(|k| k != "replicas") {
                return Err(bad(
                    "首批 deploy 仅支持 replicas；不支持滚动更新或资源限制。",
                ));
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
                    if replicas > 1 || !published.insert((port, protocol.to_owned())) {
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
        for (key, value) in [
            ("artifact", json!({})),
            ("files", json!([])),
            ("health", json!({})),
            ("resources", json!({})),
        ] {
            let mut v = sample();
            v["services"][0][key] = value;
            fails(&v, key);
        }
        let mut v = sample();
        v["services"][0]["env"] = json!([{"name":"SECRET","valueFrom":"variable/password"}]);
        fails(&v, "valueFrom");
        let mut v = sample();
        v["strategy"] = json!({"type":"rolling"});
        fails(&v, "滚动");
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
}
