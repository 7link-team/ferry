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
const MAX_INSTANCES: u32 = 100;
const DRIVERS: [&str; 5] = ["native", "docker", "compose", "stack", "k8s"];
const ENDPOINT_KINDS: [&str; 3] = ["discovery", "vip", "direct"];

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
    let Ok(blueprint) = serde_json::from_str::<Blueprint>(text) else {
        return fail(Message::new("blueprint.invalidDocument"));
    };
    validate(&blueprint)?;
    Ok(blueprint)
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
        if !valid_identifier(&role.name) {
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
        validate_ports(role)?;
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
        Some(service) if !service.contains_key("name") && !service.contains_key("ports") => Ok(()),
        _ => fail(Message::new("blueprint.serviceInvalid").with("role", &role.name)),
    }
}

fn validate_ports(role: &Role) -> Result<()> {
    let mut seen = BTreeSet::new();
    for port in &role.ports {
        let published_ok = match &port.published {
            Value::Number(n) => n.as_u64().is_some_and(|n| (1..=65535).contains(&n)),
            Value::String(s) => !s.trim().is_empty(),
            _ => false,
        };
        let protocol_ok = port.protocol.as_deref().is_none_or(|p| p == "tcp" || p == "udp");
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
        assert_eq!(b.roles[0].ports[0].published, json!("{{ .Values.client_port }}"));
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
            (b.update.max_parallel, b.update.min_healthy_seconds, b.update.health_timeout),
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
        v["hooks"] = json!([{ "event": "post-add", "role": "nats", "run": "any", "command": ["true"] }]);
        assert_eq!(err_key(&v), "blueprint.hooksNotYetSupported");
        v.as_object_mut().unwrap().remove("hooks");
        assert!(parse_v(&v).is_ok());
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
        let mut v = nats();
        v["metadata"]["name"] = json!("Bad Name");
        assert_eq!(err_key(&v), "blueprint.nameInvalid");
        let mut v = nats();
        v["surprise"] = json!(1);
        assert_eq!(err_key(&v), "blueprint.invalidDocument");
    }
}
