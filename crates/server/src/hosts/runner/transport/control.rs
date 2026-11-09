//! Long-lived authenticated control connections and heartbeat metrics.
use super::*;

/// How often the server probes a quiet control connection.
const HEARTBEAT_SECONDS: u64 = 20;
/// How often a runner reports load/memory/disk. Doubles as its heartbeat.
const METRICS_SECONDS: u64 = 30;
/// Probes allowed to go unanswered before the connection is considered dead.
const MISSED_PROBES: u32 = 3;

/// Rejected is terminal for existing agents. Infrastructure failures must close
/// the socket without that frame so the agent can retry with its stored key.
async fn authentication_failure(socket: &mut Socket, error: &ApiError) {
    if error.0.is_server_error() {
        tracing::warn!(reason=%error.1, "Runner authentication temporarily unavailable");
        let _ = socket.send(Message::Close(None)).await;
    } else {
        reject(socket, error).await;
    }
}

/// `GET /api/runners/connect` — the runner's control connection. Deliberately
/// not behind the session cookie: a runner is not a browser, and it proves
/// itself with the `Hello` frame instead.
pub async fn connect(State(s): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let locale = crate::i18n::current_locale();
    upgrade.on_upgrade(move |socket| crate::i18n::with_locale(locale, control(s, socket.into())))
}

async fn control(s: AppState, mut socket: Socket) {
    let hello: wire::Hello = match handshake(&mut socket).await {
        Ok(hello) => hello,
        Err(error) => return reject(&mut socket, &error).await,
    };
    if hello.version != wire::VERSION {
        return reject(
            &mut socket,
            &ApiError::localized(
                StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("runnerExtra.protocolMismatch")
                    .with("expected", wire::VERSION.to_string())
                    .with("actual", hello.version.to_string()),
            ),
        )
        .await;
    }

    let authenticated = match &hello.auth {
        wire::Auth::Enroll { token } => enrol(&s, &hello, token)
            .await
            .map(|(id, key)| (id, Some(key))),
        wire::Auth::Key { host_id, key } => verify_key(&s, host_id, key)
            .await
            .map(|()| (host_id.clone(), None)),
    };
    let (host_id, key) = match authenticated {
        Ok(value) => value,
        Err(error) => return authentication_failure(&mut socket, &error).await,
    };
    if let Some(key) = key.as_deref() {
        if let Err(error) = verify_key(&s, &host_id, key).await {
            return authentication_failure(&mut socket, &error).await;
        }
    }

    let row_id = match host_id.parse::<Id>() {
        Ok(id) => id,
        Err(_) => return reject(&mut socket, &runner_extra("runnerExtra.hostIdInvalid")).await,
    };
    let row = match hosts::Entity::find_by_id(i64::from(row_id))
        .one(&s.db)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return reject(&mut socket, &runner_extra("runnerExtra.hostUnavailable")).await,
        Err(error) => return authentication_failure(&mut socket, &error.into()).await,
    };
    // Deliver the newly issued key before attempting a transient cluster lease.
    // A closed socket is retryable by existing Runners; Rejected is permanent.
    let enrolling = key.is_some();
    if enrolling
        && send_frame(
            &mut socket,
            &wire::Control::Welcome {
                host_id: host_id.clone(),
                key: key.clone(),
                heartbeat_seconds: HEARTBEAT_SECONDS,
                metrics_seconds: METRICS_SECONDS,
            },
        )
        .await
        .is_err()
    {
        return;
    }
    let (owner, owner_revision) = if let Some(cluster) = &s.cluster {
        match cluster
            .acquire_owner(&s, &host_id, row.workspace_id.into(), secret())
            .await
        {
            Ok((owner, revision)) => (Some(owner), revision),
            Err(error) => {
                tracing::warn!(%host_id, reason=%error.1, "Cluster ownership temporarily unavailable");
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        }
    } else {
        (None, 0)
    };
    if !enrolling
        && send_frame(
            &mut socket,
            &wire::Control::Welcome {
                host_id: host_id.clone(),
                key: None,
                heartbeat_seconds: HEARTBEAT_SECONDS,
                metrics_seconds: METRICS_SECONDS,
            },
        )
        .await
        .is_err()
    {
        if let (Some(cluster), Some(owner)) = (&s.cluster, &owner) {
            cluster.release(&host_id, owner_revision).await;
            let _ =
                crate::lock::release(&s.db, &format!("runner-owner:{host_id}"), &owner.fence).await;
        }
        return;
    }
    let (control, inbox) = mpsc::channel(16);
    let (disconnect, disconnect_rx) = watch::channel(false);
    let registration = Arc::new(Live {
        owner,
        owner_revision: std::sync::atomic::AtomicU64::new(owner_revision),
        key_hash: row.runner_key_hash,
        control,
        disconnect,
        managed: hello.managed_updates,
        supports_monitor: hello.supports_monitor,
        supports_agent: hello.supports_agent,
        supports_agent_probe: hello.supports_agent_probe,
        supports_forward: hello.supports_forward,
        version: hello.agent_version.clone(),
        ready: std::sync::atomic::AtomicBool::new(!hello.managed_updates),
        failed_version: Mutex::new(None),
        update_error: Mutex::new(None),
    });
    // A replaced connection's own loop ends when its sender is dropped here,
    // so a half-dead socket cannot keep a host looking online.
    if let Some(old) = live().insert(host_id.clone(), registration.clone()) {
        let _ = old.disconnect.send(true);
    }
    // Enrolment already wrote the facts; a reconnect refreshes them.
    if matches!(hello.auth, wire::Auth::Key { .. }) {
        let mut update = hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("online"))
            .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
            .col_expr(hosts::Column::Os, Expr::value(hello.os.clone()))
            .col_expr(hosts::Column::Arch, Expr::value(hello.arch.clone()))
            .col_expr(
                hosts::Column::AgentVersion,
                Expr::value(hello.agent_version.clone()),
            );
        if let Some(address) = reported_address(&hello.ip_address) {
            update = update.col_expr(hosts::Column::Address, Expr::value(address));
        }
        if let Some(owner) = &registration.owner {
            update = update.filter(crate::cluster::fence_condition(&host_id, &owner.fence));
        }
        let _ = update
            .filter(hosts::Column::RunnerKeyHash.eq(&registration.key_hash))
            .filter(hosts::Column::Id.eq(row_id))
            .exec(&s.db)
            .await;
    }
    tracing::info!(%host_id, hostname = %hello.hostname, "runner 控制连接已建立");

    pump_control(
        &s,
        &host_id,
        socket,
        inbox,
        disconnect_rx,
        registration.clone(),
    )
    .await;

    // Only tear down the registration if it is still ours: a reconnect that
    // replaced us must not be marked offline by our own exit.
    let still_ours = {
        let mut map = live();
        let ours = map
            .get(&host_id)
            .is_some_and(|current| Arc::ptr_eq(current, &registration));
        if ours {
            map.remove(&host_id);
        }
        ours
    };
    if still_ours {
        let mut update = hosts::Entity::update_many()
            .col_expr(hosts::Column::State, Expr::value("offline"))
            .filter(hosts::Column::Id.eq(row_id))
            .filter(hosts::Column::RunnerKeyHash.eq(&registration.key_hash));
        if let Some(owner) = &registration.owner {
            update = update.filter(crate::cluster::fence_condition(&host_id, &owner.fence));
        }
        let _ = update.exec(&s.db).await;
        if let (Some(cluster), Some(owner)) = (&s.cluster, &registration.owner) {
            cluster
                .release(
                    &host_id,
                    registration
                        .owner_revision
                        .load(std::sync::atomic::Ordering::Acquire),
                )
                .await;
            let _ =
                crate::lock::release(&s.db, &format!("runner-owner:{host_id}"), &owner.fence).await;
        }
        tracing::info!(%host_id, "runner 控制连接已断开");
    }
}

