use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Framework extraction errors retain their HTTP status and raw diagnostics.
/// Only the application-facing summary is translated.
#[derive(Debug)]
pub struct ExtractionError {
    status: StatusCode,
    key: &'static str,
    detail: String,
}

impl IntoResponse for ExtractionError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(json!({
                "error": crate::i18n::message(self.key), "detail": self.detail,
            })),
        )
            .into_response()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T: serde::Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for Json<T> {
    type Rejection = ExtractionError;
    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(request, state)
            .await
            .map(|value| Self(value.0))
            .map_err(|error| {
                use axum::extract::rejection::JsonRejection;
                let key = match &error {
                    JsonRejection::JsonDataError(_) => "request.jsonData",
                    JsonRejection::JsonSyntaxError(_) => "request.jsonSyntax",
                    JsonRejection::MissingJsonContentType(_) => "request.jsonContentType",
                    _ => "request.body",
                };
                ExtractionError {
                    status: error.status(),
                    key,
                    detail: error.body_text(),
                }
            })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequestParts<S>
    for Query<T>
{
    type Rejection = ExtractionError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|value| Self(value.0))
            .map_err(|error| ExtractionError {
                status: error.status(),
                key: "request.query",
                detail: error.body_text(),
            })
    }
}

#[derive(Debug, Clone)]
pub struct Path<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned + Send> axum::extract::FromRequestParts<S>
    for Path<T>
{
    type Rejection = ExtractionError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|value| Self(value.0))
            .map_err(|error| ExtractionError {
                status: error.status(),
                key: "request.path",
                detail: error.body_text(),
            })
    }
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String, pub Option<crate::i18n::Message>);
pub type Result<T> = std::result::Result<T, ApiError>;

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
impl std::error::Error for ApiError {}

