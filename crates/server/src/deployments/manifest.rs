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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// Swarm service discovery (`vip` or `dnsrr`); stack only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_mode: Option<String>,
}
impl Service {
    /// Fields that only exist on Docker Swarm / Stack.
    fn uses_stack_only_fields(&self) -> bool {
        self.endpoint_mode.is_some() || self.ports.iter().any(|p| p.mode.is_some())
    }
}
/// Swarm placement is explicit and never silently applied to single-host engines.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Placement {
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_replicas_per_node: Option<u32>,
}
fn validate_placement(constraints: &[String], max: Option<u32>) -> Result<()> {
    if constraints.len() > 32 || max.is_some_and(|n| !(1..=100).contains(&n)) {
        return Err(bad("deployment.portable.placementInvalid"));
    }
    for constraint in constraints {
        let Some((key, value)) = constraint
            .split_once("==")
            .or_else(|| constraint.split_once("!="))
        else {
            return Err(bad("deployment.portable.placementInvalid"));
        };
        let (key, value) = (key.trim(), value.trim());
        let label = key
            .strip_prefix("node.labels.")
            .or_else(|| key.strip_prefix("engine.labels."));
        let known = [
            "node.id",
            "node.hostname",
            "node.role",
            "node.platform.os",
            "node.platform.arch",
        ]
        .contains(&key);
        if constraint.len() > 256
            || constraint.chars().any(char::is_control)
            || (!known
                && !label.is_some_and(|label| {
                    !label.is_empty()
                        && label
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"._-/".contains(&c))
                }))
            || value.is_empty()
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-/:".contains(&c))
            || (key == "node.role" && !["manager", "worker"].contains(&value))
        {
            return Err(bad("deployment.portable.placementInvalid"));
        }
    }
    Ok(())
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
        .ok_or_else(|| bad("deployment.manifest.001"))?;
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
        return Err(bad("deployment.manifest.002"));
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
        _ => return Err(bad("deployment.manifest.003")),
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
        return Err(bad("deployment.manifest.004"));
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
    scaled.ok_or_else(|| bad("deployment.manifest.005"))
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
            _ => return Err(bad("deployment.manifest.006")),
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
                return Err(bad("deployment.manifest.007"));
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
    /// Swarm publish mode (`host` or `ingress`); stack only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}
fn valid_publish_mode(mode: &str) -> bool {
    ["host", "ingress"].contains(&mode)
}
fn valid_endpoint_mode(mode: &str) -> bool {
    ["vip", "dnsrr"].contains(&mode)
}
/// `dnsrr` service discovery cannot be combined with ingress-published ports.
fn validate_endpoint_mode(mode: Option<&str>, port_modes: &[Option<&str>]) -> Result<()> {
    if let Some(mode) = mode {
        if !valid_endpoint_mode(mode) {
            return Err(bad("deployment.portable.endpointModeInvalid"));
        }
    }
    if port_modes.iter().flatten().any(|m| !valid_publish_mode(m)) {
        return Err(bad("deployment.portable.endpointModeInvalid"));
    }
    if mode == Some("dnsrr") && port_modes.iter().any(|m| *m != Some("host")) {
        return Err(bad("deployment.portable.endpointModeInvalid"));
    }
    Ok(())
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

fn bad(key: &str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new(key),
    )
}
fn identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 63
        && v.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && v.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}
