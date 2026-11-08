//! Workflow-owned uploads and binary items use the same encrypted content store as runs.
use super::super::{
    accounts::workspace::Scope,
    artifact_store::ArtifactStore,
    content_assets::{DEFAULT_QUOTA, Store},
    definitions::access::{self, Operation},
    entities::{asset_refs, content_assets as assets},
    error::{ApiError, Json, Path, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ferry_protocol::sandbox::{Asset, Item};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, TransactionTrait};
use serde_json::{Value, json};

pub(super) const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{workspace}/workflows/{id}/assets",
            post(upload).layer(DefaultBodyLimit::max(MAX_FILE_BYTES + 1024 * 1024)),
        )
        .route("/{workspace}/workflows/{id}/assets/{hash}", get(download))
}
fn store<'a>(s: &'a AppState, backend: &'a ArtifactStore) -> Store<'a> {
    Store {
        db: &s.db,
        backend,
        vault: &s.vault,
        ids: &s.ids,
        quota: DEFAULT_QUOTA,
    }
}
async fn backend(s: &AppState) -> Result<ArtifactStore> {
    ArtifactStore::for_state(s).await
}
pub(super) fn invalid() -> ApiError {
    ApiError::localized(
        StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("workflow.mediaInvalid"),
    )
}

async fn upload(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, workflow)): Path<(String, Id)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("workflows.update")?;
    let row = super::scoped(&s.db, scope.workspace_id, workflow).await?;
    access::require(&s.db, &scope, &row, Operation::Edit).await?;
    let mut field = multipart
        .next_field()
        .await
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    if field.name() != Some("file") {
        return Err(invalid());
    }
    let name = field
        .file_name()
        .unwrap_or("file")
        .replace(['/', '\\', '\r', '\n', '\0'], "_");
    let mime = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    if name.is_empty() || name.len() > 255 || mime.len() > 128 {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| invalid())? {
        if chunk.len() > MAX_FILE_BYTES.saturating_sub(bytes.len()) {
            return Err(ApiError::localized(
                StatusCode::PAYLOAD_TOO_LARGE,
                crate::i18n::Message::new("workflow.mediaTooLarge"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let backend = backend(&s).await?;
    let store = store(&s, &backend);
    let asset = store
        .stage_bytes(
            scope.workspace_id.into(),
            &bytes,
            &mime,
            0,
            "upload",
            scope.user.id.into(),
        )
        .await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let scope =
        crate::accounts::workspace_admin::authorize(&tx, &scope, "workflows.update").await?;
    let row = super::scoped(&tx, scope.workspace_id, workflow).await?;
    access::require(&tx, &scope, &row, Operation::Edit).await?;
    store
        .attach(
            &tx,
            scope.workspace_id.into(),
            asset.id,
            "workflow",
            workflow.into(),
        )
        .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"asset":format!("sha256:{}",asset.sha256),"mime":asset.mime,"name":name,"size":asset.size}),
        ),
    ))
}
pub(super) async fn resolve(
    s: &AppState,
    workspace: i64,
    workflow: i64,
    reference: &Asset,
) -> Result<assets::Model> {
    let hash = reference
        .asset
        .strip_prefix("sha256:")
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(invalid)?;
    let asset = assets::Entity::find()
        .filter(assets::Column::WorkspaceId.eq(workspace))
        .filter(assets::Column::Sha256.eq(hash))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if asset_refs::Entity::find()
        .filter(asset_refs::Column::AssetId.eq(asset.id))
        .filter(asset_refs::Column::OwnerKind.eq("workflow"))
        .filter(asset_refs::Column::OwnerId.eq(workflow))
        .count(&s.db)
        .await?
        == 0
        || asset.size as u64 != reference.size
        || asset.mime != reference.mime
    {
        return Err(ApiError::missing());
    }
    Ok(asset)
}
async fn download(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, workflow, hash)): Path<(String, Id, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    scope.require("workflows.read")?;
    let row = super::scoped(&s.db, scope.workspace_id, workflow).await?;
    access::require(&s.db, &scope, &row, Operation::View).await?;
    let asset = assets::Entity::find()
        .filter(assets::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(assets::Column::Sha256.eq(&hash))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let reference = Asset {
        asset: format!("sha256:{hash}"),
        mime: asset.mime.clone(),
        name: "download".into(),
        size: asset.size as u64,
    };
    resolve(&s, scope.workspace_id.into(), workflow.into(), &reference).await?;
    content_response(&s, asset, headers).await
}

/// Shared byte response; callers must authorize the asset before invoking it.
pub(crate) async fn content_response(
    s: &AppState,
    asset: assets::Model,
    headers: HeaderMap,
) -> Result<Response> {
    let backend = backend(&s).await?;
    let bytes = store(s, &backend)
        .read_bytes(&s.db, asset.workspace_id, asset.id)
        .await?;
    media_response(bytes, &asset.mime, &headers, None)
}
/// Authenticated callers supply the bytes; active document types never render inline.
pub(crate) fn media_response(
    bytes: Vec<u8>,
    mime: &str,
    headers: &HeaderMap,
    download_name: Option<&str>,
) -> Result<Response> {
    let inline = download_name.is_none()
        && matches!(
            mime,
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/avif"
                | "audio/mpeg"
                | "audio/mp4"
                | "audio/ogg"
                | "audio/wav"
                | "audio/webm"
                | "video/mp4"
                | "video/webm"
                | "video/ogg"
        );
    let disposition = if let Some(name) = download_name {
        format!(
            "attachment; filename*=UTF-8''{}",
            percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC)
        )
    } else {
        if inline { "inline" } else { "attachment" }.to_owned()
    };
    let range = headers.get(header::RANGE).and_then(|h| h.to_str().ok());
    let (status, data, content_range) = if let Some(range) = range {
        let range = range
            .strip_prefix("bytes=")
            .filter(|s| !s.contains(','))
            .and_then(|s| s.split_once('-'));
        let bounds = range.and_then(|(start, end)| {
            if bytes.is_empty() {
                return None;
            }
            if start.is_empty() {
                let suffix = end.parse::<usize>().ok()?.min(bytes.len());
                if suffix == 0 {
                    return None;
                }
                Some((bytes.len() - suffix, bytes.len() - 1))
            } else {
                let start = start.parse::<usize>().ok()?;
                let end = if end.is_empty() {
                    bytes.len() - 1
                } else {
                    end.parse::<usize>().ok()?.min(bytes.len() - 1)
                };
                if start > end {
                    return None;
                }
                Some((start, end))
            }
        });
        let Some((start, end)) = bounds else {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{}", bytes.len()))],
            )
                .into_response());
        };
        (
            StatusCode::PARTIAL_CONTENT,
            bytes[start..=end].to_vec(),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        )
    } else {
        (StatusCode::OK, bytes, None)
    };
    let mut response = (
        status,
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::ACCEPT_RANGES, "bytes"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox",
            ),
        ],
        Body::from(data),
    )
        .into_response();
    if let Some(value) = content_range {
        response
            .headers_mut()
            .insert(header::CONTENT_RANGE, value.parse().map_err(|_| invalid())?);
    }
    Ok(response)
}
pub(super) fn supports(spec: &Value) -> bool {
    matches!(
        spec["type"].as_str(),
        Some("file.input" | "image.input" | "audio.input" | "video.input")
    ) || matches!(spec["type"].as_str(), Some("file.encode" | "file.decode"))
        && matches!(spec["params"]["format"].as_str(), Some("text" | "json"))
}
pub(super) async fn execute(
    s: &AppState,
    storage: &Store<'_>,
    workspace: i64,
    workflow: i64,
    execution: i64,
    node: &str,
    spec: &Value,
    items: &[Item],
    params: &[Value],
) -> Result<Vec<Item>> {
    let mut output = Vec::new();
    if spec["type"] == "file.encode" && spec["params"]["format"] == "json" {
        let p = params.first().ok_or_else(invalid)?;
        let rows: Vec<_> = items
            .iter()
            .map(|item| Value::Object(item.json.clone()))
            .collect();
        check_shape(&rows, p)?;
        let text = serde_json::to_string(&rows).map_err(|_| invalid())?;
        check_bytes(text.len(), p, "max_input_bytes")?;
        check_bytes(text.len(), p, "max_output_bytes")?;
        let name = p["output_name"].as_str().ok_or_else(invalid)?;
        let field = p["output_field"].as_str().ok_or_else(invalid)?;
        let asset = storage
            .stage_bytes(
                workspace,
                text.as_bytes(),
                "application/json",
                execution,
                node,
                0,
            )
            .await?;
        let tx = s.db.begin().await?;
        storage
            .attach(&tx, workspace, asset.id, "workflow", workflow)
            .await?;
        tx.commit().await?;
        let mut item = items.first().cloned().unwrap_or(Item {
            json: Default::default(),
            binary: Default::default(),
            paired: vec![],
        });
        item.paired = items.iter().flat_map(|i| i.paired.clone()).collect();
        item.binary.insert(
            field.into(),
            Asset {
                asset: format!("sha256:{}", asset.sha256),
                mime: asset.mime,
                name: name.into(),
                size: asset.size as u64,
            },
        );
        return Ok(vec![item]);
    }
    let mut input_bytes = 0usize;
    let mut output_bytes = 0usize;
    let mut output_rows = 0usize;
    for (index, item) in items.iter().enumerate() {
        let p = params
            .get(index)
            .or_else(|| params.first())
            .ok_or_else(invalid)?;
        let mut item = item.clone();
        let field = p["field"]
            .as_str()
            .or_else(|| p["output_field"].as_str())
            .ok_or_else(invalid)?;
        if field.is_empty() {
            return Err(invalid());
        }
        match spec["type"].as_str().unwrap_or_default() {
            "file.input" | "image.input" | "audio.input" | "video.input" => {
                let file: Asset =
                    serde_json::from_value(p["file"].clone()).map_err(|_| invalid())?;
                resolve(s, workspace, workflow, &file).await?;
                let category = spec["type"].as_str().unwrap().split('.').next().unwrap();
                if category != "file" && !file.mime.starts_with(&format!("{category}/")) {
                    return Err(invalid());
                }
                item.binary.insert(field.into(), file);
            }
            "file.encode" => {
                let value = Value::Object(item.json.clone());
                let path = p["value_path"]
                    .as_str()
                    .filter(|path| path.starts_with('/'))
                    .ok_or_else(invalid)?;
                let text = value
                    .pointer(path)
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?;
                input_bytes = input_bytes
                    .checked_add(serde_json::to_vec(&item.json).map_err(|_| invalid())?.len())
                    .ok_or_else(invalid)?;
                output_bytes = output_bytes.checked_add(text.len()).ok_or_else(invalid)?;
                check_bytes(input_bytes, p, "max_input_bytes")?;
                check_bytes(output_bytes, p, "max_output_bytes")?;
                check_shape(&[Value::String(text.into())], p)?;
                let name = p["output_name"].as_str().ok_or_else(invalid)?;
                let mime = if p["format"] == "json" {
                    "application/json"
                } else {
                    "text/plain"
                };
                let asset = storage
                    .stage_bytes(workspace, text.as_bytes(), mime, execution, node, 0)
                    .await?;
                let tx = s.db.begin().await?;
                storage
                    .attach(&tx, workspace, asset.id, "workflow", workflow)
                    .await?;
                tx.commit().await?;
                item.binary.insert(
                    field.into(),
                    Asset {
                        asset: format!("sha256:{}", asset.sha256),
                        mime: asset.mime,
                        size: asset.size as u64,
                        name: name.into(),
                    },
                );
            }
            "file.decode" => {
                let file = item
                    .binary
                    .values()
                    .find(|file| Some(file.asset.as_str()) == p["asset"].as_str())
                    .ok_or_else(invalid)?;
                let asset = resolve(s, workspace, workflow, file).await?;
                let bytes = storage.read_bytes(&s.db, workspace, asset.id).await?;
                if bytes.len() as u64
                    > p["limits"]["max_input_bytes"]
                        .as_u64()
                        .ok_or_else(invalid)?
                {
                    return Err(invalid());
                }
                let text = String::from_utf8(bytes).map_err(|_| invalid())?;
                if text.len() as u64
                    > p["limits"]["max_output_bytes"]
                        .as_u64()
                        .ok_or_else(invalid)?
                {
                    return Err(invalid());
                }
                input_bytes = input_bytes.checked_add(text.len()).ok_or_else(invalid)?;
                check_bytes(input_bytes, p, "max_input_bytes")?;
                let values = if p["format"] == "json" {
                    match serde_json::from_str(&text).map_err(|_| invalid())? {
                        Value::Array(rows) => rows,
                        value => vec![value],
                    }
                } else {
                    vec![Value::String(text)]
                };
                check_shape(&values, p)?;
                output_rows = output_rows.checked_add(values.len()).ok_or_else(invalid)?;
                if output_rows as u64 > p["limits"]["max_rows"].as_u64().ok_or_else(invalid)? {
                    return Err(invalid());
                }
                for value in values {
                    let mut parsed = item.clone();
                    parsed.json = match value {
                        Value::Object(object) => object,
                        value => serde_json::Map::from_iter([(field.into(), value)]),
                    };
                    output_bytes = output_bytes
                        .checked_add(
                            serde_json::to_vec(&parsed.json)
                                .map_err(|_| invalid())?
                                .len(),
                        )
                        .ok_or_else(invalid)?;
                    check_bytes(output_bytes, p, "max_output_bytes")?;
                    output.push(parsed);
                }
                continue;
            }
            _ => return Err(invalid()),
        }
        output.push(item);
    }
    Ok(output)
}
fn check_bytes(size: usize, p: &Value, key: &str) -> Result<()> {
    if size as u64 > p["limits"][key].as_u64().ok_or_else(invalid)? {
        return Err(invalid());
    }
    Ok(())
}
fn check_shape(rows: &[Value], p: &Value) -> Result<()> {
    let max_rows = p["limits"]["max_rows"].as_u64().ok_or_else(invalid)?;
    let max_columns = p["limits"]["max_columns"].as_u64().ok_or_else(invalid)?;
    if rows.len() as u64>max_rows||rows.iter().any(|v|match v {Value::Object(v)=>v.len(),Value::Array(v)=>v.len(),_=>1} as u64>max_columns){return Err(invalid());}
    Ok(())
}
