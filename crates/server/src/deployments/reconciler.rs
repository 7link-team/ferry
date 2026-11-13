//! Host failure detection and automatic dispatch; replacement owns all destructive fencing.
use super::{
    events::{self, Kind, NewEvent},
    sequence, *,
};
use crate::blueprint::{
    binding,
    failover::{self, Evidence, HostFacts, PeerCheck, Response, Verdict},
    model, spec,
};
use crate::entities::{deployment_events, deployment_fences as fences, locks};
use sea_orm::ConnectionTrait;
use std::collections::{BTreeSet, HashSet};

pub(crate) const TICK: Duration = Duration::from_secs(15);
pub(crate) const LEASE_NAME: &str = "deployment-reconciler";
pub(crate) const LEASE_MS: i64 = 60_000;

// Unlike the generic lock helper, a coordinator may never revive an expired lease.
async fn renew(conn: &impl ConnectionTrait, owner: &str, now: i64) -> Result<bool> {
    Ok(locks::Entity::update_many()
        .col_expr(locks::Column::ExpiresAt, Expr::value(now + LEASE_MS))
        .filter(locks::Column::Name.eq(LEASE_NAME))
        .filter(locks::Column::Owner.eq(owner))
        .filter(locks::Column::ExpiresAt.gt(now))
        .exec(conn)
        .await?
        .rows_affected
        == 1)
}

#[async_trait::async_trait]
pub(crate) trait Probe: Send + Sync {
    async fn server(&self, s: &AppState, workspace: Id, host: Id) -> bool;
    async fn peer(
        &self,
        s: &AppState,
        workspace: Id,
        peer: Id,
        address: &str,
        port: u16,
    ) -> PeerCheck;
}
pub(crate) struct LiveProbe;

pub(crate) fn peer_check_command(address: &str, port: u16) -> String {
    let tcp = quote(&format!("/dev/tcp/{address}/{port}"));
    let bash = quote(&format!("exec 3<>{tcp}"));
    format!(
        "if ! command -v timeout >/dev/null 2>&1; then exit 127; elif command -v nc >/dev/null 2>&1; then timeout 3 nc -z -- {} {}; elif command -v bash >/dev/null 2>&1; then timeout 3 bash -c {bash}; else exit 127; fi",
        quote(address),
        quote(&port.to_string())
    )
}

#[async_trait::async_trait]
impl Probe for LiveProbe {
    async fn server(&self, s: &AppState, workspace: Id, host: Id) -> bool {
        tokio::time::timeout(Duration::from_secs(10), async {
            remote::open(s, &workspace, &host)
                .await?
                .exec("echo ferry-ok", Duration::from_secs(10))
                .await
        })
        .await
        .is_ok_and(|r| r.is_ok_and(|o| o.code == 0 && o.stdout.trim() == "ferry-ok"))
    }
    async fn peer(
        &self,
        s: &AppState,
        workspace: Id,
        peer: Id,
        address: &str,
        port: u16,
    ) -> PeerCheck {
        match tokio::time::timeout(Duration::from_secs(10), async {
            remote::open(s, &workspace, &peer)
                .await?
                .exec(&peer_check_command(address, port), Duration::from_secs(5))
                .await
        })
        .await
        {
            Ok(Ok(o)) if o.code == 0 => PeerCheck::Reachable(peer),
            Ok(Ok(o)) if o.code != 127 => PeerCheck::Unreachable(peer),
            _ => PeerCheck::Inconclusive,
        }
    }
}

#[derive(Default)]
pub(crate) struct Episodes(HashMap<(i64, i64), HashSet<String>>);

