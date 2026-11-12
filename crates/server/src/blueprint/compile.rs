//! Driver adapters: a phase-1-rendered blueprint plus a binding → one ferry/v2 Application per host.
use super::binding::Binding;
use super::model::{self, Blueprint, Identity, Role};
use super::template::{self, Context, Instance, Member, Phase};
use crate::deployments::manifest;
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::snowflake::Id;
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// One target host's deployment.
#[derive(Clone, Debug)]
pub struct HostPlan {
    pub host_id: Id,
    /// ferry/v2 Application JSON.
    pub manifest: String,
    /// Service (instance) name → the context its config files render with.
    pub contexts: BTreeMap<String, Context>,
}

/// Where each instance runs and how members address each other.
#[derive(Clone, Debug, Default)]
pub struct Topology {
    /// Role name → members ordered by index.
    pub members: BTreeMap<String, Vec<Member>>,
    /// (host, service name, instance) in blueprint role order, then index.
    pub placement: Vec<(Id, String, Instance)>,
}

pub trait DriverAdapter {
    fn id(&self) -> &'static str;
    /// Resolves every instance's advertised `.Host` and `.Port` values.
    fn resolve(
        &self,
        bp: &Blueprint,
        b: &Binding,
        values: &BTreeMap<String, Value>,
    ) -> Result<Topology>;
}

/// native / docker: members reach each other at the bound address and the published port.
struct AddressAdapter(&'static str);

impl DriverAdapter for AddressAdapter {
    fn id(&self) -> &'static str {
        self.0
    }
    fn resolve(
        &self,
        bp: &Blueprint,
        b: &Binding,
        values: &BTreeMap<String, Value>,
    ) -> Result<Topology> {
        topology(bp, b, |role, base| {
            let ports = published_ports(role, base, values, self.0)?;
            Ok((base.host.clone(), ports))
        })
    }
}

/// compose: one project on one host; members reach each other by service name and
/// container port.
struct ComposeAdapter;

impl DriverAdapter for ComposeAdapter {
    fn id(&self) -> &'static str {
        "compose"
    }
    fn resolve(
        &self,
        bp: &Blueprint,
        b: &Binding,
        _: &BTreeMap<String, Value>,
    ) -> Result<Topology> {
        let mut hosts = b.roles.values().flatten().map(|i| i.host_id);
        if let Some(first) = hosts.next()
            && hosts.any(|h| h != first)
        {
            return fail(Message::new("blueprint.bindingComposeSingleHost"));
        }
        topology(bp, b, |role, base| {
            Ok((base.name.clone(), container_ports(role)))
        })
    }
}

/// stack: one Swarm stack deployed through the manager. A stable instance is its own
/// service pinned to its node and reached at the bound address and published (host-mode)
/// port; a stateless role is one replicated service reached by its service name (VIP) and
/// container port, with one member per replica.
struct StackAdapter;

impl DriverAdapter for StackAdapter {
    fn id(&self) -> &'static str {
        "stack"
    }
    fn resolve(
        &self,
        bp: &Blueprint,
        b: &Binding,
        values: &BTreeMap<String, Value>,
    ) -> Result<Topology> {
        let Some(manager) = b.manager else {
            return fail(Message::new("blueprint.bindingInvalid"));
        };
        let mut topo = topology(bp, b, |role, base| match role.identity {
            Identity::Stable => Ok((
                base.host.clone(),
                published_ports(role, base, values, "stack")?,
            )),
            Identity::Stateless => Ok((role.name.clone(), container_ports(role))),
        })?;
        // Every service lives on the manager; a stateless role keeps only its first instance,
        // renamed to the role, as the one context all replicas share.
        let identity: BTreeMap<&str, Identity> = bp
            .roles
            .iter()
            .map(|r| (r.name.as_str(), r.identity))
            .collect();
        for (_, name, instance) in std::mem::take(&mut topo.placement) {
            match identity[instance.role.as_str()] {
                Identity::Stable => topo.placement.push((manager, name, instance)),
                Identity::Stateless if instance.index == 0 => {
                    let role = instance.role.clone();
                    let instance = Instance {
                        name: role.clone(),
                        ..instance
                    };
                    topo.placement.push((manager, role, instance));
                }
                Identity::Stateless => {}
            }
        }
        Ok(topo)
    }
}

