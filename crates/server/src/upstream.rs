//! What went wrong when calling a third-party HTTP service (AI providers,
//! model catalogs, integrations), in terms the user can act on. The cause is
//! logged with its host and status only — never keys, bodies or query strings.

use crate::{error::ApiError, i18n::Message};
use axum::http::StatusCode;

/// The host of `url`, or a neutral word when it cannot be parsed.
pub(crate) fn host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "?".into())
}

/// A non-success HTTP status from `host`. `models` adds the base-URL/model hint
/// that a 404 from an AI provider almost always means.
pub(crate) fn status(context: &str, host: &str, status: u16, models: bool) -> ApiError {
    tracing::warn!(
        context,
        host,
        status,
        "upstream service returned an error status"
    );
    let message = match status {
        401 | 403 => Message::new("upstream.keyRejected"),
        404 if models => Message::new("upstream.modelNotFound"),
        404 => Message::new("upstream.notFound"),
        429 => Message::new("upstream.rateLimited"),
        500..=599 => Message::new("upstream.serverError"),
        _ => Message::new("upstream.rejected"),
    };
    ApiError::localized(
        StatusCode::BAD_GATEWAY,
        message
            .with("host", host.to_owned())
            .with("status", status.to_string()),
    )
}

/// The request could not be completed. `Some` when the service was never
/// reached (nothing was sent, so retrying is safe); `None` when the outcome is
/// unknown and the caller keeps its own "result unknown" message.
pub(crate) fn unreachable(context: &str, host: &str, error: &reqwest::Error) -> Option<ApiError> {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else {
        "other"
    };
    tracing::warn!(context, host, kind, error = %without_url(error), "upstream request failed");
    let key = if error.is_connect() {
        "upstream.unreachable"
    } else if error.is_timeout() {
        "upstream.timeout"
    } else {
        return None;
    };
    Some(ApiError::localized(
        StatusCode::BAD_GATEWAY,
        Message::new(key).with("host", host.to_owned()),
    ))
}

/// reqwest errors print their URL, which may carry a query string.
fn without_url(error: &reqwest::Error) -> String {
    let mut error = format!("{error:#}");
    if let Some(url) = error.find("(http") {
        error.truncate(url);
    }
    error.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, render};

    #[test]
    fn statuses_say_what_to_check() {
        for (code, models, key) in [
            (401, true, "upstream.keyRejected"),
            (403, false, "upstream.keyRejected"),
            (404, true, "upstream.modelNotFound"),
            (404, false, "upstream.notFound"),
            (429, true, "upstream.rateLimited"),
            (502, true, "upstream.serverError"),
            (418, false, "upstream.rejected"),
        ] {
            let error = status("test", "api.example.com", code, models);
            let message = error.2.as_ref().unwrap();
            assert_eq!(message.key, key);
            for locale in [
                Locale::EnUs,
                Locale::ZhCn,
                Locale::ZhTw,
                Locale::JaJp,
                Locale::KoKr,
                Locale::MsMy,
            ] {
                let text = render(locale, message);
                assert!(text.contains("api.example.com"), "{text}");
                assert!(!text.contains('{') && !text.contains("upstream."), "{text}");
            }
        }
    }

    #[tokio::test]
    async fn a_service_that_cannot_be_reached_is_named() {
        // A closed local port: the connection is refused before anything is sent.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/v1/models?key=secret"))
            .send()
            .await
            .unwrap_err();
        assert!(!without_url(&error).contains("secret"));
        let error = unreachable("test", "127.0.0.1", &error).unwrap();
        assert_eq!(error.2.as_ref().unwrap().key, "upstream.unreachable");
        let text = render(Locale::ZhCn, error.2.as_ref().unwrap());
        assert!(text.contains("127.0.0.1") && !text.contains('{'), "{text}");
    }
}
