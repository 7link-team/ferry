use super::*;
use crate::{entities::locks, lock};
use ferry_protocol::contribution::{ANALYSIS_VERSION, Commit, valid_sha};
use std::collections::VecDeque;

mod pipeline;
mod provider;

#[cfg(test)]
#[path = "tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "database_tests.rs"]
mod database_tests;

const LEASE_MS: i64 = 60_000;

pub(super) struct Claim {
    job: jobs::Model,
    source: sources::Model,
    config: configs::Model,
    connection: connections::Model,
    owner: String,
    lock: String,
    slot: String,
}

#[cfg(test)]
async fn claim(s: &AppState, job: jobs::Model) -> Result<Option<Claim>> {
    try_claim(s, job, std::num::NonZeroUsize::new(1).unwrap()).await
}

pub(super) async fn try_claim(
    s: &AppState,
    job: jobs::Model,
    concurrency: std::num::NonZeroUsize,
) -> Result<Option<Claim>> {
    let tx = crate::variables::transaction(s, job.workspace_id.into()).await?;
    let Some(mut current) = jobs::Entity::find_by_id(job.id).one(&tx).await? else {
        return Ok(None);
    };
    if current.generation <= current.completed_generation
        || current.retry_at > db::now()
        || current.lease_until > db::now()
    {
        return Ok(None);
    }
    let Some(source) = sources::Entity::find_by_id(job.id).one(&tx).await? else {
        return Ok(None);
    };
    if source.state == "unavailable" {
        return Ok(None);
    }
    let Some(config) = configs::Entity::find_by_id(source.connection_id)
        .one(&tx)
        .await?
    else {
        return Ok(None);
    };
    if !decode(&config)?.selects(&source.remote_path) {
        return Ok(None);
    }
    let connection = connections::Entity::find_by_id(source.connection_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let scope = Scope::for_actor_in(
        &tx,
        connection.workspace_id.into(),
        connection.user_id.into(),
    )
    .await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    if connection.workspace_id != job.workspace_id || connection.state != "connected" {
        return Err(ApiError::forbidden());
    }
    let owner = s.next_id()?.to_string();
    let lock = format!("repository-analysis:{}", source.repository_id);
    if !lock::acquire(&tx, &lock, &owner, db::now(), LEASE_MS).await? {
        return Ok(None);
    }
    let mut slot = None;
    for index in 0..concurrency.get() {
        let name = format!("repository-analysis-slot:{index}");
        if lock::acquire(&tx, &name, &owner, db::now(), LEASE_MS).await? {
            slot = Some(name);
            break;
        }
    }
    let Some(slot) = slot else {
        // Dropping this transaction also rolls back the repository lock.
        return Ok(None);
    };
    current.state = "running".into();
    current.lease_owner = owner.clone();
    current.lease_until = db::now() + LEASE_MS;
    jobs::Entity::update(current.clone().into_active_model().reset_all())
        .exec(&tx)
        .await?;
    tx.commit().await?;
    // Resume the frozen generation, while preserving newer dirty events in DB.
    if let Ok(cursor) = serde_json::from_str::<pipeline::Cursor>(&current.cursor) {
        if cursor.config_version == source.config_version {
            current.generation = cursor.generation;
        }
    }
    Ok(Some(Claim {
        job: current,
        source,
        config,
        connection,
        owner,
        lock,
        slot,
    }))
}

async fn fence(tx: &impl ConnectionTrait, claim: &Claim) -> Result<()> {
    let source = sources::Entity::find_by_id(claim.source.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let config = configs::Entity::find_by_id(claim.config.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let connection = connections::Entity::find_by_id(claim.connection.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let lease = locks::Entity::find_by_id(claim.lock.clone())
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let slot = locks::Entity::find_by_id(claim.slot.clone())
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if source.state == "unavailable"
        || identity::is_mapping_conflict(&source.last_error)
        || collection_version(&config)? != collection_version(&claim.config)?
        || source.config_version != claim.source.config_version
        || connection.version != claim.connection.version
        || connection.user_id != claim.connection.user_id
        || !decode(&config)?.selects(&source.remote_path)
        || connection.state != "connected"
        || lease.owner != claim.owner
        || lease.expires_at <= db::now()
        || slot.owner != claim.owner
        || slot.expires_at <= db::now()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.ownershipChanged"),
        ));
    }
    let scope = Scope::for_actor_in(
        tx,
        connection.workspace_id.into(),
        connection.user_id.into(),
    )
    .await?;
    scope.require("repositories.read")?;
    scope.require("credentials.read")?;
    Ok(())
}

#[cfg(test)]
async fn run(s: &AppState, job: jobs::Model) -> Result<()> {
    let Some(claim) = claim(s, job).await? else {
        return Ok(());
    };
    run_claim(s, claim).await
}

async fn renew_claim(s: &AppState, claim: &Claim) -> Result<()> {
    let tx = crate::variables::transaction(s, claim.job.workspace_id.into()).await?;
    fence(&tx, claim).await?;
    let now = db::now();
    let renewed = locks::Entity::update_many()
        .col_expr(locks::Column::ExpiresAt, Expr::value(now + LEASE_MS))
        .filter(locks::Column::Name.is_in([claim.lock.clone(), claim.slot.clone()]))
        .filter(locks::Column::Owner.eq(&claim.owner))
        .filter(locks::Column::ExpiresAt.gt(now))
        .exec(&tx)
        .await?
        .rows_affected;
    if renewed != 2 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.leaseExpired"),
        ));
    }
    let renewed_job = jobs::Entity::update_many()
        .col_expr(jobs::Column::LeaseUntil, Expr::value(now + LEASE_MS))
        .filter(jobs::Column::Id.eq(claim.job.id))
        .filter(jobs::Column::LeaseOwner.eq(&claim.owner))
        .filter(jobs::Column::LeaseUntil.gt(now))
        .exec(&tx)
        .await?
        .rows_affected;
    if renewed_job != 1 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("repository.analysis.jobOwnershipChanged"),
        ));
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn run_claim(s: &AppState, claim: Claim) -> Result<()> {
    let process = async {
        // A persisted pipeline run is resumed before trying the API again.
        if claim.connection.provider == "git" || !claim.job.cursor.is_empty() {
            return pipeline::collect(s, &claim).await;
        }
        match collect(s, &claim).await {
            Err(_)
                if {
                    let config = decode(&claim.config)?;
                    !config.pipeline_id.is_empty()
                        || config
                            .pipeline_bindings
                            .get(&claim.source.remote_path)
                            .is_some_and(|p| !p.is_empty())
                } =>
            {
                pipeline::collect(s, &claim).await
            }
            result => result,
        }
    };
    tokio::pin!(process);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
    tick.tick().await;
    let result = loop {
        tokio::select! {
            result=&mut process=>break result,
            _=tick.tick()=>{
                if let Err(error) = renew_claim(s, &claim).await {
                    break Err(error);
                }
            }
        }
    };
    finish(s, &claim, result).await
}

