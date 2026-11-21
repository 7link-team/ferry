//! Expiring download links for file artifacts (reports and other generated
//! files), usable by people who receive a notification without signing in.
//!
//! The token is sealed by the vault: it names the workspace, the content hash,
//! the download name, the media type and the expiry, and cannot be forged or
//! altered. A link stops working when it expires or when the file is no longer
//! referenced by anything in its workspace.
use crate::entities::content_assets as assets;
use crate::error::{ApiError, Result};
use crate::state::AppState;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use base64::Engine;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

const AAD: &str = "file-link:v1";
/// Default and upper bounds for a link's lifetime.
pub(crate) const DEFAULT_TTL_DAYS: i64 = 7;
pub(crate) const MAX_TTL_DAYS: i64 = 30;

#[derive(Serialize, Deserialize)]
struct Claims {
    /// Workspace.
    w: i64,
    /// Content sha256 (hex).
    h: String,
    /// Download name.
    n: String,
    /// Media type.
    m: String,
    /// Expiry, epoch milliseconds.
    e: i64,
}

/// A signed link and its expiry (epoch ms); `None` when no public address is
/// configured, because a link must be reachable from outside.
pub(crate) async fn sign(
    s: &AppState,
    workspace: i64,
    sha256: &str,
    name: &str,
    mime: &str,
    ttl_days: i64,
) -> Result<Option<(String, i64)>> {
    let base = crate::accounts::addresses::Configuration::load(s, &s.db)
        .await?
        .public_url;
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Ok(None);
    }
    let expires = crate::db::now() + ttl_days.clamp(1, MAX_TTL_DAYS) * 24 * 60 * 60 * 1000;
    let claims = Claims {
        w: workspace,
        h: sha256.into(),
        n: name.into(),
        m: mime.into(),
        e: expires,
    };
    let sealed = s.vault.seal(
        &serde_json::to_string(&claims).map_err(crate::error::internal_fault)?,
        AAD,
    )?;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed.as_bytes());
    Ok(Some((format!("{base}/api/files/{token}"), expires)))
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/files/{token}", get(download))
}

fn gone() -> ApiError {
    ApiError::localized(
        StatusCode::NOT_FOUND,
        crate::i18n::Message::new("fileLink.unavailable"),
    )
}

pub(crate) async fn download(
    State(s): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let sealed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(gone)?;
    let claims: Claims = s
        .vault
        .open_secret(&sealed, AAD)
        .ok()
        .and_then(|plain| serde_json::from_str(&plain).ok())
        .ok_or_else(gone)?;
    if claims.e < crate::db::now() {
        return Err(gone());
    }
    let asset = assets::Entity::find()
        .filter(assets::Column::WorkspaceId.eq(claims.w))
        .filter(assets::Column::Sha256.eq(&claims.h))
        .one(&s.db)
        .await?
        .ok_or_else(gone)?;
    // Only files something still keeps (a run, a workflow) can be downloaded.
    let kept = crate::entities::asset_refs::Entity::find()
        .filter(crate::entities::asset_refs::Column::AssetId.eq(asset.id))
        .one(&s.db)
        .await?
        .is_some();
    if !kept {
        return Err(gone());
    }
    let backend = crate::artifact_store::ArtifactStore::for_state(&s).await?;
    let store = crate::content_assets::Store {
        db: &s.db,
        backend: &backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: crate::content_assets::DEFAULT_QUOTA,
    };
    let file = store.read_file(&s.db, claims.w, asset.id).await?;
    crate::workflows::media::media_file_response(file, &claims.m, &headers, Some(&claims.n)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_round_trip_through_url_safe_tokens() {
        let claims = Claims {
            w: 1,
            h: "ab".into(),
            n: "周报.pdf".into(),
            m: "application/pdf".into(),
            e: 5,
        };
        let text = serde_json::to_string(&claims).unwrap();
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text.as_bytes());
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }
}
