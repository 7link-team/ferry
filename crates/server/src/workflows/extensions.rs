//! Administrator-installed, data-only HTTP node packages. Loaded once before
//! workers start; package content never executes code or mutates a running catalog.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    format: u32,
    id: String,
    version: u32,
    name: String,
    nodes: Vec<Value>,
}

#[derive(Default)]
pub(super) struct Catalog {
    pub nodes: Vec<Value>,
    pub packages: Vec<Value>,
}
static CATALOG: OnceLock<Catalog> = OnceLock::new();
pub(super) fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(Catalog::default)
}

fn identifier(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.len() <= 48
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
fn text(value: &Value) -> Result<&str> {
    let value = value.as_str().context("expected text")?;
    ensure!(
        !value.trim().is_empty() && value.len() <= 512,
        "invalid label"
    );
    Ok(value)
}
fn keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let object = value.as_object().context("expected object")?;
    ensure!(
        object.keys().all(|key| allowed.contains(&key.as_str())),
        "unsupported declaration field"
    );
    Ok(())
}

fn validate(package: Package) -> Result<(Value, Vec<Value>)> {
    ensure!(
        package.format == 1 && identifier(&package.id) && package.version > 0,
        "invalid package identity/format"
    );
    ensure!(
        !package.name.trim().is_empty() && package.name.len() <= 100,
        "invalid package name"
    );
    ensure!(
        !package.nodes.is_empty() && package.nodes.len() <= 100,
        "package must contain 1..100 nodes"
    );
    let profile = format!("ext.{}.v{}", package.id, package.version);
    let prefix = format!("integration.{profile}.");
    let mut nodes = Vec::new();
    for mut node in package.nodes {
        keys(
            &node,
            &[
                "type",
                "type_version",
                "params_schema",
                "example_params",
                "expression_fields",
                "ui",
                "http",
            ],
        )?;
        let kind = text(&node["type"])?;
        ensure!(
            kind.strip_prefix(&prefix).is_some_and(identifier) && node["type_version"] == 1,
            "node must use its package/version namespace"
        );
        let schema = &node["params_schema"];
        keys(
            schema,
            &["type", "properties", "required", "additionalProperties"],
        )?;
        ensure!(
            schema["type"] == "object" && schema["additionalProperties"] == false,
            "parameters must be a closed object"
        );
        let fields = schema["properties"]
            .as_object()
            .context("missing properties")?;
        ensure!(
            !fields.is_empty() && fields.len() <= 64,
            "invalid parameter count"
        );
        for (name, field) in fields {
            ensure!(identifier(name), "invalid parameter name");
            keys(
                field,
                &[
                    "type",
                    "title_key",
                    "enum",
                    "minimum",
                    "maximum",
                    "minLength",
                    "maxLength",
                    "control",
                    "pattern",
                ],
            )?;
            ensure!(
                matches!(
                    field["type"].as_str(),
                    Some("string" | "integer" | "number")
                ),
                "unsupported parameter type"
            );
            ensure!(
                text(&field["title_key"])?.starts_with("literal:"),
                "package labels must use literal: text"
            );
            if let Some(control) = field.get("control") {
                ensure!(
                    control == "textarea" && field["type"] == "string",
                    "unsupported control"
                );
            }
            if let Some(values) = field.get("enum") {
                let values = values.as_array().context("invalid enum")?;
                ensure!(
                    !values.is_empty()
                        && values.len() <= 100
                        && values.iter().all(Value::is_string)
                        && field["type"] == "string",
                    "unsupported enum"
                );
            }
        }
        ensure!(
            fields
                .get("credential_id")
                .is_some_and(|f| f["type"] == "string"),
            "credential_id must be a string"
        );
        let required = schema["required"].as_array().context("missing required")?;
        ensure!(
            required
                .iter()
                .all(|v| v.as_str().is_some_and(|s| fields.contains_key(s)))
                && required.contains(&json!("credential_id")),
            "invalid required fields"
        );
        let expressions = node["expression_fields"]
            .as_array()
            .context("missing expression_fields")?;
        ensure!(
            expressions.iter().all(|v| v
                .as_str()
                .and_then(|s| s.strip_prefix('/'))
                .is_some_and(|s| s != "credential_id" && fields.contains_key(s))),
            "invalid expression path"
        );
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(schema)
            .map_err(|_| anyhow::anyhow!("invalid parameter schema"))?;
        ensure!(
            validator.is_valid(&node["example_params"]),
            "example parameters must satisfy schema"
        );
        keys(
            &node["ui"],
            &[
                "resource_key",
                "operation_key",
                "credential_help_key",
                "aliases",
            ],
        )?;
        for key in ["resource_key", "operation_key", "credential_help_key"] {
            ensure!(
                text(&node["ui"][key])?.starts_with("literal:"),
                "package labels must use literal: text"
            );
        }
        let aliases = node["ui"]["aliases"]
            .as_array()
            .context("missing aliases")?;
        ensure!(
            aliases.len() <= 30 && aliases.iter().all(|v| text(v).is_ok()),
            "invalid aliases"
        );
        keys(
            &node["http"],
            &[
                "origin", "path", "method", "auth", "headers", "query", "body", "success",
            ],
        )?;
        let origin = reqwest::Url::parse(text(&node["http"]["origin"])?)?;
        ensure!(
            origin.scheme() == "https",
            "extensions require HTTPS origins"
        );
        keys(
            &node["http"]["auth"],
            &["header", "prefix", "profile", "optional"],
        )?;
        ensure!(
            node["http"]["auth"]["profile"] == profile,
            "credentials must use package/version profile"
        );
        if let Some(optional) = node["http"]["auth"].get("optional") {
            ensure!(optional.is_boolean(), "invalid optional auth");
        }
        if node["http"]["auth"]["optional"] == true {
            ensure!(
                node["http"]["method"] == "GET",
                "writes require a credential"
            );
        }
        for map in ["query", "body"] {
            ensure!(
                node["http"][map]
                    .as_object()
                    .is_some_and(|m| m.values().all(|v| v
                        .as_str()
                        .is_some_and(|s| s != "credential_id" && fields.contains_key(s)))),
                "request mapping references an unknown field or credential"
            );
        }
        ensure!(
            !node["http"]["path"]
                .as_str()
                .unwrap_or_default()
                .contains("{credential_id}"),
            "credential ID cannot be sent in a path"
        );
        if let Some(success) = node["http"].get("success") {
            keys(success, &["pointer", "equals"])?;
            ensure!(
                success["pointer"]
                    .as_str()
                    .is_some_and(|s| s.starts_with('/'))
                    && success.get("equals").is_some(),
                "invalid success predicate"
            );
        }
        // Compile the existing request builder, without sending any request.
        super::integration::validate_declaration(&node["http"], &node["example_params"])?;
        node["ui"]["application"] = profile.clone().into();
        node["ui"]["application_name"] = format!("{} v{}", package.name, package.version).into();
        for (key, value) in [
            ("runtime", json!({"default":"server","allowed":["server"]})),
            ("inputs", json!([{"port":"main","type":"main"}])),
            ("outputs", json!([{"port":"main","type":"main"}])),
            ("credential_slots", json!({})),
            ("cache_allowed", json!(false)),
            ("purity", json!("side_effect")),
            ("item_mode", json!("per_item")),
            ("impl_version", json!(package.version.to_string())),
        ] {
            node[key] = value;
        }
        nodes.push(node);
    }
    Ok((
        json!({"id":package.id,"version":package.version,"name":package.name,"node_count":nodes.len()}),
        nodes,
    ))
}