fn facts(host: &hosts::Model) -> HostFacts {
    HostFacts {
        runner: host.connection == "runner",
        seen_at: if host.seen_at > 0 {
            host.seen_at
        } else {
            host.created_at
        },
    }
}
fn settings(app: &apps::Model) -> Result<spec::FailoverSpec> {
    let spec = app.deploy_spec.as_deref().map(spec::parse).transpose()?;
    Ok(failover::settings(spec.as_ref()))
}
async fn record(
    s: &AppState,
    conn: &impl ConnectionTrait,
    app: &apps::Model,
    kind: Kind,
    instance: &str,
    host: Id,
    message: Message,
    data: Value,
) -> Result<()> {
    events::record(
        s,
        conn,
        NewEvent {
            workspace: app.workspace_id.into(),
            deployment: app.id.into(),
            kind,
            instance: instance.into(),
            host,
            revision: app.current_revision.into(),
            message,
            data,
        },
    )
    .await?;
    Ok(())
}
async fn once(
    s: &AppState,
    app: &apps::Model,
    episodes: &mut Episodes,
    kind: Kind,
    instance: &str,
    host: Id,
    message: Message,
    data: Value,
) -> Result<()> {
    let key = format!("{}:{instance}", kind.as_str());
    let episode = episodes.0.entry((app.id, host.into())).or_default();
    if !episode.contains(&key) {
        record(s, &s.db, app, kind, instance, host, message, data).await?;
        episode.insert(key);
    }
    Ok(())
}
async fn server(
    s: &AppState,
    probe: &dyn Probe,
    cache: &mut HashMap<(i64, i64), bool>,
    workspace: Id,
    host: Id,
) -> bool {
    let key = (workspace.into(), host.into());
    if let Some(ok) = cache.get(&key) {
        return *ok;
    }
    let ok = probe.server(s, workspace, host).await;
    cache.insert(key, ok);
    ok
}

/// A tick returns intentions only; sequence::begin rechecks version, permission and exclusivity.
pub(crate) async fn tick(
    s: &AppState,
    probe: &dyn Probe,
    now_ms: i64,
    episodes: &mut Episodes,
) -> Result<Vec<sequence::Job>> {
    let apps = apps::Entity::find()
        .filter(apps::Column::TargetKind.eq("blueprint"))
        .all(&s.db)
        .await?;
    let live: HashSet<_> = apps.iter().map(|a| a.id).collect();
    episodes
        .0
        .retain(|(deployment, _), _| live.contains(deployment));
    let mut cache = HashMap::new();
    let mut jobs = Vec::new();
    for app in apps {
        // One malformed legacy deployment must not starve other deployments.
        if let Err(error) =
            deployment_tick(s, probe, now_ms, episodes, &mut cache, &app, &mut jobs).await
        {
            tracing::warn!(deployment = app.id, %error, "failover tick failed");
        }
    }
    Ok(jobs)
}

