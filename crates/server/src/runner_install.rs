//! Agent bootstrap metadata reuses the signed Runner release catalog.
use crate::error::Path;
use crate::{
    error::{ApiError, Result},
    runner,
    state::AppState,
};
use axum::{
    extract::State,
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
        ferry_protocol::release::ValidationError::new(
            "urlControl",
            "The download URL contains control characters."
        )
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
        .map_err(|_| {
            ApiError::localized(
                StatusCode::SERVICE_UNAVAILABLE,
                crate::i18n::Message::new("runner.catalogDownload"),
            )
        })?
        .error_for_status()
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.releaseUnavailable"),
            )
        })?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        ApiError::localized(
            StatusCode::SERVICE_UNAVAILABLE,
            crate::i18n::Message::new("runner.catalogInterrupted"),
        )
    })? {
        if bytes.len() + chunk.len() > limit {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.catalogSize"),
            ));
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
    runner::bootstrap_token(&s.db, token).await?;
    if ![
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]
    .contains(&target.as_str())
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.installUnsupported"),
        ));
    }
    let catalog = catalog().await?;
    let version = env!("CARGO_PKG_VERSION");
    let body = metadata(
        &catalog.bytes,
        &catalog.signature,
        &catalog.key,
        &target,
        version,
    )
    .map_err(|error| verification_error(&error))?;
    // Metadata retrieval never consumes enrollment.
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    ))
}

pub(crate) struct SignedCatalog {
    pub bytes: Vec<u8>,
    pub signature: String,
    pub key: String,
}

pub(crate) fn verification_error(error: &anyhow::Error) -> ApiError {
    let message = crate::i18n::Message::new("runner.verification");
    let message = if let Some(validation) =
        error.downcast_ref::<ferry_protocol::release::ValidationError>()
    {
        message.with_message(
            "detail",
            crate::i18n::Message::new(format!("runner.release.{}", validation.code)),
        )
    } else {
        message.with(
            "detail",
            error.to_string().chars().take(512).collect::<String>(),
        )
    };
    ApiError::localized(StatusCode::BAD_REQUEST, message)
}

/// Bootstrap and automatic upgrades use the exact running server release.
pub(crate) async fn catalog() -> Result<SignedCatalog> {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../../desktop/tauri.conf.json")).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.releaseConfig"),
            )
        })?;
    let repository = config["plugins"]["release"]["repository"]
        .as_str()
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.releaseSource"),
            )
        })?;
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
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.releaseRepository"),
        ));
    }
    let key = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.releaseKey"),
            )
        })?;
    let version = env!("CARGO_PKG_VERSION");
    let base =
        format!("https://github.com/{repository}/releases/download/v{version}/runner-update.json");
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.downloadClient"),
            )
        })?;
    let signature_url = format!("{base}.sig");
    let (bytes, signature) = tokio::try_join!(
        download(&client, &base, MAX_CATALOG),
        download(&client, &signature_url, 8192)
    )?;
    let signature = String::from_utf8(signature).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.signatureEncoding"),
        )
    })?;
    Ok(SignedCatalog {
        bytes,
        signature,
        key: key.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_validation_identity_is_rendered_for_the_reader() {
        let error: anyhow::Error =
            ferry_protocol::release::ValidationError::new("signatureInvalid", "invalid signature")
                .into();
        let error = verification_error(&error);
        let stored = error.stored();
        for locale in crate::i18n::Locale::ALL {
            let rendered = error.render(locale);
            assert_eq!(crate::i18n::render_stored(locale, &stored), rendered);
            assert!(!rendered.contains("runner.release."));
            if locale != crate::i18n::Locale::EnUs {
                assert_ne!(rendered, error.1);
            }
        }
    }
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
