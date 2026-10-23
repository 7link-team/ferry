//! Sidecar lifecycle and the launcher's two commands. The launcher window is
//! the only window with native permissions; the business window is a plain
//! webview onto the server's own origin.

use std::{process::Child, sync::Mutex};

pub const LOCAL_URL: &str = "http://127.0.0.1:4290";

#[derive(Default)]
pub struct Runtime {
    #[allow(dead_code)]
    child: Mutex<Option<Child>>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Preferences {
    /// Empty means "run the bundled server on this machine".
    pub server_url: String,
}

#[tauri::command]
pub async fn connection_settings() -> Result<Preferences, String> {
    // ponytail: not persisted yet; the launcher UI reads defaults.
    Ok(Preferences::default())
}

/// Opens the business window on the chosen server. Only the launcher may ask.
#[tauri::command]
pub async fn open_workspace(app: tauri::AppHandle, server_url: String) -> Result<(), String> {
    let target = if server_url.is_empty() {
        LOCAL_URL.to_owned()
    } else {
        server_url
    };
    let url = url::Url::parse(&target).map_err(|_| "服务地址格式不正确。".to_string())?;
    tauri::WebviewWindowBuilder::new(&app, "workspace", tauri::WebviewUrl::External(url))
        .title("Ferry")
        .inner_size(1280.0, 800.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
