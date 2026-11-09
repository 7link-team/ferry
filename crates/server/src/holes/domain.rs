use super::*;
use crate::accounts::addresses::Configuration;
use axum::{
    extract::Request,
    http::header,
    response::{IntoResponse, Response},
};

pub(super) fn label(token: &str) -> Result<String> {
    let bytes = hex::decode(token).map_err(|_| invalid())?;
    if bytes.len() != 32 {
        return Err(invalid());
    }
    Ok(data_encoding::BASE32_NOPAD
        .encode(&bytes)
        .to_ascii_lowercase())
}
pub(crate) fn token(label: &str) -> Option<String> {
    if label.len() != 52
        || !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    {
        return None;
    }
    let bytes = data_encoding::BASE32_NOPAD
        .decode(label.to_ascii_uppercase().as_bytes())
        .ok()?;
    (bytes.len() == 32).then(|| hex::encode(bytes))
}
/// Return the request unchanged for console hosts, including arbitrary aliases/IPs.
pub(crate) async fn dispatch(
    s: &AppState,
    request: Request,
) -> std::result::Result<Request, Response> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<axum::http::uri::Authority>().ok())
        .map(|v| v.host().to_ascii_lowercase())
        .unwrap_or_default();
    if host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(request);
    }
    let config = Configuration::load(s, &s.db)
        .await
        .map_err(IntoResponse::into_response)?;
    let token_host = host.split('.').next().and_then(token).is_some();
    if config.preview_domain.is_empty() {
        return if token_host {
            Err(super::proxy::not_found())
        } else {
            Ok(request)
        };
    }
    let console = reqwest::Url::parse(&config.public_url)
        .ok()
        .and_then(|v| v.host_str().map(str::to_owned));
    if console.as_deref() == Some(host.as_str()) {
        return Ok(request);
    }
    let Some(label) = host.strip_suffix(&format!(".{}", config.preview_domain)) else {
        return if token_host {
            Err(super::proxy::not_found())
        } else {
            Ok(request)
        };
    };
    let Some(token) = token(label) else {
        return Err(super::proxy::not_found());
    };
    Err(super::proxy::serve_domain(s.clone(), request, token).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::Expr;
    #[test]
    fn tokens_roundtrip_without_exceeding_dns_label_limit() {
        for value in [
            "00".repeat(32),
            "ff".repeat(32),
            crate::hosts::runner::secret(),
        ] {
            let name = label(&value).unwrap();
            assert_eq!(name.len(), 52);
            assert_eq!(token(&name).unwrap(), value);
        }
        assert!(token(&"z".repeat(52)).is_none());
        assert!(token(&"a".repeat(53)).is_none());
        assert!(token(&format!("{}b", "a".repeat(51))).is_none());
    }
    #[tokio::test]
    async fn routes_only_preview_namespace_and_never_falls_back_to_console() {
        let (_dir, s) = crate::agents::tests::setup().await;
        for domain in ["example.test", "preview.example.test"] {
            crate::entities::platform_state::Entity::update_many()
                .col_expr(crate::entities::platform_state::Column::AccessConfig,Expr::value(json!({"public_url":format!("https://console.{domain}"),"preview_domain":domain}).to_string()))
                .exec(&s.db).await.unwrap();
            for host in [
                "127.0.0.1".into(),
                "192.168.1.10:4291".into(),
                "other.test".into(),
                format!("console.{domain}"),
            ] {
                assert!(
                    dispatch(
                        &s,
                        Request::builder()
                            .uri("/api/me")
                            .header("host", host)
                            .body(axum::body::Body::empty())
                            .unwrap()
                    )
                    .await
                    .is_ok()
                );
            }
            for host in [
                format!("unknown.{domain}"),
                format!("a.b.{domain}"),
                format!("{}.{domain}", "a".repeat(52)),
            ] {
                assert_eq!(
                    dispatch(
                        &s,
                        Request::builder()
                            .uri("/api/me")
                            .header("host", host)
                            .body(axum::body::Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap_err()
                    .status(),
                    404
                );
            }
        }
    }
}
