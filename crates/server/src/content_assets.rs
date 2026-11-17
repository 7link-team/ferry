//! Workspace-scoped content addressing and durable, two-phase object deletion.
//! Staging commits a quota reservation and a GC candidate BEFORE writing bytes.
//! A failed caller transaction can therefore leave only a tracked candidate.
use crate::{
    artifact_store::ArtifactStore,
    entities::{asset_gc_jobs as jobs, asset_refs as refs, content_assets as assets},
    error::{ApiError, Result},
    i18n::Message,
    snowflake::Generator,
    vault::Vault,
};
use axum::http::StatusCode;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QuerySelect, TransactionTrait, sea_query::Expr,
};
use sha2::{Digest, Sha256};

pub(crate) const DEFAULT_QUOTA: i64 = 50 * 1024 * 1024 * 1024;
pub(crate) const GRACE_MS: i64 = 24 * 60 * 60 * 1000;
const PACKET_LIMIT: usize = 32 * 1024 * 1024;

/// A broken invariant on stored content: logged with an incident id.
#[track_caller]
fn fault() -> ApiError {
    crate::error::internal_fault("content asset invariant")
}
/// The object store failed a real read/write; administrators fix the storage.
fn storage(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "file storage operation failed");
    crate::artifact_store::storage_unavailable()
}
fn busy() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("storage.busy"))
}
fn size_text(bytes: i64) -> String {
    let gib = 1024 * 1024 * 1024;
    if bytes >= gib && bytes % gib == 0 {
        format!("{} GiB", bytes / gib)
    } else {
        format!("{} MiB", bytes / (1024 * 1024))
    }
}
pub(crate) struct Store<'a> {
    pub db: &'a DatabaseConnection,
    pub backend: &'a ArtifactStore,
    pub vault: &'a Vault,
    pub ids: &'a Generator,
    pub quota: i64,
}
fn key(asset: &assets::Model) -> String {
    // A new metadata identity means a new physical generation. A delayed DELETE
    // of an old generation cannot delete a later upload with the same hash.
    format!(
        "assets/{}/{}/{}",
        asset.workspace_id, asset.sha256, asset.id
    )
}
fn aad(asset: &assets::Model) -> String {
    format!("asset:{}:{}:{}", asset.workspace_id, asset.id, asset.sha256)
}
async fn lock(tx: &DatabaseTransaction, scope: &str) -> Result<()> {
    let owner = uuid::Uuid::new_v4().to_string();
    if !crate::lock::acquire(tx, scope, &owner, crate::db::now(), 60_000).await? {
        return Err(busy());
    }
    // The row write lock stays held until commit even after logical release.
    crate::lock::release(tx, scope, &owner).await?;
    Ok(())
}
async fn asset_lock(tx: &DatabaseTransaction, workspace: i64, id: i64) -> Result<()> {
    lock(tx, &format!("content-asset:{workspace}:{id}")).await
}
async fn job(db: &impl ConnectionTrait, workspace: i64, id: i64) -> Result<Option<jobs::Model>> {
    Ok(jobs::Entity::find()
        .filter(jobs::Column::WorkspaceId.eq(workspace))
        .filter(jobs::Column::AssetId.eq(id))
        .one(db)
        .await?)
}
async fn row(db: &impl ConnectionTrait, workspace: i64, id: i64) -> Result<assets::Model> {
    assets::Entity::find_by_id(id)
        .filter(assets::Column::WorkspaceId.eq(workspace))
        .one(db)
        .await?
        .ok_or_else(ApiError::missing)
}
impl Store<'_> {
    /// The plaintext is canonical JSON. Shared ciphertext is bound to the asset;
    /// the caller separately encrypts its locator with execution/field AAD.
    pub(crate) async fn stage(
        &self,
        workspace: i64,
        plain: &str,
        execution: i64,
        node: &str,
    ) -> Result<assets::Model> {
        self.stage_bytes(
            workspace,
            plain.as_bytes(),
            "application/json",
            execution,
            node,
            0,
        )
        .await
    }
    pub(crate) async fn stage_bytes(
        &self,
        workspace: i64,
        plain: &[u8],
        mime: &str,
        execution: i64,
        node: &str,
        actor: i64,
    ) -> Result<assets::Model> {
        if plain.len() > PACKET_LIMIT {
            return Err(ApiError::localized(
                StatusCode::PAYLOAD_TOO_LARGE,
                Message::new("storage.tooLarge").with("limit", size_text(PACKET_LIMIT as i64)),
            ));
        }
        if workspace <= 0 || self.quota < 0 {
            return Err(fault());
        }
        let hash = hex::encode(Sha256::digest(plain));
        let tx = self.db.begin().await?;
        lock(&tx, &format!("content-quota:{workspace}")).await?;
        let existing = assets::Entity::find()
            .filter(assets::Column::WorkspaceId.eq(workspace))
            .filter(assets::Column::Sha256.eq(&hash))
            .one(&tx)
            .await?;
        let asset = if let Some(asset) = existing {
            asset
        } else {
            let sizes: Vec<i64> = assets::Entity::find()
                .select_only()
                .column(assets::Column::Size)
                .filter(assets::Column::WorkspaceId.eq(workspace))
                .into_tuple()
                .all(&tx)
                .await?;
            let used = sizes
                .into_iter()
                .try_fold(
                    0_i64,
                    |sum, n| if n < 0 { None } else { sum.checked_add(n) },
                )
                .ok_or_else(|| fault())?;
            let size = i64::try_from(plain.len()).map_err(|_| fault())?;
            if used.checked_add(size).ok_or_else(|| fault())? > self.quota {
                return Err(ApiError::localized(
                    StatusCode::CONFLICT,
                    Message::new("storage.quotaExceeded").with("quota", size_text(self.quota)),
                ));
            }
            let now = crate::db::now();
            let asset = assets::Model {
                id: self.ids.next_id().map_err(|_| fault())?.into(),
                workspace_id: workspace,
                sha256: hash,
                size,
                mime: mime.into(),
                kind: mime
                    .split('/')
                    .next()
                    .filter(|kind| matches!(*kind, "image" | "audio" | "video"))
                    .unwrap_or("file")
                    .into(),
                width: None,
                height: None,
                duration_ms: None,
                thumbnail_asset_id: None,
                execution_id: (execution > 0).then_some(execution),
                node_id: Some(node.into()),
                pinned: 0,
                created_at: now,
                created_by: actor,
                version: 1,
                provenance: None,
            };
            assets::Entity::insert(asset.clone().into_active_model())
                .exec(&tx)
                .await?;
            candidate(&tx, &asset, now).await?;
            asset
        };
        tx.commit().await?;
        // Candidate metadata survives a process crash or upload/commit failure.
        let tx = self.db.begin().await?;
        asset_lock(&tx, workspace, asset.id).await?;
        let asset = row(&tx, workspace, asset.id).await?;
        if job(&tx, workspace, asset.id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        let encrypted = self.vault.seal_bytes(plain, &aad(&asset))?;
        self.backend
            .create(&key(&asset), encrypted.into())
            .await
            .map_err(storage)?;
        let stored = self.read_bytes_row(&asset).await?;
        if stored != plain {
            return Err(storage("object read back differs from what was written"));
        }
        tx.commit().await?;
        Ok(asset)
    }
    async fn read_row(&self, asset: &assets::Model) -> Result<String> {
        String::from_utf8(self.read_bytes_row(asset).await?).map_err(|_| fault())
    }
    async fn read_bytes_row(&self, asset: &assets::Model) -> Result<Vec<u8>> {
        let limit = usize::try_from(asset.size).map_err(|_| fault())?;
        if limit > PACKET_LIMIT {
            return Err(fault());
        }
        let bytes = self
            .backend
            .get(
                &key(asset),
                limit
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(128))
                    .ok_or_else(|| fault())?,
            )
            .await
            .map_err(storage)?;
        let cipher = std::str::from_utf8(&bytes).map_err(|_| fault())?;
        let plain = self
            .vault
            .open_bytes(cipher, &aad(asset))
            .map_err(|_| fault())?;
        if plain.len() != limit || hex::encode(Sha256::digest(&plain)) != asset.sha256 {
            return Err(fault());
        }
        Ok(plain)
    }
    pub(crate) async fn read_bytes(
        &self,
        db: &impl ConnectionTrait,
        workspace: i64,
        id: i64,
    ) -> Result<Vec<u8>> {
        let asset = row(db, workspace, id).await?;
        if job(db, workspace, id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        self.read_bytes_row(&asset).await
    }
    pub(crate) async fn read(
        &self,
        db: &impl ConnectionTrait,
        workspace: i64,
        id: i64,
    ) -> Result<String> {
        let asset = row(db, workspace, id).await?;
        if job(db, workspace, id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        self.read_row(&asset).await
    }
    pub(crate) async fn attach_binary(
        &self,
        tx: &DatabaseTransaction,
        workspace: i64,
        reference: &ferry_protocol::sandbox::Asset,
        owner_kind: &str,
        owner_id: i64,
    ) -> Result<()> {
        let hash = reference
            .asset
            .strip_prefix("sha256:")
            .ok_or_else(|| fault())?;
        let asset = assets::Entity::find()
            .filter(assets::Column::WorkspaceId.eq(workspace))
            .filter(assets::Column::Sha256.eq(hash))
            .one(tx)
            .await?
            .ok_or_else(|| fault())?;
        if u64::try_from(asset.size).ok() != Some(reference.size) || asset.mime != reference.mime {
            return Err(fault());
        }
        self.attach(tx, workspace, asset.id, owner_kind, owner_id)
            .await
    }

    pub(crate) async fn attach(
        &self,
        tx: &DatabaseTransaction,
        workspace: i64,
        id: i64,
        owner_kind: &str,
        owner_id: i64,
    ) -> Result<()> {
        if !matches!(
            owner_kind,
            "execution"
                | "node_run"
                | "cache"
                | "pin"
                | "user_pin"
                | "workflow"
                | "conversation_file"
        ) || owner_id <= 0
        {
            return Err(fault());
        }
        asset_lock(tx, workspace, id).await?;
        let asset = row(tx, workspace, id).await?;
        if job(tx, workspace, id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        self.read_bytes_row(&asset).await?;
        refs::Entity::insert(
            refs::Model {
                asset_id: id,
                owner_kind: owner_kind.into(),
                owner_id,
                version: 1,
            }
            .into_active_model(),
        )
        .on_conflict_do_nothing()
        .exec_without_returning(tx)
        .await?;
        jobs::Entity::delete_many()
            .filter(jobs::Column::WorkspaceId.eq(workspace))
            .filter(jobs::Column::AssetId.eq(id))
            .exec(tx)
            .await?;
        Ok(())
    }
    pub(crate) async fn pin(
        &self,
        tx: &DatabaseTransaction,
        workspace: i64,
        id: i64,
        pinned: bool,
    ) -> Result<()> {
        asset_lock(tx, workspace, id).await?;
        let asset = row(tx, workspace, id).await?;
        if job(tx, workspace, id)
            .await?
            .is_some_and(|j| j.state == "deleting")
        {
            return Err(ApiError::missing());
        }
        assets::Entity::update_many()
            .col_expr(assets::Column::Pinned, Expr::value(i64::from(pinned)))
            .col_expr(
                assets::Column::Version,
                Expr::value(asset.version.checked_add(1).ok_or_else(|| fault())?),
            )
            .filter(assets::Column::Id.eq(id))
            .exec(tx)
            .await?;
        if pinned {
            self.read_bytes_row(&asset).await?;
            jobs::Entity::delete_many()
                .filter(jobs::Column::AssetId.eq(id))
                .exec(tx)
                .await?;
        } else if refs::Entity::find()
            .filter(refs::Column::AssetId.eq(id))
            .count(tx)
            .await?
            == 0
        {
            candidate(tx, &asset, crate::db::now()).await?;
        }
        Ok(())
    }
    /// Mark under the same lock used by attach, then delete outside the DB tx.
    /// Errors leave the exact generation in `deleting` for a later retry.
    pub(crate) async fn collect(&self, workspace: i64, id: i64, now: i64) -> Result<bool> {
        let tx = self.db.begin().await?;
        asset_lock(&tx, workspace, id).await?;
        let Some(mut gc) = job(&tx, workspace, id).await? else {
            tx.rollback().await?;
            return Ok(false);
        };
        let asset = row(&tx, workspace, id).await?;
        if gc.storage_key != key(&asset) {
            return Err(fault());
        }
        if gc.state == "candidate" {
            if gc.eligible_at > now {
                tx.rollback().await?;
                return Ok(false);
            }
            if asset.pinned != 0
                || refs::Entity::find()
                    .filter(refs::Column::AssetId.eq(id))
                    .count(&tx)
                    .await?
                    != 0
            {
                jobs::Entity::delete_by_id(gc.id).exec(&tx).await?;
                tx.commit().await?;
                return Ok(false);
            }
        } else if gc.state != "deleting" {
            return Err(fault());
        }
        gc.state = "deleting".into();
        gc.attempt = gc.attempt.checked_add(1).ok_or_else(|| fault())?;
        gc.updated_at = now;
        gc.version = gc.version.checked_add(1).ok_or_else(|| fault())?;
        if jobs::Entity::update_many()
            .col_expr(jobs::Column::State, Expr::value(&gc.state))
            .col_expr(jobs::Column::Attempt, Expr::value(gc.attempt))
            .col_expr(jobs::Column::UpdatedAt, Expr::value(gc.updated_at))
            .col_expr(jobs::Column::Version, Expr::value(gc.version))
            .filter(jobs::Column::Id.eq(gc.id))
            .filter(jobs::Column::Version.eq(gc.version - 1))
            .exec(&tx)
            .await?
            .rows_affected
            != 1
        {
            return Err(fault());
        }
        tx.commit().await?;
        if let Err(error) = self.backend.delete(&gc.storage_key).await {
            jobs::Entity::update_many()
                .col_expr(jobs::Column::Error, Expr::value("object deletion failed"))
                .col_expr(jobs::Column::UpdatedAt, Expr::value(crate::db::now()))
                .col_expr(
                    jobs::Column::Version,
                    Expr::value(gc.version.checked_add(1).ok_or_else(|| fault())?),
                )
                .filter(jobs::Column::Id.eq(gc.id))
                .filter(jobs::Column::Version.eq(gc.version))
                .exec(self.db)
                .await?;
            return Err(storage(error));
        }
        let tx = self.db.begin().await?;
        asset_lock(&tx, workspace, id).await?;
        // Concurrent collectors may have finished already. No generation is reused.
        if let Some(current) = job(&tx, workspace, id).await? {
            if current.state != "deleting" || current.storage_key != gc.storage_key {
                return Err(fault());
            }
            refs::Entity::delete_many()
                .filter(refs::Column::AssetId.eq(id))
                .exec(&tx)
                .await?;
            tx.execute_unprepared(&format!("DELETE FROM asset_tags WHERE asset_id={id}"))
                .await?;
            assets::Entity::delete_by_id(id).exec(&tx).await?;
            jobs::Entity::delete_by_id(current.id).exec(&tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}
async fn candidate(tx: &DatabaseTransaction, asset: &assets::Model, now: i64) -> Result<()> {
    jobs::Entity::insert(
        jobs::Model {
            id: asset.id,
            workspace_id: asset.workspace_id,
            asset_id: asset.id,
            storage_key: key(asset),
            state: "candidate".into(),
            eligible_at: now.checked_add(GRACE_MS).ok_or_else(|| fault())?,
            eligible_by: 0,
            attempt: 0,
            error: None,
            updated_at: now,
            updated_by: 0,
            version: 1,
        }
        .into_active_model(),
    )
    .on_conflict_do_nothing()
    .exec_without_returning(tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

/// Drops one owner's reference to an asset and, when nothing else uses it,
/// queues the bytes for cleanup. Database-only: removing references never
/// needs the file storage itself, so deletes work while storage is down.
pub(crate) async fn detach_reference(
    tx: &DatabaseTransaction,
    workspace: i64,
    id: i64,
    owner_kind: &str,
    owner_id: i64,
) -> Result<()> {
    asset_lock(tx, workspace, id).await?;

    let asset = row(tx, workspace, id).await?;
    refs::Entity::delete_by_id((id, owner_kind.to_owned(), owner_id))
        .exec(tx)
        .await?;
    if asset.pinned == 0
        && refs::Entity::find()
            .filter(refs::Column::AssetId.eq(id))
            .count(tx)
            .await?
            == 0
    {
        candidate(tx, &asset, crate::db::now()).await?;
    }
    Ok(())
}
