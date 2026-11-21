//! File artifacts: the one shape every file-producing node returns.
//!
//! A node that produces a file (a PDF report today; Word, Excel, slides,
//! images, CSV… later) calls [`store`] with the bytes, media type and name. The
//! file goes into the run's encrypted content store, is kept by the workflow,
//! and comes back as
//!
//! ```json
//! {"artifact_id":"sha256:…","name":"…","mime":"…","size":123,"sha256":"…",
//!  "download_url":"https://…/api/files/…","expires_at":"…","meta":{"pages":3}}
//! ```
//!
//! plus the matching `binary` entry, so notifications (attachments), file
//! nodes and later nodes find it the same way whatever produced it.
use crate::content_assets::Store;
use crate::error::{ApiError, Result};
use crate::i18n::Message;
use crate::state::AppState;
use axum::http::StatusCode;
use ferry_protocol::sandbox::{Asset, Item};
use sea_orm::TransactionTrait;
use serde_json::{Map, Value, json};

/// Largest file a node may produce (the content store's packet limit).

/// Where a produced file came from, for storage ownership.
pub(crate) struct Origin<'a> {
    pub workspace: i64,
    pub workflow: i64,
    pub execution: i64,
    pub node: &'a str,
}

/// A stored file artifact: its public description and its binary reference.
pub(crate) struct FileArtifact {
    pub json: Value,
    pub binary: Asset,
}

/// A safe download name: no path parts or control characters, bounded length,
/// and the expected extension.
pub(crate) fn file_name(requested: &str, fallback: &str, extension: &str) -> String {
    let cleaned: String = requested
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let base = cleaned.trim_matches(['.', ' ', '_']).to_owned();
    let mut name = if base.is_empty() {
        fallback.to_owned()
    } else {
        base
    };
    let suffix = format!(".{extension}");
    if !name.to_ascii_lowercase().ends_with(&suffix) {
        name.push_str(&suffix);
    }
    if name.chars().count() > 160 {
        let keep: String = name.chars().take(160 - suffix.chars().count()).collect();
        name = format!("{keep}{suffix}");
    }
    name
}

fn too_large(limit: usize) -> ApiError {
    ApiError::localized(
        StatusCode::PAYLOAD_TOO_LARGE,
        Message::new("workflow.fileTooLarge")
            .with("limit", crate::content_assets::size_text(limit as i64)),
    )
}

/// Stores `bytes` as a run file kept by the workflow and describes it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn store(
    s: &AppState,
    storage: &Store<'_>,
    origin: &Origin<'_>,
    bytes: &[u8],
    mime: &str,
    name: &str,
    meta: Value,
    link_ttl_days: i64,
) -> Result<FileArtifact> {
    let policy = crate::accounts::resource_policy::load(&s.db).await?;
    if !ferry_protocol::resources::allows(policy.file_upload_bytes, bytes.len()) {
        return Err(too_large(ferry_protocol::resources::byte_limit(
            policy.file_upload_bytes,
        )));
    }
    let asset = storage
        .stage_bytes(
            origin.workspace,
            bytes,
            mime,
            origin.execution,
            origin.node,
            0,
        )
        .await?;
    describe(s, storage, origin, asset, mime, name, meta, link_ttl_days).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn store_file(
    s: &AppState,
    storage: &Store<'_>,
    origin: &Origin<'_>,
    path: &std::path::Path,
    mime: &str,
    name: &str,
    meta: Value,
    link_ttl_days: i64,
) -> Result<FileArtifact> {
    let asset = storage
        .stage_file(
            origin.workspace,
            path,
            mime,
            origin.execution,
            origin.node,
            0,
        )
        .await?;
    describe(s, storage, origin, asset, mime, name, meta, link_ttl_days).await
}

#[allow(clippy::too_many_arguments)]
async fn describe(
    s: &AppState,
    storage: &Store<'_>,
    origin: &Origin<'_>,
    asset: crate::entities::content_assets::Model,
    _mime: &str,
    name: &str,
    meta: Value,
    link_ttl_days: i64,
) -> Result<FileArtifact> {
    let prepared = storage.prepare(origin.workspace, asset.id).await?;
    let tx = s.db.begin().await?;
    prepared
        .attach(storage, &tx, "workflow", origin.workflow)
        .await?;
    tx.commit().await?;
    let link = crate::file_links::sign(
        s,
        origin.workspace,
        &asset.sha256,
        name,
        &asset.mime,
        link_ttl_days,
    )
    .await?;
    let binary = Asset {
        asset: format!("sha256:{}", asset.sha256),
        mime: asset.mime.clone(),
        name: name.into(),
        size: asset.size as u64,
    };
    let json = json!({
        "artifact_id": binary.asset,
        "name": name,
        "mime": asset.mime,
        "size": asset.size,
        "sha256": asset.sha256,
        "download_url": link.as_ref().map(|(url, _)| url.clone()).unwrap_or_default(),
        "expires_at": link.as_ref().and_then(|(_, at)| chrono::DateTime::from_timestamp_millis(*at)).map(|at| at.to_rfc3339()).unwrap_or_default(),
        "meta": meta,
    });
    Ok(FileArtifact { json, binary })
}

/// The input item with the file added under `field` (both its description in
/// `json` and its `binary` reference), keeping the upstream data.
pub(crate) fn with_file(item: &Item, field: &str, file: FileArtifact) -> Item {
    let mut json: Map<String, Value> = item.json.clone();
    json.insert(field.into(), file.json);
    let mut binary = item.binary.clone();
    binary.insert(field.into(), file.binary);
    Item {
        json,
        binary,
        paired: item.paired.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_names_are_safe_and_keep_their_extension() {
        assert_eq!(
            file_name("周报 2026/11", "report", "pdf"),
            "周报 2026_11.pdf"
        );
        assert_eq!(
            file_name("../../etc/passwd", "report", "pdf"),
            "etc_passwd.pdf"
        );
        assert_eq!(file_name("", "report", "pdf"), "report.pdf");
        assert_eq!(file_name("Weekly.PDF", "report", "pdf"), "Weekly.PDF");
        assert!(
            file_name(&"字".repeat(500), "report", "pdf")
                .chars()
                .count()
                <= 160
        );
    }
}
