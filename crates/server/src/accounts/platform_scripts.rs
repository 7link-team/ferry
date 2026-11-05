//! Administrator-controlled page scripts, with document-specific CSP nonces.
use crate::error::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Configuration {
    pub analytics_enabled: bool,
    pub analytics_ga_id: String,
    pub analytics_js: String,
    pub analytics_origins: String,
}
impl Configuration {
    pub fn read(value: Option<&str>) -> Result<Self> {
        Ok(value
            .map(serde_json::from_str)
            .transpose()
            .map_err(std::io::Error::other)?
            .unwrap_or_default())
    }
    pub fn update(value: Option<&str>, patch: &Map<String, Value>) -> Result<String> {
        let mut fields = serde_json::json!(Self::read(value)?);
        for key in [
            "analytics_enabled",
            "analytics_ga_id",
            "analytics_js",
            "analytics_origins",
        ] {
            if let Some(value) = patch.get(key) {
                fields[key] = value.clone();
            }
        }
        let invalid = || {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("platform.scriptsInvalid"),
            )
        };
        let mut config: Self = serde_json::from_value(fields).map_err(|_| invalid())?;
        config.analytics_ga_id = config.analytics_ga_id.trim().to_owned();
        let id = &config.analytics_ga_id;
        if (!id.is_empty()
            && (!id.starts_with("G-")
                || id.len() < 3
                || id.len() > 32
                || !id[2..]
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())))
            || config.analytics_js.len() > 65536
            || config.analytics_js.contains('\0')
            || config
                .analytics_js
                .to_ascii_lowercase()
                .contains("</script")
            || config.analytics_origins.len() > 8192
        {
            return Err(invalid());
        }
        let mut origins = Vec::new();
        for origin in config.analytics_origins.split_whitespace() {
            let url = reqwest::Url::parse(origin).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "https" | "http")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(invalid());
            }
            let normalized = url.origin().ascii_serialization();
            if normalized
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b":/.-[]".contains(&b))
            {
                return Err(invalid());
            }
            if !origins.contains(&normalized) {
                origins.push(normalized);
            }
        }
        config.analytics_origins = origins.join("\n");
        Ok(serde_json::to_string(&config).map_err(std::io::Error::other)?)
    }
    pub fn document(&self) -> Option<(String, String)> {
        if !self.analytics_enabled
            || (self.analytics_ga_id.is_empty() && self.analytics_js.trim().is_empty())
        {
            return None;
        }
        let nonce = crate::hosts::runner::secret();
        let mut js = String::new();
        let mut origins = self
            .analytics_origins
            .split_whitespace()
            .collect::<Vec<_>>();
        if !self.analytics_ga_id.is_empty() {
            let id = serde_json::to_string(&self.analytics_ga_id).ok()?;
            js.push_str(&format!("window.dataLayer=window.dataLayer||[];window.gtag=window.gtag||function(){{dataLayer.push(arguments)}};gtag('js',new Date());gtag('config',{id});var s=document.createElement('script');s.async=true;s.src='https://www.googletagmanager.com/gtag/js?id='+{id};document.head.appendChild(s);\n"));
            origins.extend([
                "https://www.googletagmanager.com",
                "https://www.google-analytics.com",
                "https://*.google-analytics.com",
                "https://*.analytics.google.com",
            ]);
        }
        js.push_str(&self.analytics_js);
        let sources = origins.join(" ");
        let policy = format!(
            "default-src 'self'; script-src 'self' 'nonce-{nonce}' {sources}; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: {sources}; media-src 'self' blob:; connect-src 'self' {sources}; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
        );
        Some((
            format!("<script nonce=\"{nonce}\" data-platform-scripts>\n{js}\n</script>"),
            policy,
        ))
    }
}
