//! Update HTTP requests and independent drain/activation jobs.
use super::*;

#[derive(Serialize, Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    pub(super) version: String,
    #[serde(default)]
    pub(super) manifest: String,
    #[serde(default)]
    pub(super) signature: String,
}

pub(super) async fn update_host(s: &AppState, scope: &Scope, id: Id) -> Result<hosts::Model> {
    hosts::Entity::find_by_id(i64::from(id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)
}

pub async fn update_status(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("hosts.read")?;
    let host = update_host(&s, &scope, id).await?;
    let registration = live().get(&id.to_string()).cloned();
    let info = match &s.cluster {
        Some(cluster) => cluster
            .owner(&id.to_string())
            .await?
            .and_then(|owner| owner.info),
        None => None,
    };
    let mut status = host
        .agent_update
        .as_deref()
        .map(serde_json::from_str::<UpdateJob>)
        .transpose()
        .map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.statusRead"),
            )
        })?;
    if let Some(job) = &mut status {
        job.message = crate::i18n::render_stored(crate::i18n::current_locale(), &job.message);
    }
    let global = global_auto_update(&s).await?;
    Ok(Json(
        json!({"available":older_version(&host.agent_version, env!("CARGO_PKG_VERSION")),"agent_version":host.agent_version,"target_version":env!("CARGO_PKG_VERSION"),"global_auto_update":global,"auto_update":host.runner_auto_update,"enabled":policy_enabled(global,&host.runner_auto_update),"supported":registration.as_ref().is_some_and(|r|r.managed)||info.as_ref().is_some_and(|info|info.managed),"ready":registration.as_ref().is_some_and(|r|r.ready.load(std::sync::atomic::Ordering::Acquire))||info.as_ref().is_some_and(|info|info.ready),"update":status}),
    ))
}

pub async fn start_update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_workspace, id)): Path<(String, Id)>,
    Json(mut body): Json<UpdateBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("hosts.update")?;
    let host = update_host(&s, &scope, id).await?;
    if host
        .agent_update
        .as_deref()
        .and_then(|value| serde_json::from_str::<UpdateJob>(value).ok())
        .is_some_and(|job| matches!(job.state.as_str(), "draining" | "updating" | "unknown"))
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.activeUpdate"),
        ));
    }
    if body.version.is_empty() && body.manifest.is_empty() && body.signature.is_empty() {
        if !older_version(&host.agent_version, env!("CARGO_PKG_VERSION")) {
            return Err(ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.alreadyVersion"),
            ));
        }
        let catalog = crate::hosts::runner::install::catalog().await?;
        ferry_protocol::release::verify_catalog(
            &catalog.bytes,
            &catalog.signature,
            &catalog.key,
            &runner_target(&host)?,
            &host.agent_version,
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|error| crate::hosts::runner::install::verification_error(&error))?;
        body = UpdateBody {
            version: env!("CARGO_PKG_VERSION").into(),
            manifest: String::from_utf8(catalog.bytes).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.manifestEncoding"),
                )
            })?,
            signature: catalog.signature,
        };
    }
    if body.version.len() > 128
        || body.manifest.len() > 65536
        || body.signature.len() > 8192
        || body.version.is_empty()
    {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.invalidUpdate"),
        ));
    }
    if let Some(cluster) = &s.cluster {
        let owner = cluster
            .owner(&id.to_string())
            .await?
            .ok_or_else(crate::cluster::unavailable)?;
        if owner.node != cluster.node {
            let value: Value = cluster
                .rpc(
                    &owner.node,
                    &crate::cluster::Request::Update {
                        workspace: scope.workspace_id,
                        actor: scope.user.id,
                        host: id,
                        body,
                    },
                )
                .await?;
            return Ok((StatusCode::ACCEPTED, Json(render_update_response(value))));
        }
    }
    let (status, Json(value)) = begin_update(
        s,
        host,
        body,
        (scope.user.id.into(), scope.user.name),
        false,
    )
    .await?;
    Ok((status, Json(render_update_response(value))))
}

pub(super) fn render_update_response(mut value: Value) -> Value {
    if let Some(message) = value.get_mut("message") {
        if let Some(stored) = message.as_str() {
            *message = Value::String(crate::i18n::render_stored(
                crate::i18n::current_locale(),
                stored,
            ));
        }
    }
    value
}