async fn finish(s: &AppState, claim: &Claim, result: Result<()>) -> Result<()> {
    let tx = crate::variables::transaction(s, claim.job.workspace_id.into()).await?;
    let mut current = jobs::Entity::find_by_id(claim.job.id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if current.lease_owner == claim.owner {
        // Publication can finish just as a lease expires. Never acknowledge a
        // generation unless both repository ownership and its slot still hold.
        let result = match result {
            Ok(()) => fence(&tx, claim).await,
            Err(error) => Err(error),
        };
        match &result {
            Ok(()) => {
                current.completed_generation = claim.job.generation;
                current.state = if current.generation > current.completed_generation {
                    "queued"
                } else {
                    "complete"
                }
                .into();
                current.last_error.clear();
                current.retry_at = 0;
                current.cursor.clear();
            }
            Err(error) => {
                current.state = "failed".into();
                current.last_error = error.stored();
                current.retry_at = db::now() + 60_000;
                sources::Entity::update_many()
                    .col_expr(sources::Column::State, Expr::value("failed"))
                    .col_expr(sources::Column::LastError, Expr::value(&current.last_error))
                    .filter(sources::Column::Id.eq(claim.source.id))
                    .filter(sources::Column::ConfigVersion.eq(claim.source.config_version))
                    .filter(sources::Column::State.ne("unavailable"))
                    .exec(&tx)
                    .await?;
            }
        }
        current.lease_until = 0;
        current.lease_owner.clear();
        current.updated_at = db::now();
        jobs::Entity::update(current.into_active_model().reset_all())
            .exec(&tx)
            .await?;
        lock::release(&tx, &claim.lock, &claim.owner).await?;
        lock::release(&tx, &claim.slot, &claim.owner).await?;
        tx.commit().await?;
        return result;
    }
    lock::release(&tx, &claim.lock, &claim.owner).await?;
    lock::release(&tx, &claim.slot, &claim.owner).await?;
    tx.commit().await?;
    result
}

async fn cached(s: &AppState, repository: i64, sha: &str) -> Result<Option<commits::Model>> {
    Ok(commits::Entity::find()
        .filter(commits::Column::RepositoryId.eq(repository))
        .filter(commits::Column::Sha.eq(sha))
        .one(&s.db)
        .await?)
}

pub(super) async fn store(s: &AppState, claim: &Claim, value: Commit) -> Result<commits::Model> {
    if !value.valid() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.commitDataInvalid"),
        ));
    }
    let tx = crate::variables::transaction(s, claim.job.workspace_id.into()).await?;
    fence(&tx, claim).await?;
    let old = commits::Entity::find()
        .filter(commits::Column::RepositoryId.eq(claim.source.repository_id))
        .filter(commits::Column::Sha.eq(&value.sha))
        .one(&tx)
        .await?;
    if let Some(old) = &old {
        if old.complete == 1 && old.analysis_version == ANALYSIS_VERSION {
            identity::index_commit(s, &tx, old).await?;
            tx.commit().await?;
            return Ok(old.clone());
        }
    }
    let row = commits::Model {
        id: old
            .as_ref()
            .map(|v| v.id)
            .unwrap_or(i64::from(s.next_id()?)),
        workspace_id: claim.source.workspace_id,
        repository_id: claim.source.repository_id,
        sha: value.sha.clone(),
        parents: encode(&value.parents)?,
        message: value.message.trim().into(),
        message_type: value.message_type(),
        author_name: value.author_name.clone(),
        author_email: value.author_email.clone(),
        author_date: value.author_date,
        committer_date: value.committer_date,
        is_merge: i64::from(value.is_merge()),
        additions: value.additions,
        deletions: value.deletions,
        effectives: value.effectives(),
        language_stats: encode(&value.language_stats)?,
        analysis_version: ANALYSIS_VERSION,
        complete: i64::from(value.complete),
    };
    if old.is_some() {
        commits::Entity::update(row.clone().into_active_model().reset_all())
            .exec(&tx)
            .await?;
    } else {
        commits::Entity::insert(row.clone().into_active_model())
            .exec(&tx)
            .await?;
    }
    identity::index_commit(s, &tx, &row).await?;
    tx.commit().await?;
    Ok(row)
}

