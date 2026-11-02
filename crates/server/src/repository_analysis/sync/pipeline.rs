//! Resumeable act fallback. Only artifacts from the configured, authorized run are accepted.
use super::*;
use crate::entities::{pipeline_artifacts, pipeline_runs};
use ferry_protocol::contribution::{Artifact, SCHEMA_VERSION};

#[derive(Serialize, Deserialize)]
pub(super) struct Cursor {
    pub generation: i64,
    pub config_version: i64,
    run_id: Option<i64>,
}

pub(super) async fn collect(s: &AppState, claim: &Claim) -> Result<()> {
    let mut config = decode(&claim.config)?;
    if let Some(pipeline) = config.pipeline_bindings.get(&claim.source.remote_path) {
        config.pipeline_id = pipeline.clone();
    }
    if config.pipeline_id.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.pipelineRequired"),
        ));
    }
    let scope = Scope::for_actor(
        s,
        claim.source.workspace_id.into(),
        claim.connection.user_id.into(),
    )
    .await?;
    scope.require("pipelines.run")?;
    let run_id = if claim.job.cursor.is_empty() {
        let known: Vec<String> = commits::Entity::find()
            .filter(commits::Column::RepositoryId.eq(claim.source.repository_id))
            .filter(commits::Column::AnalysisVersion.eq(ANALYSIS_VERSION))
            .filter(commits::Column::Complete.eq(1))
            .all(&s.db)
            .await?
            .into_iter()
            .map(|v| v.sha)
            .collect();
        // Persist launch intent first. An interruption between trigger and recording the
        // run is ambiguous and requires explicit retry, never an automatic second launch.
        save_cursor(
            s,
            claim,
            &Cursor {
                generation: claim.job.generation,
                config_version: claim.source.config_version,
                run_id: None,
            },
        )
        .await?;
        let body = json!({"inputs": {
            "analysis_task_id":claim.source.id.to_string(),
            "analysis_generation":claim.job.generation.to_string(),
            "analysis_repository":claim.source.remote_path,
            "analysis_coverage_start":claim.source.coverage_start.to_string(),
            "analysis_known_complete":encode(&known)?,
        }});
        let repository = repositories::Entity::find_by_id(claim.source.repository_id)
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        let (_, Json(value)) = crate::pipeline_runs::trigger_with_analysis_source(
            s.clone(),
            scope,
            config.pipeline_id.clone(),
            bytes::Bytes::from(encode(&body)?),
            "manual",
            Some((claim.connection.id, repository.url)),
        )
        .await?;
        let id: Id = value["id"]
            .as_str()
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.runIdMissing"),
                )
            })?
            .parse()
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.runIdInvalid"),
                )
            })?;
        save_cursor(
            s,
            claim,
            &Cursor {
                generation: claim.job.generation,
                config_version: claim.source.config_version,
                run_id: Some(id.into()),
            },
        )
        .await?;
        i64::from(id)
    } else {
        let cursor: Cursor = serde_json::from_str(&claim.job.cursor).map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.cursorInvalid"),
            )
        })?;
        if cursor.config_version != claim.source.config_version {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.cursorConfigChanged"),
            ));
        }
        cursor.run_id.ok_or_else(|| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.runUncertain"),
            )
        })?
    };
    loop {
        let run = pipeline_runs::Entity::find_by_id(run_id)
            .filter(pipeline_runs::Column::WorkspaceId.eq(claim.source.workspace_id))
            .one(&s.db)
            .await?
            .ok_or_else(|| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.runMissing"),
                )
            })?;
        if run.pipeline_id.to_string() != config.pipeline_id
            || run.triggered_by != claim.connection.user_id
        {
            return Err(ApiError::forbidden());
        }
        match run.state.as_str() {
            "succeeded" => break,
            "failed" | "cancelled" | "skipped" => {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.runFailed"),
                ));
            }
            _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
        }
    }
    let rows = pipeline_artifacts::Entity::find()
        .filter(pipeline_artifacts::Column::WorkspaceId.eq(claim.source.workspace_id))
        .filter(pipeline_artifacts::Column::RunId.eq(run_id))
        .all(&s.db)
        .await?;
    let mut found = None;
    for row in rows {
        if row.path != "contribution-analysis.json" && !row.path.ends_with(".zip") {
            continue;
        }
        let bytes = tokio::fs::read(crate::artifacts::file_path(s, &row))
            .await
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactRead"),
                )
            })?;
        if bytes.len() as i64 != row.size || hex::encode(Sha256::digest(&bytes)) != row.sha256 {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.artifactIntegrity"),
            ));
        }
        let candidate = if row.path.ends_with(".zip") {
            tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
                use std::io::Read;
                let mut archive =
                    zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.collector.archiveInvalid"),
                        )
                    })?;
                let mut result = None;
                for index in 0..archive.len() {
                    let mut file = archive.by_index(index).map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.collector.artifactFileInvalid"),
                        )
                    })?;
                    if file.name().map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.collector.artifactPathInvalid"),
                        )
                    })? != "contribution-analysis.json"
                    {
                        continue;
                    }
                    if result.is_some() {
                        return Err(ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.collector.archiveDuplicate"),
                        ));
                    }
                    let mut data = Vec::new();
                    file.read_to_end(&mut data).map_err(|_| {
                        ApiError::localized(
                            axum::http::StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("repository.collector.archiveExtract"),
                        )
                    })?;
                    result = Some(data);
                }
                Ok(result)
            })
            .await
            .map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactInterrupted"),
                )
            })??
        } else {
            Some(bytes)
        };
        if let Some(bytes) = candidate {
            if found.replace(bytes).is_some() {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactMultiple"),
                ));
            }
        }
    }
    let bytes = found.ok_or_else(|| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.artifactMissing"),
        )
    })?;
    let artifact: Artifact = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.artifactFormatInvalid"),
        )
    })?;
    import(s, claim, artifact).await
}

