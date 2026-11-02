//! Immutable build outputs streamed from HostChannel, retained with run history.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::workspace::Scope,
    db,
    entities::{pipeline_artifacts as artifacts, pipeline_runs},
    error::{ApiError, Result},
    hosts::channel::HostChannel,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::{AppState, State as ServerState},
};
use axum::{body::Body, extract::State, http::header, response::IntoResponse};
use futures::StreamExt;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SPEC: Spec = Spec {
    search: &["path"],
    filters: &["pipeline_id", "run_id"],
    sortable: &["created_at", "size"],
    default_sort: "-created_at",
};

pub fn validate_paths(paths: &[String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    if paths.len() > 100 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.fileLimit"),
        ));
    }
    for path in paths {
        if path.len() > 1024
            || path.chars().any(char::is_control)
            || path.contains('\\')
            || path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || path.contains("{{")
            || !seen.insert(path)
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.pathsInvalid"),
            ));
        }
        if path.contains('*') || path.contains('?') || path.contains('[') {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.wildcardsUnsupported"),
            ));
        }
    }
    Ok(())
}
pub fn directory(s: &ServerState, run: i64) -> PathBuf {
    s.data_dir.join("artifacts").join(run.to_string())
}
pub fn file_path(s: &ServerState, row: &artifacts::Model) -> PathBuf {
    directory(s, row.run_id).join(row.id.to_string())
}

/// The archive name identifies the original logical path, independently of the
/// common-root stripping performed by upload-artifact.
pub(crate) fn legacy_artifact_name(path: &str) -> String {
    format!("legacy-{}", hex::encode(Sha256::digest(path.as_bytes())))
}

fn archive_name(path: &str) -> Option<&str> {
    let mut parts = path.rsplit('/');
    let archive = parts.next()?;
    let name = parts.next()?;
    if archive != format!("{name}.zip") {
        return None;
    }
    Some(name)
}

/// upload-artifact normalizes Unix permissions. Capture the source metadata
/// before the job's source root is cleaned, using the existing host transport.
pub(crate) async fn legacy_modes(
    ch: &dyn HostChannel,
    workdir: &str,
    paths: &[String],
    logical_paths: &[String],
) -> Result<std::collections::BTreeMap<String, u32>> {
    let mut modes = std::collections::BTreeMap::new();
    for path in paths {
        if let Some(logical) = logical_paths.iter().find(|logical| {
            archive_name(path).is_some_and(|name| name == legacy_artifact_name(logical))
        }) {
            let (meta, _) = ch
                .read_file(&format!("{}/{logical}", workdir.trim_end_matches('/')))
                .await?;
            modes.insert(path.clone(), meta.mode & 0o777);
        }
    }
    Ok(modes)
}