async fn collect(s: &AppState, claim: &Claim) -> Result<()> {
    if claim.connection.provider == "git" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("repository.analysis.gitCollectionRequired"),
        ));
    }
    let access = MonitorAccess::load(s, &claim.connection).await?;
    let heads = provider::heads(
        &access,
        &claim.connection.provider,
        &claim.source.remote_path,
    )
    .await?;
    let old_refs = refs::Entity::find()
        .filter(refs::Column::SourceId.eq(claim.source.id))
        .all(&s.db)
        .await?;
    let mut completed = BTreeMap::new();
    for (name, head) in &heads {
        if !valid_sha(head) {
            return Err(ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("repository.analysis.branchRevisionInvalid"),
            ));
        }
        let old = old_refs.iter().find(|r| r.name == *name);
        let old_members = if let Some(old) = old {
            membership::Entity::find()
                .filter(membership::Column::RefId.eq(old.id))
                .filter(membership::Column::Current.eq(1))
                .all(&s.db)
                .await?
        } else {
            vec![]
        };
        let mut reachable = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut pending = VecDeque::from([head.clone()]);
        while let Some(sha) = pending.pop_front() {
            if !seen.insert(sha.clone()) {
                continue;
            }
            // Normal fast-forward: the old published head proves its full
            // ancestor set. Widened history must revisit excluded facts.
            if claim.source.state == "complete" && old.is_some_and(|r| r.head == sha) {
                reachable.extend(old_members.iter().map(|m| m.commit_id));
                continue;
            }
            // New branch at a known published head reuses the entire relation.
            if claim.source.state == "complete" {
                if let Some(known) = old_refs.iter().find(|r| r.head == sha && r.generation > 0) {
                    let rows = membership::Entity::find()
                        .filter(membership::Column::RefId.eq(known.id))
                        .filter(membership::Column::Current.eq(1))
                        .all(&s.db)
                        .await?;
                    reachable.extend(rows.into_iter().map(|m| m.commit_id));
                    continue;
                }
            }
            let value = match cached(s, claim.source.repository_id, &sha).await? {
                Some(row)
                    if row.analysis_version == ANALYSIS_VERSION
                        && (row.complete == 1
                            || (claim.source.coverage_start != 0
                                && row.committer_date < claim.source.coverage_start)) =>
                {
                    row
                }
                _ => {
                    let value = provider::commit(
                        &access,
                        &claim.connection.provider,
                        &claim.source.remote_path,
                        &sha,
                        claim.source.coverage_start,
                    )
                    .await?;
                    store(s, claim, value).await?
                }
            };
            let parents: Vec<String> = serde_json::from_str(&value.parents).map_err(|_| {
                ApiError::localized(
                    axum::http::StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("repository.analysis.parentInvalid"),
                )
            })?;
            pending.extend(parents);
            reachable.insert(value.id);
            // Persist facts individually; a restart reuses them, while refs
            // remain on their last complete publication until traversal ends.
        }
        completed.insert(name.clone(), (head.clone(), reachable));
    }
    publish(s, claim, &old_refs, completed).await
}