async fn save_cursor(s: &AppState, claim: &Claim, cursor: &Cursor) -> Result<()> {
    let tx = crate::variables::transaction(s, claim.source.workspace_id.into()).await?;
    fence(&tx, claim).await?;
    jobs::Entity::update_many()
        .col_expr(jobs::Column::Cursor, Expr::value(encode(cursor)?))
        .filter(jobs::Column::Id.eq(claim.job.id))
        .filter(jobs::Column::LeaseOwner.eq(&claim.owner))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn import(s: &AppState, claim: &Claim, artifact: Artifact) -> Result<()> {
    if artifact.schema_version != SCHEMA_VERSION
        || artifact.analysis_version != ANALYSIS_VERSION
        || artifact.task_id != claim.source.id.to_string()
        || artifact.generation != claim.job.generation
        || artifact.repository != claim.source.remote_path
        || artifact.coverage_start != claim.source.coverage_start
        || !artifact.complete
        || artifact.refs.keys().ne(artifact.ref_commits.keys())
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.collector.artifactScopeMismatch"),
        ));
    }
    let mut supplied = BTreeSet::new();
    for value in artifact.commits {
        if !value.valid()
            || (!value.complete
                && (claim.source.coverage_start == 0
                    || value.committer_date >= claim.source.coverage_start))
            || !supplied.insert(value.sha.clone())
        {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.artifactCommitsInvalid"),
            ));
        }
        store(s, claim, value).await?;
    }
    let mut completed = BTreeMap::new();
    for (name, head) in artifact.refs {
        if name.is_empty() || !valid_sha(&head) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.artifactBranchInvalid"),
            ));
        }
        let mut members = BTreeSet::new();
        let reachable = &artifact.ref_commits[&name];
        for sha in reachable {
            let row = cached(s, claim.source.repository_id, sha)
                .await?
                .filter(|v| {
                    v.complete == 1
                        && v.analysis_version == ANALYSIS_VERSION
                        && (claim.source.coverage_start == 0
                            || v.committer_date >= claim.source.coverage_start)
                })
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.artifactCommitMissing"),
                    )
                })?;
            if !members.insert(row.id) {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactBranchDuplicate"),
                ));
            }
        }
        // Verify ancestry to roots, continuing past old dates (Git dates are not
        // monotonic). Membership must exactly match the in-range reachable set.
        let mut graph = VecDeque::from([head.clone()]);
        let mut visited = BTreeSet::new();
        let mut expected = BTreeSet::new();
        while let Some(sha) = graph.pop_front() {
            if !visited.insert(sha.clone()) {
                continue;
            }
            let row = cached(s, claim.source.repository_id, &sha)
                .await?
                .ok_or_else(|| {
                    ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("repository.collector.artifactGraphIncomplete"),
                    )
                })?;
            if claim.source.coverage_start == 0 || row.committer_date >= claim.source.coverage_start
            {
                if row.complete != 1 || row.analysis_version != ANALYSIS_VERSION {
                    return Err(ApiError::localized(
                        axum::http::StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new(
                            "repository.collector.artifactReachableIncomplete",
                        ),
                    ));
                }
                expected.insert(row.id);
            }
            let parents: Vec<String> = serde_json::from_str(&row.parents).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactGraphInvalid"),
                )
            })?;
            if parents.iter().any(|v| !valid_sha(v)) {
                return Err(ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.collector.artifactParentInvalid"),
                ));
            }
            graph.extend(parents);
        }
        if expected != members {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.collector.artifactMembershipIncomplete"),
            ));
        }
        completed.insert(name, (head, members));
    }
    let old = refs::Entity::find()
        .filter(refs::Column::SourceId.eq(claim.source.id))
        .all(&s.db)
        .await?;
    publish(s, claim, &old, completed).await
}
