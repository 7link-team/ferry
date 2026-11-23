fn main() {
    println!("cargo:rerun-if-env-changed=MOBILE_ANDROID_KEYSTORE_PATH");
    println!(
        "cargo:rustc-env=CLIENT_PERSISTENT_ANDROID_SIGNING={}",
        std::env::var("MOBILE_ANDROID_KEYSTORE_PATH").is_ok_and(|path| !path.trim().is_empty())
    );
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        link_ios_navigation();
    }
    // Unit-test executables also link rfd's TaskDialogIndirect. Unlike the app
    // executable they do not receive Tauri's embedded Windows resource manifest.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    tauri_build::build()
}

#[cfg(target_os = "macos")]
fn link_ios_navigation() {
    use std::path::Path;
    // The pinned tauri-plugin build helper uses this same API package layout.
    // Its copy_folder helper is private; keep only the required source copy here.
    fn copy_api(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".build" | "Package.resolved" | "Tests")) {
                continue;
            }
            let target = destination.join(name);
            if entry.file_type()?.is_dir() {
                copy_api(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let api = std::env::var_os("DEP_TAURI_IOS_LIBRARY_PATH")
        .expect("Tauri iOS API package metadata missing");
    copy_api(Path::new(&api), &root.join(".tauri/tauri-api"))
        .expect("copy pinned Tauri iOS API sources");
    println!("cargo:rerun-if-changed=ios");
    tauri_utils::build::link_apple_library("client-navigation", root.join("ios"));
}