async fn publish(
    s: &AppState,
    claim: &Claim,
    old_refs: &[refs::Model],
    completed: BTreeMap<String, (String, BTreeSet<i64>)>,
) -> Result<()> {
    let tx = crate::variables::transaction(s, claim.source.workspace_id.into()).await?;
    fence(&tx, claim).await?;
    refs::Entity::update_many()
        .col_expr(refs::Column::Present, Expr::value(0_i64))
        .filter(refs::Column::SourceId.eq(claim.source.id))
        .exec(&tx)
        .await?;
    for (name, (head, reachable)) in completed {
        let old = old_refs.iter().find(|r| r.name == name);
        let id = old.map(|r| r.id).unwrap_or(i64::from(s.next_id()?));
        let reference = refs::Model {
            id,
            workspace_id: claim.source.workspace_id,
            source_id: claim.source.id,
            name,
            head,
            present: 1,
            generation: claim.job.generation,
            updated_at: db::now(),
        };
        if old.is_some() {
            refs::Entity::update(reference.into_active_model().reset_all())
                .exec(&tx)
                .await?;
        } else {
            refs::Entity::insert(reference.into_active_model())
                .exec(&tx)
                .await?;
        }
        let previous = membership::Entity::find()
            .filter(membership::Column::RefId.eq(id))
            .all(&tx)
            .await?;
        let mut existing = BTreeSet::new();
        for mut member in previous {
            existing.insert(member.commit_id);
            let current = i64::from(reachable.contains(&member.commit_id));
            if member.current != current {
                member.current = current;
                member.generation = claim.job.generation;
                membership::Entity::update(member.into_active_model().reset_all())
                    .exec(&tx)
                    .await?;
            }
        }
        let mut fresh = Vec::new();
        for commit_id in reachable.difference(&existing) {
            fresh.push(
                membership::Model {
                    id: s.next_id()?.into(),
                    workspace_id: claim.source.workspace_id,
                    source_id: claim.source.id,
                    ref_id: id,
                    commit_id: *commit_id,
                    generation: claim.job.generation,
                    current: 1,
                }
                .into_active_model(),
            );
        }
        for chunk in fresh.chunks(500) {
            membership::Entity::insert_many(chunk.to_vec())
                .exec(&tx)
                .await?;
        }
    }
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("complete"))
        .col_expr(sources::Column::LastError, Expr::value(""))
        .col_expr(sources::Column::SyncedAt, Expr::value(db::now()))
        .filter(sources::Column::Id.eq(claim.source.id))
        .exec(&tx)
        .await?;
    insight_identity::refresh_repository(s, &tx, &claim.source).await?;
    tx.commit().await?;
    Ok(())
}
