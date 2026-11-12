//! Deployment spec (`ferry/deploy-v1`): template + values + per-role instance counts and host selectors.
use super::binding::InstanceBinding;
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::snowflake::Id;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const API_VERSION: &str = "ferry/deploy-v1";
const MAX_INSTANCES: u32 = 100;
const MAX_LIST: usize = 32;
const MAX_ITEM: usize = 60;
const MAX_PREFIX: usize = 30;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploySpec {
    pub template: TemplateRef,
    pub driver: String,
    /// Swarm manager host (name or id); only for `stack`. Resolved when the spec is planned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<String>,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    pub roles: BTreeMap<String, RoleSpec>,
    /// Manual placement override, shaped like `Binding.roles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<BTreeMap<String, Vec<InstanceBinding>>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateRef {
    pub id: Id,
    pub version: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleSpec {
    pub instances: u32,
    #[serde(default)]
    pub hosts: HostSelector,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostSelector {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<Spread>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Spread {
    pub prefix: String,
}

fn fail<T>(message: Message) -> Result<T> {
    Err(ApiError::localized(StatusCode::BAD_REQUEST, message))
}

/// Parses a YAML or JSON deployment spec and validates it.
pub fn parse(text: &str) -> Result<DeploySpec> {
    // JSON is a YAML subset, so one parser handles both.
    let invalid = |detail: String| Message::new("blueprint.specInvalid").with("detail", detail);
    let mut doc: Value = match serde_yaml_ng::from_str(text) {
        Ok(v) => v,
        Err(e) => return fail(invalid(e.to_string())),
    };
    let Some(map) = doc.as_object_mut() else {
        return fail(invalid("not a mapping".to_string()));
    };
    match map.remove("apiVersion") {
        Some(Value::String(v)) if v == API_VERSION => {}
        _ => return fail(Message::new("blueprint.specApiVersion").with("expected", API_VERSION)),
    }
    let spec: DeploySpec = match serde_json::from_value(doc) {
        Ok(s) => s,
        Err(e) => return fail(invalid(e.to_string())),
    };
    validate(&spec)?;
    Ok(spec)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Document<'a> {
    api_version: &'static str,
    #[serde(flatten)]
    spec: &'a DeploySpec,
}

/// Canonical YAML (initial value of the wizard's YAML mode).
pub fn to_yaml(spec: &DeploySpec) -> String {
    let doc = Document {
        api_version: API_VERSION,
        spec,
    };
    serde_yaml_ng::to_string(&doc).unwrap_or_default()
}

fn check_list(field: &str, items: &[String]) -> Result<()> {
    if items.len() > MAX_LIST {
        return fail(
            Message::new("blueprint.specListTooLong")
                .with("field", field)
                .with("max", MAX_LIST.to_string()),
        );
    }
    for item in items {
        check_item(field, item)?;
    }
    Ok(())
}

fn check_item(field: &str, item: &str) -> Result<()> {
    let len = item.chars().count();
    if len == 0 || len > MAX_ITEM {
        return fail(
            Message::new("blueprint.specItemLength")
                .with("field", field)
                .with("max", MAX_ITEM.to_string()),
        );
    }
    Ok(())
}

pub fn validate(spec: &DeploySpec) -> Result<()> {
    match spec.driver.as_str() {
        "k8s" => return fail(Message::new("blueprint.driverNotImplemented")),
        "native" | "docker" | "compose" | "stack" => {}
        other => return fail(Message::new("blueprint.driverUnsupported").with("driver", other)),
    }
    if spec.manager.is_some() && spec.driver != "stack" {
        return fail(Message::new("blueprint.specManagerOnlyStack"));
    }
    if let Some(manager) = &spec.manager {
        check_item("manager", manager)?;
    }
    for (name, role) in &spec.roles {
        if !(1..=MAX_INSTANCES).contains(&role.instances) {
            return fail(
                Message::new("blueprint.specInstancesRange")
                    .with("role", name)
                    .with("max", MAX_INSTANCES.to_string()),
            );
        }
        let hosts = &role.hosts;
        check_list("tags", &hosts.tags)?;
        check_list("names", &hosts.names)?;
        if let Some(group) = &hosts.group {
            check_item("group", group)?;
        }
        if let Some(spread) = &hosts.spread {
            let len = spread.prefix.chars().count();
            if len == 0 || len > MAX_PREFIX {
                return fail(
                    Message::new("blueprint.specPrefixLength").with("max", MAX_PREFIX.to_string()),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const YAML: &str = r#"
apiVersion: ferry/deploy-v1
template: {id: "123", version: 2}
driver: stack
manager: swarm-1
values:
  replicas: 3
  name: demo
roles:
  web:
    instances: 2
    hosts:
      tags: [prod]
      spread: {prefix: web}
  db:
    instances: 1
    hosts: {names: [db-1], group: data}
placement:
  db:
    - {hostId: "7", address: 10.0.0.7}
"#;

    fn err(r: Result<DeploySpec>) -> String {
        match r {
            Err(e) => format!("{e:?}"),
            Ok(_) => "ok".to_string(),
        }
    }

    fn json_with(f: impl FnOnce(&mut Value)) -> String {
        let mut v = json!({
            "apiVersion": "ferry/deploy-v1",
            "template": {"id": "1", "version": 1},
            "driver": "native",
            "roles": {"web": {"instances": 1}}
        });
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn spec_parses_yaml_and_json() {
        let spec = parse(YAML).unwrap();
        assert_eq!(spec.template.version, 2);
        assert_eq!(spec.manager.as_deref(), Some("swarm-1"));
        assert_eq!(spec.roles["web"].instances, 2);
        assert_eq!(
            spec.roles["web"].hosts.spread.as_ref().unwrap().prefix,
            "web"
        );
        assert_eq!(spec.roles["db"].hosts.group.as_deref(), Some("data"));
        assert_eq!(
            spec.placement.as_ref().unwrap()["db"][0].address,
            "10.0.0.7"
        );
        let as_json = serde_json::to_string(&Document {
            api_version: API_VERSION,
            spec: &spec,
        })
        .unwrap();
        assert_eq!(parse(&as_json).unwrap(), spec);
        assert!(parse(&json_with(|_| {})).is_ok());
    }

    #[test]
    fn unknown_top_level_key_rejected() {
        for k in ["failover", "backup", "storage"] {
            let text = json_with(|v| v[k] = json!({}));
            assert!(err(parse(&text)).contains("blueprint.specInvalid"), "{k}");
        }
        let text = json_with(|v| v["roles"]["web"]["bogus"] = json!(1));
        assert!(err(parse(&text)).contains("blueprint.specInvalid"));
        let text = json_with(|v| {
            v.as_object_mut().unwrap().remove("apiVersion");
        });
        assert!(err(parse(&text)).contains("blueprint.specApiVersion"));
        let text = json_with(|v| v["apiVersion"] = json!("ferry/deploy-v2"));
        assert!(err(parse(&text)).contains("blueprint.specApiVersion"));
        assert!(err(parse("- a")).contains("blueprint.specInvalid"));
    }

    #[test]
    fn bounds_enforced() {
        let inst = |n: u32| json_with(|v| v["roles"]["web"]["instances"] = n.into());
        assert!(parse(&inst(1)).is_ok());
        assert!(parse(&inst(100)).is_ok());
        for n in [0, 101] {
            assert!(err(parse(&inst(n))).contains("blueprint.specInstancesRange"));
        }
        let hosts = |h: Value| json_with(|v| v["roles"]["web"]["hosts"] = h);
        let many: Vec<String> = (0..33).map(|i| format!("t{i}")).collect();
        let few: Vec<String> = many[..32].to_vec();
        assert!(parse(&hosts(json!({"tags": few}))).is_ok());
        assert!(err(parse(&hosts(json!({"tags": many})))).contains("specListTooLong"));
        assert!(err(parse(&hosts(json!({"names": many})))).contains("specListTooLong"));
        assert!(err(parse(&hosts(json!({"tags": [""]})))).contains("specItemLength"));
        let long = "x".repeat(61);
        assert!(err(parse(&hosts(json!({"names": [long]})))).contains("specItemLength"));
        assert!(parse(&hosts(json!({"names": ["x".repeat(60)]}))).is_ok());
        let prefix = |p: String| hosts(json!({"spread": {"prefix": p}}));
        assert!(err(parse(&prefix(String::new()))).contains("specPrefixLength"));
        assert!(err(parse(&prefix("p".repeat(31)))).contains("specPrefixLength"));
        assert!(parse(&prefix("p".repeat(30))).is_ok());
    }

    #[test]
    fn driver_checked() {
        let d = |d: &str| json_with(|v| v["driver"] = d.into());
        assert!(err(parse(&d("k8s"))).contains("blueprint.driverNotImplemented"));
        assert!(err(parse(&d("nomad"))).contains("blueprint.driverUnsupported"));
        for ok in ["native", "docker", "compose", "stack"] {
            assert!(parse(&d(ok)).is_ok());
        }
    }

    #[test]
    fn manager_only_for_stack() {
        let m = |driver: &str| {
            json_with(|v| {
                v["driver"] = driver.into();
                v["manager"] = "swarm-1".into();
            })
        };
        assert!(parse(&m("stack")).is_ok());
        for d in ["native", "docker", "compose"] {
            assert!(err(parse(&m(d))).contains("blueprint.specManagerOnlyStack"));
        }
    }

    #[test]
    fn roundtrip_to_yaml() {
        let spec = parse(YAML).unwrap();
        let yaml = to_yaml(&spec);
        assert!(yaml.starts_with("apiVersion: ferry/deploy-v1"));
        assert_eq!(parse(&yaml).unwrap(), spec);
    }
}