fn restore_legacy_mode(path: &std::path::Path, mode: u32) -> Result<(String, u64)> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.zipInvalid"),
        )
    })?;
    let temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    let mut writer = zip::ZipWriter::new(temporary);
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.zipMemberUnreadable"),
            )
        })?;
        let name = entry
            .name()
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.zipFilenameInvalid"),
                )
            })?
            .into_owned();
        let options = zip::write::SimpleFileOptions::default()
            .system(zip::System::Unix)
            .compression_method(entry.compression())
            .large_file(entry.size() >= 0xffff_ffff)
            .unix_permissions(if entry.is_dir() {
                entry.unix_mode().unwrap_or(0o755)
            } else {
                mode
            });
        if entry.is_dir() {
            writer.add_directory(&name, options).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.directoryModeFailed"),
                )
            })?;
        } else {
            writer.start_file(&name, options).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.fileModeFailed"),
                )
            })?;
            std::io::copy(&mut entry, &mut writer)?;
        }
    }
    drop(archive);
    let temporary = writer.finish().map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.zipModeFailed"),
        )
    })?;
    temporary.as_file().sync_all()?;
    let mut file = temporary
        .persist(path)
        .map_err(|error| ApiError::from(error.error))?;
    use std::io::{Read, Seek};
    file.rewind()?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        size += count as u64;
    }
    Ok((hex::encode(hash.finalize()), size))
}
fn public(r: &artifacts::Model) -> Value {
    json!({"id":Id::from(r.id),"pipeline_id":Id::from(r.pipeline_id),"run_id":Id::from(r.run_id),"path":r.path,"sha256":r.sha256,"size":r.size,"mode":r.mode,"created_at":r.created_at,"expires_at":r.expires_at})
}
pub async fn for_run(s: &ServerState, workspace: i64, run: i64) -> Result<Value> {
    let rows = artifacts::Entity::find()
        .filter(artifacts::Column::WorkspaceId.eq(workspace))
        .filter(artifacts::Column::RunId.eq(run))
        .all(&s.db)
        .await?;
    Ok(json!(rows.iter().map(public).collect::<Vec<_>>()))
}
pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("pipelines.read")?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list(
        &s.db,
        &SPEC,
        artifacts::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    let mut items: Vec<Value> = rows.iter().map(public).collect();
    crate::resources::names::attach(
        &s.db,
        scope.workspace_id,
        &mut items,
        &[
            crate::resources::names::of(
                "pipeline_id",
                "pipeline_name",
                crate::resources::names::Kind::Pipeline,
            ),
            crate::resources::names::of(
                "run_id",
                "run_number",
                crate::resources::names::Kind::RunNumber,
            ),
        ],
    )
    .await?;
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn download(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<impl IntoResponse> {
    scope.require("pipelines.read")?;
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    let row = artifacts::Entity::find_by_id(i64::from(id))
        .filter(artifacts::Column::WorkspaceId.eq(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let file = tokio::fs::File::open(file_path(&s, &row))
        .await
        .map_err(|_| ApiError::missing())?;
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0u8; 65536];
        let size = file.read(&mut buffer).await?;
        if size == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        buffer.truncate(size);
        Ok(Some((bytes::Bytes::from(buffer), file)))
    });
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"artifact-{id}.bin\""),
            ),
            (header::CONTENT_LENGTH, row.size.to_string()),
        ],
        Body::from_stream(stream),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn collect_with_modes(
    s: &AppState,
    run: &pipeline_runs::Model,
    ch: &dyn HostChannel,
    workdir: &str,
    paths: &[String],
    retention: &std::collections::BTreeMap<String, i64>,
    stop: &mut tokio::sync::watch::Receiver<bool>,
    source_modes: &std::collections::BTreeMap<String, u32>,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    validate_paths(paths)?;
    let dir = directory(s, run.id);
    tokio::fs::create_dir_all(&dir).await.map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.storageCreateFailed"),
        )
    })?;
    let mut created_files = Vec::new();
    let result = async {
        let mut rows = Vec::new();
        for path in paths {
            if *stop.borrow() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.shutdown"),
                ));
            }
            let id = i64::from(s.next_id()?);
            let (meta, mut stream) = ch
                .read_file(&format!("{}/{path}", workdir.trim_end_matches('/')))
                .await?;
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(dir.join(id.to_string())).await.map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.writeUnavailable"),
                )
            })?;
            created_files.push(dir.join(id.to_string()));
            let mut hash = Sha256::new();
            let mut size = 0u64;
            while let Some(part) = stream.next().await {
                if *stop.borrow() {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.shutdown"),
                    ));
                }
                let part = part?;
                size = size.checked_add(part.len() as u64).ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.tooLarge"),
                    )
                })?;
                if size > meta.size {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.changedDuringArchive"),
                    ));
                }
                hash.update(&part);
                file.write_all(&part).await.map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.writeFailed"),
                    )
                })?;
            }
            if size != meta.size {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.readIncomplete"),
                ));
            }
            file.sync_all().await.map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.saveFailed"),
                )
            })?;
            drop(file);
            let mut digest = hex::encode(hash.finalize());
            if let Some(mode) = source_modes.get(path).copied() {
                let local = dir.join(id.to_string());
                (digest, size) =
                    tokio::task::spawn_blocking(move || restore_legacy_mode(&local, mode))
                        .await
                        .map_err(|_| {
                            ApiError::localized(
                                axum::http::StatusCode::BAD_REQUEST,
                                crate::i18n::Message::new("artifacts.modeFailed"),
                            )
                        })??;
            }
            rows.push(artifacts::Model {
                id,
                workspace_id: run.workspace_id,
                pipeline_id: run.pipeline_id,
                run_id: run.id,
                path: path.clone(),
                sha256: digest,
                size: i64::try_from(size).map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.tooLarge"),
                    )
                })?,
                mode: i64::from(source_modes.get(path).copied().unwrap_or(meta.mode) & 0o777),
                created_at: db::now(),
                // upload-artifact retention-days; 0 leaves it to the history policy.
                expires_at: match retention.get(path) {
                    Some(days) if *days > 0 => {
                        db::now().saturating_add(days.saturating_mul(86_400_000))
                    }
                    _ => 0,
                },
            });
        }
        // Bytes are staged under globally unique artifact IDs, never the
        // shared logical path. Serialize publication across all server tasks;
        // a losing upload removes only its own staged files below.
        let tx = crate::variables::transaction(s, run.workspace_id.into()).await?;
        if artifacts::Entity::find()
            .filter(artifacts::Column::WorkspaceId.eq(run.workspace_id))
            .filter(artifacts::Column::RunId.eq(run.id))
            .filter(artifacts::Column::Path.is_in(paths.iter().cloned()))
            .one(&tx)
            .await?
            .is_some()
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("artifacts.nameConflict"),
            ));
        }
        // A cancelled run publishes only what finished jobs and its cleanup
        // jobs (`if: always()`/`cancelled()`) uploaded; interrupted workers
        // return before collection.
        if pipeline_runs::Entity::find_by_id(run.id)
            .filter(pipeline_runs::Column::State.is_in(["running", "cancelling"]))
            .one(&tx)
            .await?
            .is_none()
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.buildCancelled"),
            ));
        }
        for row in rows {
            artifacts::Entity::insert(row.into_active_model())
                .exec(&tx)
                .await?;
        }
        tx.commit().await?;
        Ok::<_, ApiError>(())
    };
    let result = result.await;
    if result.is_err() {
        for path in created_files {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
    result
}

/// Remove artifacts a job replaced (`overwrite: true`) or deleted. Archives
/// a deployment references stay: deployments must remain reproducible.
pub(crate) async fn remove(
    s: &AppState,
    run: &pipeline_runs::Model,
    paths: &[String],
) -> Result<()> {
    use crate::entities::deployment_artifact_refs as refs;
    if paths.is_empty() {
        return Ok(());
    }
    let rows = artifacts::Entity::find()
        .filter(artifacts::Column::RunId.eq(run.id))
        .filter(artifacts::Column::WorkspaceId.eq(run.workspace_id))
        .filter(artifacts::Column::Path.is_in(paths.iter().cloned()))
        .filter(
            artifacts::Column::Id.not_in_subquery(
                refs::Entity::find()
                    .select_only()
                    .column(refs::Column::ArtifactId)
                    .filter(refs::Column::RunId.eq(run.id))
                    .into_query(),
            ),
        )
        .all(&s.db)
        .await?;
    delete_rows(s, rows).await
}

/// Prune archives whose upload-artifact retention has passed.
pub async fn prune_expired(s: &AppState) -> Result<()> {
    use crate::entities::deployment_artifact_refs as refs;
    let rows = artifacts::Entity::find()
        .filter(artifacts::Column::ExpiresAt.gt(0))
        .filter(artifacts::Column::ExpiresAt.lt(db::now()))
        .filter(
            artifacts::Column::Id.not_in_subquery(
                refs::Entity::find()
                    .select_only()
                    .column(refs::Column::ArtifactId)
                    .into_query(),
            ),
        )
        .limit(200)
        .all(&s.db)
        .await?;
    delete_rows(s, rows).await
}

async fn delete_rows(s: &AppState, rows: Vec<artifacts::Model>) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    artifacts::Entity::delete_many()
        .filter(artifacts::Column::Id.is_in(rows.iter().map(|row| row.id)))
        .exec(&s.db)
        .await?;
    for row in &rows {
        if let Err(error) = tokio::fs::remove_file(file_path(s, row)).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %row.path, %error, "无法删除过期或被替换的产物");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &std::path::Path, member: &str, bytes: &[u8], mode: u32) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        writer
            .start_file(
                member,
                zip::write::SimpleFileOptions::default()
                    .system(zip::System::Unix)
                    .unix_permissions(mode),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer
            .add_directory("empty/", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn legacy_zip_restores_source_mode_and_streams_member_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        let bytes = b"#!/bin/sh\necho compatible\n";
        write_zip(&path, "server", bytes, 0o644);
        let (hash, size) = restore_legacy_mode(&path, 0o751).unwrap();
        assert_eq!(
            hash,
            hex::encode(Sha256::digest(std::fs::read(&path).unwrap()))
        );
        assert_eq!(size, std::fs::metadata(&path).unwrap().len());
        let (mut file, hash, size, mode) = inspect_member(&path, "server").unwrap().unwrap();
        let mut selected = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut selected).unwrap();
        assert_eq!(selected, bytes);
        assert_eq!(hash, hex::encode(Sha256::digest(bytes)));
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(mode, 0o751);
        assert!(inspect_member(&path, "missing").unwrap().is_none());
        assert!(inspect_member(&path, "../server").is_err());
        assert!(inspect_member(&path, "empty/").is_err());
        assert!(legacy_artifact_name(&"a".repeat(1024)).len() < 255);
    }

    #[tokio::test]
    async fn deployment_resolves_converted_zip_and_historical_raw_artifacts() {
        use crate::entities::{deployment_artifact_refs, pipelines};
        use sea_orm::{ConnectionTrait, Schema, Set};
        let dir = tempfile::tempdir().unwrap();
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let schema = Schema::new(db.get_database_backend());
        for statement in [
            schema.create_table_from_entity(pipelines::Entity),
            schema.create_table_from_entity(pipeline_runs::Entity),
            schema.create_table_from_entity(artifacts::Entity),
            schema.create_table_from_entity(deployment_artifact_refs::Entity),
        ] {
            db.execute(&statement).await.unwrap();
        }
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "artifact-test".into(),
            server_mode: true,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), true).unwrap(),
            cluster: None,
            host_monitor: Default::default(),
        });
        pipelines::Entity::insert(pipelines::ActiveModel {
            id: Set(1),
            workspace_id: Set(1),
            name: Set("build".into()),
            group_id: Set(0),
            application_id: Set(0),
            repository_id: Set(0),
            definition: Set(String::new()),
            target_kind: Set("host".into()),
            target_ref: Set(1),
            description: Set(String::new()),
            created_at: Set(0),
            updated_at: Set(0),
            version: Set(1),
            ..Default::default()
        })
        .exec(&s.db)
        .await
        .unwrap();
        pipeline_runs::Entity::insert(pipeline_runs::ActiveModel {
            id: Set(10),
            workspace_id: Set(1),
            pipeline_id: Set(1),
            environment_id: Set(0),
            environment_name: Set(String::new()),
            host_id: Set(1),
            pipeline_name: Set("build".into()),
            host_name: Set(String::new()),
            triggered_by_name: Set(String::new()),
            inputs: Set(None),
            concurrency_group: Set(String::new()),
            number: Set(1),
            state: Set("succeeded".into()),
            trigger_kind: Set("manual".into()),
            triggered_by: Set(1),
            definition: Set(String::new()),
            variable_snapshot: Set(None),
            repo_url: Set(String::new()),
            branch: Set(String::new()),
            commit: Set(String::new()),
            credential_id: Set(0),
            created_at: Set(0),
            started_at: Set(0),
            finished_at: Set(1),
        })
        .exec(&s.db)
        .await
        .unwrap();
        std::fs::create_dir_all(directory(&s, 10)).unwrap();
        let name = legacy_artifact_name("dist/server");
        let bytes = b"compiled executable";
        let mut row = artifacts::Model {
            id: 11,
            workspace_id: 1,
            pipeline_id: 1,
            run_id: 10,
            path: format!("artifacts/10/{name}/{name}.zip"),
            sha256: String::new(),
            size: 0,
            mode: 0o751,
            created_at: 2,
            expires_at: 0,
        };
        write_zip(&file_path(&s, &row), "server", bytes, 0o751);
        let archive = std::fs::read(file_path(&s, &row)).unwrap();
        row.sha256 = hex::encode(Sha256::digest(&archive));
        row.size = archive.len() as i64;
        artifacts::Entity::insert(row.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let reference = crate::deployments::manifest::BuildArtifact {
            source: "pipeline".into(),
            pipeline: "build".into(),
            path: "dist/server".into(),
            run: None,
        };
        let descriptor = resolve(&s, &s.db, 1.into(), &reference).await.unwrap();
        assert_eq!(descriptor["zip_member"], "server");
        assert_eq!(descriptor["archive_sha256"], row.sha256);
        assert_eq!(descriptor["sha256"], hex::encode(Sha256::digest(bytes)));
        assert_eq!(descriptor["mode"], 0o751);
        freeze(
            &s,
            &s.db,
            1.into(),
            20.into(),
            &json!({"services":{"server":{"artifact":descriptor}}}).to_string(),
        )
        .await
        .unwrap();
        let protected = deployment_artifact_refs::Entity::find()
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(protected.len(), 1);
        assert_eq!(protected[0].artifact_id, 11);
        let mut file = deployment_file(&s, &descriptor).await.unwrap();
        let mut selected = Vec::new();
        file.read_to_end(&mut selected).await.unwrap();
        assert_eq!(selected, bytes);
        let mut corrupt = descriptor.clone();
        corrupt["sha256"] = json!("changed");
        assert!(deployment_file(&s, &corrupt).await.is_err());
        row.id = 12;
        row.path = "dist/server".into();
        row.sha256 = hex::encode(Sha256::digest(b"historical raw executable"));
        row.size = 25;
        row.created_at = 3;
        std::fs::write(file_path(&s, &row), b"historical raw executable").unwrap();
        artifacts::Entity::insert(row.clone().into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let descriptor = resolve(&s, &s.db, 1.into(), &reference).await.unwrap();
        assert!(descriptor.get("zip_member").is_none());
        assert_eq!(descriptor["artifact_id"], "12");
        let mut file = deployment_file(&s, &descriptor).await.unwrap();
        let mut selected = Vec::new();
        file.read_to_end(&mut selected).await.unwrap();
        assert_eq!(selected, b"historical raw executable");
        assert!(resolve(&s, &s.db, 2.into(), &reference).await.is_err());
        row.id = 13;
        row.path = "artifacts/10/bundle/bundle.zip".into();
        row.created_at = 4;
        write_zip(&file_path(&s, &row), "dist/server", bytes, 0o755);
        let archive = std::fs::read(file_path(&s, &row)).unwrap();
        row.sha256 = hex::encode(Sha256::digest(&archive));
        row.size = archive.len() as i64;
        artifacts::Entity::insert(row.into_active_model())
            .exec(&s.db)
            .await
            .unwrap();
        let reference = crate::deployments::manifest::BuildArtifact {
            path: "bundle/dist/server".into(),
            ..reference
        };
        let descriptor = resolve(&s, &s.db, 1.into(), &reference).await.unwrap();
        assert_eq!(descriptor["zip_member"], "dist/server");
        assert_eq!(descriptor["artifact_id"], "13");
        assert_eq!(descriptor["mode"], 0o755);
    }
    #[test]
    fn artifact_paths_are_explicit_relative_files() {
        assert!(validate_paths(&["dist/app".into(), "config/app.toml".into()]).is_ok());
        for path in [
            "",
            "/etc/passwd",
            "../app",
            "a/../app",
            "a//app",
            "a\\app",
            "a/*",
            "{{run}}",
            "a\nb",
        ] {
            assert!(validate_paths(&[path.into()]).is_err(), "{path}");
        }
        assert!(validate_paths(&["dist/app".into(), "dist/app".into()]).is_err());
    }
}

