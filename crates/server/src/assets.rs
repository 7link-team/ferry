//! Serves the built web UI from inside the binary so one executable is the
//! whole deployment in Docker mode and the Tauri sidecar in desktop mode.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;
use sea_orm::EntityTrait;

#[derive(Embed)]
#[folder = "../../dist"]
struct Web;

pub async fn serve_configured(
    axum::extract::State(s): axum::extract::State<crate::state::AppState>,
    uri: Uri,
) -> crate::error::Result<Response> {
    let path = uri.path().trim_start_matches('/');
    // Static assets retain their immutable caching and never query platform settings.
    if path != "index.html" && Web::get(path).is_some() {
        return Ok(serve(uri).await);
    }
    let row = crate::entities::platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?;
    let config = crate::accounts::platform_scripts::Configuration::read(
        row.as_ref().and_then(|r| r.page_scripts.as_deref()),
    )?;
    if let Some((script, policy)) = config.document() {
        if let Some(file) = Web::get("index.html") {
            let html =
                String::from_utf8_lossy(&file.data).replace("</head>", &format!("{script}</head>"));
            let mut response =
                ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                policy
                    .parse()
                    .map_err(|_| crate::error::ApiError::missing())?,
            );
            return Ok(response);
        }
    }
    Ok(serve(uri).await)
}

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = Web::get(path).or_else(|| Web::get("index.html"));
    match file {
        // A client-side route is not a file; the shell resolves it.
        Some(file) => {
            let mime = mime_guess::from_path(if Web::get(path).is_some() {
                path
            } else {
                "index.html"
            })
            .first_or_octet_stream();
            let mut response = ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response();
            // Only real, content-addressed build outputs may survive an upgrade.
            // Unknown asset paths fall back to the shell and must never be cached.
            if Web::get(path).is_some()
                && path.starts_with("assets/")
                && path.rsplit_once('.').is_some_and(|(stem, _)| {
                    stem.get(stem.len().saturating_sub(9)..)
                        .is_some_and(|suffix| {
                            suffix.starts_with('-')
                                && suffix[1..].len() == 8
                                && suffix[1..].bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                                })
                        })
                })
            {
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable".parse().unwrap(),
                );
            }
            response
        }
        None => (
            StatusCode::NOT_FOUND,
            crate::i18n::message("assets.not_built"),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_assets_are_cached_but_shell_and_api_are_not() {
        let asset = Web::iter()
            .find(|path| path.starts_with("assets/index-") && path.ends_with(".js"))
            .expect("production frontend build");
        let app = axum::Router::new()
            .route(
                "/api/probe",
                axum::routing::get(|| async { "private data" }),
            )
            .route(
                "/api/private",
                axum::routing::get(|| async { [(header::CACHE_CONTROL, "private, no-store")] }),
            )
            .fallback(serve)
            .layer(axum::middleware::from_fn(|req, next| async {
                crate::guard(req, next, &["127.0.0.1".into()]).await
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        for (path, expected) in [
            (format!("/{asset}"), "public, max-age=31536000, immutable"),
            ("/".into(), "no-store"),
            ("/zh-CN/workspaces/example".into(), "no-store"),
            ("/assets/missing-AbCd1234.js".into(), "no-store"),
            ("/api/probe".into(), "no-store"),
            ("/api/private".into(), "private, no-store"),
        ] {
            let response = client.get(format!("{origin}{path}")).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                expected,
                "{path}"
            );
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
        }
        server.abort();
    }
}
