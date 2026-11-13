//! One leased replacement operation: acknowledge locally, perform remote work in the background.
use super::{
    events::{self, Kind, NewEvent},
    hooks::{self, Hook},
    replace::{self, Checked, ReplaceTrigger},
    *,
};
use crate::blueprint::{
    failover::{self, FailoverClass, HostFacts},
    model::Role,
};
use crate::entities::{health_checks, locks};
use sea_orm::ConnectionTrait;

const LEASE_MS: i64 = 120_000;
const RENEW_SECS: u64 = 30;

pub(crate) struct Job {
    pub workspace: Id,
    pub deployment: Id,
    pub role: String,
    pub index: u32,
    pub trigger: ReplaceTrigger,
    pub actor: Id,
}
pub(crate) struct Pending {
    pub operation: Id,
    job: Job,
    checked: Checked,
    owner: String,
    peers: BTreeMap<String, Vec<(Id, String)>>,
}
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    Succeeded { revision: Id },
    Failed { reason: &'static str },
    Manual { reason: &'static str },
}
pub(crate) fn quorum_ok(members: usize, healthy: usize) -> bool {
    healthy > members / 2
}
fn lock_name(deployment: Id) -> String {
    format!("failover:{deployment}")
}
fn busy() -> ApiError {
    ApiError::localized(
        StatusCode::CONFLICT,
        Message::new("deployment.failoverBusy"),
    )
}

/// Ordinary mutations cannot race an accepted replacement before its revision is queued.
pub(super) async fn ensure_idle(conn: &impl ConnectionTrait, deployment: Id) -> Result<()> {
    if locks::Entity::find_by_id(lock_name(deployment))
        .filter(locks::Column::ExpiresAt.gt(db::now()))
        .one(conn)
        .await?
        .is_some()
    {
        return Err(busy());
    }
    Ok(())
}

async fn instance_state(
    s: &AppState,
    app: &apps::Model,
    host: Id,
    instance: &str,
) -> Result<String> {
    let checks = health_checks::Entity::find()
        .filter(health_checks::Column::WorkspaceId.eq(app.workspace_id))
        .filter(health_checks::Column::DeploymentId.eq(app.id))
        .filter(health_checks::Column::HostId.eq(host))
        .filter(health_checks::Column::Kind.eq("application"))
        .filter(health_checks::Column::Enabled.eq(1))
        .order_by_desc(health_checks::Column::CheckedAt)
        .all(&s.db)
        .await?;
    for check in checks {
        if matches!(
            crate::monitoring::state(&check, db::now()).as_str(),
            "unknown" | "stale" | "paused" | "pending"
        ) {
            continue;
        }
        let result: crate::monitoring::CheckResult =
            serde_json::from_str(&check.result).unwrap_or_default();
        if result.observation.revision_id != Id::from(app.current_revision).to_string() {
            continue;
        }
        if let Some(own) = result
            .observation
            .instances
            .iter()
            .find(|item| item.service == instance)
        {
            return Ok(own.state.clone());
        }
    }
    Ok("unknown".into())
}

/// Cached quorum check only: no channel opens or remote lifecycle commands in a request.
async fn peers(
    s: &AppState,
    checked: &Checked,
    role: &Role,
    index: u32,
) -> Result<Vec<(Id, String)>> {
    let after = failover::settings(checked.spec.as_ref()).host_down_after;
    let mut healthy = Vec::new();
    for (i, member) in checked.binding.roles[&role.name].iter().enumerate() {
        if i == index as usize {
            continue;
        }
        let host = hosts::Entity::find_by_id(i64::from(member.host_id))
            .filter(hosts::Column::WorkspaceId.eq(checked.app.workspace_id))
            .one(&s.db)
            .await?;
        let Some(host) = host else {
            continue;
        };
        let facts = HostFacts {
            runner: host.connection == "runner",
            seen_at: if host.seen_at > 0 {
                host.seen_at
            } else {
                host.created_at
            },
        };
        let instance = format!("{}-{i}", role.name);
        if host.state == "online"
            && !failover::is_stale(&facts, db::now(), after)
            && instance_state(s, &checked.app, member.host_id, &instance).await? != "down"
        {
            healthy.push((member.host_id, instance));
        }
    }
    Ok(healthy)
}

async fn event(
    s: &AppState,
    job: &Job,
    operation: Id,
    kind: Kind,
    host: Id,
    revision: Id,
    mut data: Value,
) -> Result<Id> {
    if operation != Id::from(0) {
        data["operation_id"] = json!(operation);
    }
    let instance = format!("{}-{}", job.role, job.index);
    let message = match kind {
        Kind::ReplaceStarted => Message::new("deployment.event.replaceStarted")
            .with("instance", &instance)
            .with("from", host.to_string())
            .with_message("trigger", job.trigger.message()),
        Kind::RevisionQueued => Message::new("deployment.event.revisionQueued")
            .with("instance", &instance)
            .with("revision", revision.to_string()),
        Kind::ReplaceSucceeded => Message::new("deployment.event.replaceSucceeded")
            .with("instance", &instance)
            .with("to", host.to_string())
            .with("revision", revision.to_string()),
        Kind::NeedsAttention => Message::new("deployment.event.needsAttention")
            .with("instance", &instance)
            .with_message(
                "reason",
                Message::new(format!(
                    "deployment.failoverReason.{}",
                    data["reason"].as_str().unwrap_or("interrupted")
                )),
            ),
        _ => Message::new("deployment.event.replaceFailed")
            .with("instance", &instance)
            .with_message(
                "reason",
                Message::new(format!(
                    "deployment.failoverReason.{}",
                    data["reason"].as_str().unwrap_or("interrupted")
                )),
            ),
    };
    events::record(
        s,
        &s.db,
        NewEvent {
            workspace: job.workspace,
            deployment: job.deployment,
            kind,
            instance,
            host,
            revision,
            message,
            data,
        },
    )
    .await
}

pub(crate) async fn begin(
    s: &AppState,
    scope: &Scope,
    job: Job,
    expected: Option<i64>,
) -> Result<Pending> {
    if job.workspace != scope.workspace_id || job.actor != scope.user.id {
        return Err(ApiError::missing());
    }
    scope.require("deployments.apply")?;
    scoped(&s.db, scope, &job.deployment.to_string()).await?;
    ensure_idle(&s.db, job.deployment).await?;
    let checked = replace::check(&s.db, scope, job.deployment, &job.role, job.index).await?;
    if expected.is_some_and(|version| version != checked.app.version) {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("deployment.error.030"),
        ));
    }
    // A unique owner per operation; using the server instance ID would let two requests share it.
    let operation = s.next_id()?;
    let owner = operation.to_string();
    let tx = crate::variables::transaction(s, job.workspace).await?;
    if !crate::lock::acquire(&tx, &lock_name(job.deployment), &owner, db::now(), LEASE_MS).await? {
        return Err(busy());
    }
    tx.commit().await?;
    let result = async {
        let current = replace::check(&s.db, scope, job.deployment, &job.role, job.index).await?;
        if current.app.version != checked.app.version {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                Message::new("deployment.error.030"),
            ));
        }
        let history = events::history(
            &s.db,
            job.workspace,
            job.deployment,
            &format!("{}-{}", job.role, job.index),
        )
        .await?;
        if history.open {
            let interrupted = crate::entities::deployment_events::Entity::find()
                .filter(crate::entities::deployment_events::Column::WorkspaceId.eq(job.workspace))
                .filter(crate::entities::deployment_events::Column::DeploymentId.eq(job.deployment))
                .filter(
                    crate::entities::deployment_events::Column::Instance
                        .eq(format!("{}-{}", job.role, job.index)),
                )
                .filter(
                    crate::entities::deployment_events::Column::Kind
                        .eq(Kind::ReplaceStarted.as_str()),
                )
                .order_by_desc(crate::entities::deployment_events::Column::Id)
                .one(&s.db)
                .await?
                .map(|e| Id::from(e.id))
                .ok_or_else(ApiError::missing)?;
            event(
                s,
                &job,
                interrupted,
                Kind::ReplaceFailed,
                current.from,
                Id::from(0),
                json!({"reason": "interrupted"}),
            )
            .await?;
        }
        let mut healthy = BTreeMap::new();
        for role in crate::blueprint::placement::colocated(
            &current.bp,
            current
                .bp
                .roles
                .iter()
                .find(|r| r.name == job.role)
                .unwrap(),
        ) {
            if failover::class_of(role) != FailoverClass::Replicated {
                continue;
            }
            let available = peers(s, &current, role, job.index).await?;
            if !quorum_ok(current.binding.roles[&role.name].len(), available.len()) {
                event(
                    s,
                    &job,
                    Id::from(0),
                    Kind::NeedsAttention,
                    current.from,
                    Id::from(0),
                    json!({"reason": "quorumLost"}),
                )
                .await?;
                return Err(ApiError::localized(
                    StatusCode::CONFLICT,
                    Message::new("deployment.replaceQuorumLost"),
                ));
            }
            healthy.insert(role.name.clone(), available);
        }
        // The public operation ID is the start event itself, not a speculative revision.
        events::record(
            s,
            &s.db,
            NewEvent {
                workspace: job.workspace,
                deployment: job.deployment,
                kind: Kind::ReplaceStarted,
                instance: format!("{}-{}", job.role, job.index),
                host: current.from,
                revision: Id::from(0),
                message: Message::new("deployment.event.replaceStarted")
                    .with("instance", format!("{}-{}", job.role, job.index))
                    .with("from", current.from.to_string())
                    .with_message("trigger", job.trigger.message()),
                data: json!({"operation_id": operation, "trigger": job.trigger.action()}),
            },
        )
        .await
        .and_then(|start| {
            Ok(Pending {
                operation: start,
                job,
                checked: current,
                owner,
                peers: healthy,
            })
        })
    }
    .await;
    if result.is_err() {
        crate::lock::release(
            &s.db,
            &lock_name(checked.app.id.into()),
            &operation.to_string(),
        )
        .await?;
    }
    result
}

