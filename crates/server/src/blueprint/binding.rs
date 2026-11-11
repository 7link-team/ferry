//! Instance binding: which hosts/addresses run each blueprint role (pure validation, no DB).
use super::model::{Blueprint, MAX_INSTANCES};
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::snowflake::Id;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv6Addr};

const MAX_ADDRESS: usize = 253;
const MAX_LABEL: usize = 63;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub driver: String,
    pub roles: BTreeMap<String, Vec<InstanceBinding>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceBinding {
    pub host_id: Id,
    /// Advertised address other instances use to reach this one.
    pub address: String,
}

/// Facts about a workspace host; an empty `os` means an SSH host with unknown OS.
#[derive(Clone, Debug)]
pub struct HostInfo {
    #[allow(dead_code)] // part of the caller-facing shape; lookups key on the map
    pub id: Id,
    pub os: String,
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

pub fn parse(text: &str) -> Result<Binding> {
    serde_json::from_str(text).or_else(|_| fail(Message::new("blueprint.bindingInvalid")))
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_address(address: &str) -> bool {
    if address.is_empty() || address.len() > MAX_ADDRESS {
        return false;
    }
    // Brackets are IPv6 syntax only: `[10.0.0.1]` is not an address.
    let bare = match address.strip_prefix('[').and_then(|a| a.strip_suffix(']')) {
        Some(inner) if inner.parse::<Ipv6Addr>().is_ok() => inner,
        _ => address,
    };
    if let Ok(ip) = bare.parse::<IpAddr>() {
        let ip = ip.to_canonical();
        return !ip.is_unspecified() && !ip.is_loopback();
    }
    if address.contains(['[', ']', ':']) {
        return false;
    }
    let lower = address.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return false;
    }
    if !address.split('.').all(valid_label) {
        return false;
    }
    // A numeric final label would be read as an abbreviated IPv4 address, e.g. `127.1`.
    let last = address.rsplit('.').next().unwrap_or_default();
    !last.bytes().all(|b| b.is_ascii_digit())
}

pub fn validate(bp: &Blueprint, b: &Binding, hosts: &BTreeMap<Id, HostInfo>) -> Result<()> {
    match b.driver.as_str() {
        "stack" => return fail(Message::new("blueprint.driverNotYetSupported")),
        "k8s" => return fail(Message::new("blueprint.driverNotImplemented")),
        _ => {}
    }
    let driver = b.driver.as_str();
    let listed = bp.drivers.get(driver).is_some_and(|d| d.supported);
    if !matches!(driver, "native" | "docker" | "compose") || !listed {
        return fail(Message::new("blueprint.driverUnsupported").with("driver", driver));
    }
    let known: BTreeSet<&str> = bp.roles.iter().map(|r| r.name.as_str()).collect();
    if let Some(extra) = b.roles.keys().find(|k| !known.contains(k.as_str())) {
        return fail(Message::new("blueprint.bindingRoleUnknown").with("role", extra));
    }
    let total: usize = b.roles.values().map(Vec::len).sum();
    if total > MAX_INSTANCES as usize {
        return fail(
            Message::new("blueprint.instancesTotal")
                .with("total", total.to_string())
                .with("max", MAX_INSTANCES.to_string()),
        );
    }
    for role in &bp.roles {
        let Some(list) = b.roles.get(&role.name) else {
            return fail(Message::new("blueprint.bindingRoleMissing").with("role", &role.name));
        };
        let range = role.instances.min as usize..=role.instances.max as usize;
        if !range.contains(&list.len()) {
            return fail(
                Message::new("blueprint.bindingCount")
                    .with("role", &role.name)
                    .with("min", role.instances.min.to_string())
                    .with("max", role.instances.max.to_string()),
            );
        }
        if role.instances.odd && list.len() % 2 == 0 {
            return fail(Message::new("blueprint.bindingOdd").with("role", &role.name));
        }
        for (index, inst) in list.iter().enumerate() {
            let name = format!("{}-{index}", role.name);
            let Some(host) = hosts.get(&inst.host_id) else {
                return fail(Message::new("blueprint.bindingHostUnknown").with("instance", &name));
            };
            if !host.os.is_empty() && host.os != "linux" {
                return fail(
                    Message::new("blueprint.hostOsUnsupported")
                        .with("instance", &name)
                        .with("os", &host.os),
                );
            }
            if !valid_address(&inst.address) {
                return fail(
                    Message::new("blueprint.bindingAddressInvalid").with("instance", &name),
                );
            }
        }
    }
    validate_placement(bp, b)
}

fn validate_placement(bp: &Blueprint, b: &Binding) -> Result<()> {
    let compose = b.driver == "compose";
    if compose {
        if !bp.allow_single_host && bp.roles.iter().any(|r| r.placement.distinct_hosts) {
            return fail(Message::new("blueprint.bindingComposeNeedsSingleHost"));
        }
        let hosts: BTreeSet<Id> = b.roles.values().flatten().map(|i| i.host_id).collect();
        if hosts.len() > 1 {
            return fail(Message::new("blueprint.bindingComposeSingleHost"));
        }
    }
    for role in &bp.roles {
        let list = &b.roles[&role.name];
        if role.placement.distinct_hosts && !compose {
            let hosts: BTreeSet<Id> = list.iter().map(|i| i.host_id).collect();
            if hosts.len() != list.len() {
                return fail(
                    Message::new("blueprint.bindingDistinctHosts").with("role", &role.name),
                );
            }
        }
        if let Some(target) = &role.placement.colocate_with {
            let other = &b.roles[target];
            if other.len() != list.len() {
                return fail(
                    Message::new("blueprint.bindingColocateCount")
                        .with("role", &role.name)
                        .with("target", target),
                );
            }
            if let Some(i) = list
                .iter()
                .zip(other)
                .position(|(a, t)| a.host_id != t.host_id)
            {
                return fail(
                    Message::new("blueprint.bindingColocateHost")
                        .with("instance", format!("{}-{i}", role.name))
                        .with("target", format!("{target}-{i}")),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn role(name: &str, min: u32, default: u32, max: u32, odd: bool, placement: Value) -> Value {
        json!({
            "name": name,
            "instances": {"min": min, "max": max, "default": default, "odd": odd},
            "placement": placement,
            "identity": "stable",
            "service": {"image": "x"}
        })
    }

    fn blueprint(roles: Vec<Value>, drivers: Value, single: bool) -> Blueprint {
        let text = json!({
            "apiVersion": "ferry/blueprint-v1",
            "kind": "Blueprint",
            "metadata": {"name": "demo"},
            "roles": roles,
            "drivers": drivers,
            "allowSingleHost": single,
        })
        .to_string();
        super::super::model::parse(&text).expect("blueprint")
    }

    fn drivers() -> Value {
        json!({"native": {"supported": true}, "docker": {"supported": true},
               "compose": {"supported": true}, "stack": {"supported": true}, "k8s": {"supported": true}})
    }

    fn nats_roles() -> Vec<Value> {
        vec![
            role("nats", 1, 3, 5, true, json!({"distinctHosts": true})),
            role("exporter", 1, 3, 5, false, json!({"colocateWith": "nats"})),
        ]
    }

    fn nats() -> Blueprint {
        blueprint(nats_roles(), drivers(), false)
    }

    fn std_hosts() -> BTreeMap<Id, HostInfo> {
        [
            (1, "linux"),
            (2, "linux"),
            (3, ""),
            (4, "linux"),
            (9, "windows"),
        ]
        .into_iter()
        .map(|(id, os)| {
            (
                Id::from(id as i64),
                HostInfo {
                    id: Id::from(id as i64),
                    os: os.to_string(),
                },
            )
        })
        .collect()
    }

    fn binding(driver: &str, roles: Value) -> Binding {
        parse(&json!({"driver": driver, "roles": roles}).to_string()).expect("binding")
    }

    fn inst(host: &str, addr: &str) -> Value {
        json!({"hostId": host, "address": addr})
    }

    fn three() -> Value {
        json!({
            "nats": [inst("1", "10.0.0.1"), inst("2", "10.0.0.2"), inst("3", "nats-3.internal")],
            "exporter": [inst("1", "10.0.0.1"), inst("2", "10.0.0.2"), inst("3", "10.0.0.3")],
        })
    }

    fn key(r: Result<()>) -> String {
        r.expect_err("expected error")
            .2
            .map(|m| m.key)
            .unwrap_or_default()
    }

    fn check(driver: &str, roles: Value) -> Result<()> {
        validate(&nats(), &binding(driver, roles), &std_hosts())
    }

    #[test]
    fn valid_three_host_binding() {
        check("native", three()).expect("valid");
        check("docker", three()).expect("valid");
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        assert!(parse(r#"{"driver":"native","roles":{},"x":1}"#).is_err());
        assert!(
            parse(r#"{"driver":"native","roles":{"a":[{"hostId":"1","address":"a","z":1}]}}"#)
                .is_err()
        );
        assert!(parse("nope").is_err());
    }

    #[test]
    fn count_and_odd_enforced() {
        let mut r = three();
        r["nats"] = json!([inst("1", "10.0.0.1"), inst("2", "10.0.0.2")]);
        r["exporter"] = json!([inst("1", "10.0.0.1"), inst("2", "10.0.0.2")]);
        assert_eq!(key(check("native", r)), "blueprint.bindingOdd");
        let mut r = three();
        r["nats"] = json!([]);
        assert_eq!(key(check("native", r)), "blueprint.bindingCount");
        let mut r = three();
        r["nats"] = json!(
            (0..7)
                .map(|i| inst("1", &format!("10.0.0.{}", i + 1)))
                .collect::<Vec<_>>()
        );
        assert_eq!(key(check("native", r)), "blueprint.bindingCount");
        let mut r = three();
        r.as_object_mut().unwrap().remove("exporter");
        assert_eq!(key(check("native", r)), "blueprint.bindingRoleMissing");
        let mut r = three();
        r["ghost"] = json!([inst("1", "10.0.0.1")]);
        assert_eq!(key(check("native", r)), "blueprint.bindingRoleUnknown");
    }

    #[test]
    fn total_instances_capped_at_one_hundred() {
        let bp = blueprint(
            vec![
                role("a", 1, 1, 100, false, json!({})),
                role("b", 1, 1, 100, false, json!({})),
            ],
            drivers(),
            false,
        );
        let list = |n: usize| json!((0..n).map(|_| inst("1", "10.0.0.1")).collect::<Vec<_>>());
        let b = binding("native", json!({"a": list(51), "b": list(50)}));
        let m = validate(&bp, &b, &std_hosts())
            .expect_err("101 instances")
            .2
            .expect("message");
        assert_eq!(m.key, "blueprint.instancesTotal");
        assert_eq!(m.params["total"], "101");
        assert_eq!(m.params["max"], "100");
        let b = binding("native", json!({"a": list(50), "b": list(50)}));
        validate(&bp, &b, &std_hosts()).expect("100 instances ok");
    }

    #[test]
    fn distinct_hosts_enforced() {
        let mut r = three();
        r["nats"][1] = inst("1", "10.0.0.9");
        r["exporter"][1] = inst("1", "10.0.0.9");
        assert_eq!(key(check("native", r)), "blueprint.bindingDistinctHosts");
        let mut r = three();
        r["nats"][1] = inst("77", "10.0.0.9");
        assert_eq!(key(check("native", r)), "blueprint.bindingHostUnknown");
    }

    #[test]
    fn colocate_requires_same_host_and_count() {
        let mut r = three();
        r["exporter"][2] = inst("4", "10.0.0.4");
        let m = check("native", r)
            .expect_err("mismatch")
            .2
            .expect("message");
        assert_eq!(m.key, "blueprint.bindingColocateHost");
        assert_eq!(m.params["instance"], "exporter-2");
        assert_eq!(m.params["target"], "nats-2");
        let mut r = three();
        r["exporter"] = json!([inst("1", "10.0.0.1"), inst("2", "10.0.0.2")]);
        assert_eq!(key(check("native", r)), "blueprint.bindingColocateCount");
    }

    #[test]
    fn non_linux_host_rejected_empty_os_allowed() {
        let mut r = three();
        r["nats"][0] = inst("9", "10.0.0.1");
        r["exporter"][0] = inst("9", "10.0.0.1");
        assert_eq!(key(check("native", r)), "blueprint.hostOsUnsupported");
        // host 3 has an empty os and is used by three()
        check("native", three()).expect("empty os allowed");
    }

    #[test]
    fn loopback_and_empty_addresses_rejected() {
        let bad = [
            "",
            "0.0.0.0",
            "::",
            "[::]",
            "127.0.0.1",
            "127.8.8.8",
            "::1",
            "[::1]",
            "localhost",
            "LOCALHOST",
            "a.localhost",
            "::ffff:127.0.0.1",
            "127.1",
            "-a.com",
            "a-.com",
            "a..b",
            "a.b.",
            "a b",
            "a_b",
            "[10.0.0.1",
            "fe80::zz",
            "a:80",
            "\u{4f8b}.jp",
            "[10.0.0.1]",
            "[example.com]",
        ];
        for a in bad {
            let mut r = three();
            r["nats"][0]["address"] = json!(a);
            assert_eq!(
                key(check("native", r)),
                "blueprint.bindingAddressInvalid",
                "{a:?}"
            );
        }
        let dotted = |last: usize| format!("{0}.{0}.{0}.{1}", "a".repeat(63), "a".repeat(last));
        assert_eq!(dotted(61).len(), 253);
        let mut r = three();
        r["nats"][0]["address"] = json!(dotted(61));
        check("native", r).expect("253 chars with valid labels passes");
        for long in [dotted(62), "a".repeat(64)] {
            let mut r = three();
            r["nats"][0]["address"] = json!(long);
            assert_eq!(key(check("native", r)), "blueprint.bindingAddressInvalid");
        }
        for a in [
            "10.0.0.1",
            "2001:db8::1",
            "[2001:db8::1]",
            "host-1.example.com",
            "db1",
            "a1.b2",
        ] {
            let mut r = three();
            r["nats"][0]["address"] = json!(a);
            check("native", r).unwrap_or_else(|_| panic!("{a} should pass"));
        }
    }

    #[test]
    fn compose_requires_single_host_and_allow_single_host() {
        let one = json!({
            "nats": [inst("1", "10.0.0.1"), inst("1", "10.0.0.1"), inst("1", "10.0.0.1")],
            "exporter": [inst("1", "10.0.0.1"), inst("1", "10.0.0.1"), inst("1", "10.0.0.1")],
        });
        assert_eq!(
            key(check("compose", one.clone())),
            "blueprint.bindingComposeNeedsSingleHost"
        );
        let bp = blueprint(nats_roles(), drivers(), true);
        validate(&bp, &binding("compose", one), &std_hosts()).expect("single host ok");
        assert_eq!(
            key(validate(&bp, &binding("compose", three()), &std_hosts())),
            "blueprint.bindingComposeSingleHost"
        );
        let plain = blueprint(
            vec![role("web", 1, 2, 3, false, json!({}))],
            drivers(),
            false,
        );
        let b = binding(
            "compose",
            json!({"web": [inst("1", "10.0.0.1"), inst("1", "10.0.0.1")]}),
        );
        validate(&plain, &b, &std_hosts()).expect("ok");
        let b = binding(
            "compose",
            json!({"web": [inst("1", "10.0.0.1"), inst("2", "10.0.0.2")]}),
        );
        assert_eq!(
            key(validate(&plain, &b, &std_hosts())),
            "blueprint.bindingComposeSingleHost"
        );
    }

    #[test]
    fn stack_and_k8s_not_available_yet() {
        assert_eq!(
            key(check("stack", three())),
            "blueprint.driverNotYetSupported"
        );
        assert_eq!(key(check("k8s", three())), "blueprint.driverNotImplemented");
        assert_eq!(key(check("podman", three())), "blueprint.driverUnsupported");
        let bp = blueprint(
            vec![role("web", 1, 1, 1, false, json!({}))],
            json!({"native": {"supported": false}}),
            false,
        );
        let b = binding("native", json!({"web": [inst("1", "10.0.0.1")]}));
        assert_eq!(
            key(validate(&bp, &b, &std_hosts())),
            "blueprint.driverUnsupported"
        );
        let mut bp = blueprint(
            vec![role("web", 1, 1, 1, false, json!({}))],
            json!({}),
            false,
        );
        bp.drivers.insert(
            "podman".into(),
            crate::blueprint::model::DriverSupport {
                supported: true,
                member_addressing: None,
            },
        );
        let pb = binding("podman", json!({"web": [inst("1", "10.0.0.1")]}));
        assert_eq!(
            key(validate(&bp, &pb, &std_hosts())),
            "blueprint.driverUnsupported"
        );
        bp.drivers.clear();
        assert_eq!(
            key(validate(&bp, &b, &std_hosts())),
            "blueprint.driverUnsupported"
        );
    }
}
