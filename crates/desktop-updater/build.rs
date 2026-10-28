fn main() {
    println!("cargo:rerun-if-env-changed=DESKTOP_UPDATE_PUBLIC_KEY");
    println!("cargo:rerun-if-changed=../../desktop/tauri.conf.json");
    let config: serde_json::Value = serde_json::from_str(include_str!("../../desktop/tauri.conf.json")).unwrap();
    let key = std::env::var("DESKTOP_UPDATE_PUBLIC_KEY").unwrap_or_else(|_|config["plugins"]["updater"]["pubkey"].as_str().unwrap().into());
    for (name,value) in [
        ("DESKTOP_UPDATE_PUBLIC_KEY",key.as_str()),
        ("DESKTOP_PRODUCT",config["productName"].as_str().unwrap()),
        ("DESKTOP_IDENTIFIER",config["identifier"].as_str().unwrap()),
    ] {
        assert!(!value.is_empty() && !value.chars().any(char::is_control));
        println!("cargo:rustc-env={name}={value}");
    }
}