pub(super) async fn begin_update(
    s: AppState,
    host: hosts::Model,
    body: UpdateBody,
    actor: (Id, String),
    automatic: bool,
) -> Result<(StatusCode, Json<Value>)> {
    let id = Id::from(host.id);
    let host = hosts::Entity::find_by_id(host.id)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    if host
        .agent_update
        .as_deref()
        .and_then(|value| serde_json::from_str::<UpdateJob>(value).ok())
        .is_some_and(|job| matches!(job.state.as_str(), "draining" | "updating" | "unknown"))
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.activeUpdate"),
        ));
    }
    let registration = live().get(&id.to_string()).cloned().ok_or_else(|| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.updateOffline"),
        )
    })?;
    if !registration.managed {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.managedRequired"),
        ));
    }
    if registration.version == body.version {
        return Err(ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.alreadyVersion"),
        ));
    }
    if automatic
        && (body.version != env!("CARGO_PKG_VERSION")
            || !auto_due(
                &host,
                &registration,
                global_auto_update(&s).await?,
                &body.version,
                db::now(),
            ))
    {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.conditionsChanged"),
        ));
    }
    let job = UpdateJob {
        auto: automatic.then(|| AutoRetry {
            attempts: previous_auto(&host, &body.version)
                .map_or(1, |retry| retry.attempts.saturating_add(1)),
            next_retry_at: 0,
        }),
        owner: s.cluster.as_ref().map(|cluster| cluster.node.clone()),
        request: secret(),
        version: body.version.clone(),
        state: "draining".into(),
        message: crate::i18n::store_message(&crate::i18n::Message::new("runner.waiting")),
    };
    {
        let mut jobs = UPDATES.lock().unwrap_or_else(|e| e.into_inner());
        if jobs
            .get(&id.to_string())
            .is_some_and(|j| matches!(j.state.as_str(), "draining" | "updating" | "unknown"))
        {
            return Err(ApiError::localized(
                StatusCode::CONFLICT,
                crate::i18n::Message::new("runner.activeUpdate"),
            ));
        }
        jobs.insert(id.to_string(), job.clone());
    }
    // Persist/audit admission also outlives a disconnected HTTP caller.
    let admission = tokio::spawn(async move {
        if let Err(error) = persist_update(&s, id, &job).await {
            UPDATES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id.to_string());
            return Err(error);
        }
        if let Err(error) = crate::hosts::activity_record(
            &s,
            &s.db,
            host.workspace_id.into(),
            (actor.0, &actor.1),
            "runner-update",
            "hosts",
            id,
            json!({"version":body.version,"request":job.request,"automatic":automatic}),
        )
        .await
        {
            let mut failed = job.clone();
            transition_update(
                &s,
                id,
                &mut failed,
                "failed",
                crate::i18n::store_message(&crate::i18n::Message::new("runner.auditFailed")),
            )
            .await;
            UPDATES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id.to_string());
            return Err(error);
        }
        let response = serde_json::to_value(&job).map_err(|_| {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runner.statusEncode"),
            )
        })?;
        tokio::spawn(async move {
            let host = id.to_string();
            let mut job = job;
            let _exclusive = match await_drain(
                &s,
                id,
                automatic,
                crate::hosts::runner::reservations::exclusive(&host),
            )
            .await
            {
                Ok(Some(guard)) => guard,
                Ok(None) => {
                    transition_update(
                        &s,
                        id,
                        &mut job,
                        "cancelled",
                        crate::i18n::store_message(&crate::i18n::Message::new(
                            "runner.autoDisabled",
                        )),
                    )
                    .await;
                    return;
                }
                Err(_) => {
                    transition_update(
                        &s,
                        id,
                        &mut job,
                        "failed",
                        "无法读取自动升级策略，未发送更新",
                    )
                    .await;
                    return;
                }
            };
            let distributed = match &s.cluster {
                Some(cluster) => {
                    match await_drain(&s, id, automatic, cluster.reserve(&host, true)).await {
                        Ok(None) => {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "cancelled",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.autoDisabled",
                                )),
                            )
                            .await;
                            return;
                        }
                        Ok(Some(Ok(lease))) => Some(lease),
                        _ => {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "failed",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.reservationUnavailable",
                                )),
                            )
                            .await;
                            return;
                        }
                    }
                }
                None => None,
            };
            if distributed.as_ref().is_some_and(|lease| !lease.valid()) {
                orphan_update(&s, id, &mut job).await;
                return;
            }
            let peer = live().get(&host).cloned();
            let Some(peer) = peer else {
                transition_update(
                    &s,
                    id,
                    &mut job,
                    "failed",
                    crate::i18n::store_message(&crate::i18n::Message::new(
                        "runner.offlineDuringDrain",
                    )),
                )
                .await;
                return;
            };
            if !peer.managed {
                transition_update(
                    &s,
                    id,
                    &mut job,
                    "failed",
                    crate::i18n::store_message(&crate::i18n::Message::new(
                        "runner.reconnectedUnmanaged",
                    )),
                )
                .await;
                return;
            }
            if automatic {
                let allowed = automatic_allowed(&s, id).await;
                if !matches!(allowed, Ok(true)) {
                    let state = if allowed.is_ok() {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    transition_update(
                        &s,
                        id,
                        &mut job,
                        state,
                        crate::i18n::store_message(&crate::i18n::Message::new(
                            "runner.policyChanged",
                        )),
                    )
                    .await;
                    return;
                }
                let failed_target = peer
                    .failed_version
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_deref()
                    == Some(body.version.as_str());
                if !peer.ready.load(std::sync::atomic::Ordering::Acquire)
                    || !older_version(&peer.version, &body.version)
                    || failed_target
                {
                    transition_update(
                        &s,
                        id,
                        &mut job,
                        "cancelled",
                        crate::i18n::store_message(&crate::i18n::Message::new(
                            "runner.noLongerNeeded",
                        )),
                    )
                    .await;
                    return;
                }
            }
            if !transition_update(
                &s,
                id,
                &mut job,
                "updating",
                crate::i18n::store_message(&crate::i18n::Message::new("runner.verifying")),
            )
            .await
            {
                return;
            }
            if distributed.as_ref().is_some_and(|lease| !lease.valid()) {
                orphan_update(&s, id, &mut job).await;
                return;
            }
            if automatic && !matches!(automatic_allowed(&s, id).await, Ok(true)) {
                transition_update(
                    &s,
                    id,
                    &mut job,
                    "cancelled",
                    crate::i18n::store_message(&crate::i18n::Message::new("runner.policyChanged")),
                )
                .await;
                return;
            }
            if peer
                .control
                .send(wire::Control::Update {
                    request: job.request.clone(),
                    version: body.version.clone(),
                    manifest: body.manifest,
                    signature: body.signature,
                })
                .await
                .is_err()
            {
                transition_update(
                    &s,
                    id,
                    &mut job,
                    "failed",
                    crate::i18n::store_message(&crate::i18n::Message::new("runner.sendFailed")),
                )
                .await;
                return;
            }
            let started = std::time::Instant::now();
            loop {
                if distributed.as_ref().is_some_and(|lease| !lease.valid()) {
                    orphan_update(&s, id, &mut job).await;
                    return;
                }
                let failure = peer
                    .update_error
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some((request, message)) = failure {
                    if request == job.request {
                        transition_update(&s, id, &mut job, "failed", &message).await;
                        return;
                    }
                }
                if let Some(cluster) = &s.cluster {
                    let current = cluster.owner(&host).await.ok().flatten();
                    if let Some(info) = current.and_then(|current| current.info) {
                        if info.ready
                            && (info.version == body.version
                                || info.failed_version.as_deref() == Some(body.version.as_str()))
                        {
                            let (state, message) = if info.version == body.version {
                                (
                                    "succeeded",
                                    crate::i18n::store_message(&crate::i18n::Message::new(
                                        "runner.ownerConfirmed",
                                    )),
                                )
                            } else {
                                (
                                    "rolled_back",
                                    crate::i18n::store_message(&crate::i18n::Message::new(
                                        "runner.ownerRollback",
                                    )),
                                )
                            };
                            transition_update(&s, id, &mut job, state, message).await;
                            return;
                        }
                    }
                }
                let current = live().get(&host).cloned();
                if let Some(current) = current {
                    if !Arc::ptr_eq(&peer, &current)
                        && current.ready.load(std::sync::atomic::Ordering::Acquire)
                    {
                        if current.version == body.version {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "succeeded",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.confirmed",
                                )),
                            )
                            .await;
                        } else {
                            transition_update(
                                &s,
                                id,
                                &mut job,
                                "rolled_back",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.rolledBack",
                                )),
                            )
                            .await;
                        }
                        return;
                    }
                }
                if started.elapsed() > Duration::from_secs(420) {
                    // Never admit new work merely because a response timed out.
                    if job.state != "unknown" {
                        transition_update(
                            &s,
                            id,
                            &mut job,
                            "unknown",
                            crate::i18n::store_message(&crate::i18n::Message::new(
                                "runner.unknown",
                            )),
                        )
                        .await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        Ok((StatusCode::ACCEPTED, Json(response)))
    });
    admission.await.map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.initFailed"),
        )
    })?
}

/// A disabled policy cancels an unsent automatic drain, including a distributed
/// reservation waiter. Dropping the pending future releases its partial locks.
pub(super) async fn await_drain<F: std::future::Future>(
    s: &AppState,
    id: Id,
    automatic: bool,
    future: F,
) -> Result<Option<F::Output>> {
    tokio::pin!(future);
    loop {
        if automatic && !automatic_allowed(s, id).await? {
            return Ok(None);
        }
        tokio::select! {
            value = &mut future => return Ok(Some(value)),
            _ = tokio::time::sleep(Duration::from_secs(1)), if automatic => {},
        }
    }
}
