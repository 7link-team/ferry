//! The desktop shell is thin on purpose: it starts `ferry-server` as a sidecar
//! (or points at a remote one), then opens the same web interface the Docker
//! deployment serves. Business logic never lives here — only the tray, the
//! connection settings and the service lifecycle.

mod server;
mod updater;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("launcher") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updater::UpdateState::default())
        .manage(server::Runtime::default())
        .invoke_handler(tauri::generate_handler![
            server::connection_settings,
            server::open_workspace,
            updater::check_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("run Ferry desktop shell");
}