fn container_ports(role: &Role) -> BTreeMap<String, u16> {
    role.ports
        .iter()
        .map(|p| (p.name.clone(), p.container))
        .collect()
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

fn adapter(driver: &str) -> Result<Box<dyn DriverAdapter>> {
    match driver {
        "native" => Ok(Box::new(AddressAdapter("native"))),
        "docker" => Ok(Box::new(AddressAdapter("docker"))),
        "compose" => Ok(Box::new(ComposeAdapter)),
        "stack" => Ok(Box::new(StackAdapter)),
        "k8s" => fail(Message::new("blueprint.driverNotImplemented")),
        _ => fail(Message::new("blueprint.driverUnsupported").with("driver", driver)),
    }
}

/// The instance as bound: `host` is the binding address and `ports` is still empty.
fn bound(role: &Role, index: u32, address: &str) -> Instance {
    Instance {
        role: role.name.clone(),
        index,
        name: format!("{}-{index}", role.name),
        host: address.to_owned(),
        ports: BTreeMap::new(),
    }
}

/// Builds members and placement; `advertise` maps a bound instance to the host and
/// port map other members use to reach it.
fn topology(
    bp: &Blueprint,
    b: &Binding,
    advertise: impl Fn(&Role, &Instance) -> Result<(String, BTreeMap<String, u16>)>,
) -> Result<Topology> {
    let mut topo = Topology::default();
    for role in &bp.roles {
        let Some(list) = b.roles.get(&role.name) else {
            return fail(Message::new("blueprint.bindingRoleMissing").with("role", &role.name));
        };
        let mut members = Vec::with_capacity(list.len());
        for (index, inst) in (0u32..).zip(list) {
            let base = bound(role, index, &inst.address);
            let (host, ports) = advertise(role, &base)?;
            members.push(Member {
                name: base.name.clone(),
                index,
                host: host.clone(),
                ports: ports.clone(),
            });
            topo.placement.push((
                inst.host_id,
                base.name.clone(),
                Instance {
                    host,
                    ports,
                    ..base
                },
            ));
        }
        topo.members.insert(role.name.clone(), members);
    }
    Ok(topo)
}

/// Port name → published port (an integer, or text rendering to one, in 1..=65535).
/// Rendered from the bound instance without members, so every driver sees the same numbers.
fn published_ports(
    role: &Role,
    base: &Instance,
    values: &BTreeMap<String, Value>,
    driver: &str,
) -> Result<BTreeMap<String, u16>> {
    let ctx = Context {
        values: values.clone(),
        instance: Some(base.clone()),
        members: BTreeMap::new(),
        driver: driver.to_owned(),
    };
    let mut ports = BTreeMap::new();
    for port in &role.ports {
        let value = match &port.published {
            Value::String(text) => template::render_value(text, &ctx, Phase::Instance)?,
            other => other.clone(),
        };
        let number = match &value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        };
        let Some(number) = number
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n != 0)
        else {
            return fail(Message::new("blueprint.portInvalid").with("role", &role.name));
        };
        ports.insert(port.name.clone(), number);
    }
    Ok(ports)
}

/// Renders every string in `value` with typed whole-value substitution; keys stay literal.
fn render_json(value: &Value, ctx: &Context) -> Result<Value> {
    Ok(match value {
        Value::String(text) => template::render_value(text, ctx, Phase::Instance)?,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| render_json(v, ctx))
                .collect::<Result<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), render_json(v, ctx)?)))
                .collect::<Result<Map<_, _>>>()?,
        ),
        other => other.clone(),
    })
}

/// How a ferry/v2 Service field is typed, mirroring the structs in
/// `crate::deployments::manifest` (Service, BuildArtifact, Health/Probe/HttpGet, ConfigFile,
/// Environment, Volume, Placement). Keep in sync when those structs change.
enum Kind {
    /// `String` / `Option<String>`: a rendered number or boolean becomes its text form.
    Text,
    /// Numeric or free-form `Value` field: the rendered value keeps its type.
    Typed,
    List(&'static Kind),
    Object(&'static [(&'static str, Kind)]),
}

const PROBE: Kind = Kind::Object(&[
    (
        "httpGet",
        Kind::Object(&[("path", Kind::Text), ("port", Kind::Typed)]),
    ),
    ("periodSeconds", Kind::Typed),
    ("timeoutSeconds", Kind::Typed),
    ("failureThreshold", Kind::Typed),
]);

/// `name` and `ports` are generated by the compiler, so they are not listed.
const SERVICE: Kind = Kind::Object(&[
    ("image", Kind::Text),
    (
        "artifact",
        Kind::Object(&[
            ("source", Kind::Text),
            ("pipeline", Kind::Text),
            ("path", Kind::Text),
            ("run", Kind::Text),
        ]),
    ),
    ("resources", Kind::Typed),
    (
        "health",
        Kind::Object(&[("readiness", PROBE), ("liveness", PROBE)]),
    ),
    (
        "files",
        Kind::List(&Kind::Object(&[
            ("path", Kind::Text),
            ("fromTemplate", Kind::Text),
            ("values", Kind::Typed),
        ])),
    ),
    ("command", Kind::List(&Kind::Text)),
    ("replicas", Kind::Typed),
    (
        "env",
        Kind::List(&Kind::Object(&[
            ("name", Kind::Text),
            ("value", Kind::Text),
            ("valueFrom", Kind::Text),
        ])),
    ),
    (
        "volumes",
        Kind::List(&Kind::Object(&[("name", Kind::Text), ("path", Kind::Text)])),
    ),
    ("dependsOn", Kind::List(&Kind::Text)),
    ("endpointMode", Kind::Text),
    (
        "placement",
        Kind::Object(&[
            ("constraints", Kind::List(&Kind::Text)),
            ("maxReplicasPerNode", Kind::Typed),
        ]),
    ),
]);

