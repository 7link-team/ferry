//! Agent bootstrap metadata reuses the signed Runner release catalog.
use crate::{
    error::{ApiError, Result},
    runner,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use ferry_protocol::release::{MAX_CATALOG, verify_catalog};

fn metadata(
    bytes: &[u8],
    signature: &str,
    key: &str,
    target: &str,
    version: &str,
) -> anyhow::Result<String> {
    let artifact = verify_catalog(bytes, signature, key, target, "0.0.0-0", version)?;
    anyhow::ensure!(
        !artifact.url.chars().any(char::is_control),
        "下载地址包含控制字符"
    );
    Ok(format!(
        "{version}\n{}\n{}\n{}\n",
        artifact.url, artifact.sha256, artifact.size
    ))
}

pub async fn shell() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../../../scripts/runner-install.sh"),
    )
}
pub async fn powershell() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../../../scripts/runner-install.ps1"),
    )
}
async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| ApiError::bad("无法下载 Runner 发布清单，请检查服务器外网连接。"))?
        .error_for_status()
        .map_err(|_| ApiError::bad("当前版本的 Runner 发布包尚不可用，请等待发布完成。"))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad("Runner 发布清单下载中断。"))?
    {
        if bytes.len() + chunk.len() > limit {
            return Err(ApiError::bad("Runner 发布清单超过大小限制。"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn artifact(
    State(s): State<AppState>,
    Path(target): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    runner::bootstrap_token(&s, token).await?;
    if ![
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]
    .contains(&target.as_str())
    {
        return Err(ApiError::bad("当前系统或架构暂不支持安装 Runner。"));
    }
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../../desktop/tauri.conf.json"))
            .map_err(|_| ApiError::bad("Runner 发布来源配置无效。"))?;
    let repository = config["plugins"]["release"]["repository"]
        .as_str()
        .ok_or_else(|| ApiError::bad("未配置 Runner 发布来源。"))?;
    let parts: Vec<_> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err(ApiError::bad("Runner 发布仓库配置无效。"));
    }
    let key = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or_else(|| ApiError::bad("未配置 Runner 发布公钥。"))?;
    let version = env!("CARGO_PKG_VERSION");
    let base =
        format!("https://github.com/{repository}/releases/download/v{version}/runner-update.json");
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| ApiError::bad("无法创建 Runner 下载连接。"))?;
    let signature_url = format!("{base}.sig");
    let (bytes, signature) = tokio::try_join!(
        download(&client, &base, MAX_CATALOG),
        download(&client, &signature_url, 8192)
    )?;
    let signature =
        String::from_utf8(signature).map_err(|_| ApiError::bad("Runner 发布签名格式无效。"))?;
    let body = metadata(&bytes, &signature, key, &target, version)
        .map_err(|e| ApiError::bad(format!("Runner 发布校验失败：{e}")))?;
    // Downloading metadata does not consume enrolment; only the real runner's
    // authenticated Hello can claim its one host identity.
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires scripts/test-runner-update.py signing fixtures"]
    fn bootstrap_uses_verified_exact_release_and_never_returns_untrusted_metadata() {
        let root = std::path::PathBuf::from(std::env::var_os("RUNNER_UPDATE_TEST_DIR").unwrap());
        let bytes = std::fs::read(root.join("valid.json")).unwrap();
        let sig = std::fs::read_to_string(root.join("valid.json.sig")).unwrap();
        let key = std::fs::read_to_string(root.join("test.key.pub")).unwrap();
        let output = metadata(&bytes, &sig, &key, "fixture-target", "9.9.9").unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "9.9.9");
        assert_eq!(lines[1], "https://example.invalid/runner");
        assert_eq!(lines[3], "31");
        assert!(metadata(b"tampered", &sig, &key, "fixture-target", "9.9.9").is_err());
        assert!(metadata(&bytes, &sig, &key, "fixture-target", "9.9.10").is_err());
        assert!(metadata(&bytes, &sig, &key, "unsupported", "9.9.9").is_err());
    }
}
