//! Client-only connection path; no service process or desktop plugins.

fn target_url(input: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(input.trim()).map_err(|_| "invalid_url")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("invalid_url".into());
    }
    Ok(url)
}

async fn verify_server(target: &url::Url) -> Result<(), String> {
    let mut status_url = target.clone();
    status_url.set_path(&format!(
        "{}/api/status",
        target.path().trim_end_matches('/')
    ));
    status_url.set_query(None);
    status_url.set_fragment(None);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "server_unavailable")?;
    let response = client
        .get(status_url)
        .send()
        .await
        .map_err(|_| "server_unavailable")?;
    if !response.status().is_success() {
        return Err("invalid_server".into());
    }
    let status: serde_json::Value = response.json().await.map_err(|_| "invalid_server")?;
    if status["ok"] == true {
        Ok(())
    } else {
        Err("invalid_server".into())
    }
}

fn local_document(actual: &url::Url, bundled: &url::Url) -> bool {
    // Custom-protocol root URLs retain an empty path in url::Url, while
    // WebKit may expose that same root with a slash. Other paths stay exact.
    let same_path = actual.path() == bundled.path()
        || matches!((actual.path(), bundled.path()), ("", "/") | ("/", ""));
    actual.scheme() == bundled.scheme()
        && actual.host_str() == bundled.host_str()
        && actual.port_or_known_default() == bundled.port_or_known_default()
        && actual.username().is_empty()
        && actual.password().is_none()
        && same_path
        && actual.query().is_none()
}

fn capture_bundled_document(
    bundled: &mut Option<url::Url>,
    creation: &url::Url,
    candidate: Option<url::Url>,
) -> bool {
    if bundled.is_none()
        && candidate
            .as_ref()
            .is_some_and(|url| local_document(url, creation))
    {
        *bundled = candidate;
        return true;
    }
    false
}

#[derive(Default)]
struct NavigationReadiness {
    generation: u64,
    previous_document: Option<String>,
    last_document: Option<String>,
    armed: bool,
    acknowledged: bool,
    finished: bool,
}

impl NavigationReadiness {
    fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.previous_document = self.last_document.clone();
        self.armed = true;
        self.acknowledged = false;
        self.finished = false;
    }

    fn finish(&mut self, generation: u64) {
        if self.generation == generation {
            self.finished = true;
        }
    }

    fn observe(&mut self, generation: u64, document: &str) {
        if self.armed
            && self.generation == generation
            && !document.is_empty()
            && self.previous_document.as_deref() != Some(document)
        {
            self.last_document = Some(document.to_owned());
        }
    }

    fn acknowledge(&mut self, generation: u64, document: &str) -> bool {
        if !self.armed
            || self.generation != generation
            || self.previous_document.as_deref() == Some(document)
            || document.is_empty()
        {
            return false;
        }
        self.last_document = Some(document.to_owned());
        // Remember the actual top document before load completion, so a rapid
        // reload snapshots its nonce. A UI report from an old provisional
        // context must not finish the new navigation.
        if !self.finished {
            return false;
        }
        self.acknowledged = true;
        self.armed = false;
        true
    }

    fn ready(&self, generation: u64) -> bool {
        self.generation == generation && self.acknowledged
    }
}

#[cfg(mobile)]
mod native {
    use super::*;
    use crate::connection::{Preferences, read_preferences, save_preferences};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    use tauri::Manager;

    struct Connection {
        creation: url::Url,
        bundled: Mutex<Option<url::Url>>,
        remote: Mutex<Option<url::Url>>,
        generation: AtomicU64,
        readiness: Mutex<NavigationReadiness>,
        restore_once: AtomicBool,
        opening: tokio::sync::Mutex<()>,
    }

