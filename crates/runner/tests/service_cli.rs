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