/// Loading is deliberately fail-closed: an invalid enabled package must not be
/// silently omitted while a saved workflow expects it. Files are operator-owned.
fn load(directory: &Path, clustered: bool) -> Result<Catalog> {
    let packages_dir = directory.join("workflow-extensions");
    if !packages_dir.exists() {
        return Ok(Catalog::default());
    }
    let mut paths = std::fs::read_dir(&packages_dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    ensure!(paths.len() <= 100, "too many extension packages");
    ensure!(
        !clustered || paths.is_empty(),
        "runtime extension packages are not supported in cluster mode yet"
    );
    let lock_path = directory.join("workflow-extensions.lock.json");
    let mut locks: BTreeMap<String, String> = if lock_path.exists() {
        serde_json::from_slice(&std::fs::read(&lock_path)?)
            .context("invalid extension version lock")?
    } else {
        BTreeMap::new()
    };
    let original = locks.clone();
    let mut catalog = Catalog::default();
    let mut types = std::collections::BTreeSet::new();
    for path in paths {
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "extension symlinks are not supported"
        );
        let mut file = std::fs::File::open(&path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(&mut file, 1024 * 1024 + 1),
            &mut bytes,
        )?;
        ensure!(bytes.len() <= 1024 * 1024, "extension exceeds 1 MiB");
        let value: Value = serde_json::from_slice(&bytes).context("invalid extension JSON")?;
        let canonical = crate::definitions::canonical(&value.to_string())
            .map_err(|_| anyhow::anyhow!("invalid package JSON"))?;
        let hash = hex::encode(Sha256::digest(canonical.as_bytes()));
        let (mut info, nodes) = validate(serde_json::from_value(value)?)
            .with_context(|| format!("invalid extension {}", path.display()))?;
        let identity = format!("{}@{}", info["id"].as_str().unwrap(), info["version"]);
        if let Some(previous) = locks.get(&identity) {
            ensure!(
                previous == &hash,
                "extension {identity} content changed; publish a new version"
            );
        }
        for node in &nodes {
            ensure!(
                types.insert(node["type"].as_str().unwrap().to_owned()),
                "duplicate extension node"
            );
        }
        locks.insert(identity, hash.clone());
        info["sha256"] = hash.into();
        catalog.packages.push(info);
        catalog.nodes.extend(nodes);
    }
    if original != locks {
        // Only written after every package passed validation. A failed write
        // aborts startup; a truncated lock is never accepted on restart.
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&locks)?)?;
        file.as_file().sync_all()?;
        file.persist(&lock_path).map_err(|error| error.error)?;
    }
    Ok(catalog)
}

pub(crate) fn initialize(directory: &Path, clustered: bool) -> Result<()> {
    let catalog = load(directory, clustered)?;
    crate::credentials::register_profiles(&catalog.nodes);
    if CATALOG.set(catalog).is_err() {
        bail!("workflow extensions already initialized");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
