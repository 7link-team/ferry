//! Administrator-configured ceilings, deliberately independent of legacy limits.
use crate::{
    entities::platform_state,
    error::{ApiError, Result},
    i18n::Message,
};
use ferry_protocol::resources::ResourcePolicy;
use sea_orm::{ConnectionTrait, EntityTrait};
use serde_json::{Map, Value};

pub async fn load(conn: &impl ConnectionTrait) -> Result<ResourcePolicy> {
    let row = platform_state::Entity::find_by_id(1)
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    read(row.resource_config.as_deref())
}

pub fn read(value: Option<&str>) -> Result<ResourcePolicy> {
    value
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())
        .map(Option::unwrap_or_default)
}

pub fn update(value: Option<&str>, patch: &Map<String, Value>) -> Result<Option<String>> {
    let Some(fields) = patch.get("resource_limits") else {
        return Ok(value.map(str::to_owned));
    };
    let fields = fields.as_object().ok_or_else(invalid)?;
    let mut merged = serde_json::to_value(read(value)?).map_err(std::io::Error::other)?;
    merged.as_object_mut().unwrap().extend(fields.clone());
    let policy: ResourcePolicy = serde_json::from_value(merged).map_err(|_| invalid())?;
    if policy == ResourcePolicy::default() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::to_string(&policy).map_err(std::io::Error::other)?,
    ))
}

/// Apply the persisted request ceiling before extraction, including chunked bodies.
/// Missing policy deliberately disables Axum's implicit 2 MiB body ceiling.
pub async fn middleware(
    axum::extract::State(s): axum::extract::State<crate::state::AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let policy = match load(&s.db).await {
        Ok(policy) => policy,
        Err(error) => return error.into_response(),
    };
    request.extensions_mut().insert(policy.clone());
    let limit = ferry_protocol::resources::byte_limit(policy.http_body_bytes);
    // Admin extraction precedes JSON parsing on this endpoint. A small ceiling
    // must never prevent an administrator from clearing that same ceiling.
    let manages_policy = request.uri().path() == "/api/platform/settings/1";
    if policy.http_body_bytes.is_some() && !manages_policy {
        if request
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|value| value > limit as u64)
        {
            return ApiError::localized(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                Message::new("request.resourceLimit"),
            )
            .into_response();
        }
        let (parts, body) = request.into_parts();
        let body = axum::body::Body::new(http_body_util::Limited::new(body, limit));
        next.run(axum::http::Request::from_parts(parts, body)).await
    } else {
        next.run(request).await
    }
}

fn invalid() -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        Message::new("request.jsonData"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn request_ceiling_is_optional_counts_chunked_data_and_keeps_admin_recovery() {
        use axum::{
            Router, body::Bytes, extract::DefaultBodyLimit, middleware::from_fn_with_state,
            routing::post,
        };
        use sea_orm::sea_query::Expr;
        let (_dir, s) = crate::agents::tests::setup().await;
        let router = Router::new()
            .route(
                "/echo",
                post(|bytes: Bytes| async move { bytes.len().to_string() }),
            )
            .route(
                "/api/platform/settings/1",
                post(|bytes: Bytes| async move { bytes.len().to_string() }),
            )
            .layer(DefaultBodyLimit::disable())
            .layer(from_fn_with_state(s.clone(), middleware));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let response = client
            .post(format!("http://{address}/echo"))
            .body(vec![b'x'; 3 * 1024 * 1024])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.text().await.unwrap(),
            (3 * 1024 * 1024).to_string()
        );
        platform_state::Entity::update_many()
            .col_expr(
                platform_state::Column::ResourceConfig,
                Expr::value(r#"{"http_body_bytes":3}"#),
            )
            .exec(&s.db)
            .await
            .unwrap();
        assert_eq!(
            client
                .post(format!("http://{address}/echo"))
                .body("123")
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("http://{address}/echo"))
                .body("1234")
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::PAYLOAD_TOO_LARGE
        );
        let chunks = futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"12")),
            Ok(Bytes::from_static(b"34")),
        ]);
        assert_eq!(
            client
                .post(format!("http://{address}/echo"))
                .body(reqwest::Body::wrap_stream(chunks))
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            client
                .post(format!("http://{address}/api/platform/settings/1"))
                .body(r#"{"resource_limits":{"http_body_bytes":null}}"#)
                .send()
                .await
                .unwrap()
                .status(),
            axum::http::StatusCode::OK
        );
    }
    #[test]
    fn policy_patch_merges_clears_and_rejects_invalid_input() {
        assert_eq!(read(None).unwrap(), ResourcePolicy::default());
        assert_eq!(update(None, json!({}).as_object().unwrap()).unwrap(), None);
        let saved = update(
            None,
            json!({"resource_limits":{"file_count":4,"file_upload_bytes":9007199254740991u64}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let cleared = update(
            saved.as_deref(),
            json!({"resource_limits":{"file_count":null}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let loaded = read(cleared.as_deref()).unwrap();
        assert_eq!(loaded.file_count, None);
        assert_eq!(loaded.file_upload_bytes, Some(9007199254740991));
        for invalid in [
            json!({"resource_limits":{"file_count":0}}),
            json!({"resource_limits":{"file_count":-1}}),
            json!({"resource_limits":{"file_count":1.2}}),
            json!({"resource_limits":{"file_count":9007199254740992u64}}),
            json!({"resource_limits":{"bad_key":1}}),
            json!({"resource_limits":null}),
            json!({"resource_limits":[]}),
        ] {
            assert!(update(saved.as_deref(), invalid.as_object().unwrap()).is_err());
        }
        assert_eq!(
            update(
                saved.as_deref(),
                json!({"registration_enabled":true}).as_object().unwrap()
            )
            .unwrap(),
            saved
        );
    }
}
