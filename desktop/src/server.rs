//! The launcher owns the bundled service; remote workspaces have no native permissions.
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::Manager;

pub const LOCAL_URL: &str = "http://127.0.0.1:4290";

#[derive(Default)]
pub struct Runtime {
    child: Mutex<Option<Child>>,
    draining: AtomicBool,
    pub(crate) opening: tokio::sync::Mutex<()>,
}
impl Runtime {
    pub async fn drain(&self) -> Result<(), String> {
        {
            let mut guard = self.child.lock().unwrap();
            if let Some(child) = guard.as_mut() {
                // EOF is the private, cross-platform shutdown signal. No network endpoint.
                self.draining.store(true, Ordering::Release);
                child.stdin.take();
            } else {
                return Ok(());
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            {
                let mut guard = self.child.lock().unwrap();
                if let Some(child) = guard.as_mut() {
                    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                        guard.take();
                        self.draining.store(false, Ordering::Release);
                        return if status.success() {
                            Ok(())
                        } else {
                            Err(format!("本地服务未正常退出（{status}），已取消升级。"))
                        };
                    }
                } else {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "本地任务仍在结束中，未强制停止服务，也未安装更新。稍后可重新检查更新。".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Preferences {
    pub server_url: String,
}

fn read_preferences(data: &Path) -> Result<Option<Preferences>, String> {
    // Prefer the durable selection; the marker also migrates older updater resumes.
    for name in ["connection.json", "update-resume.json"] {
        match std::fs::read(data.join(name)) {
            Ok(bytes) => {
                return serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| format!("无法读取已保存的连接设置，请重新选择：{e}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("无法读取连接设置：{e}")),
        }
    }
    Ok(None)
}
fn save_preferences(data: &Path, settings: &Preferences) -> Result<(), String> {
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    std::fs::write(
        data.join("connection.json"),
        serde_json::to_vec(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("无法保存连接设置：{e}"))?;
    let _ = std::fs::remove_file(data.join("update-resume.json"));
    Ok(())
}

#[tauri::command]
pub async fn connection_settings(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<Preferences>, String> {
    if window.label() != "launcher" {
        return Err("只能从连接设置窗口读取连接设置。".into());
    }
    let result = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())
        .and_then(|data| read_preferences(&data));
    if !matches!(&result, Ok(Some(_))) {
        window.show().map_err(|e| e.to_string())?;
    }
    result
}

fn spawn_server(executable: &Path, data: &Path, bind: &str) -> Result<Child, String> {
    // Never mistake a service already listening on this port for our child.
    let probe = std::net::TcpListener::bind(bind)
        .map_err(|e| format!("本地服务端口 {bind} 不可用：{e}"))?;
    drop(probe);
    std::fs::create_dir_all(data).map_err(|e| format!("无法创建本地数据目录：{e}"))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join("server.log"))
        .map_err(|e| format!("无法打开服务日志：{e}"))?;
    let mut command = Command::new(executable);
    command
        .args(["--desktop-control", "--bind", bind, "--data-dir"])
        .arg(data)
        .current_dir(data)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
        .spawn()
        .map_err(|e| format!("无法启动本地服务：{e}"))
}

async fn wait_ready(runtime: &Runtime, data: &Path, target: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        {
            let mut guard = runtime.child.lock().unwrap();
            let child = guard.as_mut().ok_or("本地服务已停止。")?;
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "本地服务启动失败（{status}）。请查看 {}",
                    data.join("server.log").display()
                ));
            }
        }
        if let Ok(response) = client.get(format!("{target}/api/status")).send().await {
            if response.status().is_success() {
                if let Ok(status) = response.json::<serde_json::Value>().await {
                    let identity =
                        std::fs::read_to_string(data.join("instance-id")).unwrap_or_default();
                    if !identity.trim().is_empty()
                        && status["ok"] == true
                        && status["instance_id"].as_str() == Some(identity.trim())
                    {
                        return Ok(());
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "本地服务未能在 30 秒内就绪，请查看 {}",
                data.join("server.log").display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tauri::command]
pub async fn open_workspace(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    server_url: String,
) -> Result<(), String> {
    if window.label() != "launcher" {
        return Err("只能从连接设置窗口打开工作空间。".into());
    }
    let result = open_workspace_inner(&app, &window, server_url).await;
    if result.is_err() {
        let _ = window.show();
        let _ = window.set_focus();
    }
    result
}
async fn open_workspace_inner(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    server_url: String,
) -> Result<(), String> {
    let runtime = app.state::<Runtime>();
    let _opening = runtime.opening.lock().await;
    let target = server_url.trim();
    let local = target.is_empty();
    let mut url = url::Url::parse(if local { LOCAL_URL } else { target })
        .map_err(|_| "服务地址格式不正确。".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("服务地址必须是 HTTP 或 HTTPS 地址。".into());
    }
    let mut restarted = false;
    if local {
        let data = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
        {
            let mut guard = runtime.child.lock().unwrap();
            let running = match guard.as_mut() {
                Some(child) => child.try_wait().map_err(|e| e.to_string())?.is_none(),
                None => false,
            };
            if running && runtime.draining.load(Ordering::Acquire) {
                return Err("本地服务仍在等待任务结束，请稍后再打开工作空间。".into());
            }
            if !running {
                runtime.draining.store(false, Ordering::Release);
                restarted = true;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let sidecar =
                    exe.with_file_name(format!("ferry-server{}", std::env::consts::EXE_SUFFIX));
                *guard = Some(spawn_server(&sidecar, &data, "127.0.0.1:4290")?);
            }
        }
        if let Err(error) = wait_ready(&runtime, &data, LOCAL_URL).await {
            runtime.stop();
            return Err(error);
        }
        // Only the owned, identity-checked sidecar may supply setup proof.
        // A fragment is never sent in HTTP requests or server access logs.
        match std::fs::read_to_string(data.join("setup-token")) {
            Ok(token) if !token.trim().is_empty() => {
                url.set_fragment(Some(&format!("setup={}", token.trim())))
            }
            Ok(_) => return Err("初始化凭据为空，请重启本地服务。".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法读取初始化凭据：{error}")),
        }
    }
    if let Some(workspace) = app.get_webview_window("workspace") {
        if !local
            || url.fragment().is_some()
            || workspace.url().map_err(|e| e.to_string())?.origin() != url.origin()
        {
            workspace.navigate(url).map_err(|e| e.to_string())?;
        } else if restarted {
            workspace.reload().map_err(|e| e.to_string())?;
        }
        workspace.show().map_err(|e| e.to_string())?;
        workspace.unminimize().map_err(|e| e.to_string())?;
        workspace.set_focus().map_err(|e| e.to_string())?;
        save_preferences(
            &app.path().app_local_data_dir().map_err(|e| e.to_string())?,
            &Preferences {
                server_url: target.to_owned(),
            },
        )?;
        window.hide().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let workspace =
        tauri::WebviewWindowBuilder::new(app, "workspace", tauri::WebviewUrl::External(url))
            .title("Ferry")
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|e| e.to_string())?;
    let handle = app.clone();
    workspace.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
            if let Some(launcher) = handle.get_webview_window("launcher") {
                let _ = launcher.show();
                let _ = launcher.set_focus();
            }
        }
    });
    save_preferences(
        &app.path().app_local_data_dir().map_err(|e| e.to_string())?,
        &Preferences {
            server_url: target.to_owned(),
        },
    )?;
    window.hide().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_selection_survives_reload_and_migrates_update_resume() {
        let data = std::env::temp_dir().join(format!("desktop-preferences-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        assert!(read_preferences(&data).unwrap().is_none());
        std::fs::write(
            data.join("update-resume.json"),
            br#"{"server_url":"https://old.example.invalid"}"#,
        )
        .unwrap();
        assert_eq!(
            read_preferences(&data).unwrap().unwrap().server_url,
            "https://old.example.invalid"
        );
        for target in ["", "https://remote.example.invalid/base", ""] {
            save_preferences(
                &data,
                &Preferences {
                    server_url: target.into(),
                },
            )
            .unwrap();
            assert!(!data.join("update-resume.json").exists());
            assert_eq!(read_preferences(&data).unwrap().unwrap().server_url, target);
        }
        std::fs::write(data.join("connection.json"), "broken").unwrap();
        assert!(read_preferences(&data).is_err());
        save_preferences(&data, &Preferences::default()).unwrap();
        assert_eq!(read_preferences(&data).unwrap().unwrap().server_url, "");
        std::fs::remove_dir_all(data).unwrap();
    }

    // Run against the real built server, including migrations and restart persistence.
    #[tokio::test]
    #[ignore = "requires DESKTOP_TEST_SERVER pointing to a built server"]
    async fn bundled_service_lifecycle() {
        let executable = std::env::var_os("DESKTOP_TEST_SERVER").expect("DESKTOP_TEST_SERVER");
        let data = std::env::temp_dir().join(format!("desktop-lifecycle-{}", std::process::id()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let bind = listener.local_addr().unwrap().to_string();
        drop(listener);
        let target = format!("http://{bind}");
        let runtime = Runtime::default();
        *runtime.child.lock().unwrap() =
            Some(spawn_server(Path::new(&executable), &data, &bind).unwrap());
        wait_ready(&runtime, &data, &target).await.unwrap();
        let identity = std::fs::read_to_string(data.join("instance-id")).unwrap();
        assert!(data.join("ferry.sqlite").exists());
        assert!(data.join("master.key").exists());
        let token = std::fs::read_to_string(data.join("setup-token")).unwrap();
        let client = reqwest::Client::new();
        let initialized = client.post(format!("{target}/api/setup"))
            .header("X-Ferry-Request", "1")
            .json(&serde_json::json!({"token": token, "email": "desktop@example.invalid", "name": "Desktop admin", "password": "temporary-desktop-password"}))
            .send().await.unwrap();
        assert!(initialized.status().is_success());
        assert!(initialized.headers().contains_key("set-cookie"));
        assert!(!data.join("setup-token").exists());
        // A second process cannot silently attach to the already running service.
        assert!(
            spawn_server(Path::new(&executable), &data, &bind)
                .unwrap_err()
                .contains("不可用")
        );
        runtime.drain().await.unwrap();
        assert!(std::net::TcpStream::connect(&bind).is_err());
        *runtime.child.lock().unwrap() =
            Some(spawn_server(Path::new(&executable), &data, &bind).unwrap());
        wait_ready(&runtime, &data, &target).await.unwrap();
        assert_eq!(
            identity,
            std::fs::read_to_string(data.join("instance-id")).unwrap()
        );
        assert!(!data.join("setup-token").exists());
        let setup: serde_json::Value = client
            .get(format!("{target}/api/setup"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(setup["required"], false);
        runtime.drain().await.unwrap();
        assert!(
            std::fs::read_to_string(data.join("server.log"))
                .unwrap()
                .contains("服务已安全关闭")
        );
        std::fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn missing_sidecar_is_actionable() {
        let data = std::env::temp_dir().join(format!("desktop-missing-{}", std::process::id()));
        assert!(
            spawn_server(Path::new("/missing/server"), &data, "127.0.0.1:0")
                .unwrap_err()
                .contains("无法启动本地服务")
        );
        std::fs::remove_dir_all(data).unwrap();
    }
}