    fn creation_url(config: &tauri::Config) -> Result<url::Url, String> {
        use tauri::utils::config::{FrontendDist, WebviewUrl};
        if config.build.dev_url.is_some()
            || matches!(
                config.build.frontend_dist.as_ref(),
                Some(FrontendDist::Url(_))
            )
        {
            return Err("connection launcher must use bundled assets".into());
        }
        let window = config
            .app
            .windows
            .iter()
            .find(|window| window.label == "launcher")
            .ok_or("connection launcher missing")?;
        let WebviewUrl::App(path) = &window.url else {
            return Err("connection launcher must use an App URL".into());
        };
        // Pinned Tauri 2.12.5 protocol::origin + manager/webview.rs App URL
        // resolution. These establish creation provenance, not a server default.
        let origin = if cfg!(target_os = "android") {
            if window.use_https_scheme {
                "https://tauri.localhost/"
            } else {
                "http://tauri.localhost/"
            }
        } else {
            "tauri://localhost"
        };
        let base = url::Url::parse(origin).unwrap();
        if path.to_str() == Some("index.html") {
            Ok(base)
        } else {
            base.join(&path.to_string_lossy())
                .map_err(|error| error.to_string())
        }
    }

    fn require_settings(window: &tauri::WebviewWindow) -> Result<(), String> {
        let state = window.state::<Connection>();
        let bundled = state.bundled.lock().unwrap().clone();
        if window.label() != "launcher"
            || !bundled.as_ref().is_some_and(|local| {
                window
                    .url()
                    .is_ok_and(|actual| local_document(&actual, local))
            })
        {
            return Err("settings_unavailable".into());
        }
        Ok(())
    }

    fn return_to_settings(window: &tauri::WebviewWindow, failed: bool) {
        let state = window.state::<Connection>();
        state.generation.fetch_add(1, Ordering::SeqCst);
        let bundled = state.bundled.lock().unwrap().clone();
        if let Some(mut url) = bundled {
            url.set_fragment(Some(if failed {
                "connection-settings/navigation-failed"
            } else {
                "connection-settings"
            }));
            let _ = window.navigate(url);
        }
    }