pub(crate) async fn pipeline_id(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    reference: &str,
) -> Result<i64> {
    use crate::entities::pipelines;
    let query = pipelines::Entity::find().filter(pipelines::Column::WorkspaceId.eq(workspace));
    let rows = if let Ok(id) = reference.parse::<Id>() {
        query.filter(pipelines::Column::Id.eq(id)).all(conn).await?
    } else {
        query
            .filter(pipelines::Column::Name.eq(reference))
            .all(conn)
            .await?
    };
    if rows.len() != 1 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.pipelineInvalid"),
        ));
    }
    Ok(rows[0].id)
}
pub(crate) async fn resolve(
    s: &ServerState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    reference: &crate::deployments::manifest::BuildArtifact,
) -> Result<Value> {
    let pipeline = pipeline_id(conn, workspace, &reference.pipeline).await?;
    let mut query = artifacts::Entity::find()
        .filter(artifacts::Column::WorkspaceId.eq(workspace))
        .filter(artifacts::Column::PipelineId.eq(pipeline))
        .filter(
            sea_orm::Condition::any()
                .add(artifacts::Column::Path.eq(&reference.path))
                .add(artifacts::Column::Path.ends_with(".zip")),
        )
        .filter(
            artifacts::Column::RunId.in_subquery(
                pipeline_runs::Entity::find()
                    .select_only()
                    .column(pipeline_runs::Column::Id)
                    .filter(pipeline_runs::Column::WorkspaceId.eq(workspace))
                    .filter(pipeline_runs::Column::State.eq("succeeded"))
                    .into_query(),
            ),
        );
    if let Some(run) = &reference.run {
        query = query.filter(artifacts::Column::RunId.eq(run.parse::<Id>().map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.runIdInvalid"),
            )
        })?));
    }
    let rows = query
        .order_by_desc(artifacts::Column::CreatedAt)
        .order_by_desc(artifacts::Column::Id)
        .all(conn)
        .await?;
    for row in rows {
        let mut descriptor = json!({"artifact_id":row.id.to_string(),"run_id":row.run_id.to_string(),"pipeline_id":row.pipeline_id.to_string(),"path":reference.path,"sha256":row.sha256,"size":row.size,"mode":row.mode});
        // Exact raw paths retain their historical meaning, even if the file
        // happens to end in .zip.
        if row.path == reference.path {
            return Ok(descriptor);
        }
        if !row.path.starts_with(&format!("artifacts/{}/", row.run_id)) {
            continue;
        }
        let name = archive_name(&row.path);
        let member = if name == Some(legacy_artifact_name(&reference.path).as_str()) {
            reference.path.rsplit('/').next().unwrap().to_owned()
        } else if let Some(relative) =
            name.and_then(|name| reference.path.strip_prefix(&format!("{name}/")))
        {
            relative.to_owned()
        } else {
            reference.path.clone()
        };
        let local = file_path(s, &row);
        let lookup = member.clone();
        let selected = tokio::task::spawn_blocking(move || inspect_member(&local, &lookup))
            .await
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("artifacts.archiveUnreadable"),
                )
            })??;
        if let Some((_, hash, size, mode)) = selected {
            descriptor["archive_sha256"] = json!(row.sha256);
            descriptor["zip_member"] = json!(member);
            descriptor["sha256"] = json!(hash);
            descriptor["size"] = json!(size);
            descriptor["mode"] = json!(if name
                == Some(legacy_artifact_name(&reference.path).as_str())
            {
                row.mode as u32
            } else {
                mode
            });
            return Ok(descriptor);
        }
    }
    Err(ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new("artifacts.successfulBuildRequired"),
    ))
}

