//! Official marketplace blueprints render, bind and compile for every driver they declare.
use super::*;
use crate::blueprint::{binding, compile, model, template};
use crate::snowflake::Id;

/// Binds each role's default instance count. Docker and stack spread instances over hosts
/// 1–3 in role order (stack manager: host 1); compose puts every instance on host 1.
fn sample_binding(bp: &model::Blueprint, driver: &str) -> binding::Binding {
    let mut next = 0u32;
    let mut roles = serde_json::Map::new();
    for role in &bp.roles {
        let list: Vec<Value> = (0..role.instances.default)
            .map(|_| {
                let host = if driver == "compose" { 1 } else { next % 3 + 1 };
                next += 1;
                json!({"hostId": Id::from(i64::from(host)), "address": format!("10.0.0.{host}")})
            })
            .collect();
        roles.insert(role.name.clone(), json!(list));
    }
    let mut b = json!({"driver": driver, "roles": roles});
    if driver == "stack" {
        b["manager"] = json!(Id::from(1));
    }
    let b = binding::parse(&b.to_string()).unwrap();
    let hosts = (1..=3)
        .map(|h| {
            let id = Id::from(h);
            (
                id,
                binding::HostInfo {
                    id,
                    os: "linux".into(),
                },
            )
        })
        .collect();
    binding::validate(bp, &b, &hosts).unwrap();
    b
}

/// Stand-in Swarm node IDs as `swarm::assign` would resolve them without reaching hosts:
/// every instance runs on the node of its bound host (`node<host id>`).
fn sample_nodes(b: &binding::Binding) -> BTreeMap<String, String> {
    b.roles
        .iter()
        .flat_map(|(role, list)| {
            list.iter()
                .enumerate()
                .map(move |(i, inst)| (format!("{role}-{i}"), format!("node{}", inst.host_id)))
        })
        .collect()
}

/// (driver, instance) → rendered config file text, for one package version.
type Rendered = BTreeMap<(String, String), String>;

/// A package version's rendered config files and its single stack Application.
struct Compiled {
    files: Rendered,
    stack: Value,
}

