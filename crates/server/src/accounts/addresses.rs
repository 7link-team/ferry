//! Deployment-independent public link settings; Host is never a source of canonical URLs.
use crate::{
    entities::platform_state,
    error::{ApiError, Result},
    state::AppState,
};
use sea_orm::{ConnectionTrait, EntityTrait};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Configuration {
    #[serde(default)]
    pub public_url: String,
    #[serde(default)]
    pub preview_domain: String,
}
impl Configuration {
    pub fn read(s: &AppState, row: &platform_state::Model) -> Result<Self> {
        if let Some(raw) = &row.access_config {
            return Ok(serde_json::from_str(raw).map_err(std::io::Error::other)?);
        }
        // Preserve the existing encrypted public address until settings are first saved.
        Ok(Self {
            public_url: crate::repositories::auth::OAuthSettings::read(
                s,
                row.repository_oauth.as_deref(),
            )?
            .public_url,
            ..Self::default()
        })
    }
    pub async fn load(s: &AppState, conn: &impl ConnectionTrait) -> Result<Self> {
        let row = platform_state::Entity::find_by_id(1)
            .one(conn)
            .await?
            .ok_or_else(ApiError::missing)?;
        Self::read(s, &row)
    }
    pub fn view(&self) -> Value {
        json!({"public_url": self.public_url, "preview_domain": self.preview_domain})
    }
    pub fn update(&mut self, patch: &serde_json::Map<String, Value>) -> Result<()> {
        for (key, target) in [
            ("public_url", &mut self.public_url),
            ("preview_domain", &mut self.preview_domain),
        ] {
            let value = patch.get(key).or_else(|| {
                if key == "public_url" {
                    patch.get("oauth_public_url")
                } else {
                    None
                }
            });
            if let Some(value) = value {
                *target = value.as_str().ok_or_else(invalid)?.trim().to_owned();
            }
        }
        if !self.public_url.is_empty() {
            let url = reqwest::Url::parse(&self.public_url).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
                || self.public_url.len() > 2048
                || url
                    .host_str()
                    .and_then(|host| host.split('.').next())
                    .and_then(crate::holes::preview_token)
                    .is_some()
            {
                return Err(invalid());
            }
            self.public_url = url.as_str().trim_end_matches('/').into();
        }
        self.preview_domain = self.preview_domain.to_ascii_lowercase();
        let domain = self.preview_domain.as_str();
        if !domain.is_empty()
            && (domain.len() > 200
                || !domain.contains('.')
                || domain.parse::<std::net::IpAddr>().is_ok()
                || !domain.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                }))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn require_preview(&self) -> Result<()> {
        if self.preview_domain.is_empty() {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("holes.domainRequired"),
            ));
        }
        Ok(())
    }
    pub fn preview_origin(&self, token: &str) -> Result<String> {
        self.require_preview()?;
        // Public deployments default to HTTPS; explicit HTTP public URLs support LAN/dev gateways.
        let public = reqwest::Url::parse(&self.public_url).ok();
        let scheme = public.as_ref().map_or("https", |url| url.scheme());
        let port = public
            .as_ref()
            .and_then(|url| url.port())
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        Ok(format!("{scheme}://{token}.{}{port}", self.preview_domain))
    }
}
fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("platform.addressInvalid"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_are_optional_and_domains_accept_both_depths() {
        let mut c = Configuration::default();
        assert!(c.require_preview().is_err());
        for domain in ["example.test", "preview.example.test"] {
            c.update(
                json!({"public_url":"http://console.example.test:4291", "preview_domain":domain})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                c.preview_origin("abc").unwrap(),
                format!("http://abc.{domain}:4291")
            );
        }
        c.update(
            json!({"public_url":"", "preview_domain":"preview.example.test"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            c.preview_origin("abc").unwrap(),
            "https://abc.preview.example.test"
        );
        assert!(c.update(json!({"public_url":format!("https://{}.preview.example.test", "a".repeat(52)),"preview_domain":"preview.example.test"}).as_object().unwrap()).is_err(), "A preview token host must never become the console address");
        for domain in [
            "*.example.test",
            "https://example.test",
            "example.test/path",
            "127.0.0.1",
            "example.test:443",
            ".example.test",
            "a..test",
            "-a.test",
        ] {
            assert!(
                c.update(json!({"preview_domain":domain}).as_object().unwrap())
                    .is_err(),
                "{domain}"
            );
        }
    }
    #[tokio::test]
    async fn retains_saved_legacy_public_url_and_new_settings_win() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let mut row = platform_state::Entity::find_by_id(1)
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        let mut legacy = crate::repositories::auth::OAuthSettings::read(&s, None).unwrap();
        legacy.public_url = "https://console.example.test".into();
        row.repository_oauth = Some(legacy.seal(&s).unwrap());
        row.access_config = None;
        let mut config = Configuration::read(&s, &row).unwrap();
        assert_eq!(config.public_url, "https://console.example.test");
        config
            .update(
                json!({"preview_domain":"preview.example.test"})
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        row.access_config = Some(serde_json::to_string(&config).unwrap());
        assert_eq!(
            Configuration::read(&s, &row).unwrap().public_url,
            "https://console.example.test"
        );
    }
}