impl ApiError {
    /// Canonical English remains available to logs and legacy display callers;
    /// the identity is retained for localization at response and storage boundaries.
    pub fn localized(status: StatusCode, message: crate::i18n::Message) -> Self {
        Self(
            status,
            crate::i18n::render(crate::i18n::Locale::EnUs, &message),
            Some(message),
        )
    }
    pub fn raw(status: StatusCode, detail: String) -> Self {
        Self(status, detail, None)
    }
    pub fn render(&self, locale: crate::i18n::Locale) -> String {
        self.2
            .as_ref()
            .map(|message| crate::i18n::render(locale, message))
            .unwrap_or_else(|| self.1.clone())
    }
    /// Use only at an application-error storage boundary, never for user data.
    pub fn stored(&self) -> String {
        self.2
            .as_ref()
            .map(crate::i18n::store_message)
            .unwrap_or_else(|| crate::i18n::store_raw(&self.1))
    }
    pub fn bad(message: impl Into<String>) -> Self {
        Self::raw(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::raw(StatusCode::CONFLICT, message.into())
    }
    pub fn unauthorized() -> Self {
        Self::localized(
            StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("error.unauthorized"),
        )
    }
    pub fn forbidden() -> Self {
        Self::localized(
            StatusCode::FORBIDDEN,
            crate::i18n::Message::new("error.forbidden"),
        )
    }
    pub fn missing() -> Self {
        Self::localized(
            StatusCode::NOT_FOUND,
            crate::i18n::Message::new("error.missing"),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let detail = self.render(crate::i18n::current_locale());
        (self.0, Json(json!({"error": detail}))).into_response()
    }
}

impl From<sea_orm::DbErr> for ApiError {
    fn from(error: sea_orm::DbErr) -> Self {
        tracing::error!(%error, "database operation failed");
        Self::localized(
            StatusCode::INTERNAL_SERVER_ERROR,
            crate::i18n::Message::new("error.database"),
        )
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        tracing::error!(%error, "file operation failed");
        Self::localized(
            StatusCode::INTERNAL_SERVER_ERROR,
            crate::i18n::Message::new("error.file"),
        )
    }
}

#[cfg(test)]
mod extraction_tests {
    use super::*;
    use axum::{
        Router,
        extract::DefaultBodyLimit,
        middleware,
        routing::{get, post},
    };
    use serde::Deserialize;

    #[tokio::test]
    async fn typed_extraction_rejections_keep_status_and_diagnostics_in_all_six_languages() {
        #[derive(Deserialize)]
        struct Input {
            count: u32,
            text: String,
        }
        #[derive(Deserialize)]
        struct Page {
            page: u32,
        }
        let app = Router::new()
            .route(
                "/body",
                post(|Json(input): Json<Input>| async move {
                    Json(json!({"count":input.count,"text":input.text}))
                }),
            )
            .route(
                "/query",
                get(|Query(page): Query<Page>| async move { Json(json!({"page":page.page})) }),
            )
            .route(
                "/path/{id}",
                get(|Path(id): Path<crate::snowflake::Id>| async move { Json(json!({"id":id})) }),
            )
            .layer(DefaultBodyLimit::max(128))
            .layer(middleware::from_fn(crate::i18n::middleware));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let mut diagnostics = std::collections::BTreeMap::new();
        for locale in crate::i18n::Locale::ALL {
            for (label, method, path, body, content_type, status, key) in [
                (
                    "syntax",
                    reqwest::Method::POST,
                    "/body",
                    "{",
                    Some("application/json"),
                    StatusCode::BAD_REQUEST,
                    "request.jsonSyntax",
                ),
                (
                    "missing",
                    reqwest::Method::POST,
                    "/body",
                    "{}",
                    Some("application/json"),
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "request.jsonData",
                ),
                (
                    "type",
                    reqwest::Method::POST,
                    "/body",
                    r#"{"count":"invalid","text":"raw"}"#,
                    Some("application/json"),
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "request.jsonData",
                ),
                (
                    "content-type",
                    reqwest::Method::POST,
                    "/body",
                    "{}",
                    Some("text/plain"),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "request.jsonContentType",
                ),
                (
                    "missing-content-type",
                    reqwest::Method::POST,
                    "/body",
                    "{}",
                    None,
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "request.jsonContentType",
                ),
                (
                    "size",
                    reqwest::Method::POST,
                    "/body",
                    &"x".repeat(129),
                    Some("application/json"),
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "request.body",
                ),
                (
                    "path",
                    reqwest::Method::GET,
                    "/path/not-an-id",
                    "",
                    None,
                    StatusCode::BAD_REQUEST,
                    "request.path",
                ),
                (
                    "query",
                    reqwest::Method::GET,
                    "/query?page=invalid",
                    "",
                    None,
                    StatusCode::BAD_REQUEST,
                    "request.query",
                ),
                (
                    "missing-query",
                    reqwest::Method::GET,
                    "/query",
                    "",
                    None,
                    StatusCode::BAD_REQUEST,
                    "request.query",
                ),
            ] {
                let mut request = client
                    .request(method, format!("{url}{path}"))
                    .header("Accept-Language", locale.tag());
                if !body.is_empty() {
                    request = request.body(body.to_owned());
                }
                if let Some(content_type) = content_type {
                    request = request.header("Content-Type", content_type);
                }
                let response = request.send().await.unwrap();
                assert_eq!(response.status(), status, "{} {label}", locale.tag());
                assert_eq!(response.headers()["Content-Language"], locale.tag());
                assert_eq!(response.headers()["Content-Type"], "application/json");
                let value: serde_json::Value = response.json().await.unwrap();
                assert_eq!(
                    value["error"],
                    crate::i18n::render(locale, &crate::i18n::Message::new(key))
                );
                let detail = value["detail"].as_str().unwrap();
                assert!(!detail.is_empty());
                if locale == crate::i18n::Locale::EnUs {
                    diagnostics.insert(label, detail.to_owned());
                } else {
                    assert_eq!(
                        detail, diagnostics[label],
                        "framework diagnostics are literal"
                    );
                }
            }
            let literal = "用户原文 {request.path}";
            let response = client
                .post(format!("{url}/body"))
                .header("Accept-Language", locale.tag())
                .json(&json!({"count":7,"text":literal}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.json::<serde_json::Value>().await.unwrap(),
                json!({"count":7,"text":literal})
            );
            let response = client
                .get(format!("{url}/path/7"))
                .header("Accept-Language", locale.tag())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.json::<serde_json::Value>().await.unwrap(),
                json!({"id":"7"})
            );
        }
        server.abort();
    }
}