/// Turns rendered numbers/booleans back into text where the schema expects a string, so a
/// whole-value action such as `{{ .Instance.Port "data" }}` works in `command` or `env`.
/// Unknown keys and mismatched shapes are left alone for `parse_for` to report.
fn coerce(value: &mut Value, kind: &Kind) {
    match (kind, value) {
        (Kind::Text, v @ (Value::Number(_) | Value::Bool(_))) => *v = Value::String(v.to_string()),
        (Kind::List(item), Value::Array(items)) => items.iter_mut().for_each(|v| coerce(v, item)),
        (Kind::Object(fields), Value::Object(map)) => {
            for (key, field) in fields.iter() {
                if let Some(v) = map.get_mut(*key) {
                    coerce(v, field);
                }
            }
        }
        _ => {}
    }
}

#[derive(Default)]
struct Target {
    services: Vec<Value>,
    contexts: BTreeMap<String, Context>,
    /// (published, protocol) → (where it is bound, service name) for each claim.
    ports: BTreeMap<(u16, String), Vec<(Bind, String)>>,
}

/// Where a published port is bound within one target.
#[derive(Clone, PartialEq, Eq)]
enum Bind {
    /// On one machine: the target host, or one Swarm node (host mode) by node ID.
    Node(String),
    /// On every node of the Swarm (ingress routing mesh).
    Everywhere,
}

impl Target {
    /// Records `name`'s claim on a published port; two claims collide when they can land on
    /// the same machine.
    fn claim(&mut self, name: &str, bind: Bind, number: u16, protocol: &str) -> Result<()> {
        let claims = self.ports.entry((number, protocol.to_owned())).or_default();
        let clash = claims.iter().find(|(other, _)| {
            *other == bind || *other == Bind::Everywhere || bind == Bind::Everywhere
        });
        if let Some((_, other)) = clash {
            return fail(
                Message::new("blueprint.portConflict")
                    .with("instance", name)
                    .with("other", other)
                    .with("port", format!("{number}/{protocol}")),
            );
        }
        claims.push((bind, name.to_owned()));
        Ok(())
    }
}

