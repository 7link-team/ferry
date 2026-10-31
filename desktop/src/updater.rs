//! Signed desktop updates. Only the local launcher may check/install them.
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState {
    pending: tokio::sync::Mutex<Option<Update>>,
    busy: AtomicBool,
    installing: AtomicBool,
}
struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl UpdateState {
    pub fn installing(&self) -> bool {
        self.installing.load(Ordering::Acquire)
    }
    fn begin(&self) -> Result<Busy<'_>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "正在检查或安装更新，请稍候。".to_string())?;
        Ok(Busy(&self.busy))
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Channel {
    repository: String,
    #[serde(default)]
    include_prereleases: bool,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Serialize)]
pub struct UpdateInfo {
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
}
fn launcher(label: &str) -> Result<(), String> {
    if label != "launcher" {
        return Err("只能从本机连接设置窗口管理应用更新。".into());
    }
    Ok(())
}

fn channel(app: &tauri::AppHandle) -> Result<Channel, String> {
    let channel: Channel = serde_json::from_value(
        app.config()
            .plugins
            .0
            .get("release")
            .cloned()
            .ok_or("未配置更新来源。")?,
    )
    .map_err(|_| "更新来源配置无效。")?;
    let parts: Vec<_> = channel.repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err("更新仓库配置无效。".into());
    }
    Ok(channel)
}

