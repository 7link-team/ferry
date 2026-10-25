//! The desktop shell is thin on purpose: it starts `ferry-server` as a sidecar
//! (or points at a remote one), then opens the same web interface the Docker
//! deployment serves. Business logic never lives here — only the tray, the
//! connection settings and the service lifecycle.

mod server;
mod updater;

use tauri::Manager;

fn show_primary(app: &tauri::AppHandle) {
    if let Some(workspace) = app.get_webview_window("workspace") {
        let _ = workspace.show();
        let _ = workspace.unminimize();
        let _ = workspace.set_focus();
        if let Some(launcher) = app.get_webview_window("launcher") {
            let _ = launcher.hide();
        }
    } else if let Some(launcher) = app.get_webview_window("launcher") {
        let _ = launcher.show();
        let _ = launcher.unminimize();
        let _ = launcher.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let open = MenuItem::with_id(app, "open", "打开工作空间", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "连接设置 / 检查更新", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &settings, &quit])?;
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::other("application icon missing"))?
        .clone();
    TrayIconBuilder::with_id("desktop")
        .icon(icon)
        .tooltip(&app.package_info().name)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_primary(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_primary(app),
            "settings" => {
                if let Some(window) = app.get_webview_window("launcher") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_primary(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            setup_tray(app)?;
            let resume = app.path().app_local_data_dir()?.join("update-resume.json");
            if let Ok(bytes) = std::fs::read(&resume) {
                std::fs::remove_file(&resume)?;
                if let Ok(settings) = serde_json::from_slice::<server::Preferences>(&bytes) {
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(window) = handle.get_webview_window("launcher") {
                            if let Err(error) = server::open_workspace(
                                handle.clone(),
                                window.clone(),
                                settings.server_url,
                            )
                            .await
                            {
                                use tauri::Emitter;
                                let _ = window.emit("connection-error", error);
                                let _ = window.show();
                            }
                        }
                    });
                }
            }
            Ok(())
        })
        .manage(updater::UpdateState::default())
        .manage(server::Runtime::default())
        .invoke_handler(tauri::generate_handler![
            server::connection_settings,
            server::open_workspace,
            updater::check_update,
            updater::install_update,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().state::<updater::UpdateState>().busy() {
                    api.prevent_close();
                } else if window.label() == "launcher"
                    && window
                        .app_handle()
                        .get_webview_window("workspace")
                        .is_some()
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("build Ferry desktop shell")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if app.state::<updater::UpdateState>().busy() {
                    api.prevent_exit();
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                app.state::<server::Runtime>().stop();
            }
        });
}