async fn deployment_tick(
    s: &AppState,
    probe: &dyn Probe,
    now: i64,
    episodes: &mut Episodes,
    cache: &mut HashMap<(i64, i64), bool>,
    app: &apps::Model,
    jobs: &mut Vec<sequence::Job>,
) -> Result<()> {
    let config = settings(app)?;
    recover(s, probe, now, cache, app, config.host_down_after).await?;
    if app.current_revision == 0 {
        return Ok(());
    }
    let bp = model::parse(&app.manifest)?;
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let ids: BTreeSet<i64> = binding
        .roles
        .values()
        .flatten()
        .map(|i| i.host_id.into())
        .collect();
    let hosts = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(app.workspace_id))
        .filter(hosts::Column::Id.is_in(ids))
        .all(&s.db)
        .await?;
    let revision = revisions::Entity::find_by_id(app.current_revision)
        .filter(revisions::Column::WorkspaceId.eq(app.workspace_id))
        .filter(revisions::Column::DeploymentId.eq(app.id))
        .one(&s.db)
        .await?;
    let mut scheduled = false;
    for host in &hosts {
        let roles: Vec<_> =
            bp.roles
                .iter()
                .filter(|r| {
                    binding.roles.get(&r.name).is_some_and(|members| {
                        members.iter().any(|i| i.host_id == Id::from(host.id))
                    })
                })
                .collect();
        if roles
            .iter()
            .all(|r| failover::respond(config.mode, &binding.driver, r) == Response::Ignore)
        {
            continue;
        }
        let facts = facts(host);
        let evidence = if failover::is_stale(&facts, now, config.host_down_after) {
            let ok = server(s, probe, cache, app.workspace_id.into(), host.id.into()).await;
            let mut peer = PeerCheck::NoPeer;
            if !ok && !facts.runner {
                // Prefer a fresh bound peer; a failed peer channel is inconclusive, never evidence of target failure.
                for other in &hosts {
                    if other.id != host.id
                        && !failover::is_stale(&self::facts(other), now, config.host_down_after)
                    {
                        peer = probe
                            .peer(
                                s,
                                app.workspace_id.into(),
                                other.id.into(),
                                &host.address,
                                u16::try_from(host.port).unwrap_or(22),
                            )
                            .await;
                        if !matches!(peer, PeerCheck::Inconclusive | PeerCheck::NoPeer) {
                            break;
                        }
                    }
                }
            }
            Some(Evidence {
                server_ok: ok,
                peer: Some(peer),
            })
        } else {
            None
        };
        let host_id = host.id.into();
        let verdict = failover::judge(&facts, now, config.host_down_after, evidence.as_ref());
        match verdict {
            Verdict::Healthy => {
                episodes.0.remove(&(app.id, host.id));
                continue;
            }
            Verdict::Vetoed(peer) => {
                once(
                    s,
                    app,
                    episodes,
                    Kind::PeerVeto,
                    "",
                    host_id,
                    Message::new("deployment.event.peerVeto")
                        .with("peer", peer.to_string())
                        .with("host", &host.name),
                    json!({"peer":peer}),
                )
                .await?;
                continue;
            }
            Verdict::Suspected | Verdict::NeedsProbe => {
                once(
                    s,
                    app,
                    episodes,
                    Kind::HostSuspected,
                    "",
                    host_id,
                    Message::new("deployment.event.hostSuspected").with("host", &host.name),
                    json!({}),
                )
                .await?;
                continue;
            }
            Verdict::Down { single_source } => {
                let message = if single_source {
                    Message::new("deployment.event.hostConfirmedDownSingleSource")
                        .with("host", &host.name)
                        .with("window", (config.host_down_after * 2).to_string())
                } else {
                    let peer = match evidence.as_ref().and_then(|e| e.peer) {
                        Some(PeerCheck::Unreachable(id)) => id.to_string(),
                        _ => String::new(),
                    };
                    Message::new("deployment.event.hostConfirmedDown")
                        .with("host", &host.name)
                        .with("peer", peer)
                };
                once(
                    s,
                    app,
                    episodes,
                    Kind::HostConfirmedDown,
                    "",
                    host_id,
                    message,
                    json!({"single_source":single_source}),
                )
                .await?;
            }
        }
        for role in roles {
            for (index, member) in binding.roles[&role.name].iter().enumerate() {
                if member.host_id != host_id {
                    continue;
                }
                let instance = format!("{}-{index}", role.name);
                match failover::respond(config.mode, &binding.driver, role) {
                    Response::Ignore => {}
                    Response::AlertOnly(reason) => {
                        let reason = match reason {
                            failover::AlertReason::ModeAlert => "modeAlert",
                            failover::AlertReason::NeedsBackup => "needsBackup",
                            failover::AlertReason::DriverUnsupported => "driverUnsupported",
                        };
                        once(
                            s,
                            app,
                            episodes,
                            Kind::AlertOnly,
                            &instance,
                            host_id,
                            Message::new("deployment.event.alertOnly")
                                .with("host", &host.name)
                                .with("instance", &instance)
                                .with_message(
                                    "reason",
                                    Message::new(format!("deployment.failoverReason.{reason}")),
                                ),
                            json!({"reason":reason}),
                        )
                        .await?;
                    }
                    Response::Replace(_) => {
                        if scheduled
                            || matches!(app.state.as_str(), "queued" | "running" | "needs-review")
                            || sequence::ensure_idle(&s.db, app.id.into()).await.is_err()
                        {
                            continue;
                        }
                        let history = events::history(
                            &s.db,
                            app.workspace_id.into(),
                            app.id.into(),
                            &instance,
                        )
                        .await?;
                        if history.attention || history.open || events::in_cooldown(&history, now) {
                            continue;
                        }
                        if let Some(revision) = &revision {
                            jobs.push(sequence::Job {
                                workspace: app.workspace_id.into(),
                                deployment: app.id.into(),
                                role: role.name.clone(),
                                index: index as u32,
                                trigger: replace::ReplaceTrigger::Auto {
                                    failed_host: host_id,
                                },
                                actor: revision.triggered_by.into(),
                            });
                            scheduled = true;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn recover(
    s: &AppState,
    probe: &dyn Probe,
    now: i64,
    cache: &mut HashMap<(i64, i64), bool>,
    app: &apps::Model,
    after: u32,
) -> Result<()> {
    let pending = fences::Entity::find()
        .filter(fences::Column::WorkspaceId.eq(app.workspace_id))
        .filter(fences::Column::DeploymentId.eq(app.id))
        .filter(fences::Column::ClearedAt.eq(0))
        .all(&s.db)
        .await?;
    for fence in pending {
        let host = hosts::Entity::find_by_id(fence.host_id)
            .filter(hosts::Column::WorkspaceId.eq(app.workspace_id))
            .one(&s.db)
            .await?;
        let Some(host) = host else {
            continue;
        };
        if failover::is_stale(&facts(&host), now, after)
            || !server(s, probe, cache, app.workspace_id.into(), host.id.into()).await
        {
            continue;
        }
        let stopped = tokio::time::timeout(Duration::from_secs(60), async {
            remote::open(s, &app.workspace_id.into(), &host.id.into())
                .await?
                .exec(
                    &replace::stop_command(app, &fence.instance),
                    Duration::from_secs(60),
                )
                .await
        })
        .await
        .is_ok_and(|r| r.is_ok_and(|o| o.code == 0));
        if !stopped {
            continue;
        }
        let tx = crate::variables::transaction(s, app.workspace_id.into()).await?;
        let changed = fences::Entity::update_many()
            .col_expr(fences::Column::ClearedAt, Expr::value(now))
            .filter(fences::Column::Id.eq(fence.id))
            .filter(fences::Column::WorkspaceId.eq(app.workspace_id))
            .filter(fences::Column::ClearedAt.eq(0))
            .exec(&tx)
            .await?
            .rows_affected;
        if changed == 1 {
            record(
                s,
                &tx,
                app,
                Kind::Unfenced,
                &fence.instance,
                host.id.into(),
                Message::new("deployment.event.unfenced")
                    .with("host", &host.name)
                    .with("instance", &fence.instance),
                json!({}),
            )
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

pub async fn run(s: AppState, mut drain: tokio::sync::watch::Receiver<bool>) {
    let mut timer = tokio::time::interval(TICK);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut episodes = Episodes::default();
    loop {
        tokio::select! { biased;
            changed = drain.changed() => { if changed.is_err() || *drain.borrow() { break; } }
            _ = timer.tick() => {
                if *drain.borrow() { break; }
                match crate::lock::acquire(&s.db, LEASE_NAME, &s.instance_id, db::now(), LEASE_MS).await {
                    Ok(true) => {}, Ok(false) => continue,
                    Err(error) => { tracing::warn!(%error, "failover lease acquisition failed"); continue; }
                }
                let action_drain = drain.clone();
                let action = async {
                    for job in tick(&s, &LiveProbe, db::now(), &mut episodes).await? {
                        if *action_drain.borrow() { break; }
                        dispatch(&s, job).await?;
                    }
                    Ok::<_, ApiError>(())
                };
                tokio::pin!(action);
                let mut renewal = tokio::time::interval(Duration::from_secs(20));
                renewal.tick().await;
                loop {
                    tokio::select! { biased;
                        _ = drain.changed() => { break; }
                        _ = renewal.tick() => {
                            match renew(&s.db, &s.instance_id, db::now()).await {
                                Ok(true) => {}, _ => { tracing::warn!("failover lease lost"); break; }
                            }
                        }
                        result = &mut action => { if let Err(error) = result { tracing::warn!(%error, "failover dispatch failed"); } break; }
                    }
                }
            }
        }
    }
    if let Err(error) = crate::lock::release(&s.db, LEASE_NAME, &s.instance_id).await {
        tracing::warn!(%error, "failover lease release failed");
    }
}

pub(super) async fn dispatch(s: &AppState, job: sequence::Job) -> Result<()> {
    // Probing a whole fleet can take time. A saved intention cannot override a
    // subsequent heartbeat, placement, revision owner or failover-mode change.
    let app = apps::Entity::find_by_id(i64::from(job.deployment))
        .filter(apps::Column::WorkspaceId.eq(job.workspace))
        .one(&s.db)
        .await?;
    let Some(app) = app else {
        return Ok(());
    };
    if app.target_kind != "blueprint" {
        return Ok(());
    }
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let Some(member) = binding
        .roles
        .get(&job.role)
        .and_then(|v| v.get(job.index as usize))
    else {
        return Ok(());
    };
    let replace::ReplaceTrigger::Auto { failed_host } = job.trigger else {
        return Ok(());
    };
    if member.host_id != failed_host {
        return Ok(());
    }
    let host = hosts::Entity::find_by_id(i64::from(failed_host))
        .filter(hosts::Column::WorkspaceId.eq(job.workspace))
        .one(&s.db)
        .await?;
    let config = settings(&app)?;
    if !host.is_some_and(|h| failover::is_stale(&facts(&h), db::now(), config.host_down_after)) {
        return Ok(());
    }
    let bp = model::parse(&app.manifest)?;
    let Some(role) = bp.roles.iter().find(|r| r.name == job.role) else {
        return Ok(());
    };
    if !matches!(
        failover::respond(config.mode, &binding.driver, role),
        Response::Replace(_)
    ) {
        return Ok(());
    }
    let revision = revisions::Entity::find_by_id(app.current_revision)
        .filter(revisions::Column::WorkspaceId.eq(job.workspace))
        .filter(revisions::Column::DeploymentId.eq(app.id))
        .one(&s.db)
        .await?;
    if !revision.is_some_and(|r| r.triggered_by == i64::from(job.actor)) {
        return Ok(());
    }
    let scope = Scope::for_actor(s, job.workspace, job.actor).await;
    let scope = match scope.and_then(|scope| {
        scope.require("deployments.apply")?;
        Ok(scope)
    }) {
        Ok(scope) => scope,
        Err(error)
            if matches!(
                error.0,
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED
            ) =>
        {
            // Durable attention prevents a revoked owner causing a failure every tick.
            let app = apps::Entity::find_by_id(i64::from(job.deployment))
                .filter(apps::Column::WorkspaceId.eq(job.workspace))
                .one(&s.db)
                .await?
                .ok_or_else(ApiError::missing)?;
            let instance = format!("{}-{}", job.role, job.index);
            let host = match job.trigger {
                replace::ReplaceTrigger::Auto { failed_host } => failed_host,
                _ => 0.into(),
            };
            let tx = crate::variables::transaction(s, job.workspace).await?;
            for kind in [Kind::ReplaceFailed, Kind::NeedsAttention] {
                let key = if kind == Kind::ReplaceFailed {
                    "replaceFailed"
                } else {
                    "needsAttention"
                };
                record(
                    s,
                    &tx,
                    &app,
                    kind,
                    &instance,
                    host,
                    Message::new(format!("deployment.event.{key}"))
                        .with("instance", &instance)
                        .with_message(
                            "reason",
                            Message::new("deployment.failoverReason.actorRevoked"),
                        ),
                    json!({"reason":"actorRevoked"}),
                )
                .await?;
            }
            tx.commit().await?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match sequence::begin(s, &scope, job, Some(app.version)).await {
        Ok(pending) => {
            let s = s.clone();
            tokio::spawn(async move {
                if let Err(error) = sequence::run(s, pending).await {
                    tracing::warn!(%error, "automatic replacement failed");
                }
            });
        }
        Err(error) => tracing::warn!(%error, "automatic replacement was not admitted"),
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckBody {
    version: i64,
}

pub async fn acknowledge_instance(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id, role, index)): Path<(String, String, String, u32)>,
    Json(body): Json<AckBody>,
) -> Result<StatusCode> {
    scope.require("deployments.apply")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let app = scoped(&tx, &scope, &id).await?;
    sequence::ensure_idle(&tx, app.id.into()).await?;
    if body.version != app.version {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("deployment.error.030"),
        ));
    }
    if app.target_kind != "blueprint" {
        return Err(ApiError::missing());
    }
    let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
    let host = binding
        .roles
        .get(&role)
        .and_then(|v| v.get(index as usize))
        .ok_or_else(ApiError::missing)?
        .host_id;
    let instance = format!("{role}-{index}");
    if !events::history(&tx, scope.workspace_id, app.id.into(), &instance)
        .await?
        .attention
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("deployment.failoverNotAttention"),
        ));
    }
    record(
        &s,
        &tx,
        &app,
        Kind::Acknowledged,
        &instance,
        host,
        Message::new("deployment.event.acknowledged")
            .with("user", scope.user.id.to_string())
            .with("instance", &instance),
        json!({"user_id":scope.user.id}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn attention(conn: &impl ConnectionTrait, app: &apps::Model) -> Result<Vec<Value>> {
    let rows = deployment_events::Entity::find()
        .filter(deployment_events::Column::WorkspaceId.eq(app.workspace_id))
        .filter(deployment_events::Column::DeploymentId.eq(app.id))
        .filter(
            deployment_events::Column::Kind
                .is_in([Kind::NeedsAttention.as_str(), Kind::Acknowledged.as_str()]),
        )
        .order_by_asc(deployment_events::Column::CreatedAt)
        .order_by_asc(deployment_events::Column::Id)
        .all(conn)
        .await?;
    let mut pending = BTreeMap::new();
    for row in rows {
        if row.kind == Kind::Acknowledged.as_str() {
            pending.remove(&row.instance);
        } else {
            pending.entry(row.instance).or_insert(row.created_at);
        }
    }
    Ok(pending
        .into_iter()
        .map(|(instance, since)| json!({"instance":instance,"since":since}))
        .collect())
}
pub(crate) async fn summary(conn: &impl ConnectionTrait, app: &apps::Model) -> Result<Value> {
    if app.target_kind != "blueprint" {
        return Ok(Value::Null);
    }
    let config = settings(app)?;
    let fences = fences::Entity::find()
        .filter(fences::Column::WorkspaceId.eq(app.workspace_id))
        .filter(fences::Column::DeploymentId.eq(app.id))
        .filter(fences::Column::ClearedAt.eq(0))
        .order_by_asc(fences::Column::Id)
        .all(conn)
        .await?;
    Ok(
        json!({"mode":config.mode,"host_down_after":config.host_down_after,"fences":fences.into_iter().map(|f| json!({"host_id":Id::from(f.host_id),"instance":f.instance,"created_at":f.created_at})).collect::<Vec<_>>(),"attention":attention(conn, app).await?}),
    )
}

pub(crate) async fn alert(
    conn: &impl ConnectionTrait,
    app: &apps::Model,
    now: i64,
) -> Result<(bool, Message)> {
    let mut instances: HashSet<String> = attention(conn, app)
        .await?
        .into_iter()
        .filter_map(|v| v["instance"].as_str().map(str::to_owned))
        .collect();
    if app.target_kind == "blueprint" {
        let after = settings(app)?.host_down_after;
        let rows = deployment_events::Entity::find()
            .filter(deployment_events::Column::WorkspaceId.eq(app.workspace_id))
            .filter(deployment_events::Column::DeploymentId.eq(app.id))
            .filter(deployment_events::Column::Kind.eq(Kind::AlertOnly.as_str()))
            .all(conn)
            .await?;
        let binding = binding::parse(app.blueprint_binding.as_deref().unwrap_or_default())?;
        for row in rows {
            // Historical alerts do not attach to an instance that has since moved.
            let current = row.instance.rsplit_once('-').and_then(|(role, index)| {
                binding.roles.get(role)?.get(index.parse::<usize>().ok()?)
            });
            if !current.is_some_and(|i| i.host_id == Id::from(row.host_id)) {
                continue;
            }
            let host = hosts::Entity::find_by_id(row.host_id)
                .filter(hosts::Column::WorkspaceId.eq(app.workspace_id))
                .one(conn)
                .await?;
            if host.is_some_and(|h| failover::is_stale(&facts(&h), now, after)) {
                instances.insert(row.instance);
            }
        }
    }
    Ok(if instances.is_empty() {
        (false, Message::new("deployment.alert.failoverRecovered"))
    } else {
        (
            true,
            Message::new("deployment.alert.failoverAttention")
                .with("count", instances.len().to_string()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lease_excludes_other_servers_and_expired_owner_cannot_renew() {
        let (_dir, s) = super::super::blueprint_tests::fixture(&[]).await;
        let other = "another-server";
        assert!(
            crate::lock::acquire(&s.db, LEASE_NAME, &s.instance_id, db::now(), LEASE_MS)
                .await
                .unwrap()
        );
        assert!(
            !crate::lock::acquire(&s.db, LEASE_NAME, other, db::now(), LEASE_MS)
                .await
                .unwrap()
        );
        assert!(renew(&s.db, &s.instance_id, db::now()).await.unwrap());
        assert!(!renew(&s.db, other, db::now()).await.unwrap());
        locks::Entity::update_many()
            .col_expr(locks::Column::ExpiresAt, Expr::value(db::now() - 1))
            .filter(locks::Column::Name.eq(LEASE_NAME))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(!renew(&s.db, &s.instance_id, db::now()).await.unwrap());
        assert!(
            crate::lock::acquire(&s.db, LEASE_NAME, other, db::now(), LEASE_MS)
                .await
                .unwrap()
        );
        assert!(!renew(&s.db, &s.instance_id, db::now()).await.unwrap());
        assert_eq!(
            crate::lock::release(&s.db, LEASE_NAME, &s.instance_id)
                .await
                .unwrap(),
            0
        );
        assert!(renew(&s.db, other, db::now()).await.unwrap());
    }
    #[test]
    fn peer_check_command_prefers_nc_then_bash() {
        let command = peer_check_command("10.0.0.5", 22);
        assert!(command.find("nc -z").unwrap() < command.find("bash -c").unwrap());
        assert!(command.contains("/dev/tcp/10.0.0.5/22"));
        assert!(command.contains("exit 127"));
        assert!(command.contains("'10.0.0.5' '22'"));
        assert!(!command.contains("timeout 3 sh -c"));
    }
}
