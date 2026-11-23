//! Mobile release selection and streaming verification. Native installation is
//! exposed only to the existing trusted bundled connection document.
use crate::release::{Asset, Channel, Release};
use semver::Version;
use sha2::{Digest, Sha256};

fn asset_url(channel: &Channel, release: &Release, asset: &Asset) -> bool {
    let Ok(url) = url::Url::parse(&asset.browser_download_url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path()
            == format!(
                "/{}/releases/download/{}/{}",
                channel.repository, release.tag_name, asset.name
            )
}

fn select<'a>(
    releases: &'a [Release],
    channel: &Channel,
    current: &Version,
    platform: &str,
    abi: &str,
) -> Option<(&'a Release, Option<&'a Asset>, Version)> {
    releases
        .iter()
        .filter(|r| !r.draft && (channel.include_prereleases || !r.prerelease))
        .filter_map(|r| {
            let version = Version::parse(r.tag_name.strip_prefix('v')?).ok()?;
            if version <= *current || (!channel.include_prereleases && !version.pre.is_empty()) {
                return None;
            }
            let asset = if platform == "android" {
                let name = format!(
                    "{}-v{}-android-{}-debug.apk",
                    channel.mobile.package_name, version, abi
                );
                let asset = r.assets.iter().find(|a| a.name == name)?;
                if asset.size == 0 || !asset_url(channel, r, asset) || digest(asset).is_none() {
                    return None;
                }
                Some(asset)
            } else {
                None
            };
            Some((r, asset, version))
        })
        .max_by(|a, b| a.2.cmp(&b.2))
}

fn digest(asset: &Asset) -> Option<&str> {
    let value = asset.digest.as_deref()?.strip_prefix("sha256:")?;
    (value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())).then_some(value)
}

fn distribution_url(input: Option<&str>) -> Option<String> {
    let url = url::Url::parse(input?).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn verified(size: u64, hasher: Sha256, asset: &Asset) -> bool {
    size == asset.size
        && digest(asset).is_some_and(|expected| {
            format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected)
        })
}

async fn download(
    client: reqwest::Client,
    asset: &Asset,
    path: &std::path::Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;
    let result = async {
        let mut response = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|_| "update_unavailable")?
            .error_for_status()
            .map_err(|_| "update_unavailable")?;
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|_| "install_failed")?;
        let mut hash = Sha256::new();
        let mut downloaded = 0u64;
        let mut progress_at = Instant::now();
        while let Some(chunk) = response.chunk().await.map_err(|_| "update_unavailable")? {
            downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or("update_invalid")?;
            if downloaded > asset.size {
                return Err("update_invalid");
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|_| "install_failed")?;
            if progress_at.elapsed() >= Duration::from_millis(100) || downloaded == asset.size {
                progress(downloaded, asset.size);
                progress_at = Instant::now();
            }
        }
        file.flush().await.map_err(|_| "install_failed")?;
        if !verified(downloaded, hash, asset) {
            return Err("update_invalid");
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(path).await;
        return Err(error.into());
    }
    Ok(())
}

#[cfg(mobile)]
pub(crate) mod native {
    use super::*;
    use serde::Serialize;
    use std::time::Duration;
    use tauri::{Emitter, Manager};
    pub struct Native(pub tauri::plugin::PluginHandle<tauri::Wry>);
    #[derive(Default)]
    pub struct State(pub tokio::sync::Mutex<Option<Pending>>);
    pub struct Pending {
        asset: Option<Asset>,
        distribution: Option<String>,
    }
    #[derive(Serialize)]
    pub struct Info {
        version: Option<String>,
        notes: Option<String>,
        installable: bool,
        platform: &'static str,
        installation_configured: bool,
    }
    #[derive(Clone, Serialize)]
    struct Progress {
        downloaded: u64,
        total: u64,
    }

