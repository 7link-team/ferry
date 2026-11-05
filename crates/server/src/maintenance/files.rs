//! Workspace-scoped attachments for typed dictionary/configuration values.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    error::{ApiError, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_BYTES: usize = 50 * 1024 * 1024;
#[derive(Deserialize)]
pub struct UploadQuery {
    resource: String,
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    id: Id,
    url: String,
    name: String,
    size: usize,
    content_type: String,
    resource: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{workspace}/value-files",
            post(upload).layer(DefaultBodyLimit::max(MAX_BYTES + 1024 * 1024)),
        )
        .route("/{workspace}/value-files/{id}", get(download))
}
fn resource(value: &str) -> Result<()> {
    if matches!(value, "dictionaries" | "settings") {
        Ok(())
    } else {
        Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("files.resourceInvalid"),
        ))
    }
}
fn directory(s: &AppState, workspace: Id, id: Id) -> std::path::PathBuf {
    s.data_dir
        .join("value-files")
        .join(workspace.to_string())
        .join(id.to_string())
}
fn io_error(error: std::io::Error) -> ApiError {
    tracing::error!(%error, "typed value attachment storage failed");
    ApiError::localized(
        StatusCode::INTERNAL_SERVER_ERROR,
        crate::i18n::Message::new("files.storageUnavailable"),
    )
}
async fn metadata(s: &AppState, workspace: Id, id: Id) -> Result<Metadata> {
    let path = directory(s, workspace, id).join("metadata.json");
    if !path.exists() && crate::artifact_store::ArtifactStore::is_remote(s).await? {
        crate::artifact_store::ArtifactStore::for_state(s)
            .await?
            .download_file(
                &format!("value-files/{workspace}/{id}/metadata.json"),
                &path,
            )
            .await
            .map_err(|_| ApiError::missing())?;
    }
    let bytes = tokio::fs::read(directory(s, workspace, id).join("metadata.json"))
        .await
        .map_err(|_| ApiError::missing())?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::missing())
}
async fn upload(
    State(s): State<AppState>,
    scope: Scope,
    Query(query): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Value>)> {
    resource(&query.resource)?;
    if scope
        .require(&format!("{}.create", query.resource))
        .is_err()
    {
        scope.require(&format!("{}.update", query.resource))?;
    }
    let mut field = multipart
        .next_field()
        .await
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.multipartInvalid"),
            )
        })?
        .ok_or_else(|| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.required"),
            )
        })?;
    if field.name() != Some("file") {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("files.fieldRequired"),
        ));
    }
    let name = field
        .file_name()
        .unwrap_or("attachment")
        .replace(['/', '\\', '\r', '\n', '\0'], "_");
    if name.is_empty() || name.len() > 255 {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("files.filenameInvalid"),
        ));
    }
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    let id = s.next_id()?;
    let dir = directory(&s, scope.workspace_id, id);
    tokio::fs::create_dir_all(&dir).await.map_err(io_error)?;
    let result = async {
        let mut file = tokio::fs::File::create(dir.join("content"))
            .await
            .map_err(io_error)?;
        let mut size = 0;
        while let Some(chunk) = field.chunk().await.map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.dataInvalid"),
            )
        })? {
            size += chunk.len();
            if size > MAX_BYTES {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("files.tooLarge"),
                ));
            }
            file.write_all(&chunk).await.map_err(io_error)?;
        }
        drop(field);
        file.flush().await.map_err(io_error)?;
        let meta = Metadata {
            id,
            url: format!("/api/workspaces/{}/value-files/{id}", scope.workspace_id),
            name,
            size,
            content_type,
            resource: query.resource,
        };
        let bytes = serde_json::to_vec(&meta).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.metadataInvalid"),
            )
        })?;
        tokio::fs::write(dir.join("metadata.json"), bytes)
            .await
            .map_err(io_error)?;
        if crate::artifact_store::ArtifactStore::is_remote(&s).await? {
            let backend = crate::artifact_store::ArtifactStore::for_state(&s).await?;
            for name in ["content", "metadata.json"] {
                backend
                    .upload_file(
                        &format!("value-files/{}/{id}/{name}", scope.workspace_id),
                        &dir.join(name),
                    )
                    .await
                    .map_err(|_| ApiError::missing())?;
            }
        }
        Ok((StatusCode::CREATED, Json(json!(meta))))
    }
    .await;
    let result = match result {
        Ok(value) => match multipart.next_field().await {
            Ok(None) => Ok(value),
            _ => Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.oneAtATime"),
            )),
        },
        Err(error) => Err(error),
    };
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    result
}
async fn download(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Response> {
    let meta = metadata(&s, scope.workspace_id, id).await?;
    resource(&meta.resource)?;
    scope.require(&format!("{}.read", meta.resource))?;
    let path = directory(&s, scope.workspace_id, id).join("content");
    if !path.exists() && crate::artifact_store::ArtifactStore::is_remote(&s).await? {
        crate::artifact_store::ArtifactStore::for_state(&s)
            .await?
            .download_file(
                &format!("value-files/{}/{id}/content", scope.workspace_id),
                &path,
            )
            .await
            .map_err(|_| ApiError::missing())?;
    }
    let file = tokio::fs::File::open(directory(&s, scope.workspace_id, id).join("content"))
        .await
        .map_err(|_| ApiError::missing())?;
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0; 64 * 1024];
        let length = file.read(&mut buffer).await?;
        buffer.truncate(length);
        Ok::<_, std::io::Error>(if length == 0 {
            None
        } else {
            Some((buffer, file))
        })
    });
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
pub async fn validate_references(
    s: &AppState,
    workspace: Id,
    kind: &str,
    value: &Value,
    schema: &Value,
) -> Result<()> {
    for item in value.as_array().ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("files.listRequired"),
        )
    })? {
        let id: Id = serde_json::from_value(item.get("id").cloned().unwrap_or(Value::Null))
            .map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("files.uploadRequired"),
                )
            })?;
        let meta = metadata(s, workspace, id).await?;
        if meta.resource != kind
            || item.get("url").and_then(Value::as_str) != Some(meta.url.as_str())
            || item.get("size").and_then(Value::as_u64) != Some(meta.size as u64)
            || item.get("content_type").and_then(Value::as_str) != Some(meta.content_type.as_str())
            || item.get("name").and_then(Value::as_str) != Some(meta.name.as_str())
        {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("files.metadataMismatch"),
            ));
        }
        if let Some(max) = schema
            .get("maxSizeMb")
            .and_then(Value::as_f64)
            .filter(|n| *n > 0.0)
        {
            if meta.size as f64 > max * 1024.0 * 1024.0 {
                return Err(ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("files.configuredSizeExceeded"),
                ));
            }
        }
    }
    Ok(())
}