#[tauri::command]
pub fn release_page(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<String, String> {
    launcher(window.label())?;
    Ok(format!(
        "https://github.com/{}/releases",
        channel(&app)?.repository
    ))
}

#[tauri::command]
pub fn open_release_page(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let url = release_page(app, window)?;
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("rundll32.exe");
        command
            .creation_flags(0x08000000)
            .arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .spawn()
        .map_err(|_| "无法打开浏览器，请复制发布页地址。".to_string())?;
    Ok(())
}
fn select_release<'a>(
    releases: &'a [Release],
    current: &Version,
    prereleases: bool,
) -> Option<(&'a Asset, Version)> {
    releases
        .iter()
        .filter(|r| !r.draft && (prereleases || !r.prerelease))
        .filter_map(|r| {
            let version = Version::parse(r.tag_name.strip_prefix('v')?).ok()?;
            if version <= *current || (!prereleases && !version.pre.is_empty()) {
                return None;
            }
            let asset = r.assets.iter().find(|a| a.name == "latest.json")?;
            Some((asset, version))
        })
        .max_by(|a, b| a.1.cmp(&b.1))
}
#[tauri::command]
pub async fn check_update(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    launcher(window.label())?;
    let _busy = state.begin()?;
    *state.pending.lock().await = None;
    let current = app.package_info().version.clone();
    let channel = channel(&app)?;
    let releases: Vec<Release> = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            channel.repository
        ))
        .header("User-Agent", format!("desktop-updater/{current}"))
        .send()
        .await
        .map_err(|_| "无法连接更新服务，请检查网络。")?
        .error_for_status()
        .map_err(|e| format!("更新服务返回错误：{e}"))?
        .json()
        .await
        .map_err(|_| "更新服务返回格式无效。")?;
    let Some((asset, version)) = select_release(&releases, &current, channel.include_prereleases)
    else {
        return Ok(UpdateInfo {
            current_version: current.to_string(),
            version: None,
            notes: None,
        });
    };
    let endpoint = url::Url::parse(&asset.browser_download_url).map_err(|_| "更新地址无效。")?;
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some("github.com")
        || !endpoint
            .path()
            .starts_with(&format!("/{}/releases/download/", channel.repository))
    {
        return Err("更新地址与配置仓库不一致。".into());
    }
    let update = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("检查签名更新失败：{e}"))?
        .ok_or("该版本没有适用本平台的更新。")?;
    if update.version != version.to_string() {
        return Err("更新清单与版本标签不一致。".into());
    }
    let info = UpdateInfo {
        current_version: current.to_string(),
        version: Some(update.version.clone()),
        notes: update.body.clone(),
    };
    *state.pending.lock().await = Some(update);
    Ok(info)
}
#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UpdateState>,
) -> Result<(), String> {
    launcher(window.label())?;
    let _busy = state.begin()?;
    state.installing.store(true, Ordering::Release);
    let installing = Busy(&state.installing);
    let update = state.pending.lock().await.take().ok_or("请先检查更新。")?;
    let mut downloaded = 0;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = window.emit("update-progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("更新未安装：{e}。请重新检查更新后再试。"))?;
    let runtime = app.state::<crate::server::Runtime>();
    let _opening = runtime.opening.lock().await;
    let _ = window.emit("update-stage", "正在等待本地任务完成并安全关闭服务…");
    let resume = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("update-resume.json");
    if let Some(workspace) = app.get_webview_window("workspace") {
        std::fs::create_dir_all(resume.parent().ok_or("更新恢复路径无效。")?)
            .map_err(|e| e.to_string())?;
        let target = workspace
            .url()
            .map_err(|e| e.to_string())?
            .origin()
            .ascii_serialization();
        std::fs::write(
            &resume,
            serde_json::to_vec(&crate::server::Preferences {
                server_url: if target == crate::server::LOCAL_URL {
                    String::new()
                } else {
                    target
                },
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    if let Err(error) = runtime.drain().await {
        let _ = std::fs::remove_file(&resume);
        return Err(error);
    }
    let _ = window.emit("update-stage", "正在安装更新，完成后将自动启动…");
    update.install(bytes).map_err(|e| {
        let _ = std::fs::remove_file(&resume);
        format!("更新未安装：{e}。可重新打开工作空间或重试更新。")
    })?;
    // Official updater replaces only the application bundle; service data lives elsewhere.
    drop(installing);
    drop(_busy);
    app.restart();
}
#[cfg(test)]
mod tests {
    use super::*;
    fn release(v: &str, pre: bool, draft: bool, asset: bool) -> Release {
        Release {
            tag_name: v.into(),
            prerelease: pre,
            draft,
            assets: if asset {
                vec![Asset {
                    name: "latest.json".into(),
                    browser_download_url: "https://example.invalid/latest.json".into(),
                }]
            } else {
                vec![]
            },
        }
    }
    #[test]
    fn semver_prerelease_order_and_stable_channel() {
        let rs = vec![
            release("v0.1.0-alpha.2", true, false, true),
            release("v0.1.0-alpha.10", true, false, true),
            release("v0.1.0-alpha.99", true, true, true),
            release("v0.2.0", false, false, false),
        ];
        let current = Version::parse("0.1.0-alpha.1").unwrap();
        assert_eq!(
            select_release(&rs, &current, true).unwrap().1.to_string(),
            "0.1.0-alpha.10"
        );
        assert!(select_release(&rs, &current, false).is_none());
        assert!(select_release(&rs, &Version::parse("0.1.0").unwrap(), true).is_none());
    }
    #[test]
    fn business_window_cannot_update_and_operations_are_serial() {
        assert!(launcher("workspace").is_err());
        assert!(launcher("launcher").is_ok());
        let state = UpdateState::default();
        let first = state.begin().unwrap();
        assert!(state.begin().is_err());
        drop(first);
        assert!(state.begin().is_ok());
    }
    #[tokio::test]
    #[ignore = "run scripts/test-updater.py to create temporary signed artifacts"]
    async fn signed_download_rejects_tampering_and_relabelled_versions() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            sync::{Arc, atomic::AtomicUsize},
        };
        let dir = std::path::PathBuf::from(
            std::env::var("UPDATER_TEST_DIR").expect("signed fixture directory"),
        );
        let payload = std::fs::read(dir.join("payload.bin")).unwrap();
        let signature = std::fs::read_to_string(dir.join("payload.bin.sig")).unwrap();
        let key = std::fs::read_to_string(dir.join("test.key.pub")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let mode = Arc::new(AtomicUsize::new(0));
        let server_mode = mode.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let expected = payload.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                // macOS inherits the listener's nonblocking flag on accepted sockets.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 8192];
                let n = stream.read(&mut request).unwrap();
                let scenario = server_mode.load(Ordering::Acquire);
                let body = if String::from_utf8_lossy(&request[..n]).contains("GET /manifest") {
                    serde_json::to_vec(&serde_json::json!({"version":if scenario==2 {"9.9.10"}else{"9.9.9"},"platforms":{"fixture":{"url":format!("http://{address}/payload"),"signature":signature.trim()}}})).unwrap()
                } else if scenario == 1 {
                    b"tampered".to_vec()
                } else {
                    payload.clone()
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert("updater".into(),serde_json::json!({"pubkey":key.trim(),"requireSignedVersion":true,"dangerousInsecureTransportProtocol":true}));
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let mut results = Vec::new();
        for scenario in 0..3 {
            mode.store(scenario, Ordering::Release);
            let update = app
                .updater_builder()
                .target("fixture")
                .no_proxy()
                .executable_path(dir.join("Fixture.app/Contents/MacOS/Fixture"))
                .endpoints(vec![format!("http://{address}/manifest").parse().unwrap()])
                .unwrap()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap()
                .check()
                .await
                .unwrap()
                .unwrap();
            results.push(update.download(|_, _| {}, || {}).await);
        }
        stopping.store(true, Ordering::Release);
        thread.join().unwrap();
        assert_eq!(results[0].as_ref().unwrap(), &expected);
        assert!(
            results[1].is_err(),
            "modified download must fail signature verification"
        );
        assert!(
            results[2].is_err(),
            "old signed payload must not be relabelled as a newer version"
        );
    }
}
