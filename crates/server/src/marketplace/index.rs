//! Repository index parsing: legacy v1 template lists and v2 versioned package bundles.
//! Pure functions only; storage and transport live elsewhere.
use crate::error::{ApiError, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;

pub struct PackageVersion {
    pub key: String,
    pub kind: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub meta: Value,
    pub content: Value,
    pub parameters: Value,
    pub files: Vec<PackageFile>,
    pub readme: String,
    pub compatible: bool,
    pub incompatible_reason: Option<String>,
    pub digest: String,
}
pub struct PackageFile {
    pub name: String,
    pub content: String,
    pub parameters: Value,
}

const MAX_PACKAGES: usize = 500;
const MAX_VERSIONS: usize = 20;
const MAX_FILES: usize = 20;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_ICON_BYTES: usize = 64 * 1024;
const MAX_README_BYTES: usize = 64 * 1024;
/// Config template names are at most 60 characters; keep room for the "-20" conflict suffix.
const MAX_FILE_NAME: usize = 57;
const CATEGORIES: &[&str] = &["database", "cache", "messaging", "web", "scheduler", "platform", "other"];
const DRIVERS: &[&str] = &["native", "docker", "compose", "stack", "k8s"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Index {
    version: u32,
    pub(crate) templates: Vec<IndexEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexEntry {
    pub(crate) key: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) content: Value,
    #[serde(default = "empty_parameters")]
    pub(crate) parameters: Value,
}
fn empty_parameters() -> Value {
    json!([])
}
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 100 && key.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
/// Legacy v1 index: one application template per entry, all validated or none stored.
pub(crate) fn parse_index(text: &str) -> Result<Index> {
    let index: Index = serde_json::from_str(text).map_err(|_| bad("marketplace.indexObjectRequired"))?;
    if index.version != 1 || index.templates.len() > MAX_PACKAGES {
        return Err(bad("marketplace.indexVersionInvalid"));
    }
    let mut keys = HashSet::new();
    for entry in &index.templates {
        if !valid_key(&entry.key)
            || !keys.insert(&entry.key)
            || entry.name.trim().is_empty()
            || entry.name.chars().count() > 60
        {
            return Err(bad("marketplace.templateInvalid"));
        }
        crate::templates::validate_app(&entry.content.to_string(), &entry.parameters.to_string())?;
    }
    Ok(index)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Bundle {
    schema_version: u32,
    #[serde(default)]
    #[allow(dead_code)]
    generated_at: String,
    packages: Vec<Package>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    key: String,
    kind: String,
    meta: Value,
    #[serde(default)]
    icon: String,
    versions: Vec<Version>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    version: String,
    #[serde(default)]
    requires: Requires,
    blueprint: Value,
    #[serde(default = "empty_parameters")]
    parameters: Value,
    #[serde(default)]
    files: Vec<File>,
    #[serde(default)]
    readme: String,
    #[serde(default = "empty_parameters")]
    verified: Value,
    #[serde(default)]
    #[allow(dead_code)]
    sha256: String,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Requires {
    #[serde(default)]
    ferry: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: String,
    content: String,
    #[serde(default = "empty_parameters")]
    parameters: Value,
}

/// Picks a localized string: requested locale, then en-US, then zh-CN.
pub(crate) fn localized(map: &Value, locale: &str) -> Option<String> {
    [locale, "en-US", "zh-CN"]
        .iter()
        .find_map(|l| map.get(*l).and_then(Value::as_str).filter(|v| !v.trim().is_empty()))
        .map(str::to_owned)
}

fn validate_meta(meta: &Value) -> Result<()> {
    let invalid = || bad("marketplace.packageMetaInvalid");
    let object = meta.as_object().ok_or_else(invalid)?;
    let texts = |key: &str, max: usize, required: bool| -> Result<()> {
        match object.get(key) {
            None if !required => Ok(()),
            Some(Value::Object(map)) => {
                let has_base = map.contains_key("zh-CN") || map.contains_key("en-US");
                if (required && !has_base)
                    || map.values().any(|v| v.as_str().is_none_or(|t| t.chars().count() > max || (required && t.trim().is_empty())))
                {
                    return Err(invalid());
                }
                Ok(())
            }
            _ => Err(invalid()),
        }
    };
    texts("name", 60, true)?;
    texts("description", 500, false)?;
    let text = |key: &str| object.get(key).and_then(Value::as_str);
    if !text("category").is_some_and(|c| CATEGORIES.contains(&c)) || !matches!(text("topology"), Some("single" | "cluster")) {
        return Err(invalid());
    }
    let drivers = object.get("drivers").and_then(Value::as_array).ok_or_else(invalid)?;
    if drivers.is_empty() || drivers.iter().any(|d| !d.as_str().is_some_and(|d| DRIVERS.contains(&d))) {
        return Err(invalid());
    }
    let list = |key: &str, count: usize, len: usize| match object.get(key) {
        None => true,
        Some(Value::Array(items)) => items.len() <= count && items.iter().all(|i| i.as_str().is_some_and(|t| !t.is_empty() && t.chars().count() <= len)),
        _ => false,
    };
    if !list("tags", 10, 30) || !list("maintainers", 10, 100) {
        return Err(invalid());
    }
    if let Some(home) = object.get("homepage") {
        if !home.as_str().and_then(|h| reqwest::Url::parse(h).ok()).is_some_and(|u| u.scheme() == "https") {
            return Err(invalid());
        }
    }
    Ok(())
}

fn digest(content: &Value, parameters: &Value, files: &[PackageFile]) -> String {
    use sha2::Digest;
    let files: Vec<Value> = files.iter().map(|f| json!({"name":f.name,"content":f.content,"parameters":f.parameters})).collect();
    let canonical = json!({"content":content,"parameters":parameters,"files":files}).to_string();
    hex::encode(sha2::Sha256::digest(canonical.as_bytes()))
}

/// Compatibility is judged against the release triple so pre-release builds satisfy `>=x.y.z`.
fn compatibility(kind: &str, requires: &str, content: &Value, ferry: &semver::Version) -> Result<Option<String>> {
    if kind != "app" {
        return Ok(Some("marketplace.kindUnsupported".into()));
    }
    if !requires.trim().is_empty() {
        let requirement = semver::VersionReq::parse(requires).map_err(|_| bad("marketplace.requirementInvalid"))?;
        if !requirement.matches(&semver::Version::new(ferry.major, ferry.minor, ferry.patch)) {
            return Ok(Some("marketplace.ferryVersionUnsupported".into()));
        }
    }
    if content.get("kind").and_then(Value::as_str) == Some("Blueprint") {
        return Ok(Some("marketplace.blueprintRequiresUpgrade".into()));
    }
    Ok(None)
}

pub fn parse_bundle(text: &str, ferry_version: &semver::Version, locale: &str) -> Result<Vec<PackageVersion>> {
    let document: Value = serde_json::from_str(text).map_err(|_| bad("marketplace.indexObjectRequired"))?;
    if document.get("schemaVersion").is_none() {
        return Ok(parse_index(text)?
            .templates
            .into_iter()
            .map(|e| {
                let digest = digest(&e.content, &e.parameters, &[]);
                PackageVersion {
                    key: e.key,
                    kind: "app".into(),
                    version: "0.0.0".into(),
                    name: e.name,
                    description: e.description,
                    meta: json!({}),
                    content: e.content,
                    parameters: e.parameters,
                    files: Vec::new(),
                    readme: String::new(),
                    compatible: true,
                    incompatible_reason: None,
                    digest,
                }
            })
            .collect());
    }
    if document["schemaVersion"] != json!(2) {
        return Err(bad("marketplace.indexVersionInvalid"));
    }
    let bundle: Bundle = serde_json::from_value(document).map_err(|_| bad("marketplace.indexObjectRequired"))?;
    if bundle.schema_version != 2 || bundle.packages.len() > MAX_PACKAGES {
        return Err(bad("marketplace.indexVersionInvalid"));
    }
    let mut keys = HashSet::new();
    let mut rows = Vec::new();
    for package in bundle.packages {
        if !valid_key(&package.key) || !keys.insert(package.key.clone()) || package.versions.is_empty() || package.versions.len() > MAX_VERSIONS || package.icon.len() > MAX_ICON_BYTES {
            return Err(bad("marketplace.templateInvalid"));
        }
        validate_meta(&package.meta)?;
        let name = localized(&package.meta["name"], locale).ok_or_else(|| bad("marketplace.packageMetaInvalid"))?;
        let description = localized(&package.meta["description"], locale).unwrap_or_default();
        let mut versions = HashSet::new();
        for v in package.versions {
            let parsed = semver::Version::parse(&v.version).map_err(|_| bad("marketplace.packageVersionInvalid"))?;
            if !versions.insert(parsed) || v.files.len() > MAX_FILES || v.readme.len() > MAX_README_BYTES || !v.verified.is_array() {
                return Err(bad("marketplace.templateInvalid"));
            }
            let mut names = HashSet::new();
            let mut files = Vec::with_capacity(v.files.len());
            for f in v.files {
                if !valid_key(&f.name) || f.name.len() > MAX_FILE_NAME || !names.insert(f.name.clone()) || f.content.len() > MAX_FILE_BYTES {
                    return Err(bad("marketplace.packageFileInvalid"));
                }
                files.push(PackageFile { name: f.name, content: f.content, parameters: f.parameters });
            }
            let reason = compatibility(&package.kind, &v.requires.ferry, &v.blueprint, ferry_version)?;
            if reason.is_none() {
                crate::templates::validate_app(&v.blueprint.to_string(), &v.parameters.to_string())?;
                for f in &files {
                    crate::templates::validate_config(&f.content, &f.parameters.to_string())?;
                }
            }
            let mut meta = package.meta.clone();
            meta["icon"] = json!(package.icon);
            meta["verified"] = v.verified;
            rows.push(PackageVersion {
                key: package.key.clone(),
                kind: package.kind.clone(),
                version: v.version,
                name: name.clone(),
                description: description.clone(),
                meta,
                digest: digest(&v.blueprint, &v.parameters, &files),
                content: v.blueprint,
                parameters: v.parameters,
                files,
                readme: v.readme,
                compatible: reason.is_none(),
                incompatible_reason: reason,
            });
        }
    }
    Ok(rows)
}

fn bad(key: &str) -> ApiError {
    ApiError::localized(axum::http::StatusCode::BAD_REQUEST, crate::i18n::Message::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ferry() -> semver::Version {
        semver::Version::parse("0.3.0").unwrap()
    }
    fn app() -> Value {
        json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"busybox:1.36"}]})
    }
    fn package(versions: Value) -> Value {
        json!({"key":"web","kind":"app","meta":{"name":{"zh-CN":"网站","en-US":"Website"},"description":{"en-US":"Demo"},"category":"web","topology":"single","drivers":["docker","compose"]},"versions":versions})
    }
    fn version(v: &str) -> Value {
        json!({"version":v,"blueprint":app(),"parameters":[],"files":[{"name":"nginx.conf","content":"listen 80;","parameters":[]}],"readme":"# Web"})
    }
    fn bundle(packages: Value) -> String {
        json!({"schemaVersion":2,"generatedAt":"2026-11-11T00:00:00Z","packages":packages}).to_string()
    }

    #[test]
    fn v1_index_maps_to_single_version_package() {
        let text = json!({"version":1,"templates":[{"key":"web","name":"Imported app","description":"fixture","content":app(),"parameters":[]}]}).to_string();
        let rows = parse_bundle(&text, &ferry(), "zh-CN").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "0.0.0");
        assert_eq!(rows[0].kind, "app");
        assert_eq!(rows[0].name, "Imported app");
        assert!(rows[0].compatible);
        assert!(rows[0].files.is_empty());
    }

    #[test]
    fn v2_bundle_parses_versions_files_and_locale_fallback() {
        let text = bundle(json!([package(json!([version("1.0.0"), version("1.1.0")]))]));
        let rows = parse_bundle(&text, &ferry(), "ms-MY").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Website");
        assert_eq!(rows[0].description, "Demo");
        assert_eq!(rows[1].version, "1.1.0");
        assert_eq!(rows[0].files.len(), 1);
        assert_eq!(rows[0].files[0].name, "nginx.conf");
        assert_eq!(rows[0].readme, "# Web");
        assert_eq!(rows[0].meta["category"], "web");
        let zh = parse_bundle(&text, &ferry(), "zh-CN").unwrap();
        assert_eq!(zh[0].name, "网站");
    }

    #[test]
    fn v2_unknown_schema_version_rejected() {
        let text = json!({"schemaVersion":3,"packages":[]}).to_string();
        assert!(parse_bundle(&text, &ferry(), "en-US").is_err());
    }

    #[test]
    fn v2_unknown_kind_or_newer_ferry_marked_incompatible() {
        let mut pipeline = package(json!([version("1.0.0")]));
        pipeline["kind"] = json!("pipeline");
        pipeline["key"] = json!("build");
        let mut newer = version("2.0.0");
        newer["requires"] = json!({"ferry":">=99.0.0"});
        let mut blueprint = version("3.0.0");
        blueprint["blueprint"] = json!({"apiVersion":"ferry/blueprint-v1","kind":"Blueprint","metadata":{"name":"web"},"roles":[]});
        let text = bundle(json!([pipeline, package(json!([newer, blueprint]))]));
        let rows = parse_bundle(&text, &ferry(), "en-US").unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| !r.compatible && r.incompatible_reason.is_some()));
    }

    #[test]
    fn prerelease_ferry_satisfies_release_requirement() {
        let mut v = version("1.0.0");
        v["requires"] = json!({"ferry":">=0.3.0"});
        let text = bundle(json!([package(json!([v]))]));
        let rows = parse_bundle(&text, &semver::Version::parse("0.3.0-alpha.18").unwrap(), "en-US").unwrap();
        assert!(rows[0].compatible);
    }

    #[test]
    fn v2_limits_enforced() {
        let many: Vec<Value> = (0..21).map(|i| version(&format!("1.0.{i}"))).collect();
        assert!(parse_bundle(&bundle(json!([package(json!(many))])), &ferry(), "en-US").is_err());
        let mut files = version("1.0.0");
        files["files"] = json!((0..21).map(|i| json!({"name":format!("f{i}"),"content":"x"})).collect::<Vec<_>>());
        assert!(parse_bundle(&bundle(json!([package(json!([files]))])), &ferry(), "en-US").is_err());
        let mut big = version("1.0.0");
        big["files"] = json!([{"name":"big","content":"x".repeat(256 * 1024 + 1)}]);
        assert!(parse_bundle(&bundle(json!([package(json!([big]))])), &ferry(), "en-US").is_err());
        let mut bad_key = package(json!([version("1.0.0")]));
        bad_key["key"] = json!("a b");
        assert!(parse_bundle(&bundle(json!([bad_key])), &ferry(), "en-US").is_err());
        assert!(parse_bundle(&bundle(json!([package(json!([version("latest")]))])), &ferry(), "en-US").is_err());
        let duplicate = package(json!([version("1.0.0"), version("1.0.0")]));
        assert!(parse_bundle(&bundle(json!([duplicate])), &ferry(), "en-US").is_err());
        let mut no_name = package(json!([version("1.0.0")]));
        no_name["meta"]["name"] = json!({"ja-JP":"ウェブ"});
        assert!(parse_bundle(&bundle(json!([no_name])), &ferry(), "en-US").is_err());
    }

    #[test]
    fn repository_bundle_parses() {
        let rows = parse_bundle(include_str!("../../../../marketplace/index.json"), &semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap(), "en-US").unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.compatible), "official templates must be importable by this build");
        assert!(rows.iter().any(|r| r.key == "http-service" && r.name == "HTTP sample service"));
    }

    #[test]
    fn file_names_leave_room_for_conflict_suffix() {
        let named = |name: String| {
            let mut v = version("1.0.0");
            v["files"] = json!([{"name":name,"content":"x"}]);
            bundle(json!([package(json!([v]))]))
        };
        assert!(parse_bundle(&named("f".repeat(57)), &ferry(), "en-US").is_ok());
        assert!(parse_bundle(&named("f".repeat(58)), &ferry(), "en-US").is_err());
    }

    #[test]
    fn digest_is_stable() {
        let text = bundle(json!([package(json!([version("1.0.0")]))]));
        let a = parse_bundle(&text, &ferry(), "en-US").unwrap();
        let b = parse_bundle(&text, &ferry(), "zh-CN").unwrap();
        assert_eq!(a[0].digest, b[0].digest);
        assert_eq!(a[0].digest.len(), 64);
        let changed = text.replace("listen 80;", "listen 81;");
        assert_ne!(parse_bundle(&changed, &ferry(), "en-US").unwrap()[0].digest, a[0].digest);
    }
}
