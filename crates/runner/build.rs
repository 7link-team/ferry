fn main() {
    println!("cargo:rerun-if-env-changed=RUNNER_UPDATE_PUBLIC_KEY");
    println!("cargo:rerun-if-changed=../../desktop/tauri.conf.json");
    let key = std::env::var("RUNNER_UPDATE_PUBLIC_KEY").unwrap_or_else(|_| {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../desktop/tauri.conf.json"))
                .expect("desktop release configuration");
        config["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("release public key")
            .to_owned()
    });
    assert!(
        !key.is_empty() && !key.chars().any(char::is_control),
        "invalid release public key"
    );
    println!("cargo:rustc-env=RUNNER_UPDATE_PUBLIC_KEY={key}");
    println!(
        "cargo:rustc-env=RUNNER_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