fn compile_version(key: &str, version: &Value) -> Compiled {
    let stored = render_content(
        &version["blueprint"].to_string(),
        &version["parameters"].to_string(),
        &BTreeMap::new(),
        Kind::App,
    )
    .unwrap_or_else(|e| panic!("{key}: {:?}", e.2));
    let bp = model::parse(&stored).unwrap();
    let files: BTreeMap<&str, &str> = version["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap(), f["content"].as_str().unwrap()))
        .collect();
    let drivers: Vec<&str> = bp
        .drivers
        .iter()
        .filter(|(_, d)| d.supported)
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(drivers, ["compose", "docker", "stack"], "{key}");
    let mut rendered = Rendered::new();
    let mut stack = Value::Null;
    for driver in drivers {
        let bound = sample_binding(&bp, driver);
        let nodes = sample_nodes(&bound);
        let plans = compile::compile(&stored, &bound, (driver == "stack").then_some(&nodes))
            .unwrap_or_else(|e| panic!("{key} {driver}: {:?}", e.2));
        assert_eq!(plans.len(), if driver == "docker" { 3 } else { 1 });
        for plan in &plans {
            let doc: Value = serde_json::from_str(&plan.manifest).unwrap();
            if driver == "stack" {
                assert_eq!(
                    plan.host_id,
                    Id::from(1),
                    "{key}: stack deploys on the manager"
                );
                stack = doc.clone();
            }
            for service in doc["services"].as_array().unwrap() {
                let name = service["name"].as_str().unwrap();
                for file in service["files"].as_array().into_iter().flatten() {
                    let source = files[file["fromTemplate"].as_str().unwrap()];
                    let text = config_content(source, "[]", &BTreeMap::new()).unwrap();
                    let out = template::render_text(
                        &text,
                        &plan.contexts[name],
                        template::Phase::Instance,
                    )
                    .unwrap_or_else(|e| panic!("{key} {driver} {name}: {:?}", e.2));
                    rendered.insert((driver.to_owned(), name.to_owned()), out);
                }
            }
        }
    }
    Compiled {
        files: rendered,
        stack,
    }
}

#[test]
fn official_blueprints_compile_for_supported_drivers() {
    let index: Value =
        serde_json::from_str(include_str!("../../../../marketplace/index.json")).unwrap();
    let mut seen = BTreeMap::new();
    for package in index["packages"].as_array().unwrap() {
        let key = package["key"].as_str().unwrap();
        for version in package["versions"].as_array().unwrap() {
            if version["blueprint"]["kind"] != "Blueprint" {
                continue;
            }
            assert_eq!(package["meta"]["topology"], "cluster", "{key}");
            for field in ["name", "description"] {
                for locale in ["zh-CN", "zh-TW", "en-US", "ms-MY", "ja-JP", "ko-KR"] {
                    let text = package["meta"][field][locale].as_str().unwrap_or_default();
                    assert!(!text.trim().is_empty(), "{key} {field} {locale}");
                }
            }
            assert_eq!(
                package["meta"]["drivers"],
                json!(["docker", "compose", "stack"]),
                "{key}"
            );
            seen.insert(key.to_owned(), compile_version(key, version));
        }
    }

    let nats = &seen["nats-cluster"].files;
    for i in 0..3 {
        let docker = &nats[&("docker".into(), format!("nats-{i}"))];
        assert!(
            docker.contains(&format!("server_name: nats-{i}\n")),
            "{docker}"
        );
        for j in 0..3 {
            let route = format!("nats://10.0.0.{}:{}\n", j + 1, 16222 + j);
            assert!(
                docker.contains(&route),
                "docker nats-{i} lacks {route}: {docker}"
            );
        }
        let compose = &nats[&("compose".into(), format!("nats-{i}"))];
        for j in 0..3 {
            let route = format!("nats://nats-{j}:6222\n");
            assert!(
                compose.contains(&route),
                "compose nats-{i} lacks {route}: {compose}"
            );
        }
    }

    let nginx = &seen["nginx-ha"].files;
    let docker = &nginx[&("docker".into(), "lb-0".into())];
    assert!(docker.contains("server 10.0.0.1:18080 "), "{docker}");
    assert!(docker.contains("server 10.0.0.2:18081 "), "{docker}");
    let compose = &nginx[&("compose".into(), "lb-0".into())];
    assert!(
        compose.contains("server web-0:80 ") && compose.contains("server web-1:80 "),
        "{compose}"
    );

    // stack: each stable NATS member is its own service pinned to its node, publishing
    // per-index host-mode ports at its bound address.
    let services = |key: &str| -> BTreeMap<String, Value> {
        seen[key].stack["services"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap().to_owned(), s.clone()))
            .collect()
    };
    let nats_services = services("nats-cluster");
    assert_eq!(
        nats_services.keys().collect::<Vec<_>>(),
        ["nats-0", "nats-1", "nats-2"]
    );
    for i in 0..3u16 {
        let s = &nats_services[&format!("nats-{i}")];
        assert_eq!(
            s["placement"]["constraints"],
            json!([format!("node.id==node{}", i + 1)])
        );
        assert_eq!(s["endpointMode"], "dnsrr");
        for (port, base) in [("client", 14222), ("cluster", 16222), ("monitor", 18222)] {
            let entry = s["ports"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == port)
                .unwrap();
            assert_eq!(
                (&entry["published"], &entry["mode"]),
                (&json!(base + i), &json!("host")),
                "nats-{i} {port}"
            );
        }
        let stack = &nats[&("stack".into(), format!("nats-{i}"))];
        let advertise = format!("advertise: \"10.0.0.{}:{}\"\n", i + 1, 16222 + i);
        assert!(stack.contains(&advertise), "{stack}");
        for j in 0..3 {
            let route = format!("nats://10.0.0.{}:{}\n", j + 1, 16222 + j);
            assert!(
                stack.contains(&route),
                "stack nats-{i} lacks {route}: {stack}"
            );
        }
    }

    // stack: stateless web and lb are one replicated service each with ingress ports, one
    // replica per node and updated stop-first; the
    // balancer reaches web replicas through the `web` service VIP on the container port.
    let nginx_services = services("nginx-ha");
    assert_eq!(nginx_services.keys().collect::<Vec<_>>(), ["lb", "web"]);
    for (name, replicas, published) in [("web", 2, 18080), ("lb", 1, 18000)] {
        let s = &nginx_services[name];
        assert_eq!(s["replicas"], replicas, "{name}");
        // Both roles want distinct hosts: at most one replica per Swarm node.
        assert_eq!(s["placement"], json!({"maxReplicasPerNode": 1}), "{name}");
        assert_eq!(
            s["ports"],
            json!([{"name": "http", "container": 80, "published": published,
                    "protocol": "tcp", "mode": "ingress"}]),
            "{name}"
        );
    }
    let order = |key: &str, name: &str| {
        seen[key].stack["overrides"]["stack"]["services"][name]["deploy"]["update_config"]["order"]
            .clone()
    };
    for name in ["web", "lb"] {
        assert_eq!(order("nginx-ha", name), "stop-first", "{name}");
    }
    for i in 0..3 {
        assert_eq!(order("nats-cluster", &format!("nats-{i}")), "stop-first");
    }
    let stack = &nginx[&("stack".into(), "lb".into())];
    assert_eq!(stack.matches("server web:80 ").count(), 2, "{stack}");
}