fn valid_image(image: &str) -> bool {
    !image.is_empty()
        && !image.starts_with('-')
        && !image
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '$')
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
        return Err(bad("deployment.manifest.008"));
    }
    let m: Manifest = serde_yaml_ng::from_str(input).map_err(|e| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("deployment.manifest.085").with("detail", e.to_string()),
        )
    })?;
    let mode = effective_mode(&m, mode);
    if !["ferry/v1", "ferry/v2"].contains(&m.api_version.as_str()) || m.kind != "Application" {
        return Err(bad("deployment.manifest.009"));
    }
    if !identifier(&m.metadata.name) || m.services.is_empty() || m.services.len() > 50 {
        return Err(bad("deployment.manifest.010"));
    }
    if let Some(s) = &m.strategy {
        if s.kind != "recreate" && !(mode == "stack" && s.kind == "rolling") {
            return Err(bad("deployment.manifest.011"));
        }
        if s.max_unavailable.is_some_and(|n| n > 100)
            || (mode != "stack" && s.max_unavailable.is_some())
        {
            return Err(bad("deployment.manifest.012"));
        }
        if !(1..=600).contains(&s.health_timeout) {
            return Err(bad("deployment.manifest.013"));
        }
    }
    let mut names = BTreeSet::new();
    for s in &m.services {
        if !identifier(&s.name) || !names.insert(&s.name) {
            return Err(bad("deployment.manifest.014"));
        }
        if let Some(resources) = &s.resources {
            validate_resources(resources)?;
        }
        if m.api_version == "ferry/v1" && s.image.is_empty() == s.artifact.is_none() {
            return Err(bad("deployment.manifest.015"));
        }
        if s.image.is_empty() && s.artifact.is_none() {
            return Err(bad("deployment.portable.sourceRequired"));
        }
        if let Some(placement) = &s.placement {
            if m.api_version != "ferry/v2" || mode != "stack" {
                return Err(bad("deployment.portable.placementMode"));
            }
            validate_placement(&placement.constraints, placement.max_replicas_per_node)?;
        }
        if s.uses_stack_only_fields() {
            if m.api_version != "ferry/v2" || mode != "stack" {
                return Err(bad("deployment.portable.endpointModeInvalid"));
            }
            let port_modes: Vec<_> = s.ports.iter().map(|p| p.mode.as_deref()).collect();
            validate_endpoint_mode(s.endpoint_mode.as_deref(), &port_modes)?;
        }
        if m.api_version == "ferry/v2" && !s.image.is_empty() && !valid_image(&s.image) {
            return Err(bad("deployment.manifest.059"));
        }
        if let Some(artifact) = &s.artifact {
            if artifact.source != "pipeline" || artifact.pipeline.trim().is_empty() {
                return Err(bad("deployment.manifest.016"));
            }
            crate::pipelines::artifacts::validate_paths(&[artifact.path.clone()])?;
            if artifact
                .run
                .as_ref()
                .is_some_and(|r| r.parse::<crate::snowflake::Id>().is_err())
            {
                return Err(bad("deployment.manifest.017"));
            }
        }
        if s.replicas == 0 || s.replicas > 100 {
            return Err(bad("deployment.manifest.018"));
        }
        let mut env = BTreeSet::new();
        for e in &s.env {
            if e.value.is_some() == e.value_from.is_some() {
                return Err(bad("deployment.manifest.019"));
            }
            if e.value_from.as_deref().is_some_and(|reference| {
                !reference
                    .strip_prefix("variable/")
                    .is_some_and(crate::variables::valid_name)
            }) {
                return Err(bad("deployment.manifest.020"));
            }
            if !environment_name(&e.name) || !env.insert(&e.name) {
                return Err(bad("deployment.manifest.021"));
            }
        }
        let mut paths = BTreeSet::new();
        if s.files.len() > 20 {
            return Err(bad("deployment.manifest.022"));
        }
        for file in &s.files {
            if crate::hosts::channel::check_path(&file.path).is_err()
                || file.path == "/"
                || !paths.insert(&file.path)
                || file.from_template.trim().is_empty()
                || file.from_template.chars().count() > 60
            {
                return Err(bad("deployment.manifest.023"));
            }
        }
        for v in &s.volumes {
            if !identifier(&v.name)
                || crate::hosts::channel::check_path(&v.path).is_err()
                || !paths.insert(&v.path)
            {
                return Err(bad("deployment.manifest.024"));
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
    if contains_nul(&serde_json::to_value(&m).map_err(|_| bad("deployment.error.006"))?) {
        return Err(bad("deployment.manifest.025"));
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
    if m.services.iter().any(|s| s.artifact.is_some())
        && m.services.iter().all(|s| s.image.is_empty())
    {
        "native"
    } else if m
        .services
        .iter()
        .any(|s| s.placement.is_some() || s.uses_stack_only_fields())
        || !m.overrides.stack.is_empty()
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
    // Also guard direct rendering callers, independently of parse_for.
    if m.services.iter().any(|s| s.placement.is_some())
        && (mode != "stack" || m.api_version != "ferry/v2")
    {
        return Err(bad("deployment.portable.placementMode"));
    }
    if m.services.iter().any(Service::uses_stack_only_fields)
        && (mode != "stack" || m.api_version != "ferry/v2")
    {
        return Err(bad("deployment.portable.endpointModeInvalid"));
    }
    if mode == "native" {
        return render_native(m);
    }
    if m.services
        .iter()
        .any(|s| s.image.is_empty() || (m.api_version == "ferry/v1" && s.artifact.is_some()))
    {
        return Err(bad("deployment.manifest.026"));
    }
    if !["compose", "docker", "stack"].contains(&mode) {
        return Err(bad("deployment.manifest.027"));
    }
    if mode == "docker" && m.services.iter().any(|s| s.replicas != 1) {
        return Err(bad("deployment.manifest.028"));
    }
    if mode == "docker"
        && m.services.iter().any(|s| {
            s.resources
                .as_ref()
                .is_some_and(|r| r.requests.contains_key("cpu"))
        })
    {
        return Err(bad("deployment.manifest.029"));
    }

    let mut services = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    for s in &m.services {
        if mode == "stack" && !s.depends_on.is_empty() {
            return Err(bad("deployment.manifest.030"));
        }
        let mut v = json!({"image":s.image,"deploy":{"replicas":s.replicas}});
        if let Some(placement) = &s.placement {
            validate_placement(&placement.constraints, placement.max_replicas_per_node)?;
            let mut value = json!({"constraints": placement.constraints});
            if let Some(max) = placement.max_replicas_per_node {
                value["max_replicas_per_node"] = json!(max);
            }
            v["deploy"]["placement"] = value;
        }
        if let Some(endpoint) = &s.endpoint_mode {
            v["deploy"]["endpoint_mode"] = json!(endpoint);
        }
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
                return Err(bad("deployment.manifest.031"));
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
                        if let Some(mode) = &p.mode {
                            v["mode"] = json!(mode);
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
    validate_compose(&document, mode, m.api_version == "ferry/v2")?;
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
            return Err(bad("deployment.manifest.032"));
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
                return Err(bad("deployment.manifest.033"));
            }
            if service["deploy"]["resources"]["reservations"]
                .get("cpus")
                .is_some()
            {
                return Err(bad("deployment.manifest.034"));
            }
        }
    }
    if mode == "compose" {
        escape(&mut document);
    }
    serde_json::to_string_pretty(&document)
        .map(|s| s + "\n")
        .map_err(|_| bad("deployment.error.021"))
}

fn render_native(m: &Manifest) -> Result<String> {
    let mut services = BTreeMap::new();
    let mut volumes = BTreeMap::new();
    for service in &m.services {
        if service.artifact.is_none()
            || (m.api_version == "ferry/v1" && !service.image.is_empty())
            || service.replicas != 1
        {
            return Err(bad("deployment.manifest.035"));
        }
        if service
            .ports
            .iter()
            .any(|p| p.published.is_some_and(|port| port != p.container))
        {
            return Err(bad("deployment.manifest.036"));
        }
        if service
            .resources
            .as_ref()
            .is_some_and(|r| !r.requests.is_empty())
        {
            return Err(bad("deployment.manifest.037"));
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
                return Err(bad("deployment.manifest.038"));
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
            return Err(bad("deployment.manifest.039"));
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
            return Err(bad("deployment.manifest.040"));
        }
        for name in ready {
            remaining.remove(&name);
        }
    }
    if m.overrides.native.keys().any(|key| key != "units") {
        return Err(bad("deployment.manifest.041"));
    }
    if let Some(units) = m.overrides.native.get("units") {
        let units = units
            .as_object()
            .ok_or_else(|| bad("deployment.manifest.042"))?;
        if units
            .iter()
            .any(|(name, value)| !services.contains_key(name) || !value.is_string())
        {
            return Err(bad("deployment.manifest.043"));
        }
    }
    serde_json::to_string_pretty(
        &json!({"services":services,"volumes":volumes,"x-native-overrides":m.overrides.native}),
    )
    .map(|s| s + "\n")
    .map_err(|_| bad("deployment.manifest.044"))
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
fn validate_compose(doc: &Value, mode: &str, portable: bool) -> Result<()> {
    let root = doc
        .as_object()
        .ok_or_else(|| bad("deployment.manifest.045"))?;
    if root
        .keys()
        .any(|k| !["services", "volumes"].contains(&k.as_str()))
    {
        return Err(bad("deployment.manifest.046"));
    }
    let services = doc["services"]
        .as_object()
        .filter(|s| !s.is_empty() && s.len() <= 50)
        .ok_or_else(|| bad("deployment.manifest.047"))?;
    let mut published = BTreeSet::new();
    let mut dependencies = BTreeMap::<String, Vec<String>>::new();
    if let Some(volumes) = doc.get("volumes") {
        let volumes = volumes
            .as_object()
            .ok_or_else(|| bad("deployment.manifest.048"))?;
        for (name, config) in volumes {
            if !identifier(name) || config.as_object().is_none_or(|v| !v.is_empty()) {
                return Err(bad("deployment.manifest.049"));
            }
        }
    }
    for (name, config) in services {
        let service = config
            .as_object()
            .ok_or_else(|| bad("deployment.manifest.050"))?;
        if !identifier(name) {
            return Err(bad("deployment.manifest.051"));
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
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("deployment.manifest.086").with("field", key),
                ));
            }
        }
        if let Some(check) = service.get("healthcheck") {
            let check = check
                .as_object()
                .ok_or_else(|| bad("deployment.manifest.052"))?;
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
                return Err(bad("deployment.manifest.053"));
            }
            if let Some(test) = check.get("test") {
                let test = test
                    .as_array()
                    .ok_or_else(|| bad("deployment.manifest.054"))?;
                if test.is_empty()
                    || !test.iter().all(Value::is_string)
                    || !["CMD", "CMD-SHELL", "NONE"].contains(&test[0].as_str().unwrap_or(""))
                {
                    return Err(bad("deployment.manifest.055"));
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
                    return Err(bad("deployment.manifest.056"));
                }
            }
            if check
                .get("retries")
                .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=100).contains(&n)))
                || check.get("disable").is_some_and(|v| !v.is_boolean())
            {
                return Err(bad("deployment.manifest.057"));
            }
        }
        if mode == "stack"
            && (service.contains_key("depends_on") || service.contains_key("restart"))
        {
            return Err(bad("deployment.manifest.058"));
        }
        let image = config["image"].as_str().unwrap_or("");
        if !valid_image(image) {
            return Err(bad("deployment.manifest.059"));
        }
        let replicas = if let Some(deploy) = service.get("deploy") {
            let d = deploy
                .as_object()
                .ok_or_else(|| bad("deployment.manifest.060"))?;
            if d.keys().any(|k| {
                k != "replicas"
                    && k != "resources"
                    && !(mode == "stack"
                        && (k == "update_config"
                            || (portable && (k == "placement" || k == "endpoint_mode"))))
            }) {
                return Err(bad("deployment.manifest.061"));
            }
            if let Some(endpoint) = d.get("endpoint_mode") {
                let endpoint = endpoint
                    .as_str()
                    .ok_or_else(|| bad("deployment.portable.endpointModeInvalid"))?;
                if !valid_endpoint_mode(endpoint) {
                    return Err(bad("deployment.portable.endpointModeInvalid"));
                }
            }
            if let Some(placement) = d.get("placement") {
                let placement = placement
                    .as_object()
                    .ok_or_else(|| bad("deployment.portable.placementInvalid"))?;
                if placement
                    .keys()
                    .any(|k| !["constraints", "max_replicas_per_node"].contains(&k.as_str()))
                {
                    return Err(bad("deployment.portable.placementInvalid"));
                }
                let constraints: Vec<String> = serde_json::from_value(
                    placement.get("constraints").cloned().unwrap_or(json!([])),
                )
                .map_err(|_| bad("deployment.portable.placementInvalid"))?;
                let max = placement
                    .get("max_replicas_per_node")
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| bad("deployment.portable.placementInvalid"))
                    })
                    .transpose()?;
                validate_placement(&constraints, max)?;
            }
            if let Some(update) = d.get("update_config") {
                let update = update
                    .as_object()
                    .ok_or_else(|| bad("deployment.manifest.062"))?;
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
                    return Err(bad("deployment.manifest.063"));
                }
            }
            if let Some(resources) = d.get("resources") {
                let resources = resources
                    .as_object()
                    .ok_or_else(|| bad("deployment.manifest.064"))?;
                let mut normalized = Resources::default();
                for (kind, values) in resources {
                    let target = match kind.as_str() {
                        "limits" => &mut normalized.limits,
                        "reservations" => &mut normalized.requests,
                        _ => return Err(bad("deployment.manifest.065")),
                    };
                    for (name, value) in values
                        .as_object()
                        .ok_or_else(|| bad("deployment.manifest.066"))?
                    {
                        let name = match name.as_str() {
                            "cpus" => "cpu",
                            "memory" => "memory",
                            _ => return Err(bad("deployment.manifest.067")),
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
                    .ok_or_else(|| bad("deployment.manifest.018"))?,
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
                return Err(bad("deployment.manifest.068"));
            }
        }
        for field in ["environment", "labels"] {
            if let Some(value) = service.get(field) {
                let entries = value.as_object().ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.manifest.087").with("field", field),
                    )
                })?;
                if entries.iter().any(|(k, v)| {
                    !v.is_string() || (field == "environment" && !environment_name(k))
                }) {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("deployment.manifest.088").with("field", field),
                    ));
                }
            }
        }
        if let Some(ports) = service.get("ports") {
            for p in ports
                .as_array()
                .ok_or_else(|| bad("deployment.manifest.069"))?
            {
                let p = p
                    .as_object()
                    .ok_or_else(|| bad("deployment.manifest.070"))?;
                if p.keys().any(|k| {
                    !["target", "published", "protocol"].contains(&k.as_str())
                        && !(mode == "stack" && portable && k == "mode")
                }) {
                    return Err(bad("deployment.manifest.071"));
                }
                if let Some(mode) = p.get("mode") {
                    if !mode.as_str().is_some_and(valid_publish_mode) {
                        return Err(bad("deployment.portable.endpointModeInvalid"));
                    }
                }
                if !p
                    .get("target")
                    .and_then(Value::as_u64)
                    .is_some_and(|v| (1..=65535).contains(&v))
                {
                    return Err(bad("deployment.manifest.072"));
                }
                let protocol = p.get("protocol").and_then(Value::as_str).unwrap_or("tcp");
                if !["tcp", "udp"].contains(&protocol) {
                    return Err(bad("deployment.manifest.073"));
                }
                if let Some(port) = p.get("published") {
                    let port = port
                        .as_str()
                        .and_then(|s| s.parse::<u16>().ok())
                        .filter(|n| *n > 0)
                        .ok_or_else(|| bad("deployment.manifest.074"))?;
                    if (mode != "stack" && replicas > 1)
                        || !published.insert((port, protocol.to_owned()))
                    {
                        return Err(bad("deployment.manifest.075"));
                    }
                }
            }
        }
        if service
            .get("deploy")
            .and_then(|d| d.get("endpoint_mode"))
            .and_then(Value::as_str)
            == Some("dnsrr")
        {
            let all_host = service
                .get("ports")
                .and_then(Value::as_array)
                .is_none_or(|ports| ports.iter().all(|p| p.get("mode") == Some(&json!("host"))));
            if !all_host {
                return Err(bad("deployment.portable.endpointModeInvalid"));
            }
        }
        let mut deps = Vec::new();
        if let Some(value) = service.get("depends_on") {
            for d in value
                .as_array()
                .ok_or_else(|| bad("deployment.manifest.076"))?
            {
                let d = d.as_str().ok_or_else(|| bad("deployment.manifest.076"))?;
                if !services.contains_key(d) || deps.contains(&d.to_owned()) {
                    return Err(bad("deployment.manifest.077"));
                }
                deps.push(d.to_owned());
            }
        }
        dependencies.insert(name.clone(), deps);
        if let Some(value) = service.get("volumes") {
            let mut paths = BTreeSet::new();
            for v in value
                .as_array()
                .ok_or_else(|| bad("deployment.manifest.078"))?
            {
                let v = v
                    .as_object()
                    .ok_or_else(|| bad("deployment.manifest.079"))?;
                if v.keys()
                    .any(|k| !["type", "source", "target"].contains(&k.as_str()))
                    || v.get("type").and_then(Value::as_str) != Some("volume")
                {
                    return Err(bad("deployment.manifest.080"));
                }
                let source = v.get("source").and_then(Value::as_str).unwrap_or("");
                let target = v.get("target").and_then(Value::as_str).unwrap_or("");
                if doc["volumes"].get(source).is_none()
                    || crate::hosts::channel::check_path(target).is_err()
                    || !paths.insert(target)
                {
                    return Err(bad("deployment.manifest.081"));
                }
            }
        }
        if let Some(value) = service.get("restart") {
            if !value
                .as_str()
                .is_some_and(|v| ["no", "always", "on-failure", "unless-stopped"].contains(&v))
            {
                return Err(bad("deployment.manifest.082"));
            }
        }
        if let Some(value) = service.get("logging") {
            let l = value
                .as_object()
                .ok_or_else(|| bad("deployment.manifest.083"))?;
            if l.keys()
                .any(|k| !["driver", "options"].contains(&k.as_str()))
                || l.get("driver").is_some_and(|v| !v.is_string())
                || l.get("options").is_some_and(|v| {
                    v.as_object()
                        .is_none_or(|m| m.values().any(|v| !v.is_string()))
                })
            {
                return Err(bad("deployment.manifest.084"));
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
            return Err(bad("deployment.manifest.040"));
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
    fn portable_sample() -> Value {
        let mut value = sample();
        value["apiVersion"] = json!("ferry/v2");
        // Use no port mapping so the exact same definition is valid on systemd.
        value["services"][0]["ports"] = json!([]);
        value["services"][0]["artifact"] =
            json!({"source":"pipeline", "pipeline":"build", "path":"bin/web"});
        value
    }
    #[test]
    fn portable_sources_select_runtime_without_changing_v1_artifacts() {
        let portable = portable_sample();
        for mode in ["native", "docker", "compose", "stack"] {
            let m = parse_for(&portable.to_string(), mode).unwrap();
            let rendered = render_for(&m, mode).unwrap();
            let doc: Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(
                doc["services"]["web"].get("artifact").is_some(),
                mode == "native"
            );
            assert_eq!(
                doc["services"]["web"].get("image").is_some(),
                mode != "native"
            );
            // Existing v1 and v2 use exactly the same renderer and runtime artifact.
            let mut legacy = portable.clone();
            legacy["apiVersion"] = json!("ferry/v1");
            legacy["services"][0]
                .as_object_mut()
                .unwrap()
                .remove(if mode == "native" {
                    "image"
                } else {
                    "artifact"
                });
            assert_eq!(
                rendered,
                render_for(&parse_for(&legacy.to_string(), mode).unwrap(), mode).unwrap()
            );
            assert_eq!(rendered, render_for(&m, mode).unwrap());
        }
        let mut legacy = portable.clone();
        legacy["apiVersion"] = json!("ferry/v1");
        assert!(parse_for(&legacy.to_string(), "native").is_err());
        assert!(parse_for(&legacy.to_string(), "compose").is_err());
        // Auto/template parsing defaults to containers when both implementations exist.
        assert!(parse(&portable.to_string()).is_ok());
    }
    #[test]
    fn portable_inactive_sources_are_validated_and_missing_active_sources_fail() {
        let mut value = portable_sample();
        value["services"][0]["artifact"]["path"] = json!("../escape");
        assert!(parse_for(&value.to_string(), "compose").is_err());
        value = portable_sample();
        value["services"][0]["image"] = json!("bad image");
        assert!(parse_for(&value.to_string(), "native").is_err());
        value = portable_sample();
        value["services"][0]
            .as_object_mut()
            .unwrap()
            .remove("artifact");
        assert!(parse_for(&value.to_string(), "native").is_err());
        value["services"][0]
            .as_object_mut()
            .unwrap()
            .remove("image");
        assert!(parse_for(&value.to_string(), "compose").is_err());
        value = portable_sample();
        value["services"][0]
            .as_object_mut()
            .unwrap()
            .remove("image");
        assert!(parse_for(&value.to_string(), "docker").is_err());
    }
    #[test]
    fn swarm_placement_is_versioned_bounded_and_not_silently_dropped() {
        let mut value = portable_sample();
        value["services"][0]["placement"] = json!({"constraints":["node.role == worker", "node.labels.zone != zone-a"], "maxReplicasPerNode":1});
        let manifest = parse_for(&value.to_string(), "stack").unwrap();
        let doc: Value = serde_json::from_str(&render_for(&manifest, "stack").unwrap()).unwrap();
        assert_eq!(
            doc["services"]["web"]["deploy"]["placement"],
            json!({"constraints":["node.role == worker", "node.labels.zone != zone-a"], "max_replicas_per_node":1})
        );
        for mode in ["native", "docker", "compose"] {
            assert!(parse_for(&value.to_string(), mode).is_err());
            assert!(render_for(&manifest, mode).is_err());
        }
        for constraint in [
            "node.role = worker",
            "node.role == admin",
            "node.labels. == x",
            "unknown == value",
            "node.hostname == x;touch /tmp/x",
            "node.hostname == ",
        ] {
            value["services"][0]["placement"]["constraints"] = json!([constraint]);
            assert!(
                parse_for(&value.to_string(), "stack").is_err(),
                "{constraint}"
            );
        }
        value["services"][0]["placement"]["constraints"] =
            json!(vec!["node.platform.os == linux"; 33]);
        assert!(parse_for(&value.to_string(), "stack").is_err());
        value["services"][0]["placement"]["constraints"] = json!([]);
        for max in [0, 101] {
            value["services"][0]["placement"]["maxReplicasPerNode"] = json!(max);
            assert!(parse_for(&value.to_string(), "stack").is_err());
        }
        value["apiVersion"] = json!("ferry/v1");
        value["services"][0]
            .as_object_mut()
            .unwrap()
            .remove("artifact");
        assert!(parse_for(&value.to_string(), "stack").is_err());
    }
    #[test]
    fn swarm_placement_overrides_cannot_bypass_validation_or_engine_capabilities() {
        let mut value = portable_sample();
        for mode in ["native", "docker", "compose", "stack"] {
            value["overrides"] = json!({mode:{"services":{"web":{"deploy":{"placement":{"max_replicas_per_node":0}}}}}});
            // Native rejects unsupported override root entries too.
            assert!(parse_for(&value.to_string(), mode).is_err(), "{mode}");
        }
        value["overrides"] = json!({"stack":{"services":{"web":{"deploy":{"placement":{"constraints":["node.role == worker"], "max_replicas_per_node":1}}}}}});
        assert!(parse_for(&value.to_string(), "stack").is_ok());
        value["apiVersion"] = json!("ferry/v1");
        value["services"][0]
            .as_object_mut()
            .unwrap()
            .remove("artifact");
        assert!(parse_for(&value.to_string(), "stack").is_err());
    }
    #[test]
    fn stack_renders_host_ports_and_dnsrr() {
        let mut value = portable_sample();
        value["services"][0]["endpointMode"] = json!("dnsrr");
        value["services"][0]["ports"] =
            json!([{"container":80,"published":8080,"mode":"host"},{"container":81,"mode":"host"}]);
        let manifest = parse_for(&value.to_string(), "stack").unwrap();
        let doc: Value = serde_json::from_str(&render_for(&manifest, "stack").unwrap()).unwrap();
        assert_eq!(doc["services"]["web"]["deploy"]["endpoint_mode"], "dnsrr");
        assert_eq!(doc["services"]["web"]["ports"][0]["mode"], "host");
        assert_eq!(doc["services"]["web"]["ports"][1]["mode"], "host");
        let mut ingress = portable_sample();
        ingress["services"][0]["endpointMode"] = json!("vip");
        ingress["services"][0]["ports"] =
            json!([{"container":80,"published":8080,"mode":"ingress"}]);
        let manifest = parse_for(&ingress.to_string(), "stack").unwrap();
        let doc: Value = serde_json::from_str(&render_for(&manifest, "stack").unwrap()).unwrap();
        assert_eq!(doc["services"]["web"]["deploy"]["endpoint_mode"], "vip");
        assert_eq!(doc["services"]["web"]["ports"][0]["mode"], "ingress");
        // Unset fields stay absent.
        let mut plain = portable_sample();
        plain["services"][0]["ports"] = json!([{"container":80,"published":8080}]);
        let manifest = parse_for(&plain.to_string(), "stack").unwrap();
        let doc: Value = serde_json::from_str(&render_for(&manifest, "stack").unwrap()).unwrap();
        assert!(
            doc["services"]["web"]["deploy"]
                .get("endpoint_mode")
                .is_none()
        );
        assert!(doc["services"]["web"]["ports"][0].get("mode").is_none());
    }
    #[test]
    fn stack_only_fields_rejected_elsewhere() {
        let mut with_mode = portable_sample();
        with_mode["services"][0]["ports"] = json!([{"container":80,"mode":"host"}]);
        let mut with_endpoint = portable_sample();
        with_endpoint["services"][0]["endpointMode"] = json!("vip");
        for value in [&with_mode, &with_endpoint] {
            assert!(parse_for(&value.to_string(), "stack").is_ok());
            for mode in ["native", "docker", "compose"] {
                assert!(parse_for(&value.to_string(), mode).is_err(), "{mode}");
            }
            let manifest = parse_for(&value.to_string(), "stack").unwrap();
            for mode in ["native", "docker", "compose"] {
                assert!(render_for(&manifest, mode).is_err(), "{mode}");
            }
            let mut v1 = value.clone();
            v1["apiVersion"] = json!("ferry/v1");
            v1["services"][0]
                .as_object_mut()
                .unwrap()
                .remove("artifact");
            assert!(parse_for(&v1.to_string(), "stack").is_err());
        }
    }
    #[test]
    fn dnsrr_requires_host_ports() {
        let mut value = portable_sample();
        value["services"][0]["endpointMode"] = json!("dnsrr");
        for ports in [
            json!([{"container":80,"published":8080}]),
            json!([{"container":80,"published":8080,"mode":"ingress"}]),
            json!([{"container":80,"mode":"host"},{"container":81}]),
        ] {
            value["services"][0]["ports"] = ports;
            let e = parse_for(&value.to_string(), "stack").unwrap_err();
            assert!(
                e.render(crate::i18n::Locale::EnUs).contains("endpoint"),
                "{}",
                e.render(crate::i18n::Locale::EnUs)
            );
        }
        // A dnsrr service without published ports is fine.
        value["services"][0]["ports"] = json!([]);
        assert!(parse_for(&value.to_string(), "stack").is_ok());
        // Overrides cannot bypass the rule.
        let mut over = portable_sample();
        over["services"][0]["ports"] = json!([{"container":80,"published":8080}]);
        over["overrides"] =
            json!({"stack":{"services":{"web":{"deploy":{"endpoint_mode":"dnsrr"}}}}});
        assert!(parse_for(&over.to_string(), "stack").is_err());
    }
    #[test]
    fn invalid_port_mode_rejected() {
        let mut value = portable_sample();
        for mode in ["bridge", "", "HOST"] {
            value["services"][0]["ports"] = json!([{"container":80,"mode":mode}]);
            assert!(parse_for(&value.to_string(), "stack").is_err(), "{mode}");
        }
        value["services"][0]["ports"] = json!([]);
        for mode in ["rr", "", "DNSRR"] {
            value["services"][0]["endpointMode"] = json!(mode);
            assert!(parse_for(&value.to_string(), "stack").is_err(), "{mode}");
        }
        let mut over = portable_sample();
        over["overrides"] = json!({"stack":{"services":{"web":{"deploy":{"endpoint_mode":"rr"},"ports":[{"target":80,"mode":"bridge"}]}}}});
        assert!(parse_for(&over.to_string(), "stack").is_err());
    }
    fn fails(v: &Value, part: &str) {
        let e = parse(&v.to_string()).unwrap_err();
        let rendered = e.render(crate::i18n::Locale::ZhCn);
        assert!(rendered.contains(part), "{rendered}");
    }
    #[test]
    fn manifest_errors_keep_identity_and_literal_parser_details_in_six_languages() {
        use crate::i18n::{Locale, Message, render};
        let mut value = sample();
        value["services"][0]["replicas"] = json!(0);
        let error = parse(&value.to_string()).unwrap_err();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(error.2.as_ref().unwrap().key, "deployment.manifest.018");
        for locale in Locale::ALL {
            assert_eq!(
                error.render(locale),
                render(locale, &Message::new("deployment.manifest.018"))
            );
        }
        let error = parse("not: [valid\nraw: '{detail} 中文 사용자'").unwrap_err();
        let message = error.2.as_ref().unwrap();
        assert_eq!(message.key, "deployment.manifest.085");
        let detail = &message.params["detail"];
        for locale in Locale::ALL {
            assert!(error.render(locale).contains(detail));
        }
        assert!(error.1.starts_with("Invalid manifest YAML/JSON:"));
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