    fn client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(30))
            .user_agent("native-client-update")
            .build()
            .map_err(|_| "update_unavailable".into())
    }
    #[tauri::command]
    pub async fn mobile_check_update(
        window: tauri::WebviewWindow,
        state: tauri::State<'_, State>,
    ) -> Result<Info, String> {
        crate::mobile::native::require_settings(&window)?;
        let mut pending = state.0.try_lock().map_err(|_| "update_busy")?;
        *pending = None;
        let app = window.app_handle();
        let channel = crate::release::channel(app).map_err(|_| "update_unavailable")?;
        let current = &app.package_info().version;
        let mut releases = Vec::new();
        // Follow complete pages, rather than reporting up-to-date from a partial list.
        for page in 1.. {
            let batch: Vec<Release> = client()?
                .get(format!(
                    "https://api.github.com/repos/{}/releases?per_page=100&page={page}",
                    channel.repository
                ))
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| "update_unavailable")?
                .error_for_status()
                .map_err(|_| "update_unavailable")?
                .json()
                .await
                .map_err(|_| "update_unavailable")?;
            let finished = batch.len() < 100;
            releases.extend(batch);
            if finished {
                break;
            }
        }
        let platform = if cfg!(target_os = "android") {
            "android"
        } else {
            "ios"
        };
        let abi = match std::env::consts::ARCH {
            "aarch64" => "arm64-v8a",
            "x86_64" => "x86_64",
            _ => return Err("update_unavailable".into()),
        };
        let distribution = distribution_url(channel.mobile.ios_distribution_url.as_deref());
        let installation_configured = if platform == "android" {
            option_env!("CLIENT_PERSISTENT_ANDROID_SIGNING") == Some("true")
        } else {
            distribution.is_some()
        };
        let Some((release, asset, version)) = select(&releases, &channel, current, platform, abi)
        else {
            if releases.iter().any(|release| {
                !release.draft
                    && (channel.include_prereleases || !release.prerelease)
                    && release
                        .tag_name
                        .strip_prefix('v')
                        .and_then(|value| Version::parse(value).ok())
                        .is_some_and(|version| {
                            version > *current
                                && (channel.include_prereleases || version.pre.is_empty())
                        })
            }) {
                return Err("update_unavailable".into());
            }
            return Ok(Info {
                version: None,
                notes: None,
                installable: false,
                platform,
                installation_configured,
            });
        };
        let installable = platform == "android" || distribution.is_some();
        let info = Info {
            version: Some(version.to_string()),
            notes: release.body.clone(),
            installable,
            platform,
            installation_configured,
        };
        let asset = asset.map(|a| Asset {
            name: a.name.clone(),
            browser_download_url: a.browser_download_url.clone(),
            digest: a.digest.clone(),
            size: a.size,
        });
        crate::mobile::native::require_settings(&window)?;
        *pending = Some(Pending {
            asset,
            distribution,
        });
        Ok(info)
    }

    #[tauri::command]
    pub async fn mobile_install_update(
        window: tauri::WebviewWindow,
        state: tauri::State<'_, State>,
    ) -> Result<serde_json::Value, String> {
        crate::mobile::native::require_settings(&window)?;
        let pending = state.0.try_lock().map_err(|_| "update_busy")?;
        let update = pending.as_ref().ok_or("update_missing")?;
        let app = window.app_handle();
        let native = app.state::<Native>();
        #[cfg(target_os = "ios")]
        {
            let url = update.distribution.as_ref().ok_or("update_missing")?;
            native
                .0
                .run_mobile_plugin_async("openDistribution", serde_json::json!({ "url": url }))
                .await
                .map_err(|_| "install_failed".into())
        }
        #[cfg(target_os = "android")]
        {
            let asset = update.asset.as_ref().ok_or("update_missing")?;
            let directory = app
                .path()
                .app_cache_dir()
                .map_err(|_| "install_failed")?
                .join("native-updates");
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|_| "install_failed")?;
            let path = directory.join("update.apk");
            download(client()?, asset, &path, |downloaded, total| {
                let _ = window.emit("mobile-update-progress", Progress { downloaded, total });
            })
            .await?;
            crate::mobile::native::require_settings(&window)?;
            // Native verifies package identity/version/signers before granting a content URI.
            native
                .0
                .run_mobile_plugin_async("install", serde_json::json!({ "path": path }))
                .await
                .map_err(|error| {
                    if error.to_string().contains("signature_mismatch") {
                        "signature_mismatch".into()
                    } else {
                        "install_failed".into()
                    }
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn streams_real_http_bytes_and_removes_bad_partial_files() {
        use std::io::{Read, Write};
        for (declared_size, contents, valid) in [
            (3, "apk", true),
            (3, "bad", false),
            (2, "apk", false),
            (4, "apk", false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/asset", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                stream.read(&mut request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    contents.len(),
                    contents
                )
                .unwrap();
            });
            let (_, mut releases) = fixture();
            let asset = &mut releases[0].assets[0];
            asset.browser_download_url = url;
            asset.size = declared_size;
            let path = std::env::temp_dir().join(format!(
                "native-update-test-{}-{}.apk",
                std::process::id(),
                asset.size
            ));
            let mut progress = Vec::new();
            let result = download(
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .build()
                    .unwrap(),
                asset,
                &path,
                |done, total| progress.push((done, total)),
            )
            .await;
            server.join().unwrap();
            assert_eq!(result.is_ok(), valid);
            assert_eq!(path.exists(), valid);
            if valid {
                assert_eq!(std::fs::read(&path).unwrap(), b"apk");
                assert_eq!(progress.last(), Some(&(3, 3)));
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    fn fixture() -> (Channel, Vec<Release>) {
        let channel = serde_json::from_value(
            serde_json::json!({ "repository": "example/client", "includePrereleases": true,
            "mobile": { "packageName": "client" } }),
        )
        .unwrap();
        let releases = serde_json::from_value(serde_json::json!([{
            "tag_name": "v1.2.0-alpha", "draft": false, "prerelease": true, "assets": [{
                "name": "client-v1.2.0-alpha-android-arm64-v8a-debug.apk", "size": 3,
                "digest": format!("sha256:{:x}", Sha256::digest(b"apk")),
                "browser_download_url": "https://github.com/example/client/releases/download/v1.2.0-alpha/client-v1.2.0-alpha-android-arm64-v8a-debug.apk"
            }]}])).unwrap();
        (channel, releases)
    }
    #[test]
    fn selects_newer_exact_abi_version_and_channel() {
        let (mut channel, releases) = fixture();
        let current = Version::parse("1.0.0").unwrap();
        assert!(select(&releases, &channel, &current, "android", "arm64-v8a").is_some());
        assert!(select(&releases, &channel, &current, "android", "x86_64").is_none());
        assert!(
            select(
                &releases,
                &channel,
                &Version::parse("1.2.0-alpha").unwrap(),
                "android",
                "arm64-v8a"
            )
            .is_none()
        );
        channel.include_prereleases = false;
        assert!(select(&releases, &channel, &current, "android", "arm64-v8a").is_none());
    }
    #[test]
    fn refuses_foreign_or_unverifiable_assets_and_corruption() {
        let (channel, mut releases) = fixture();
        let asset = &mut releases[0].assets[0];
        let mut hash = Sha256::new();
        hash.update(b"apk");
        assert!(verified(3, hash.clone(), asset));
        assert!(!verified(4, hash.clone(), asset));
        hash.update(b"bad");
        assert!(!verified(3, hash, asset));
        asset.browser_download_url = asset
            .browser_download_url
            .replace("example/client", "other/client");
        assert!(
            select(
                &releases,
                &channel,
                &Version::parse("1.0.0").unwrap(),
                "android",
                "arm64-v8a"
            )
            .is_none()
        );
        assert!(distribution_url(Some("javascript:alert(1)")).is_none());
        assert!(distribution_url(Some("https://user:secret@example.invalid/")).is_none());
        assert!(distribution_url(None).is_none());
        assert!(distribution_url(Some("https://testflight.apple.com/join/example")).is_some());
    }

    #[test]
    fn rejects_drafts_missing_digests_and_unbound_tags() {
        let (channel, mut releases) = fixture();
        let current = Version::parse("1.0.0").unwrap();
        releases[0].draft = true;
        assert!(select(&releases, &channel, &current, "android", "arm64-v8a").is_none());
        releases[0].draft = false;
        let expected = releases[0].assets[0].digest.take();
        assert!(select(&releases, &channel, &current, "android", "arm64-v8a").is_none());
        releases[0].assets[0].digest = expected;
        releases[0].assets[0].browser_download_url = releases[0].assets[0]
            .browser_download_url
            .replace("download/v1.2.0-alpha", "download/v9.9.9");
        assert!(select(&releases, &channel, &current, "android", "arm64-v8a").is_none());
    }
}
