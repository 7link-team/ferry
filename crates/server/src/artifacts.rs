//! Immutable build outputs streamed from HostChannel, retained with run history.
use crate::{
    channel::HostChannel,
    db,
    entities::{pipeline_artifacts as artifacts, pipeline_runs},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::{AppState, State as ServerState},
    workspace::Scope,
};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use futures::StreamExt;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    TransactionTrait,
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
        return Err(ApiError::bad("每次构建最多归档100个文件。"));
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
            return Err(ApiError::bad(
                "artifacts 必须是唯一、相对于构建工作目录的文件路径，不支持通配符或模板变量。",
            ));
        }
        if path.contains('*') || path.contains('?') || path.contains('[') {
            return Err(ApiError::bad("产物路径必须指向具体文件，不支持通配符。"));
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
fn public(r: &artifacts::Model) -> Value {
    json!({"id":Id::from(r.id),"pipeline_id":Id::from(r.pipeline_id),"run_id":Id::from(r.run_id),"path":r.path,"sha256":r.sha256,"size":r.size,"mode":r.mode,"created_at":r.created_at})
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
    Ok(Json(
        json!({"items":rows.iter().map(public).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
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

pub async fn collect(
    s: &AppState,
    run: &pipeline_runs::Model,
    ch: &dyn HostChannel,
    workdir: &str,
    paths: &[String],
    stop: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    validate_paths(paths)?;
    let dir = directory(s, run.id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| ApiError::bad("无法创建产物存储目录。"))?;
    let result = async {
        let mut rows = Vec::new();
        for path in paths {
            if *stop.borrow() {
                return Err(ApiError::bad("服务正在关闭，产物归档中止。"));
            }
            let id = i64::from(s.next_id()?);
            let (meta, mut stream) = ch
                .read_file(&format!("{}/{path}", workdir.trim_end_matches('/')))
                .await?;
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(dir.join(id.to_string()))
                .await
                .map_err(|_| ApiError::bad("无法写入构建产物。"))?;
            let mut hash = Sha256::new();
            let mut size = 0u64;
            while let Some(part) = stream.next().await {
                if *stop.borrow() {
                    return Err(ApiError::bad("服务正在关闭，产物归档中止。"));
                }
                let part = part?;
                size = size
                    .checked_add(part.len() as u64)
                    .ok_or_else(|| ApiError::bad("产物过大。"))?;
                if size > meta.size {
                    return Err(ApiError::bad("归档期间产物发生变化，请重新构建。"));
                }
                hash.update(&part);
                file.write_all(&part)
                    .await
                    .map_err(|_| ApiError::bad("写入构建产物失败。"))?;
            }
            if size != meta.size {
                return Err(ApiError::bad("构建产物读取不完整。"));
            }
            file.sync_all()
                .await
                .map_err(|_| ApiError::bad("保存构建产物失败。"))?;
            rows.push(artifacts::Model {
                id,
                workspace_id: run.workspace_id,
                pipeline_id: run.pipeline_id,
                run_id: run.id,
                path: path.clone(),
                sha256: hex::encode(hash.finalize()),
                size: i64::try_from(size).map_err(|_| ApiError::bad("产物过大。"))?,
                mode: i64::from(meta.mode & 0o777),
                created_at: db::now(),
            });
        }
        let tx = s.db.begin().await?;
        // Cancellation must not publish a completed-looking archive.
        if pipeline_runs::Entity::find_by_id(run.id)
            .filter(pipeline_runs::Column::State.eq("running"))
            .one(&tx)
            .await?
            .is_none()
        {
            return Err(ApiError::bad("构建已取消，未发布产物。"));
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
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
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
        return Err(ApiError::bad(
            "产物流水线不存在或名称不唯一，请使用当前空间的流水线ID。",
        ));
    }
    Ok(rows[0].id)
}
pub(crate) async fn resolve(
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    reference: &crate::manifest::BuildArtifact,
) -> Result<artifacts::Model> {
    let pipeline = pipeline_id(conn, workspace, &reference.pipeline).await?;
    let mut query = artifacts::Entity::find()
        .filter(artifacts::Column::WorkspaceId.eq(workspace))
        .filter(artifacts::Column::PipelineId.eq(pipeline))
        .filter(artifacts::Column::Path.eq(&reference.path))
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
        query = query.filter(
            artifacts::Column::RunId.eq(run
                .parse::<Id>()
                .map_err(|_| ApiError::bad("构建执行ID无效。"))?),
        );
    }
    query
        .order_by_desc(artifacts::Column::CreatedAt)
        .order_by_desc(artifacts::Column::Id)
        .one(conn)
        .await?
        .ok_or_else(|| ApiError::bad("没有找到成功构建的声明产物，请先构建或选择已有执行。"))
}
pub(crate) async fn freeze(
    s: &AppState,
    conn: &impl sea_orm::ConnectionTrait,
    workspace: Id,
    revision: Id,
    artifact: &str,
) -> Result<()> {
    use crate::entities::deployment_artifact_refs as refs;
    let doc: Value = serde_json::from_str(artifact).map_err(|_| ApiError::bad("部署产物无效。"))?;
    let mut docs = vec![doc.clone()];
    if let Some(targets) = doc["targets"].as_array() {
        for target in targets {
            docs.push(
                serde_json::from_str(
                    target["artifact"]
                        .as_str()
                        .ok_or_else(|| ApiError::bad("主机组产物缺失。"))?,
                )
                .map_err(|_| ApiError::bad("主机组产物无效。"))?,
            );
        }
    }
    let mut seen = BTreeSet::new();
    for doc in docs {
        if let Some(services) = doc["services"].as_object() {
            for service in services.values() {
                if let Some(id) = service["artifact"]["artifact_id"].as_str() {
                    let id = id
                        .parse::<Id>()
                        .map_err(|_| ApiError::bad("构建产物ID无效。"))?;
                    let row = artifacts::Entity::find_by_id(i64::from(id))
                        .filter(artifacts::Column::WorkspaceId.eq(workspace))
                        .one(conn)
                        .await?
                        .ok_or_else(|| ApiError::conflict("构建产物已清理，请重新预览。"))?;
                    if service["artifact"]["sha256"] != row.sha256 {
                        return Err(ApiError::conflict("构建产物摘要不匹配，请重新预览。"));
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
