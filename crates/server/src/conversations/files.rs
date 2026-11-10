use super::*;
use crate::{
    artifact_store::ArtifactStore,
    content_assets::{DEFAULT_QUOTA, Store},
    entities::ai_chat_files as files,
};
use axum::{
    extract::{DefaultBodyLimit, Multipart},
    http::HeaderMap,
    response::Response,
};
const MAX_BYTES: usize = 32 * 1024 * 1024;
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{workspace}/conversations/{id}/files",
            post(upload).layer(DefaultBodyLimit::max(MAX_BYTES + 1024 * 1024)),
        )
        .route(
            "/{workspace}/conversations/{id}/files/{file}",
            get(download).delete(remove),
        )
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
pub(super) fn view(f: &files::Model) -> Value {
    json!({"file_id":Id::from(f.id),"name":f.name,"mime":f.mime,"size":f.size})
}
async fn upload(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    mut data: Multipart,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("conversations.run")?;
    let c = find(&s.db, &scope, id).await?;
    if c.state != "active" {
        return Err(conflict());
    }
    let mut field = data
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
    let declared_audio = field
        .content_type()
        .is_some_and(|mime| mime.split(';').next() == Some("audio/webm"));
    if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| invalid())? {
        if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "conversations.fileTooLarge",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    // MIME comes from a bounded extension mapping, never arbitrary browser headers.
    let guessed = mime_guess::from_path(&name)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();
    let mime = match guessed.as_str() {
        "video/webm" if declared_audio => "audio/webm".to_owned(),
        "image/svg+xml" | "text/html" | "application/xhtml+xml" => {
            "application/octet-stream".to_owned()
        }
        _ => guessed,
    };
    let backend = ArtifactStore::for_state(&s).await?;
    let storage = store(&s, &backend);
    let asset = storage
        .stage_bytes(
            scope.workspace_id.into(),
            &bytes,
            &mime,
            0,
            "conversation-upload",
            scope.user.id.into(),
        )
        .await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let c = find(&tx, &scope, id).await?;
    if c.state != "active" {
        return Err(conflict());
    }
    let file = files::Model {
        id: s.next_id()?.into(),
        workspace_id: scope.workspace_id.into(),
        conversation_id: id.into(),
        owner_id: scope.user.id.into(),
        asset_id: asset.id,
        size: asset.size,
        name,
        mime,
        created_at: db::now(),
    };
    storage
        .attach(
            &tx,
            file.workspace_id,
            file.asset_id,
            "conversation_file",
            file.id,
        )
        .await?;
    files::Entity::insert(file.clone().into_active_model())
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view(&file))))
}
pub(super) async fn resolve(
    conn: &impl ConnectionTrait,
    workspace: Id,
    conversation: Id,
    file: Id,
) -> Result<files::Model> {
    files::Entity::find_by_id(file)
        .filter(files::Column::WorkspaceId.eq(workspace))
        .filter(files::Column::ConversationId.eq(conversation))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
#[derive(Deserialize)]
pub(super) struct Download {
    #[serde(default)]
    download: bool,
}
async fn download(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, file)): Path<(String, Id, Id)>,
    Query(query): Query<Download>,
    headers: HeaderMap,
) -> Result<Response> {
    find(&s.db, &scope, id).await?;
    let f = resolve(&s.db, scope.workspace_id, id, file).await?;
    // Keep asset references for retention/cleanup, but a withdrawn message must
    // not leave an otherwise unshared attachment downloadable by its old URL.
    use crate::entities::ai_chat_messages as chat;
    let references = chat::Entity::find()
        .filter(chat::Column::ConversationId.eq(id))
        .filter(chat::Column::Attachments.contains(format!("\"file_id\":\"{file}\"")))
        .all(&s.db)
        .await?;
    if !references.is_empty() && references.iter().all(|m| m.retracted_at.is_some()) {
        return Err(ApiError::missing());
    }
    let backend = ArtifactStore::for_state(&s).await?;
    let bytes = store(&s, &backend)
        .read_bytes(&s.db, f.workspace_id, f.asset_id)
        .await?;
    crate::workflows::media::media_response(
        bytes,
        &f.mime,
        &headers,
        query.download.then_some(f.name.as_str()),
    )
}
async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, file)): Path<(String, Id, Id)>,
) -> Result<Json<Value>> {
    scope.require("conversations.run")?;
    let backend = ArtifactStore::for_state(&s).await?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let conversation = find(&tx, &scope, id).await?;
    if serde_json::from_str::<Value>(&conversation.config)
        .ok()
        .and_then(|v| v["appearance"]["image"].as_str().map(str::to_owned))
        .as_deref()
        == Some(file.to_string().as_str())
    {
        return Err(conflict());
    }
    let f = resolve(&tx, scope.workspace_id, id, file).await?;
    if f.owner_id != i64::from(scope.user.id) {
        return Err(ApiError::missing());
    }
    // Only unsent draft uploads can be removed independently of their conversation.
    let needle = format!("\"file_id\":\"{}\"", file);
    if turns::Entity::find()
        .filter(turns::Column::ConversationId.eq(id))
        .filter(turns::Column::Attachments.contains(&needle))
        .count(&tx)
        .await?
        > 0
        || crate::entities::ai_chat_messages::Entity::find()
            .filter(crate::entities::ai_chat_messages::Column::ConversationId.eq(id))
            .filter(crate::entities::ai_chat_messages::Column::Attachments.contains(&needle))
            .count(&tx)
            .await?
            > 0
    {
        return Err(conflict());
    }
    store(&s, &backend)
        .detach(&tx, f.workspace_id, f.asset_id, "conversation_file", f.id)
        .await?;
    files::Entity::delete_by_id(file).exec(&tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
pub(super) async fn canonicalize(
    s: &AppState,
    scope: &Scope,
    id: Id,
    attachments: &mut [Attachment],
) -> Result<()> {
    let backend = ArtifactStore::for_state(s).await?;
    let mut text_budget = 24 * 1024;
    for a in attachments {
        if let Some(file) = a.file_id {
            let f = resolve(&s.db, scope.workspace_id, id, file).await?;
            a.name = f.name;
            a.mime = f.mime;
            a.size = f.size as u64;
            a.text.clear();
            if f.size <= text_budget as i64
                && (a.mime.starts_with("text/")
                    || matches!(
                        a.mime.as_str(),
                        "application/json" | "application/yaml" | "application/toml"
                    ))
            {
                let bytes = store(s, &backend)
                    .read_bytes(&s.db, f.workspace_id, f.asset_id)
                    .await?;
                if let Ok(text) = String::from_utf8(bytes) {
                    if !text.contains('\0') {
                        text_budget -= text.len();
                        a.text = text;
                    }
                }
            }
        } else {
            a.mime.clear();
            a.size = 0;
        }
    }
    Ok(())
}
pub(super) async fn validate_refs(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    id: Id,
    attachments: &[Attachment],
) -> Result<()> {
    for a in attachments {
        if let Some(file) = a.file_id {
            resolve(conn, scope.workspace_id, id, file).await?;
        }
    }
    Ok(())
}
pub(super) async fn detach_all(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    id: Id,
    backend: &ArtifactStore,
) -> Result<()> {
    for f in files::Entity::find()
        .filter(files::Column::ConversationId.eq(id))
        .all(tx)
        .await?
    {
        store(s, backend)
            .detach(tx, f.workspace_id, f.asset_id, "conversation_file", f.id)
            .await?;
    }
    files::Entity::delete_many()
        .filter(files::Column::ConversationId.eq(id))
        .exec(tx)
        .await?;
    Ok(())
}