/// The last heartbeat's numbers, small enough for the `hosts.metrics` column.
/// Short keys on purpose: MySQL stores that column as `VARCHAR(255)`.
fn metrics_json(
    load1: f64,
    cpu_percent: f64,
    mem_used: u64,
    mem_total: u64,
    disk_used: u64,
    disk_total: u64,
) -> String {
    json!({
        "load1": (load1 * 100.0).round() / 100.0,
        "cpu_percent": (cpu_percent * 10.0).round() / 10.0,
        "mem_used": mem_used,
        "mem_total": mem_total,
        "disk_used": disk_used,
        "disk_total": disk_total,
    })
    .to_string()
}

async fn pump_control(
    s: &AppState,
    host_id: &str,
    mut socket: Socket,
    mut inbox: mpsc::Receiver<wire::Control>,
    mut disconnect: watch::Receiver<bool>,
    registration: Arc<Live>,
) {
    let Ok(row_id) = host_id.parse::<Id>() else {
        return;
    };
    let mut probe = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
    probe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let workspace = hosts::Entity::find_by_id(i64::from(row_id))
        .one(&s.db)
        .await
        .ok()
        .flatten()
        .map(|h| Id::from(h.workspace_id));
    let mut unanswered = 0;
    let mut renew = tokio::time::interval(Duration::from_secs(5));
    let mut revoked = if let Some(cluster) = &s.cluster {
        match cluster.subscribe_revoke(host_id).await {
            Ok(value) => Some(value),
            Err(_) => return,
        }
    } else {
        None
    };
    loop {
        tokio::select! {
            _ = async { match &mut revoked {Some(revoked)=>{loop {let notice=revoked.next().await;if notice.is_none_or(|notice|registration.owner.as_ref().is_some_and(|owner|notice.payload.as_ref()==owner.fence.as_bytes())) {break;}}},None=>std::future::pending().await} } => break,
            _ = renew.tick(), if s.cluster.is_some() => {
                if !matches!(hosts::Entity::find_by_id(i64::from(row_id)).filter(hosts::Column::RunnerKeyHash.eq(&registration.key_hash)).one(&s.db).await,Ok(Some(_))) {break;}
                let cluster=s.cluster.as_ref().unwrap();
                let mut owner=registration.owner.as_ref().unwrap().clone();
                owner.info=Some(crate::cluster::RunnerInfo {managed:registration.managed,ready:registration.ready.load(std::sync::atomic::Ordering::Acquire), version:registration.version.clone(), failed_version:registration.failed_version.lock().unwrap_or_else(|error|error.into_inner()).clone()});
                let revision=registration.owner_revision.load(std::sync::atomic::Ordering::Acquire);
                match cluster.renew_owner(s,host_id,&owner,revision).await {
                    Ok(revision)=>registration.owner_revision.store(revision,std::sync::atomic::Ordering::Release),
                    Err(_)=>break,
                }
            },
            _ = disconnect.changed() => {
                tracing::info!(%host_id, "runner 控制连接收到重置通知");
                break;
            },
            outgoing = inbox.recv() => match outgoing {
                Some(frame) => if let Err(error) = send_frame(&mut socket, &frame).await {
                    tracing::info!(%host_id, %error, "runner 控制帧发送失败");
                    break;
                },
                // Dropped sender: this connection was replaced.
                None => {
                    tracing::info!(%host_id, "runner 控制连接被新连接替换");
                    break;
                },
            },
            incoming = socket.recv() => {
                let Some(message) = incoming else {
                    tracing::info!(%host_id, "runner 控制连接已到达流末尾");
                    break;
                };
                let Ok(message) = message else {
                    tracing::info!(%host_id, error = %message.unwrap_err(), "runner 控制连接读取失败");
                    break;
                };
                unanswered = 0;
                match message {
                    Message::Text(text) => match serde_json::from_str::<wire::Report>(&text) {
                        Ok(wire::Report::Pong) => {}
                        Ok(wire::Report::UpdateHealth { version, failed_version }) => {
                            if registration.managed && version == registration.version {
                                *registration.failed_version.lock().unwrap_or_else(|e|e.into_inner()) = failed_version;
                                registration.ready.store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        Ok(wire::Report::UpdateFailed { request, error, message }) => {
                            let stored = stored_update_failure(&error, message);
                            *registration.update_error.lock().unwrap_or_else(|e|e.into_inner()) = Some((request, stored));
                        }
                        Ok(wire::Report::Metrics { load1, cpu_percent, memory_used, memory_total, disk_used, disk_total }) => {
                            // No version bump: a heartbeat is not an edit, and
                            // bumping it would invalidate an operator's open form
                            // every thirty seconds.
                            let mut update = hosts::Entity::update_many()
                                .col_expr(hosts::Column::State, Expr::value("online"))
                                .col_expr(hosts::Column::SeenAt, Expr::value(db::now()))
                                .col_expr(hosts::Column::Metrics, Expr::value(metrics_json(load1, cpu_percent, memory_used, memory_total, disk_used, disk_total)))
                                .filter(hosts::Column::Id.eq(row_id))
                                .filter(hosts::Column::RunnerKeyHash.eq(&registration.key_hash));
                            if let Some(owner)=&registration.owner {update=update.filter(crate::cluster::fence_condition(host_id,&owner.fence));}
                            let _=update.exec(&s.db).await;
                        }
                        Ok(wire::Report::SessionFailed { session, error, message }) => {
                            if let Some(waiting) = claim_pending(&session, host_id) {
                                let _ = waiting.socket.send(Err(session_failed(error, message)));
                            }
                        }
                        Err(error) => tracing::warn!(%host_id, %error, "无法解析 runner 上报帧"),
                    },
                    // The control connection carries no bulk data, by design.
                    Message::Binary(_) => tracing::warn!(%host_id, "控制连接收到二进制帧，已忽略"),
                    Message::Close(frame) => {
                        tracing::info!(%host_id, ?frame, "runner 主动关闭控制连接");
                        break;
                    },
                    _ => {}
                }
            }
            _ = probe.tick() => {
                let allowed = if let Some(workspace) = workspace {
                    crate::maintenance::setting(&s.db,workspace,"heartbeat_timeout_seconds").await.map(|n|(n as u64).div_ceil(HEARTBEAT_SECONDS) as u32).unwrap_or(MISSED_PROBES)
                } else { MISSED_PROBES };
                if unanswered >= allowed {
                    tracing::warn!(%host_id, "runner 连续 {allowed} 次心跳无响应，断开连接");
                    break;
                }
                unanswered += 1;
                if let Err(error) = send_frame(&mut socket, &wire::Control::Ping).await {
                    tracing::info!(%host_id, %error, "runner 心跳发送失败");
                    break;
                }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_stay_inside_the_column_they_are_stored_in() {
        let worst = metrics_json(
            u64::MAX as f64,
            100.0,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
        );
        assert!(
            worst.len() <= 255,
            "MySQL stores hosts.metrics as VARCHAR(255), got {} chars: {worst}",
            worst.len()
        );
        let normal = metrics_json(0.567, 42.0, 1 << 30, 1 << 34, 1 << 36, 1 << 38);
        assert!(normal.contains("\"load1\":0.57"), "{normal}");
    }
}