/// Read a single regular ZIP member into an anonymous file, keeping large
/// executables off the async runtime and out of memory. The file is deleted
/// automatically when the deployment upload closes it.
fn inspect_member(
    archive: &std::path::Path,
    member: &str,
) -> Result<Option<(std::fs::File, String, u64, u32)>> {
    validate_paths(&[member.to_owned()])?;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.zipInvalid"),
        )
    })?;
    let mut entry = match archive.by_name(member) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.zipMemberUnreadable"),
            ));
        }
    };
    if entry.is_dir() || entry.is_symlink() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.nativeExecutableRequired"),
        ));
    }
    let mode = entry.unix_mode().unwrap_or(0o644) & 0o777;
    let mut file = tempfile::tempfile()?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    use std::io::{Read, Seek, Write};
    let mut buffer = [0u8; 65536];
    loop {
        let count = entry.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        size += count as u64;
    }
    file.rewind()?;
    Ok(Some((file, hex::encode(hash.finalize()), size, mode)))
}

pub(crate) async fn deployment_file(s: &ServerState, reference: &Value) -> Result<tokio::fs::File> {
    let run = reference["run_id"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.historyRunInvalid"),
            )
        })?;
    let id = reference["artifact_id"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.historyIdInvalid"),
            )
        })?;
    let path = directory(s, run).join(id.to_string());
    if let Some(member) = reference["zip_member"].as_str() {
        let member = member.to_owned();
        let (file, hash, _, _) =
            tokio::task::spawn_blocking(move || inspect_member(&path, &member))
                .await
                .map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.historyUnreadable"),
                    )
                })??
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.historyMemberMissing"),
                    )
                })?;
        if reference["sha256"] != hash {
            return Err(ApiError::localized(
                axum::http::StatusCode::CONFLICT,
                crate::i18n::Message::new("artifacts.historyDigestMismatch"),
            ));
        }
        Ok(tokio::fs::File::from_std(file))
    } else {
        tokio::fs::File::open(path).await.map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("artifacts.historyFileMissing"),
            )
        })
    }
}
pub(crate) async fn freeze(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    revision: Id,
    artifact: &str,
) -> Result<()> {
    use crate::entities::deployment_artifact_refs as refs;
    let doc: Value = serde_json::from_str(artifact).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("artifacts.deploymentInvalid"),
        )
    })?;
    let mut docs = vec![doc.clone()];
    if let Some(targets) = doc["targets"].as_array() {
        for target in targets {
            docs.push(
                serde_json::from_str(target["artifact"].as_str().ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.groupMissing"),
                    )
                })?)
                .map_err(|_| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("artifacts.groupInvalid"),
                    )
                })?,
            );
        }
    }
    let mut seen = BTreeSet::new();
    for doc in docs {
        if let Some(services) = doc["services"].as_object() {
            for service in services.values() {
                if let Some(id) = service["artifact"]["artifact_id"].as_str() {
                    let id = id.parse::<Id>().map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("artifacts.idInvalid"),
                        )
                    })?;
                    let row = artifacts::Entity::find_by_id(i64::from(id))
                        .filter(artifacts::Column::WorkspaceId.eq(workspace))
                        .one(conn)
                        .await?
                        .ok_or_else(|| {
                            ApiError::localized(
                                axum::http::StatusCode::CONFLICT,
                                crate::i18n::Message::new("artifacts.cleaned"),
                            )
                        })?;
                    if service["artifact"]
                        .get("archive_sha256")
                        .unwrap_or(&service["artifact"]["sha256"])
                        != &row.sha256
                    {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::CONFLICT,
                            crate::i18n::Message::new("artifacts.digestMismatch"),
                        ));
                    }
                    if !seen.insert(row.id) {
                        continue;
                    }
                    refs::Entity::insert(
                        refs::Model {
                            id: s.next_id()?.into(),
                            workspace_id: workspace.into(),
                            revision_id: revision.into(),
                            artifact_id: row.id,
                            run_id: row.run_id,
                        }
                        .into_active_model(),
                    )
                    .exec(conn)
                    .await?;
                }
            }
        }
    }
    Ok(())
}
