//! Official marketplace blueprints render, bind and compile for every driver they declare.
use super::*;
use crate::blueprint::{binding, compile, model, template};
use crate::snowflake::Id;

/// Binds each role's default instance count. Docker spreads instances over hosts 1–3 in
/// role order; compose puts every instance on host 1.
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
    let b = binding::parse(&json!({"driver": driver, "roles": roles}).to_string()).unwrap();
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

/// (driver, instance) → rendered config file text, for one package version.
type Rendered = BTreeMap<(String, String), String>;

fn compile_version(key: &str, version: &Value) -> Rendered {
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
    assert_eq!(drivers, ["compose", "docker"], "{key}");
    let mut rendered = Rendered::new();
    for driver in drivers {
        let plans = compile::compile(&stored, &sample_binding(&bp, driver), key)
            .unwrap_or_else(|e| panic!("{key} {driver}: {:?}", e.2));
        assert_eq!(plans.len(), if driver == "compose" { 1 } else { 3 });
        for plan in &plans {
            let doc: Value = serde_json::from_str(&plan.manifest).unwrap();
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
    rendered
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
            assert_eq!(
                package["meta"]["drivers"],
                json!(["docker", "compose"]),
                "{key}"
            );
            seen.insert(key.to_owned(), compile_version(key, version));
        }
    }

    let nats = &seen["nats-cluster"];
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

    let nginx = &seen["nginx-ha"];
    let docker = &nginx[&("docker".into(), "lb-0".into())];
    assert!(docker.contains("server 10.0.0.1:18080 "), "{docker}");
    assert!(docker.contains("server 10.0.0.2:18081 "), "{docker}");
    let compose = &nginx[&("compose".into(), "lb-0".into())];
    assert!(
        compose.contains("server web-0:80 ") && compose.contains("server web-1:80 "),
        "{compose}"
    );
}