/// Compiles a stored (phase-1) blueprint text and a validated binding into one ferry/v2
/// Application per host, named after the blueprint (`metadata.name`, owned by the template
/// and stable across deployment renames). `.Values` are already inlined, so only instance
/// actions remain.
///
/// `stack` compiles to a single Application on `binding.manager` and needs `nodes`
/// (instance name → Swarm node ID, from `swarm::assign`) to pin stable instances; other
/// drivers ignore it.
pub fn compile(
    bp_text: &str,
    binding: &Binding,
    nodes: Option<&BTreeMap<String, String>>,
) -> Result<Vec<HostPlan>> {
    let bp = model::parse(bp_text)?;
    let adapter = adapter(&binding.driver)?;
    let driver = adapter.id();
    let stack = driver == "stack";
    let no_nodes = BTreeMap::new();
    let nodes = match nodes {
        Some(nodes) => nodes,
        None if stack => return fail(Message::new("blueprint.swarmNodesRequired")),
        None => &no_nodes,
    };
    let values = BTreeMap::new();
    let topo = adapter.resolve(&bp, binding, &values)?;
    let roles: BTreeMap<&str, &Role> = bp.roles.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut targets: BTreeMap<Id, Target> = BTreeMap::new();
    // Placement was resolved from this `bp` and `binding`, so these lookups always hit.
    for (host_id, name, instance) in topo.placement {
        let role = roles[instance.role.as_str()];
        let bound_at = &binding.roles[&role.name][instance.index as usize];
        let address = &bound_at.address;
        // A non-stack target is one host; a stack binds host-mode ports on the instance's
        // node and ingress ports on every node.
        let (bind, mode) = match (stack, role.identity) {
            (false, _) => (Bind::Node(String::new()), None),
            (true, Identity::Stable) => {
                let Some(node) = nodes.get(&name) else {
                    return fail(
                        Message::new("blueprint.swarmNodeUnavailable")
                            .with("host", bound_at.host_id.to_string()),
                    );
                };
                (Bind::Node(node.clone()), Some("host"))
            }
            (true, Identity::Stateless) => (Bind::Everywhere, Some("ingress")),
        };
        let published =
            published_ports(role, &bound(role, instance.index, address), &values, driver)?;
        let ctx = Context {
            values: values.clone(),
            instance: Some(instance),
            members: topo.members.clone(),
            driver: driver.to_owned(),
        };
        let mut rendered = render_json(&role.service, &ctx)?;
        coerce(&mut rendered, &SERVICE);
        let Value::Object(mut service) = rendered else {
            return fail(Message::new("blueprint.serviceInvalid").with("role", &role.name));
        };
        let target = targets.entry(host_id).or_default();
        let mut ports = Vec::with_capacity(role.ports.len());
        for port in &role.ports {
            let protocol = port.protocol.clone().unwrap_or_else(|| "tcp".to_owned());
            let number = published[&port.name];
            target.claim(&name, bind.clone(), number, &protocol)?;
            let mut entry = json!({"name": port.name, "container": port.container,
                                   "published": number, "protocol": protocol});
            if let Some(mode) = mode {
                entry["mode"] = json!(mode);
            }
            ports.push(entry);
        }
        service.insert("name".into(), json!(name));
        service.insert("ports".into(), json!(ports));
        match (&bind, mode) {
            // Stable stack instance: pinned to its node, addressed directly (no VIP).
            (Bind::Node(node), Some(_)) => {
                let placement = service.entry("placement").or_insert_with(|| json!({}));
                if let Value::Object(placement) = placement {
                    placement.insert("constraints".into(), json!([format!("node.id=={node}")]));
                }
                service.insert("endpointMode".into(), json!("dnsrr"));
            }
            // Stateless stack role: one service with a replica per bound instance.
            (Bind::Everywhere, _) => {
                service.insert("replicas".into(), json!(binding.roles[&role.name].len()));
            }
            (Bind::Node(_), None) => {}
        }
        target.services.push(Value::Object(service));
        target.contexts.insert(name, ctx);
    }

    let mut plans = Vec::with_capacity(targets.len());
    for (host_id, target) in targets {
        let mut doc = json!({
            "apiVersion": "ferry/v2",
            "kind": "Application",
            "metadata": {"name": bp.metadata.name},
            "services": target.services,
        });
        if let Some(strategy) = &bp.strategy {
            doc["strategy"] = strategy.clone();
        }
        let manifest = doc.to_string();
        manifest::parse_for(&manifest, driver)?;
        plans.push(HostPlan {
            host_id,
            manifest,
            contexts: target.contexts,
        });
    }
    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deployments::manifest;
    use serde_json::json;

    fn blueprint(roles: Vec<Value>) -> String {
        json!({
            "apiVersion": "ferry/blueprint-v1",
            "kind": "Blueprint",
            "metadata": {"name": "demo"},
            "roles": roles,
            "drivers": {"native": {"supported": true}, "docker": {"supported": true},
                        "compose": {"supported": true}, "stack": {"supported": true},
                        "k8s": {"supported": true}},
            "allowSingleHost": true,
            "strategy": {"type": "recreate", "healthTimeout": 180}
        })
        .to_string()
    }

    fn role(name: &str, count: u32, placement: Value, ports: Value, service: Value) -> Value {
        json!({
            "name": name,
            "instances": {"min": 1, "max": 9, "default": count, "odd": false},
            "placement": placement,
            "identity": "stable",
            "ports": ports,
            "service": service
        })
    }

    const ROUTES: &str =
        r#"{{ range .Role "nats" }}nats://{{ .Host }}:{{ .Port "cluster" }},{{ end }}"#;

    fn nats_native() -> String {
        blueprint(vec![role(
            "nats",
            3,
            json!({"distinctHosts": true}),
            json!([{"name": "client", "container": 4222, "published": 4222},
                   {"name": "cluster", "container": 6222, "published": "6222", "protocol": "tcp"}]),
            json!({"artifact": {"source": "pipeline", "pipeline": "build", "path": "bin/nats"},
                   "command": ["nats-server", "--name", "{{ .Instance.Name }}", "--routes", ROUTES]}),
        )])
    }

    fn binding(driver: &str, roles: &[(&str, &[(i64, &str)])]) -> Binding {
        let roles = roles
            .iter()
            .map(|(role, list)| {
                let list: Vec<Value> = list
                    .iter()
                    .map(|(h, a)| json!({"hostId": Id::from(*h), "address": a}))
                    .collect();
                (role.to_string(), json!(list))
            })
            .collect::<serde_json::Map<_, _>>();
        super::super::binding::parse(&json!({"driver": driver, "roles": roles}).to_string())
            .expect("binding")
    }

    fn three_hosts(role: &'static str) -> (&'static str, &'static [(i64, &'static str)]) {
        (role, &[(1, "10.0.0.1"), (2, "10.0.0.2"), (3, "10.0.0.3")])
    }

    fn doc(plan: &HostPlan) -> Value {
        serde_json::from_str(&plan.manifest).unwrap()
    }

    fn err_key(r: Result<Vec<HostPlan>>) -> (String, BTreeMap<String, String>) {
        let m = r.expect_err("expected error").2.expect("localized");
        (m.key, m.params)
    }

    #[test]
    fn native_three_hosts_one_application_each() {
        let b = binding("native", &[three_hosts("nats")]);
        let plans = compile(&nats_native(), &b, None).unwrap();
        assert_eq!(plans.len(), 3);
        for (i, plan) in plans.iter().enumerate() {
            assert_eq!(plan.host_id, Id::from(i as i64 + 1));
            let d = doc(plan);
            assert_eq!(d["apiVersion"], "ferry/v2");
            assert_eq!(d["kind"], "Application");
            assert_eq!(d["metadata"]["name"], "demo");
            assert_eq!(d["strategy"]["type"], "recreate");
            let services = d["services"].as_array().unwrap();
            assert_eq!(services.len(), 1);
            let s = &services[0];
            assert_eq!(s["name"], format!("nats-{i}"));
            assert_eq!(s["command"][2], format!("nats-{i}"));
            assert_eq!(
                s["command"][4],
                "nats://10.0.0.1:6222,nats://10.0.0.2:6222,nats://10.0.0.3:6222,"
            );
            assert_eq!(
                s["ports"],
                json!([{"name": "client", "container": 4222, "published": 4222, "protocol": "tcp"},
                       {"name": "cluster", "container": 6222, "published": 6222, "protocol": "tcp"}])
            );
            let ctx = &plan.contexts[&format!("nats-{i}")];
            let inst = ctx.instance.as_ref().unwrap();
            assert_eq!(inst.host, format!("10.0.0.{}", i + 1));
            assert_eq!(inst.ports["cluster"], 6222);
            assert_eq!(ctx.driver, "native");
            assert!(ctx.values.is_empty());
            assert_eq!(ctx.members["nats"].len(), 3);
            assert_eq!(ctx.members["nats"][2].host, "10.0.0.3");
        }
    }

    #[test]
    fn compose_single_host_uses_service_names() {
        let text = blueprint(vec![role(
            "nats",
            3,
            json!({"distinctHosts": true}),
            json!([{"name": "client", "container": 4222, "published": "{{ add 14222 .Instance.Index }}"},
                   {"name": "cluster", "container": 6222, "published": "{{ add 16222 .Instance.Index }}"}]),
            json!({"image": "nats:2", "command": ["--routes", ROUTES]}),
        )]);
        let b = binding(
            "compose",
            &[("nats", &[(7, "10.0.0.7"), (7, "10.0.0.7"), (7, "10.0.0.7")])],
        );
        let plans = compile(&text, &b, None).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].host_id, Id::from(7));
        let d = doc(&plans[0]);
        let services = d["services"].as_array().unwrap();
        assert_eq!(services.len(), 3);
        for (i, s) in services.iter().enumerate() {
            assert_eq!(s["name"], format!("nats-{i}"));
            assert_eq!(
                s["command"][1],
                "nats://nats-0:6222,nats://nats-1:6222,nats://nats-2:6222,"
            );
            assert_eq!(s["ports"][0]["published"], json!(14222 + i));
            assert_eq!(s["ports"][1]["published"], json!(16222 + i));
        }
        let inst = plans[0].contexts["nats-1"].instance.clone().unwrap();
        assert_eq!((inst.host.as_str(), inst.ports["client"]), ("nats-1", 4222));
        let split = binding("compose", &[("nats", &[(7, "a"), (8, "b"), (7, "a")])]);
        assert_eq!(
            err_key(compile(&text, &split, None)).0,
            "blueprint.bindingComposeSingleHost"
        );
    }

    fn redis_sentinel(sentinel_published: u16, protocol: &str) -> String {
        blueprint(vec![
            role(
                "redis",
                3,
                json!({"distinctHosts": true}),
                json!([{"name": "data", "container": 6379, "published": 6379}]),
                json!({"image": "redis:7"}),
            ),
            role(
                "sentinel",
                3,
                json!({"colocateWith": "redis"}),
                json!([{"name": "sentinel", "container": 26379, "published": sentinel_published,
                        "protocol": protocol}]),
                json!({"image": "redis:7", "command": ["redis-sentinel",
                    r#"{{ range .Role "redis" }}{{ .Host }}:{{ .Port "data" }} {{ end }}"#]}),
            ),
        ])
    }

    #[test]
    fn colocated_sentinel_and_redis_share_host_application() {
        let b = binding("docker", &[three_hosts("redis"), three_hosts("sentinel")]);
        let plans = compile(&redis_sentinel(26379, "tcp"), &b, None).unwrap();
        assert_eq!(plans.len(), 3);
        for (i, plan) in plans.iter().enumerate() {
            let d = doc(plan);
            let names: Vec<&str> = d["services"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["name"].as_str().unwrap())
                .collect();
            assert_eq!(names, [format!("redis-{i}"), format!("sentinel-{i}")]);
            assert_eq!(
                d["services"][1]["command"][1],
                "10.0.0.1:6379 10.0.0.2:6379 10.0.0.3:6379 "
            );
            assert_eq!(
                plan.contexts.keys().collect::<Vec<_>>(),
                [&format!("redis-{i}"), &format!("sentinel-{i}")]
            );
        }
    }

    #[test]
    fn colocated_instances_with_same_published_port_rejected() {
        let b = binding("docker", &[three_hosts("redis"), three_hosts("sentinel")]);
        let (key, params) = err_key(compile(&redis_sentinel(6379, "tcp"), &b, None));
        assert_eq!(key, "blueprint.portConflict");
        assert_eq!(params["instance"], "sentinel-0");
        assert_eq!(params["other"], "redis-0");
        assert_eq!(params["port"], "6379/tcp");
        // Same number on another protocol does not collide.
        assert!(compile(&redis_sentinel(6379, "udp"), &b, None).is_ok());
    }

    #[test]
    fn compiled_manifest_passes_driver_validation() {
        let b = binding("docker", &[three_hosts("redis"), three_hosts("sentinel")]);
        for plan in compile(&redis_sentinel(26379, "tcp"), &b, None).unwrap() {
            assert!(manifest::parse_for(&plan.manifest, "docker").is_ok());
        }
        // A driver-level rule (docker forbids replicas) surfaces unchanged; the role is
        // stateless because the model already rejects replicas on stable roles.
        let mut v: Value = serde_json::from_str(&redis_sentinel(26379, "tcp")).unwrap();
        v["roles"][0]["identity"] = json!("stateless");
        v["roles"][0]["service"]["replicas"] = json!(2);
        assert_eq!(
            err_key(compile(&v.to_string(), &b, None)).0,
            "deployment.manifest.028"
        );
        // Native requires published == container.
        let mut v: Value = serde_json::from_str(&nats_native()).unwrap();
        v["roles"][0]["ports"][0]["published"] = json!(14222);
        let b = binding("native", &[three_hosts("nats")]);
        assert_eq!(
            err_key(compile(&v.to_string(), &b, None)).0,
            "deployment.manifest.036"
        );
    }

    #[test]
    fn whole_value_actions_follow_service_schema_types() {
        let text = blueprint(vec![role(
            "redis",
            3,
            json!({"distinctHosts": true}),
            json!([{"name": "data", "container": 6379, "published": 6379}]),
            json!({"image": "redis:7",
                   "command": ["redis-server", "--port", r#"{{ .Instance.Port "data" }}"#],
                   "env": [{"name": "NODE_ID", "value": "{{ .Instance.Index }}"}],
                   "health": {"readiness": {
                       "httpGet": {"path": "/", "port": r#"{{ .Instance.Port "data" }}"#},
                       "periodSeconds": "{{ add 10 .Instance.Index }}"}}}),
        )]);
        let b = binding("docker", &[three_hosts("redis")]);
        let plans = compile(&text, &b, None).unwrap();
        for (i, plan) in plans.iter().enumerate() {
            let s = &doc(plan)["services"][0];
            // String fields in the schema receive text.
            assert_eq!(s["command"][2], json!("6379"));
            assert_eq!(s["env"][0]["value"], json!(i.to_string()));
            // Numeric / free-form fields keep the typed value.
            let probe = &s["health"]["readiness"];
            assert_eq!(probe["httpGet"]["port"], json!(6379));
            assert_eq!(probe["periodSeconds"], json!(10 + i));
        }
    }

    #[test]
    fn unsupported_drivers_rejected() {
        let b = binding("k8s", &[three_hosts("nats")]);
        assert_eq!(
            err_key(compile(&nats_native(), &b, None)).0,
            "blueprint.driverNotImplemented"
        );
        let b = binding("podman", &[three_hosts("nats")]);
        let (key, params) = err_key(compile(&nats_native(), &b, None));
        assert_eq!(
            (key.as_str(), params["driver"].as_str()),
            ("blueprint.driverUnsupported", "podman")
        );
    }

    const MANAGER: i64 = 9;

    fn stack_binding(roles: &[(&str, &[(i64, &str)])]) -> Binding {
        let mut b = binding("stack", roles);
        b.manager = Some(Id::from(MANAGER));
        b
    }

    /// Stable NATS (cluster published per index) plus a stateless gateway listing its peers.
    fn nats_with_gateway(gateway_published: u16) -> String {
        let mut gateway = role(
            "gateway",
            2,
            json!({}),
            json!([{"name": "http", "container": 80, "published": gateway_published}]),
            json!({"image": "nginx:1",
                   "command": ["gw", "{{ .Instance.Name }}", "{{ .Instance.Index }}",
                       r#"{{ range .Role "gateway" }}{{ .Host }}:{{ .Port "http" }} {{ end }}"#,
                       ROUTES]}),
        );
        gateway["identity"] = json!("stateless");
        blueprint(vec![
            role(
                "nats",
                3,
                json!({"distinctHosts": true}),
                json!([{"name": "client", "container": 4222, "published": 4222},
                       {"name": "cluster", "container": 6222,
                        "published": "{{ add 16222 .Instance.Index }}"}]),
                json!({"image": "nats:2", "command": ["--name", "{{ .Instance.Name }}", ROUTES]}),
            ),
            gateway,
        ])
    }

    fn stack_hosts() -> Binding {
        stack_binding(&[
            three_hosts("nats"),
            ("gateway", &[(1, "10.0.0.1"), (2, "10.0.0.2")]),
        ])
    }

    fn nodes(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn stack_nodes() -> BTreeMap<String, String> {
        nodes(&[
            ("nats-0", "n0node"),
            ("nats-1", "n1node"),
            ("nats-2", "n2node"),
            ("gateway-0", "n0node"),
            ("gateway-1", "n1node"),
        ])
    }

    fn service<'a>(d: &'a Value, name: &str) -> &'a Value {
        d["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("service {name}"))
    }

    #[test]
    fn stack_stable_role_one_service_per_instance_pinned_to_node() {
        let plans = compile(
            &nats_with_gateway(8080),
            &stack_hosts(),
            Some(&stack_nodes()),
        )
        .unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].host_id, Id::from(MANAGER));
        let d = doc(&plans[0]);
        let names: Vec<&str> = d["services"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["nats-0", "nats-1", "nats-2", "gateway"]);
        for i in 0..3u16 {
            let s = service(&d, &format!("nats-{i}"));
            assert_eq!(
                s["placement"]["constraints"],
                json!([format!("node.id==n{i}node")])
            );
            assert_eq!(s["endpointMode"], "dnsrr");
            assert!(s.get("replicas").is_none());
            assert_eq!(
                s["ports"],
                json!([{"name": "client", "container": 4222, "published": 4222,
                        "protocol": "tcp", "mode": "host"},
                       {"name": "cluster", "container": 6222, "published": 16222 + i,
                        "protocol": "tcp", "mode": "host"}])
            );
            let inst = plans[0].contexts[&format!("nats-{i}")]
                .instance
                .clone()
                .unwrap();
            assert_eq!(inst.host, format!("10.0.0.{}", i + 1));
            assert_eq!(inst.ports["cluster"], 16222 + i);
        }
    }

    #[test]
    fn stack_stateless_role_single_service_with_replicas() {
        let plans = compile(
            &nats_with_gateway(8080),
            &stack_hosts(),
            Some(&stack_nodes()),
        )
        .unwrap();
        let d = doc(&plans[0]);
        let s = service(&d, "gateway");
        assert_eq!(s["replicas"], 2);
        assert!(s.get("placement").is_none());
        assert!(s.get("endpointMode").is_none());
        assert_eq!(
            s["ports"],
            json!([{"name": "http", "container": 80, "published": 8080,
                    "protocol": "tcp", "mode": "ingress"}])
        );
        // All replicas share one context rendered as index 0 named after the role.
        assert_eq!(s["command"][1], "gateway");
        assert_eq!(s["command"][2], "0");
        assert_eq!(
            plans[0].contexts.keys().collect::<Vec<_>>(),
            ["gateway", "nats-0", "nats-1", "nats-2"]
        );
        let inst = plans[0].contexts["gateway"].instance.clone().unwrap();
        assert_eq!(
            (inst.name.as_str(), inst.index, inst.host.as_str()),
            ("gateway", 0, "gateway")
        );
        assert_eq!(inst.ports["http"], 80);
        // The binding's instance count wins over replicas written by the author.
        let mut v: Value = serde_json::from_str(&nats_with_gateway(8080)).unwrap();
        v["roles"][1]["service"]["replicas"] = json!(5);
        let plans = compile(&v.to_string(), &stack_hosts(), Some(&stack_nodes())).unwrap();
        assert_eq!(service(&doc(&plans[0]), "gateway")["replicas"], 2);
    }

    #[test]
    fn stack_members_resolve_host_and_port() {
        let plans = compile(
            &nats_with_gateway(8080),
            &stack_hosts(),
            Some(&stack_nodes()),
        )
        .unwrap();
        let d = doc(&plans[0]);
        let routes = "nats://10.0.0.1:16222,nats://10.0.0.2:16223,nats://10.0.0.3:16224,";
        assert_eq!(service(&d, "nats-1")["command"][2], routes);
        let gateway = service(&d, "gateway");
        assert_eq!(gateway["command"][3], "gateway:80 gateway:80 ");
        assert_eq!(gateway["command"][4], routes);
        let members = &plans[0].contexts["nats-0"].members;
        assert_eq!(members["gateway"].len(), 2);
        assert_eq!(members["gateway"][1].index, 1);
        assert_eq!(members["nats"][2].host, "10.0.0.3");
    }

    #[test]
    fn host_mode_port_conflict_on_same_node() {
        let text = nats_with_gateway(8080);
        let b = stack_hosts();
        // Two stable instances on one node publish the same host port.
        let mut shared = stack_nodes();
        shared.insert("nats-1".into(), "n0node".into());
        let (key, params) = err_key(compile(&text, &b, Some(&shared)));
        assert_eq!(key, "blueprint.portConflict");
        assert_eq!(params["instance"], "nats-1");
        assert_eq!(params["other"], "nats-0");
        assert_eq!(params["port"], "4222/tcp");
        // An ingress port is published on every node, so it collides with any host port.
        let (key, params) = err_key(compile(&nats_with_gateway(4222), &b, Some(&stack_nodes())));
        assert_eq!(key, "blueprint.portConflict");
        assert_eq!(params["instance"], "gateway");
        assert_eq!(params["other"], "nats-0");
        // Two stateless roles publishing the same ingress port collide stack-wide.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        let mut second = v["roles"][1].clone();
        second["name"] = json!("proxy");
        v["roles"].as_array_mut().unwrap().push(second);
        let mut b = stack_hosts();
        b.roles.insert("proxy".into(), b.roles["gateway"].clone());
        let mut n = stack_nodes();
        n.insert("proxy-0".into(), "n2node".into());
        n.insert("proxy-1".into(), "n2node".into());
        let (key, params) = err_key(compile(&v.to_string(), &b, Some(&n)));
        assert_eq!(key, "blueprint.portConflict");
        assert_eq!(params["instance"], "proxy");
        assert_eq!(params["other"], "gateway");
        assert_eq!(params["port"], "8080/tcp");
    }

    #[test]
    fn stack_manifest_passes_driver_validation() {
        let plans = compile(
            &nats_with_gateway(8080),
            &stack_hosts(),
            Some(&stack_nodes()),
        )
        .unwrap();
        let m = manifest::parse_for(&plans[0].manifest, "stack").unwrap();
        assert_eq!(m.services.len(), 4);
        // A driver-level rule (stack forbids dependsOn) surfaces unchanged.
        let mut v: Value = serde_json::from_str(&nats_with_gateway(8080)).unwrap();
        v["roles"][1]["service"]["dependsOn"] = json!(["nats-0"]);
        assert_eq!(
            err_key(compile(
                &v.to_string(),
                &stack_hosts(),
                Some(&stack_nodes())
            ))
            .0,
            "deployment.manifest.030"
        );
    }

    #[test]
    fn stack_requires_nodes() {
        let text = nats_with_gateway(8080);
        assert_eq!(
            err_key(compile(&text, &stack_hosts(), None)).0,
            "blueprint.swarmNodesRequired"
        );
        // A stable instance without a resolved node cannot be pinned.
        let mut partial = stack_nodes();
        partial.remove("nats-2");
        let (key, params) = err_key(compile(&text, &stack_hosts(), Some(&partial)));
        assert_eq!(key, "blueprint.swarmNodeUnavailable");
        assert_eq!(params["host"], Id::from(3).to_string());
        // A stack binding names its manager.
        let mut b = stack_hosts();
        b.manager = None;
        assert_eq!(
            err_key(compile(&text, &b, Some(&stack_nodes()))).0,
            "blueprint.bindingInvalid"
        );
    }

    #[test]
    fn inconsistent_inputs_return_errors() {
        let b = binding("native", &[]);
        assert_eq!(
            err_key(compile(&nats_native(), &b, None)).0,
            "blueprint.bindingRoleMissing"
        );
        let mut v: Value = serde_json::from_str(&nats_native()).unwrap();
        v["roles"][0]["ports"][0]["published"] = json!("{{ .Instance.Name }}");
        let b = binding("native", &[three_hosts("nats")]);
        let (key, params) = err_key(compile(&v.to_string(), &b, None));
        assert_eq!(
            (key.as_str(), params["role"].as_str()),
            ("blueprint.portInvalid", "nats")
        );
        assert_eq!(
            err_key(compile("{}", &b, None)).0,
            "blueprint.invalidDocument"
        );
    }
}
