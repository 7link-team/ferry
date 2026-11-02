//! Automatic update eligibility, retry policy, and periodic discovery.
use super::*;

pub(super) fn retry_delay(attempts: u32) -> i64 {
    (30_000_i64 * (1_i64 << attempts.saturating_sub(1).min(7))).min(3_600_000)
}

pub(super) fn previous_auto(host: &hosts::Model, version: &str) -> Option<AutoRetry> {
    let job: UpdateJob = serde_json::from_str(host.agent_update.as_deref()?).ok()?;
    (job.version == version).then_some(job.auto).flatten()
}

pub(super) fn policy_enabled(global: bool, policy: &str) -> bool {
    match policy {
        "on" => true,
        "off" => false,
        "inherit" => global,
        _ => false,
    }
}

pub(super) async fn global_auto_update(s: &AppState) -> Result<bool> {
    Ok(crate::entities::platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?
        .runner_auto_update
        != 0)
}

pub(super) async fn automatic_allowed(s: &AppState, id: Id) -> Result<bool> {
    let host = hosts::Entity::find_by_id(i64::from(id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(host.connection == "runner"
        && policy_enabled(global_auto_update(s).await?, &host.runner_auto_update))
}

pub(super) fn older_version(current: &str, target: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(target),
    ) {
        (Ok(current), Ok(target)) => current.cmp_precedence(&target).is_lt(),
        _ => false,
    }
}

pub(super) fn auto_due(
    host: &hosts::Model,
    peer: &Live,
    global: bool,
    target: &str,
    now: i64,
) -> bool {
    if host.connection != "runner"
        || !peer.managed
        || !peer.ready.load(std::sync::atomic::Ordering::Acquire)
        || !policy_enabled(global, &host.runner_auto_update)
        || !older_version(&peer.version, target)
        || peer
            .failed_version
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            == Some(target)
    {
        return false;
    }
    match host
        .agent_update
        .as_deref()
        .map(serde_json::from_str::<UpdateJob>)
    {
        Some(Ok(job)) => {
            !matches!(job.state.as_str(), "draining" | "updating" | "unknown")
                && !(job.version == target && job.state == "rolled_back")
                && (job.version != target
                    || job.auto.is_none_or(|retry| retry.next_retry_at <= now))
        }
        Some(Err(_)) => false,
        None => true,
    }
}

pub(super) fn runner_target(host: &hosts::Model) -> Result<String> {
    let suffix = match host.os.as_str() {
        "linux" => "unknown-linux-gnu",
        "macos" => "apple-darwin",
        "windows" => "pc-windows-msvc",
        _ => {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.autoUnsupported"),
            ));
        }
    };
    Ok(format!("{}-{suffix}", host.arch))
}

/// Network discovery runs outside control/heartbeat handling. A failed attempt
/// is persisted per target, so reconnects and server restarts cannot reset backoff.
pub(crate) async fn automatic_updates(s: AppState, mut stopped: watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(Duration::from_secs(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut catalog = None;
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            _ = tick.tick() => {
                tokio::select! {
                    _ = stopped.changed() => break,
                    result = automatic_update_cycle(&s, &mut catalog) => {
                        if let Err(error) = result { tracing::warn!(error=%error.1,"Runner automatic update check failed"); }
                    }
                }
            }
        }
    }
}

pub(super) async fn automatic_update_cycle(
    s: &AppState,
    catalog: &mut Option<crate::hosts::runner::install::SignedCatalog>,
) -> Result<()> {
    let global = global_auto_update(s).await?;
    let peers: Vec<_> = live()
        .iter()
        .map(|(id, peer)| (id.clone(), peer.clone()))
        .collect();
    let target = env!("CARGO_PKG_VERSION");
    // One discovery result per cycle prevents an unavailable release endpoint
    // causing one network timeout for every connected host.
    let mut discovery_error = None;
    for (id, peer) in peers {
        let Ok(id) = id.parse::<Id>() else { continue };
        let Some(host) = hosts::Entity::find_by_id(i64::from(id)).one(&s.db).await? else {
            continue;
        };
        if !auto_due(&host, &peer, global, target, db::now()) {
            continue;
        }
        if let Some(cluster) = &s.cluster {
            if !cluster
                .owner(&id.to_string())
                .await?
                .is_some_and(|owner| owner.node == cluster.node)
            {
                continue;
            }
        }
        if catalog.is_none() && discovery_error.is_none() {
            match crate::hosts::runner::install::catalog().await {
                Ok(value) => *catalog = Some(value),
                Err(error) => discovery_error = Some(error),
            }
        }
        let body: Result<UpdateBody> = (|| {
            let catalog = catalog.as_ref().ok_or_else(|| {
                discovery_error
                    .as_ref()
                    .map(|error| ApiError(error.0, error.1.clone(), error.2.clone()))
                    .unwrap_or_else(|| {
                        ApiError::localized(
                            StatusCode::BAD_REQUEST,
                            crate::i18n::Message::new("runner.releaseUnavailableFallback"),
                        )
                    })
            })?;
            ferry_protocol::release::verify_catalog(
                &catalog.bytes,
                &catalog.signature,
                &catalog.key,
                &runner_target(&host)?,
                &peer.version,
                target,
            )
            .map_err(|error| crate::hosts::runner::install::verification_error(&error))?;
            Ok(UpdateBody {
                version: target.into(),
                manifest: String::from_utf8(catalog.bytes.clone()).map_err(|_| {
                    ApiError::localized(
                        StatusCode::BAD_REQUEST,
                        crate::i18n::Message::new("runner.manifestEncoding"),
                    )
                })?,
                signature: catalog.signature.clone(),
            })
        })();
        match body {
            Ok(body) => {
                // Re-read policy/readiness and persisted state inside admission.
                if let Err(error) =
                    begin_update(s.clone(), host, body, (Id::from(0), "系统".into()), true).await
                {
                    tracing::debug!(host=%id,error=%error.1,"Runner automatic update admission deferred");
                }
            }
            Err(error) => {
                *catalog = None;
                discovery_error = Some(ApiError(error.0, error.1.clone(), error.2.clone()));
                let attempts = previous_auto(&host, target)
                    .map_or(1, |retry| retry.attempts.saturating_add(1));
                let job = UpdateJob {
                    auto: Some(AutoRetry {
                        attempts,
                        next_retry_at: db::now() + retry_delay(attempts),
                    }),
                    owner: s.cluster.as_ref().map(|cluster| cluster.node.clone()),
                    request: secret(),
                    version: target.into(),
                    state: "retrying".into(),
                    message: error.stored(),
                };
                persist_update(s, id, &job).await?;
            }
        }
    }
    Ok(())
}
