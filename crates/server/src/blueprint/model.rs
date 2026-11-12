//! Cluster blueprint document (`ferry/blueprint-v1`): structure and rule validation.
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const API_VERSION: &str = "ferry/blueprint-v1";
const KIND: &str = "Blueprint";
const MAX_ROLES: usize = 10;
pub(super) const MAX_INSTANCES: u32 = 100;
const DRIVERS: [&str; 5] = ["native", "docker", "compose", "stack", "k8s"];
const ENDPOINT_KINDS: [&str; 3] = ["discovery", "vip", "direct"];
const HOOK_NAMES: [&str; 3] = ["join", "leave", "healthy"];
const MAX_HOOK_ARGS: usize = 32;
const MAX_HOOK_ARG_CHARS: usize = 1024;
const MAX_HOOK_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Blueprint {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub roles: Vec<Role>,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub drivers: BTreeMap<String, DriverSupport>,
    #[serde(default)]
    pub update: Update,
    /// Reserved: must stay empty until hooks are supported.
    #[serde(default)]
    pub hooks: Vec<Value>,
    #[serde(default)]
    pub strategy: Option<Value>,
    #[serde(default)]
    pub allow_single_host: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Role {
    pub name: String,
    pub instances: Instances,
    #[serde(default)]
    pub placement: Placement,
    pub identity: Identity,
    #[serde(default)]
    pub ports: Vec<RolePort>,
    /// ferry/v2 Service template without `name` and `ports`.
    pub service: Value,
    /// Read by failover only; the compiler ignores it.
    #[serde(default, skip_serializing_if = "RoleFailover::is_default")]
    pub failover: RoleFailover,
    /// Per-instance commands run around a replacement; the compiler ignores them.
    #[serde(default, skip_serializing_if = "RoleHooks::is_empty")]
    pub hooks: RoleHooks,
}

/// Whether a stable role's members may be replaced on another host (replicated data).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Replace {
    Supported,
    #[default]
    Unsupported,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleFailover {
    #[serde(default)]
    pub replace: Replace,
}

impl RoleFailover {
    fn is_default(&self) -> bool {
        self.replace == Replace::default()
    }
}

/// Argv lists rendered with the instance context (`.Instance`, `.Role`) when they run.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleHooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leave: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthy: Option<Vec<String>>,
}

impl RoleHooks {
    fn is_empty(&self) -> bool {
        self.join.is_none() && self.leave.is_none() && self.healthy.is_none()
    }