    fn request_readiness(window: &tauri::WebviewWindow, generation: u64) {
        let state = window.state::<Connection>();
        let bundled = state
            .bundled
            .lock()
            .unwrap()
            .clone()
            .map(|url| url.to_string());
        let local_url = serde_json::to_string(&bundled).unwrap();
        let finished_hint = state.readiness.lock().unwrap().finished;
        // Native reads the main document directly. No remote IPC or URL navigation
        // is involved in readiness, and iframe documents cannot supply a nonce.
        let script = r#"(() => {
            if (window.top !== window) return null;
            const nonce = window.__CLIENT_DOCUMENT_ID__;
            if (typeof nonce !== 'string') return null;
            const bundled = __BUNDLED__;
            const actual = new URL(location.href);
            const local = bundled && (() => {
                const expected = new URL(bundled);
                return actual.protocol === expected.protocol && actual.host === expected.host
                    && actual.pathname === expected.pathname && !actual.search;
            })();
            const selector = local ? '#open' : '[data-client-connection-entry], #native-connection-settings';
            const height = window.visualViewport?.height || window.innerHeight;
            const reachable = Array.from(document.querySelectorAll(selector)).some((element) => {
                const box = element.getBoundingClientRect();
                const style = getComputedStyle(element);
                return !element.disabled && !element.closest('[inert]') && style.visibility !== 'hidden'
                    && style.display !== 'none' && box.width > 0 && box.height > 0
                    && box.bottom > 0 && box.top < height && box.right > 0 && box.left < innerWidth;
            });
            return { document: nonce, reachable };
        })();"#.replace("__BUNDLED__", &local_url);
        let handle = window.clone();
        let _ = window.eval_with_callback(script, move |json| {
            #[derive(serde::Deserialize)]
            struct Report {
                document: String,
                reachable: bool,
            }
            let Ok(report) = serde_json::from_str::<Report>(&json) else {
                return;
            };
            let state = handle.state::<Connection>();
            if state.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let mut readiness = state.readiness.lock().unwrap();
            if finished_hint && report.reachable {
                readiness.acknowledge(generation, &report.document);
            } else {
                readiness.observe(generation, &report.document);
            }
        });
    }

    #[derive(serde::Serialize)]
    struct LaunchSettings {
        #[serde(flatten)]
        preferences: Preferences,
        auto_connect: bool,
    }

    #[tauri::command]
    async fn connection_settings(
        window: tauri::WebviewWindow,
    ) -> Result<Option<LaunchSettings>, String> {
        require_settings(&window)?;
        let auto_connect = window
            .state::<Connection>()
            .restore_once
            .swap(false, Ordering::AcqRel);
        let data = window
            .app_handle()
            .path()
            .app_local_data_dir()
            .map_err(|_| "settings_unavailable")?;
        read_preferences(&data)
            .map(|settings| {
                settings.map(|preferences| LaunchSettings {
                    preferences,
                    auto_connect,
                })
            })
            .map_err(|_| "settings_unavailable".into())
    }

    #[tauri::command]
    async fn open_workspace(
        window: tauri::WebviewWindow,
        server_url: String,
    ) -> Result<(), String> {
        require_settings(&window)?;
        let state = window.state::<Connection>();
        let _opening = state.opening.lock().await;
        let url = target_url(&server_url)?;
        if state
            .bundled
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|local| local.origin() == url.origin())
        {
            return Err("invalid_url".into());
        }
        verify_server(&url).await?;
        // The await above may outlive a settings navigation. Recheck the local document.
        require_settings(&window)?;
        let data = window
            .app_handle()
            .path()
            .app_local_data_dir()
            .map_err(|_| "settings_unavailable")?;
        save_preferences(
            &data,
            &Preferences {
                server_url: url.to_string(),
            },
        )
        .map_err(|_| "settings_unavailable")?;
        *state.remote.lock().unwrap() = Some(url.clone());
        window.navigate(url).map_err(|_| "navigation_failed".into())
    }

    pub fn run() {
        #[allow(unused_mut)]
        let mut context = tauri::generate_context!();
        let creation =
            creation_url(context.config()).expect("bundled connection launcher configuration");
        #[cfg(target_os = "ios")]
        {
            let mut failure = creation.clone();
            failure.set_fragment(Some("connection-settings/navigation-failed"));
            context.config_mut().plugins.0.insert(
                "mobile-connection".into(),
                serde_json::json!({ "failureURL": failure.as_str() }),
            );
        }
        tauri::Builder::default()
            .manage(Connection {
                creation,
                bundled: Mutex::default(),
                remote: Mutex::default(),
                generation: AtomicU64::new(0),
                readiness: Mutex::default(),
                restore_once: AtomicBool::new(true),
                opening: tokio::sync::Mutex::default(),
            })
            .plugin(
                tauri::plugin::Builder::<tauri::Wry, serde_json::Value>::new("mobile-connection")
                    .setup(|_app, _api| {
                        #[cfg(target_os = "ios")]
                        {
                            tauri::ios_plugin_binding!(init_plugin_client_navigation);
                            _api.register_ios_plugin(init_plugin_client_navigation)?;
                        }
                        Ok(())
                    })
                    // This adds a plain navigation control, never an IPC bridge or ACL grant.
                    .js_init_script(include_str!("../ui/mobile-navigation.js"))
                    .on_navigation(|webview, target| {
                        let Some(window) = webview.app_handle().get_webview_window("launcher")
                        else {
                            return false;
                        };
                        let state = window.state::<Connection>();
                        if target.scheme() == "app-settings" {
                            if target.host_str() == Some("connection") {
                                return_to_settings(&window, false);
                            }
                            return false;
                        }
                        // The configured bundled page can begin loading before setup captures its URL.
                        let bundled = state.bundled.lock().unwrap().clone();
                        if bundled.is_none() {
                            return local_document(target, &state.creation);
                        }
                        if bundled
                            .as_ref()
                            .is_some_and(|local| local_document(target, local))
                        {
                            if state.remote.lock().unwrap().is_some()
                                && !target
                                    .fragment()
                                    .is_some_and(|hash| hash.starts_with("connection-settings"))
                            {
                                return_to_settings(&window, false);
                                return false;
                            }
                            return true;
                        }
                        matches!(target.scheme(), "http" | "https")
                    })
                    .on_page_load(|webview, payload| {
                        let Some(window) = webview.app_handle().get_webview_window("launcher")
                        else {
                            return;
                        };
                        let state = window.state::<Connection>();
                        match payload.event() {
                            tauri::webview::PageLoadEvent::Started => {
                                capture_bundled_document(
                                    &mut state.bundled.lock().unwrap(),
                                    &state.creation,
                                    Some(payload.url().clone()),
                                );
                                // These native lifecycle callbacks describe the main document. The
                                // navigation policy hook also receives child-frame URLs on Android.
                                let generation =
                                    state.generation.fetch_add(1, Ordering::SeqCst) + 1;
                                state.readiness.lock().unwrap().begin(generation);
                                let bundled = state.bundled.lock().unwrap().clone();
                                if bundled
                                    .as_ref()
                                    .is_some_and(|local| local_document(payload.url(), local))
                                {
                                    return;
                                }
                                let remote = state.remote.lock().unwrap().clone();
                                let Some(remote) = remote else {
                                    return;
                                };
                                let probe_window = window.clone();
                                tauri::async_runtime::spawn(async move {
                                    if verify_server(&remote).await.is_err()
                                        && probe_window
                                            .state::<Connection>()
                                            .generation
                                            .load(Ordering::SeqCst)
                                            == generation
                                    {
                                        return_to_settings(&probe_window, true);
                                    }
                                });
                                let window = window.clone();
                                tauri::async_runtime::spawn(async move {
                                    // Match the existing Desktop service readiness window. Finished
                                    // alone never proves the page has a reachable settings control.
                                    let deadline = tokio::time::Instant::now()
                                        + std::time::Duration::from_secs(30);
                                    loop {
                                        let state = window.state::<Connection>();
                                        if state.generation.load(Ordering::SeqCst) != generation
                                            || state.readiness.lock().unwrap().ready(generation)
                                        {
                                            return;
                                        }
                                        if tokio::time::Instant::now() >= deadline {
                                            break;
                                        }
                                        request_readiness(&window, generation);
                                        tokio::time::sleep(std::time::Duration::from_millis(500))
                                            .await;
                                    }
                                    let state = window.state::<Connection>();
                                    request_readiness(&window, generation);
                                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                    if state.generation.load(Ordering::SeqCst) == generation
                                        && !state.readiness.lock().unwrap().ready(generation)
                                    {
                                        return_to_settings(&window, true);
                                    }
                                });
                            }
                            tauri::webview::PageLoadEvent::Finished => {
                                let generation = state.generation.load(Ordering::SeqCst);
                                state.readiness.lock().unwrap().finish(generation);
                                request_readiness(&window, state.generation.load(Ordering::SeqCst));
                            }
                        }
                    })
                    .build(),
            )
            .setup(|app| {
                let window = app
                    .get_webview_window("launcher")
                    .ok_or("launcher missing")?;
                // Use the platform's actual bundled URL, not a guessed custom-protocol origin.
                // Android's getter is empty until its first main-document load.
                // Keep native preference commands closed until the real URL is
                // observed and matches the trusted App creation intent.
                let state = app.state::<Connection>();
                let candidate = window.url().ok();
                capture_bundled_document(
                    &mut state.bundled.lock().unwrap(),
                    &state.creation,
                    candidate,
                );
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                connection_settings,
                open_workspace
            ])
            .run(context)
            .expect("run client shell");
    }
}

