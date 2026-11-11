use std::process::Command;

#[test]
fn service_commands_are_available_without_a_server_or_enrolment_token() {
    let output = Command::new(env!("CARGO_BIN_EXE_ferry-runner"))
        .args(["service", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8_lossy(&output.stdout);
    for verb in ["install", "start", "stop", "status", "uninstall"] {
        assert!(help.contains(verb), "missing {verb}");
    }
}

#[test]
fn invalid_service_identity_is_rejected_before_any_system_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_ferry-runner"))
        .args(["service", "status", "--name", "../outside"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("服务名称"));
}

#[test]
fn existing_foreground_cli_still_accepts_its_original_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_ferry-runner"))
        .args(["--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--server") && help.contains("--token") && help.contains("--state-dir"));
}

#[test]
fn default_service_identity_has_a_product_prefix() {
    let output = Command::new(env!("CARGO_BIN_EXE_ferry-runner"))
        .args(["service", "install", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("ferry-runner-agent"));
}

#[cfg(windows)]
#[test]
fn default_service_resolution_preserves_legacy_identity_and_prefers_new_installation() {
    let home = tempfile::tempdir().unwrap();
    let status = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_ferry-runner"))
            .args(["service", "status", "--user"])
            .args(extra)
            .env("LOCALAPPDATA", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert!(status(&[]).starts_with("ferry-runner-agent:"));
    let legacy = home.path().join("runner-agent");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(legacy.join("service.json"), "{}").unwrap();
    assert!(status(&[]).starts_with("runner-agent:"));
    assert!(status(&["--name", "custom-fixture"]).starts_with("custom-fixture:"));
    let current = home.path().join("ferry-runner-agent");
    std::fs::create_dir(&current).unwrap();
    std::fs::write(current.join("service.json"), "{}").unwrap();
    assert!(status(&[]).starts_with("ferry-runner-agent:"));
}