async fn authorized(s: &AppState, pending: &Pending) -> Result<Scope> {
    let job = &pending.job;
    let lease = locks::Entity::find_by_id(lock_name(job.deployment))
        .one(&s.db)
        .await?;
    if !lease.is_some_and(|lock| lock.owner == pending.owner && lock.expires_at > db::now()) {
        return Err(ApiError::localized(
            StatusCode::SERVICE_UNAVAILABLE,
            Message::new("deployment.failoverReason.interrupted"),
        ));
    }
    let scope = Scope::for_actor(s, job.workspace, job.actor).await?;
    scope.require("deployments.apply")?;
    Ok(scope)
}

fn reason(error: &ApiError) -> &'static str {
    match error.2.as_ref().map(|m| m.key.as_str()) {
        Some("blueprint.replaceNoCandidate") => "noCandidate",
        _ if error.0 == StatusCode::FORBIDDEN => "actorRevoked",
        _ if error.0 == StatusCode::CONFLICT => "conflict",
        _ => "interrupted",
    }
}

async fn perform(s: &AppState, pending: &Pending) -> Result<Outcome> {
    let job = &pending.job;
    let checked = &pending.checked;
    authorized(s, pending).await?;
    // Revalidate cached quorum with lifecycle health hooks, where the template supplies one.
    let mut verified = pending.peers.clone();
    for (name, available) in &mut verified {
        let role = checked.bp.roles.iter().find(|r| &r.name == name).unwrap();
        if let Some(argv) = &role.hooks.healthy {
            let mut healthy = Vec::new();
            for (host, instance) in available.iter() {
                authorized(s, pending).await?;
                if hooks::run(
                    s,
                    &checked.app,
                    *host,
                    instance,
                    Hook::Healthy,
                    argv,
                    hooks::HEALTHY_ATTEMPT_SECS,
                    pending.operation,
                )
                .await?
                .code
                    == 0
                {
                    healthy.push((*host, instance.clone()));
                }
            }
            if !quorum_ok(checked.binding.roles[name].len(), healthy.len()) {
                return Ok(Outcome::Manual {
                    reason: "quorumLost",
                });
            }
            *available = healthy;
        }
    }
    for (name, available) in &verified {
        let role = checked.bp.roles.iter().find(|r| &r.name == name).unwrap();
        if let Some(argv) = &role.hooks.leave {
            authorized(s, pending).await?;
            let (host, instance) = &available[0];
            if hooks::run(
                s,
                &checked.app,
                *host,
                instance,
                Hook::Leave,
                argv,
                hooks::HOOK_TIMEOUT_SECS,
                pending.operation,
            )
            .await?
            .code
                != 0
            {
                return Ok(Outcome::Failed {
                    reason: "hookFailed",
                });
            }
        }
    }
    let scope = authorized(s, pending).await?;
    let current = replace::check(&s.db, &scope, job.deployment, &job.role, job.index).await?;
    if current.app.version != checked.app.version {
        return Ok(Outcome::Failed { reason: "conflict" });
    }
    let trigger = match job.trigger {
        ReplaceTrigger::Manual => ReplaceTrigger::Manual,
        ReplaceTrigger::Auto { failed_host } => ReplaceTrigger::Auto { failed_host },
    };
    let revision = replace::replace_checked(
        s,
        &scope,
        current,
        &job.role,
        job.index,
        trigger,
        Some(checked.app.version),
    )
    .await
    .map_err(|stopped| {
        tracing::debug!(operation = %pending.operation, binding_moved = stopped.binding_moved, "replacement stopped before queueing");
        stopped.error
    })?;
    let moved = scoped(&s.db, &scope, &job.deployment.to_string()).await?;
    let binding =
        crate::blueprint::binding::parse(moved.blueprint_binding.as_deref().unwrap_or_default())?;
    let destination = binding.roles[&job.role][job.index as usize].host_id;
    event(
        s,
        job,
        pending.operation,
        Kind::RevisionQueued,
        destination,
        revision,
        json!({}),
    )
    .await?;
    let health_timeout = u64::from(checked.bp.update.health_timeout);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2 * health_timeout + 600);
    loop {
        let rev = revisions::Entity::find_by_id(i64::from(revision))
            .filter(revisions::Column::WorkspaceId.eq(job.workspace))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if rev.state == "succeeded" {
            break;
        }
        if matches!(rev.state.as_str(), "failed" | "needs-review" | "cancelled")
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(Outcome::Failed {
                reason: "revisionFailed",
            });
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let app = scoped(
        &s.db,
        &authorized(s, pending).await?,
        &job.deployment.to_string(),
    )
    .await?;
    for instance in &checked.instances {
        let (name, _) = instance.rsplit_once('-').unwrap();
        let role = checked.bp.roles.iter().find(|r| r.name == name).unwrap();
        if failover::class_of(role) != FailoverClass::Replicated {
            continue;
        }
        if let Some(argv) = &role.hooks.join {
            authorized(s, pending).await?;
            if hooks::run(
                s,
                &app,
                destination,
                instance,
                Hook::Join,
                argv,
                hooks::HOOK_TIMEOUT_SECS,
                pending.operation,
            )
            .await?
            .code
                != 0
            {
                return Ok(Outcome::Failed {
                    reason: "hookFailed",
                });
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(health_timeout);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Ok(Outcome::Failed {
                    reason: "unhealthy",
                });
            }
            let remaining = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_secs()
                .saturating_add(1);
            authorized(s, pending).await?;
            let healthy = match &role.hooks.healthy {
                Some(argv) => {
                    hooks::run(
                        s,
                        &app,
                        destination,
                        instance,
                        Hook::Healthy,
                        argv,
                        std::cmp::min(hooks::HEALTHY_ATTEMPT_SECS, remaining),
                        pending.operation,
                    )
                    .await?
                    .code
                        == 0
                }
                None => matches!(
                    instance_state(s, &app, destination, instance)
                        .await?
                        .as_str(),
                    "up" | "unknown"
                ),
            };
            if healthy {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(Outcome::Failed {
                    reason: "unhealthy",
                });
            }
            tokio::time::sleep(
                Duration::from_secs(5)
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            )
            .await;
        }
    }
    event(
        s,
        job,
        pending.operation,
        Kind::ReplaceSucceeded,
        destination,
        revision,
        json!({}),
    )
    .await?;
    Ok(Outcome::Succeeded { revision })
}

