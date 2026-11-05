use super::*;

fn package() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../examples/workflow-extensions/public-repository.json"
    ))
    .unwrap()
}
fn validate_value(value: Value) -> Result<(Value, Vec<Value>)> {
    validate(serde_json::from_value(value)?)
}

#[test]
fn package_normalizes_server_contract_and_rejects_foreign_credentials_and_code() {
    let (_, nodes) = validate_value(package()).unwrap();
    assert_eq!(
        nodes[0]["runtime"],
        json!({"default":"server","allowed":["server"]})
    );
    assert_eq!(nodes[0]["ui"]["application"], "ext.public_repository.v1");
    assert_eq!(nodes[0]["cache_allowed"], false);
    for (pointer, value) in [
        ("/nodes/0/http/auth/profile", json!("github")),
        ("/nodes/0/type", json!("integration.github.repository.get")),
        ("/nodes/0/type_version", json!(2)),
        ("/nodes/0/http/origin", json!("http://127.0.0.1")),
        (
            "/nodes/0/http/origin",
            json!("https://user:pass@example.org"),
        ),
        ("/nodes/0/http/path", json!("//example.org")),
        ("/nodes/0/http/method", json!("POST")),
        ("/nodes/0/expression_fields", json!(["/credential_id"])),
        (
            "/nodes/0/params_schema/properties/owner/type",
            json!("object"),
        ),
        ("/nodes/0/example_params/owner", json!(42)),
    ] {
        let mut value_package = package();
        *value_package.pointer_mut(pointer).unwrap() = value;
        assert!(validate_value(value_package).is_err(), "{pointer}");
    }
    let mut code = package();
    code["nodes"][0]["execute"] = "arbitrary-code".into();
    assert!(validate_value(code).is_err());
    let mut remote_schema = package();
    remote_schema["nodes"][0]["params_schema"]["$ref"] = "https://example.org/schema".into();
    assert!(validate_value(remote_schema).is_err());
}

#[test]
fn installation_versions_are_immutable_and_disabling_preserves_locks() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("workflow-extensions");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("sample.json");
    let value = package();
    std::fs::write(&path, value.to_string()).unwrap();
    let loaded = load(dir.path(), false).unwrap();
    assert_eq!(loaded.packages.len(), 1);
    assert_eq!(loaded.nodes.len(), 1);
    // Formatting changes preserve identity; semantic edits require a new version.
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    assert_eq!(load(dir.path(), false).unwrap().nodes.len(), 1);
    let mut changed = value.clone();
    changed["nodes"][0]["http"]["path"] = "/users/{owner}".into();
    std::fs::write(&path, changed.to_string()).unwrap();
    assert!(
        load(dir.path(), false)
            .err()
            .unwrap()
            .to_string()
            .contains("content changed")
    );
    std::fs::rename(&path, folder.join("sample.json.disabled")).unwrap();
    assert!(load(dir.path(), false).unwrap().nodes.is_empty());
    assert!(dir.path().join("workflow-extensions.lock.json").exists());
    std::fs::write(&path, value.to_string()).unwrap();
    assert_eq!(load(dir.path(), false).unwrap().nodes.len(), 1);
    assert!(load(dir.path(), true).is_err());
    std::fs::write(folder.join("duplicate.json"), value.to_string()).unwrap();
    assert!(load(dir.path(), false).is_err());
}

#[test]
fn broken_installation_never_updates_version_lock() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("workflow-extensions");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a.json"), package().to_string()).unwrap();
    std::fs::write(folder.join("b.json"), b"not-json").unwrap();
    assert!(load(dir.path(), false).is_err());
    assert!(!dir.path().join("workflow-extensions.lock.json").exists());
}