#[cfg(mobile)]
pub use native::run;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn ios_custom_protocol_empty_root_matches_the_bundled_root_only() {
        let configured = url::Url::parse("tauri://localhost/").unwrap();
        let native = url::Url::parse("tauri://localhost").unwrap();
        assert!(local_document(&native, &configured));
        assert!(local_document(&configured, &native));
        assert!(capture_bundled_document(&mut None, &configured, Some(native)));
        for rejected in [
            "tauri://localhost/untrusted.html", "tauri://localhost/?query=1",
            "tauri://other/", "https://localhost/", "tauri://user@localhost/",
        ] {
            assert!(!local_document(&url::Url::parse(rejected).unwrap(), &configured));
        }
    }

    #[test]
    fn deferred_bootstrap_capture_never_trusts_blank_or_remote_navigation() {
        for expected in ["http://tauri.localhost/", "tauri://localhost/"] {
            let creation = url::Url::parse(expected).unwrap();
            let mut bundled = None;
            assert!(!capture_bundled_document(&mut bundled, &creation, None));
            assert!(!capture_bundled_document(
                &mut bundled,
                &creation,
                Some(url::Url::parse("https://untrusted.example.invalid/").unwrap())
            ));
            assert!(bundled.is_none());
            assert!(!capture_bundled_document(
                &mut bundled,
                &creation,
                Some(url::Url::parse("about:blank").unwrap())
            ));
            assert!(capture_bundled_document(
                &mut bundled,
                &creation,
                Some(creation.clone())
            ));
            assert_eq!(bundled.as_ref().unwrap().as_str(), expected);
            assert!(!capture_bundled_document(
                &mut bundled,
                &creation,
                Some(url::Url::parse("https://untrusted.example.invalid/").unwrap())
            ));
            assert_eq!(bundled.as_ref().unwrap().as_str(), expected);
        }
    }

    #[test]
    fn readiness_rejects_stale_generation_and_old_document_during_same_url_reload() {
        let mut readiness = NavigationReadiness::default();
        readiness.begin(1);
        assert!(!readiness.acknowledge(1, "local-document"));
        readiness.finish(1);
        assert!(readiness.acknowledge(1, "local-document"));
        readiness.begin(2);
        assert!(!readiness.acknowledge(1, "local-document"));
        assert!(!readiness.acknowledge(2, "local-document"));
        assert!(!readiness.ready(2));
        readiness.observe(2, "remote-document-one");
        assert!(!readiness.ready(2));
        readiness.finish(2);
        assert!(!readiness.ready(2));
        assert!(readiness.acknowledge(2, "remote-document-one"));
        assert!(readiness.ready(2));
        readiness.begin(3);
        readiness.finish(3);
        assert!(!readiness.acknowledge(2, "remote-document-one"));
        assert!(!readiness.acknowledge(3, "remote-document-one"));
        assert!(!readiness.ready(3));
        assert!(readiness.acknowledge(3, "remote-document-two"));
        assert!(!readiness.acknowledge(3, "remote-document-two"));
        assert!(readiness.ready(3));
    }

    #[test]
    fn only_bundled_settings_document_can_read_or_write_preferences_on_each_platform() {
        for local in ["http://tauri.localhost/", "tauri://localhost/"] {
            let bundled = url::Url::parse(local).unwrap();
            assert!(local_document(&bundled, &bundled));
            assert!(local_document(
                &bundled.join("#connection-settings").unwrap(),
                &bundled
            ));
            assert!(!local_document(
                &url::Url::parse("https://server.example.invalid/").unwrap(),
                &bundled
            ));
            assert!(!local_document(
                &bundled.join("untrusted.html").unwrap(),
                &bundled
            ));
            assert!(!local_document(
                &bundled.join("?untrusted=1").unwrap(),
                &bundled
            ));
        }
    }

    #[test]
    fn mobile_requires_http_server_without_embedded_credentials() {
        for input in [
            "",
            " ",
            "file:///tmp/page",
            "https://user:secret@example.invalid",
        ] {
            assert!(target_url(input).is_err(), "accepted {input}");
        }
        assert_eq!(
            target_url(" https://example.invalid/base/ ")
                .unwrap()
                .as_str(),
            "https://example.invalid/base/"
        );
    }

    async fn probe(body: &'static str) -> Result<(), String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let len = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..len]).starts_with("GET /base/api/status "));
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let result = verify_server(&target_url(&format!("http://{address}/base")).unwrap()).await;
        thread.join().unwrap();
        result
    }

    #[tokio::test]
    async fn connection_requires_real_api_status_not_just_loaded_html() {
        assert!(probe(r#"{"ok":true}"#).await.is_ok());
        assert!(probe(r#"{"ok":false}"#).await.is_err());
        assert!(probe("<html>not the service</html>").await.is_err());
    }

    #[tokio::test]
    async fn offline_server_returns_editable_connection_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        assert_eq!(
            verify_server(&target_url(&format!("http://{address}")).unwrap()).await,
            Err("server_unavailable".into())
        );
    }
}