pub(crate) async fn run(s: AppState, pending: Pending) -> Result<Outcome> {
    let work = perform(&s, &pending);
    tokio::pin!(work);
    let mut renew = tokio::time::interval(Duration::from_secs(RENEW_SECS));
    renew.tick().await;
    let outcome = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = renew.tick() => {
                let renewed = locks::Entity::update_many().col_expr(locks::Column::ExpiresAt, Expr::value(db::now() + LEASE_MS))
                    .filter(locks::Column::Name.eq(lock_name(pending.job.deployment)))
                    .filter(locks::Column::Owner.eq(&pending.owner)).filter(locks::Column::ExpiresAt.gt(db::now()))
                    .exec(&s.db).await;
                if !renewed.is_ok_and(|result| result.rows_affected == 1) { break Ok(Outcome::Failed { reason: "interrupted" }); }
            }
        }
    };
    let outcome = outcome.unwrap_or_else(|error| Outcome::Failed {
        reason: reason(&error),
    });
    let terminal = async {
        let why = match outcome { Outcome::Failed { reason } | Outcome::Manual { reason } => Some(reason), _ => None };
        if let Some(reason) = why {
            // A new lease owner may already have closed this abandoned operation.
            let closed = crate::entities::deployment_events::Entity::find()
                .filter(crate::entities::deployment_events::Column::WorkspaceId.eq(pending.job.workspace))
                .filter(crate::entities::deployment_events::Column::DeploymentId.eq(pending.job.deployment))
                .filter(crate::entities::deployment_events::Column::Instance.eq(format!("{}-{}", pending.job.role, pending.job.index)))
                .filter(crate::entities::deployment_events::Column::Kind.is_in([Kind::ReplaceFailed.as_str(), Kind::ReplaceSucceeded.as_str()]))
                .all(&s.db).await?.into_iter().any(|e| serde_json::from_str::<Value>(&e.detail).ok().is_some_and(|v| v["operation_id"] == json!(pending.operation)));
            if closed { return Ok(()); }
            let queued = crate::entities::deployment_events::Entity::find()
                .filter(crate::entities::deployment_events::Column::WorkspaceId.eq(pending.job.workspace))
                .filter(crate::entities::deployment_events::Column::DeploymentId.eq(pending.job.deployment))
                .filter(crate::entities::deployment_events::Column::Kind.eq(Kind::RevisionQueued.as_str()))
                .order_by_desc(crate::entities::deployment_events::Column::Id).all(&s.db).await?
                .into_iter().find(|e| serde_json::from_str::<Value>(&e.detail).ok().is_some_and(|v| v["operation_id"] == json!(pending.operation)));
            let app = apps::Entity::find_by_id(i64::from(pending.job.deployment))
                .filter(apps::Column::WorkspaceId.eq(pending.job.workspace)).one(&s.db).await?;
            let moved = app.is_some_and(|a| a.blueprint_binding != pending.checked.app.blueprint_binding);
            event(&s, &pending.job, pending.operation, Kind::ReplaceFailed, pending.checked.from,
                queued.map_or(Id::from(0), |e| e.revision_id.into()), json!({"reason": reason, "bindingMoved": moved})).await?;
            let history = events::history(&s.db, pending.job.workspace, pending.job.deployment, &format!("{}-{}", pending.job.role, pending.job.index)).await?;
            if matches!(outcome, Outcome::Manual { .. }) || history.failures >= events::MAX_FAILURES {
                event(&s, &pending.job, pending.operation, Kind::NeedsAttention, pending.checked.from, Id::from(0), json!({"reason": if matches!(outcome, Outcome::Manual { .. }) { reason } else { "tooManyFailures" }})).await?;
            }
        }
        Ok::<_, ApiError>(())
    }.await;
    crate::lock::release(&s.db, &lock_name(pending.job.deployment), &pending.owner).await?;
    terminal?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quorum_rule() {
        for (members, healthy, expected) in [
            (3, 2, true),
            (3, 1, false),
            (5, 3, true),
            (5, 2, false),
            (1, 0, false),
            (0, 0, false),
        ] {
            assert_eq!(quorum_ok(members, healthy), expected);
        }
    }
}