    fn named(&self) -> [(&'static str, Option<&Vec<String>>); 3] {
        [
            ("join", self.join.as_ref()),
            ("leave", self.leave.as_ref()),
            ("healthy", self.healthy.as_ref()),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Instances {
    pub min: u32,
    pub max: u32,
    pub default: u32,
    #[serde(default)]
    pub odd: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Placement {
    #[serde(default)]
    pub distinct_hosts: bool,
    #[serde(default)]
    pub colocate_with: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Identity {
    Stable,
    Stateless,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RolePort {
    pub name: String,
    pub container: u16,
    /// Integer or template string.
    pub published: Value,
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub name: String,
    pub role: String,
    pub port: String,
    /// discovery | vip | direct
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DriverSupport {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_addressing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    #[serde(default = "one")]
    pub max_parallel: u32,
    #[serde(default = "ten")]
    pub min_healthy_seconds: u32,
    #[serde(default = "health_timeout")]
    pub health_timeout: u32,
    #[serde(default = "yes")]
    pub auto_revert: bool,
}

fn one() -> u32 {
    1
}
fn ten() -> u32 {
    10
}
fn health_timeout() -> u32 {
    180
}
fn yes() -> bool {
    true
}

impl Default for Update {
    fn default() -> Self {
        Self {
            max_parallel: one(),
            min_healthy_seconds: ten(),
            health_timeout: health_timeout(),
            auto_revert: yes(),
        }
    }
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 63
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn is_blueprint(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|v| v.get("kind").and_then(Value::as_str) == Some(KIND))
}

pub fn parse(text: &str) -> Result<Blueprint> {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return fail(Message::new("blueprint.invalidDocument"));
    };
    check_hook_shapes(&doc)?;
    let Ok(blueprint) = serde_json::from_value::<Blueprint>(doc) else {
        return fail(Message::new("blueprint.invalidDocument"));
    };
    validate(&blueprint)?;
    Ok(blueprint)
}

fn hook_invalid<T>(role: &str, hook: &str) -> Result<T> {
    fail(
        Message::new("blueprint.hookInvalid")
            .with("role", role)
            .with("hook", hook),
    )
}

/// A role hook that is not a list of strings would otherwise surface only as an invalid
/// document; name the role and hook instead.
fn check_hook_shapes(doc: &Value) -> Result<()> {
    let roles = doc
        .get("roles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for role in roles {
        let Some(hooks) = role.get("hooks").and_then(Value::as_object) else {
            continue;
        };
        let name = role.get("name").and_then(Value::as_str).unwrap_or_default();
        for hook in HOOK_NAMES {
            let argv_ok = match hooks.get(hook) {
                None | Some(Value::Null) => true,
                Some(Value::Array(items)) => items.iter().all(Value::is_string),
                Some(_) => false,
            };
            if !argv_ok {
                return hook_invalid(name, hook);
            }
        }
    }
    Ok(())
}

/// Each hook: 1–32 arguments of 1–1024 characters, at most 4096 bytes in all, each a valid
/// template.
fn validate_hooks(role: &Role) -> Result<()> {
    for (hook, argv) in role.hooks.named() {
        let Some(argv) = argv else { continue };
        let bounded = (1..=MAX_HOOK_ARGS).contains(&argv.len())
            && argv
                .iter()
                .all(|a| (1..=MAX_HOOK_ARG_CHARS).contains(&a.chars().count()))
            && argv.iter().map(String::len).sum::<usize>() <= MAX_HOOK_BYTES;
        if !bounded || argv.iter().any(|a| super::template::check(a).is_err()) {
            return hook_invalid(&role.name, hook);
        }
    }
    Ok(())
}

fn validate(b: &Blueprint) -> Result<()> {
    if b.api_version != API_VERSION || b.kind != KIND {
        return fail(Message::new("blueprint.unsupportedVersion"));
    }
    if !valid_identifier(&b.metadata.name) {
        return fail(Message::new("blueprint.nameInvalid"));
    }
    if !b.hooks.is_empty() {
        return fail(Message::new("blueprint.hooksNotYetSupported"));
    }
    if b.roles.is_empty() || b.roles.len() > MAX_ROLES {
        return fail(Message::new("blueprint.rolesCount"));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u32;
    for role in &b.roles {
        if role.name.len() > 56 || !valid_identifier(&role.name) {
            return fail(Message::new("blueprint.roleNameInvalid").with("role", &role.name));
        }
        if !names.insert(role.name.as_str()) {
            return fail(Message::new("blueprint.roleNameDuplicate").with("role", &role.name));
        }
        let i = &role.instances;
        if i.min < 1 || i.min > i.default || i.default > i.max || i.max > MAX_INSTANCES {
            return fail(Message::new("blueprint.instancesInvalid").with("role", &role.name));
        }
        if i.odd && i.default % 2 == 0 {
            return fail(Message::new("blueprint.instancesOdd").with("role", &role.name));
        }
        total += i.default;
        validate_service(role)?;
        validate_identity(role)?;
        validate_ports(role)?;
        validate_hooks(role)?;
    }
    if total > MAX_INSTANCES {
        return fail(Message::new("blueprint.instancesTotal"));
    }
    for role in &b.roles {
        if let Some(target) = &role.placement.colocate_with
            && (target == &role.name || !names.contains(target.as_str()))
        {
            return fail(Message::new("blueprint.colocateInvalid").with("role", &role.name));
        }
    }
    let targets: BTreeMap<&str, &str> = b
        .roles
        .iter()
        .filter_map(|r| Some((r.name.as_str(), r.placement.colocate_with.as_deref()?)))
        .collect();
    for role in &b.roles {
        let mut seen = BTreeSet::from([role.name.as_str()]);
        let mut current = role.name.as_str();
        while let Some(next) = targets.get(current).copied() {
            if !seen.insert(next) {
                return fail(Message::new("blueprint.colocateInvalid").with("role", &role.name));
            }
            current = next;
        }
    }
    validate_endpoints(b)?;
    if b.drivers.keys().any(|k| !DRIVERS.contains(&k.as_str())) {
        return fail(Message::new("blueprint.driverUnknown"));
    }
    let u = &b.update;
    if u.max_parallel < 1 || u.health_timeout < 1 {
        return fail(Message::new("blueprint.updateInvalid"));
    }
    if u.max_parallel != 1 && b.roles.iter().any(|r| r.identity == Identity::Stable) {
        return fail(Message::new("blueprint.stableMaxParallel"));
    }
    Ok(())
}

fn validate_service(role: &Role) -> Result<()> {
    match role.service.as_object() {
        // A pipeline is referenced by name or id text, which deployment checks resolve (and
        // authorize) before storage; a number would bypass that and be stringified later.
        Some(service)
            if !service.contains_key("name")
                && !service.contains_key("ports")
                && service
                    .get("artifact")
                    .and_then(|a| a.get("pipeline"))
                    .is_none_or(Value::is_string) =>
        {
            Ok(())
        }
        _ => fail(Message::new("blueprint.serviceInvalid").with("role", &role.name)),
    }
}

/// A stable instance is one pinned Swarm service, so it runs exactly one task; a stateless
/// role must not pin nodes itself, since its bound hosts are what the binding checks.
fn validate_identity(role: &Role) -> Result<()> {
    let service = &role.service;
    match role.identity {
        Identity::Stable => {
            if service
                .get("replicas")
                .is_some_and(|r| r.as_u64() != Some(1))
            {
                return fail(
                    Message::new("blueprint.stableReplicasInvalid").with("role", &role.name),
                );
            }
        }
        Identity::Stateless => {
            let pins_node = service
                .pointer("/placement/constraints")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|c| {
                    let key = c.split(['=', '!']).next().unwrap_or_default();
                    let key = key.trim().to_ascii_lowercase();
                    key == "node.id" || key == "node.hostname"
                });
            if pins_node {
                return fail(
                    Message::new("blueprint.statelessPlacementInvalid").with("role", &role.name),
                );
            }
        }
    }
    Ok(())
}

fn validate_ports(role: &Role) -> Result<()> {
    let mut seen = BTreeSet::new();
    for port in &role.ports {
        let published_ok = match &port.published {
            Value::Number(n) => n.as_u64().is_some_and(|n| (1..=65535).contains(&n)),
            Value::String(s) => !s.trim().is_empty(),
            _ => false,
        };
        let protocol_ok = port
            .protocol
            .as_deref()
            .is_none_or(|p| p == "tcp" || p == "udp");
        if !valid_identifier(&port.name) || port.container == 0 || !published_ok || !protocol_ok {
            return fail(Message::new("blueprint.portInvalid").with("role", &role.name));
        }
        if !seen.insert(port.name.as_str()) {
            return fail(Message::new("blueprint.portDuplicate").with("role", &role.name));
        }
    }
    Ok(())
}

fn validate_endpoints(b: &Blueprint) -> Result<()> {
    let mut seen = BTreeSet::new();
    for e in &b.endpoints {
        let port_exists = b
            .roles
            .iter()
            .find(|r| r.name == e.role)
            .is_some_and(|r| r.ports.iter().any(|p| p.name == e.port));
        if !valid_identifier(&e.name)
            || !seen.insert(e.name.as_str())
            || !port_exists
            || !ENDPOINT_KINDS.contains(&e.kind.as_str())
        {
            return fail(Message::new("blueprint.endpointInvalid").with("endpoint", &e.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn nats() -> Value {
        json!({
          "apiVersion": "ferry/blueprint-v1",
          "kind": "Blueprint",
          "metadata": { "name": "nats-cluster", "labels": { "tier": "mq" } },
          "allowSingleHost": false,
          "roles": [
            { "name": "nats", "instances": { "min": 3, "max": 9, "default": 3, "odd": true },
              "placement": { "distinctHosts": true }, "identity": "stable",
              "ports": [{ "name": "client", "container": 4222, "published": "{{ .Values.client_port }}" },
                        { "name": "cluster", "container": 6222, "published": 6222, "protocol": "tcp" }],
              "service": { "image": "nats:2" } },
            { "name": "exporter", "instances": { "min": 3, "max": 9, "default": 3, "odd": false },
              "placement": { "colocateWith": "nats" }, "identity": "stateless",
              "ports": [{ "name": "metrics", "container": 7777, "published": 7777 }],
              "service": { "image": "natsio/prometheus-nats-exporter" } },
            { "name": "gateway", "instances": { "min": 1, "max": 2, "default": 1, "odd": false },
              "placement": {}, "identity": "stateless", "ports": [],
              "service": { "image": "nginx" } }
          ],
          "endpoints": [{ "name": "client", "role": "nats", "port": "client", "kind": "discovery" }],
          "drivers": { "stack": { "supported": true, "memberAddressing": "per-instance-service" }, "native": { "supported": true } },
          "update": { "maxParallel": 1, "minHealthySeconds": 10, "healthTimeout": 180, "autoRevert": true },
          "hooks": [],
          "strategy": { "type": "recreate" }
        })
    }

    fn parse_v(v: &Value) -> Result<Blueprint> {
        parse(&v.to_string())
    }

    fn err_key(v: &Value) -> String {
        let e = parse_v(v).err().expect("expected error");
        e.2.map(|m| m.key).unwrap_or_default()
    }

    #[test]
    fn nats_blueprint_parses() {
        let b = parse_v(&nats()).unwrap();
        assert_eq!(b.metadata.name, "nats-cluster");
        assert_eq!(b.roles.len(), 3);
        assert_eq!(b.roles[0].identity, Identity::Stable);
        assert_eq!(b.roles[0].instances.default, 3);
        assert!(b.roles[0].instances.odd);
        assert!(b.roles[0].placement.distinct_hosts);
        assert_eq!(b.roles[1].placement.colocate_with.as_deref(), Some("nats"));
        assert_eq!(b.roles[0].ports[1].protocol.as_deref(), Some("tcp"));
        assert_eq!(
            b.roles[0].ports[0].published,
            json!("{{ .Values.client_port }}")
        );
        assert_eq!(b.endpoints[0].kind, "discovery");
        assert!(b.drivers["stack"].supported);
        assert_eq!(b.update.health_timeout, 180);
        assert!(!b.allow_single_host);
        assert!(b.strategy.is_some());
        assert!(is_blueprint(&nats().to_string()));
        assert!(!is_blueprint(r#"{"kind":"Application"}"#));
        assert!(!is_blueprint("not json"));
    }

    #[test]
    fn update_defaults_when_omitted() {
        let mut v = nats();
        v.as_object_mut().unwrap().remove("update");
        let b = parse_v(&v).unwrap();
        assert_eq!(
            (
                b.update.max_parallel,
                b.update.min_healthy_seconds,
                b.update.health_timeout
            ),
            (1, 10, 180)
        );
        assert!(b.update.auto_revert);
    }

    #[test]
    fn instance_bounds_enforced() {
        let cases = [
            json!({ "min": 4, "max": 9, "default": 3, "odd": false }),
            json!({ "min": 0, "max": 9, "default": 3, "odd": false }),
            json!({ "min": 3, "max": 101, "default": 3, "odd": false }),
            json!({ "min": 2, "max": 9, "default": 4, "odd": true }),
            json!({ "min": 3, "max": 2, "default": 3, "odd": false }),
        ];
        for c in cases {
            let mut v = nats();
            v["roles"][0]["instances"] = c.clone();
            assert!(parse_v(&v).is_err(), "{c}");
        }
        let mut v = nats();
        v["roles"][0]["instances"] = json!({ "min": 3, "max": 100, "default": 3, "odd": true });
        assert!(parse_v(&v).is_ok());
    }

    #[test]
    fn role_and_total_limits() {
        let mut v = nats();
        v["roles"][0]["instances"] = json!({ "min": 1, "max": 100, "default": 99, "odd": true });
        assert_eq!(err_key(&v), "blueprint.instancesTotal");
        let mut v = nats();
        let role = v["roles"][2].clone();
        let roles: Vec<Value> = (0..11)
            .map(|i| {
                let mut r = role.clone();
                r["name"] = json!(format!("r{i}"));
                r
            })
            .collect();
        v["roles"] = json!(roles);
        v["endpoints"] = json!([]);
        assert_eq!(err_key(&v), "blueprint.rolesCount");
        v["roles"] = json!([]);
        assert_eq!(err_key(&v), "blueprint.rolesCount");
    }

    #[test]
    fn colocate_target_must_exist_and_differ() {
        let mut v = nats();
        v["roles"][1]["placement"]["colocateWith"] = json!("missing");
        assert_eq!(err_key(&v), "blueprint.colocateInvalid");
        v["roles"][1]["placement"]["colocateWith"] = json!("exporter");
        assert_eq!(err_key(&v), "blueprint.colocateInvalid");
    }

    #[test]
    fn colocate_cycles_rejected() {
        let mut v = nats();
        v["roles"][0]["placement"]["colocateWith"] = json!("exporter");
        assert_eq!(err_key(&v), "blueprint.colocateInvalid");
        let mut v = nats();
        v["roles"][0]["placement"]["colocateWith"] = json!("gateway");
        v["roles"][2]["placement"]["colocateWith"] = json!("exporter");
        assert_eq!(err_key(&v), "blueprint.colocateInvalid");
    }

    #[test]
    fn role_name_length_capped() {
        let mut v = nats();
        v["roles"][2]["name"] = json!("a".repeat(56));
        assert!(parse_v(&v).is_ok());
        v["roles"][2]["name"] = json!("a".repeat(57));
        assert_eq!(err_key(&v), "blueprint.roleNameInvalid");
    }

    #[test]
    fn duplicate_role_or_port_names_rejected() {
        let mut v = nats();
        v["roles"][1]["name"] = json!("nats");
        v["roles"][1]["placement"] = json!({});
        assert_eq!(err_key(&v), "blueprint.roleNameDuplicate");
        let mut v = nats();
        v["roles"][0]["ports"][1]["name"] = json!("client");
        assert_eq!(err_key(&v), "blueprint.portDuplicate");
    }

    #[test]
    fn hooks_not_yet_supported() {
        let mut v = nats();
        v["hooks"] =
            json!([{ "event": "post-add", "role": "nats", "run": "any", "command": ["true"] }]);
        assert_eq!(err_key(&v), "blueprint.hooksNotYetSupported");
        v.as_object_mut().unwrap().remove("hooks");
        assert!(parse_v(&v).is_ok());
    }

    #[test]
    fn role_failover_and_hooks_parse() {
        let b = parse_v(&nats()).unwrap();
        for role in &b.roles {
            assert_eq!(role.failover.replace, Replace::Unsupported);
            assert_eq!(role.hooks.join, None);
            assert_eq!(role.hooks.leave, None);
            assert_eq!(role.hooks.healthy, None);
        }
        let mut v = nats();
        v["roles"][0]["failover"] = json!({"replace": "supported"});
        v["roles"][0]["hooks"] = json!({
            "join": ["nats-join", "{{ .Instance.Name }}"],
            "leave": ["nats-leave", "{{ .Instance.Host }}"],
            "healthy": ["curl", "-f", "http://127.0.0.1:8222/healthz"]
        });
        let b = parse_v(&v).unwrap();
        let role = &b.roles[0];
        assert_eq!(role.failover.replace, Replace::Supported);
        assert_eq!(
            role.hooks.join.as_deref(),
            Some(&["nats-join".to_string(), "{{ .Instance.Name }}".to_string()][..])
        );
        assert_eq!(role.hooks.leave.as_ref().unwrap().len(), 2);
        assert_eq!(role.hooks.healthy.as_ref().unwrap()[0], "curl");
        // Defaults are not written back.
        let text = serde_json::to_string(&parse_v(&nats()).unwrap().roles[0]).unwrap();
        assert!(
            !text.contains("failover") && !text.contains("hooks"),
            "{text}"
        );
        let mut v = nats();
        v["roles"][0]["failover"] = json!({"replace": "sometimes"});
        assert_eq!(err_key(&v), "blueprint.invalidDocument");
        let mut v = nats();
        v["roles"][0]["failover"] = json!({"replace": "supported", "promote": true});
        assert_eq!(err_key(&v), "blueprint.invalidDocument");
        let mut v = nats();
        v["roles"][0]["hooks"] = json!({"backup": ["true"]});
        assert_eq!(err_key(&v), "blueprint.invalidDocument");
    }

    #[test]
    fn hooks_must_be_bounded_argv() {
        let long_items: Vec<String> = (0..5).map(|_| "x".repeat(1000)).collect();
        let cases = [
            ("join", json!([])),
            (
                "leave",
                json!((0..33).map(|i| format!("a{i}")).collect::<Vec<_>>()),
            ),
            ("healthy", json!(["ok", "x".repeat(1025)])),
            ("join", json!(long_items)),
            ("leave", json!(["ok", 7])),
            ("healthy", json!("true")),
            ("join", json!(["echo", "{{ .Bogus }}"])),
            ("join", json!([""])),
        ];
        for (hook, list) in cases {
            let mut v = nats();
            v["roles"][1]["hooks"] = json!({ hook: list.clone() });
            let e = parse_v(&v).expect_err(&format!("{hook}: {list}"));
            let m = e.2.expect("localized");
            assert_eq!(m.key, "blueprint.hookInvalid", "{hook}: {list}");
            assert_eq!(m.params["role"], "exporter");
            assert_eq!(m.params["hook"], hook);
        }
        // The limits themselves are allowed.
        let mut v = nats();
        v["roles"][1]["hooks"] = json!({
            "join": (0..32).map(|i| format!("a{i}")).collect::<Vec<_>>(),
            "leave": ["x".repeat(1024)],
            "healthy": (0..4).map(|_| "y".repeat(1024)).collect::<Vec<_>>(),
        });
        assert!(parse_v(&v).is_ok());
        // An explicit null is the same as no hook.
        let mut v = nats();
        v["roles"][1]["hooks"] = json!({"join": null});
        assert_eq!(parse_v(&v).unwrap().roles[1].hooks.join, None);
    }

    #[test]
    fn role_hooks_do_not_change_compiled_manifest() {
        let mut plain = nats();
        plain["roles"][0]["ports"][0]["published"] = json!(4222);
        plain["drivers"] = json!({"docker": {"supported": true}});
        let mut hooked = plain.clone();
        hooked["roles"][0]["failover"] = json!({"replace": "supported"});
        hooked["roles"][0]["hooks"] = json!({
            "join": ["nats-join", "{{ .Instance.Name }}"],
            "healthy": ["true"]
        });
        hooked["roles"][2]["failover"] = json!({"replace": "unsupported"});
        let hosts = |n: usize| -> Vec<Value> {
            (1..=n)
                .map(|i| json!({"hostId": i.to_string(), "address": format!("10.0.0.{i}")}))
                .collect()
        };
        let binding = super::super::binding::parse(
            &json!({"driver": "docker", "roles": {
                "nats": hosts(3), "exporter": hosts(3), "gateway": hosts(1)}})
            .to_string(),
        )
        .unwrap();
        let compile = |v: &Value| {
            super::super::compile::compile_unresolved(&v.to_string(), &binding)
                .unwrap()
                .into_iter()
                .map(|p| (p.host_id, p.manifest))
                .collect::<Vec<_>>()
        };
        let before = compile(&plain);
        assert_eq!(before.len(), 3);
        assert_eq!(compile(&hooked), before);
    }

    #[test]
    fn stable_role_forces_max_parallel_one() {
        let mut v = nats();
        v["update"]["maxParallel"] = json!(2);
        assert_eq!(err_key(&v), "blueprint.stableMaxParallel");
        v["roles"][0]["identity"] = json!("stateless");
        assert!(parse_v(&v).is_ok());
    }

    #[test]
    fn stable_role_replicas_must_be_one() {
        let mut v = nats();
        v["roles"][0]["service"]["replicas"] = json!(3);
        assert_eq!(err_key(&v), "blueprint.stableReplicasInvalid");
        let e = parse_v(&v).unwrap_err().2.unwrap();
        assert_eq!(e.params["role"], "nats");
        v["roles"][0]["service"]["replicas"] = json!("3");
        assert_eq!(err_key(&v), "blueprint.stableReplicasInvalid");
        v["roles"][0]["service"]["replicas"] = json!(1);
        assert!(parse_v(&v).is_ok());
        v["roles"][0]["service"]
            .as_object_mut()
            .unwrap()
            .remove("replicas");
        assert!(parse_v(&v).is_ok());
        // Stateless roles may carry replicas; the stack compiler replaces them.
        let mut v = nats();
        v["roles"][2]["service"]["replicas"] = json!(3);
        assert!(parse_v(&v).is_ok());
    }

    #[test]
    fn stateless_role_cannot_pin_nodes() {
        for constraint in [
            "node.id==abc",
            "node.hostname==web1",
            " Node.ID != abc",
            "node.hostname!=web1",
        ] {
            let mut v = nats();
            v["roles"][2]["service"]["placement"] = json!({"constraints": [constraint]});
            assert_eq!(
                err_key(&v),
                "blueprint.statelessPlacementInvalid",
                "{constraint}"
            );
            let e = parse_v(&v).unwrap_err().2.unwrap();
            assert_eq!(e.params["role"], "gateway");
        }
        let mut v = nats();
        v["roles"][2]["service"]["placement"] = json!({"constraints": [
            "node.role==worker", "node.labels.zone==a", "node.platform.os==linux"]});
        assert!(parse_v(&v).is_ok());
    }

    #[test]
    fn endpoint_must_reference_role_port() {
        let mut v = nats();
        v["endpoints"][0]["port"] = json!("nope");
        assert_eq!(err_key(&v), "blueprint.endpointInvalid");
        let mut v = nats();
        v["endpoints"][0]["role"] = json!("nope");
        assert_eq!(err_key(&v), "blueprint.endpointInvalid");
        let mut v = nats();
        v["endpoints"][0]["kind"] = json!("lb");
        assert_eq!(err_key(&v), "blueprint.endpointInvalid");
    }

    #[test]
    fn unknown_driver_key_rejected() {
        let mut v = nats();
        v["drivers"]["podman"] = json!({ "supported": true });
        assert_eq!(err_key(&v), "blueprint.driverUnknown");
        for k in ["native", "docker", "compose", "stack", "k8s"] {
            let mut v = nats();
            v["drivers"] = json!({ k: { "supported": true } });
            assert!(parse_v(&v).is_ok(), "{k}");
        }
    }

    #[test]
    fn envelope_name_and_service_rules() {
        let mut v = nats();
        v["apiVersion"] = json!("ferry/blueprint-v2");
        assert_eq!(err_key(&v), "blueprint.unsupportedVersion");
        let mut v = nats();
        v["roles"][0]["service"]["ports"] = json!([]);
        assert_eq!(err_key(&v), "blueprint.serviceInvalid");
        let mut v = nats();
        v["roles"][0]["service"]["name"] = json!("x");
        assert_eq!(err_key(&v), "blueprint.serviceInvalid");
        for pipeline in [json!(7), json!(null), json!(["build"])] {
            let mut v = nats();
            v["roles"][0]["service"]["artifact"] =
                json!({"source": "pipeline", "pipeline": pipeline, "path": "bin/nats"});
            assert_eq!(err_key(&v), "blueprint.serviceInvalid", "{pipeline}");
        }
        let mut v = nats();
        v["roles"][0]["service"]["artifact"] =
            json!({"source": "pipeline", "pipeline": "7", "path": "bin/nats"});
        assert!(parse_v(&v).is_ok());
        let mut v = nats();
        v["metadata"]["name"] = json!("Bad Name");
        assert_eq!(err_key(&v), "blueprint.nameInvalid");
        let mut v = nats();
        v["surprise"] = json!(1);
        assert_eq!(err_key(&v), "blueprint.invalidDocument");
    }
}
