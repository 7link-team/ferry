//! Restore update exclusion and reconcile interrupted jobs before admission.
use super::*;

/// Restore exclusion before the schedulers or HTTP listener can admit new work.
pub(crate) async fn recover_updates(s: &AppState) -> Result<()> {
    let rows = hosts::Entity::find()
        .filter(hosts::Column::AgentUpdate.is_not_null())
        .all(&s.db)
        .await?;
    for row in rows {
        let mut job: UpdateJob = serde_json::from_str(row.agent_update.as_deref().unwrap())
            .map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.persistedRead"),
                )
            })?;
        let id = Id::from(row.id);
        if !matches!(job.state.as_str(), "draining" | "updating" | "unknown") {
            continue;
        }
        if let Some(cluster) = &s.cluster {
            if let Some(owner) = &job.owner {
                if cluster.node_alive(owner).await? {
                    continue;
                }
            }
            if !cluster
                .owner(&id.to_string())
                .await?
                .is_some_and(|owner| owner.node == cluster.node)
            {
                continue;
            }
            // Only the current connection owner recovers an orphaned update.
            job.owner = Some(cluster.node.clone());
            let value = serde_json::to_string(&job).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.invalidState"),
                )
            })?;
            if hosts::Entity::update_many()
                .col_expr(hosts::Column::AgentUpdate, Expr::value(value))
                .filter(hosts::Column::Id.eq(id))
                .filter(hosts::Column::AgentUpdate.eq(row.agent_update.clone().unwrap()))
                .exec(&s.db)
                .await?
                .rows_affected
                != 1
            {
                continue;
            }
        }
        if job.state == "draining" {
            job.state = "failed".into();
            if let Some(retry) = &mut job.auto {
                retry.next_retry_at = db::now() + retry_delay(retry.attempts);
            }
            job.message =
                crate::i18n::store_message(&crate::i18n::Message::new("runner.restartDrain"));
            persist_update(s, id, &job).await?;
        }
        UPDATES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), job.clone());
        if !matches!(job.state.as_str(), "updating" | "unknown") {
            continue;
        }
        // Standalone startup must restore the local exclusion before listeners
        // start. Cluster admission also consults the persisted active state.
        let exclusion = if s.cluster.is_none() {
            Some(crate::hosts::runner::reservations::exclusive(&id.to_string()).await)
        } else {
            None
        };
        let s = s.clone();
        tokio::spawn(async move {
            let _exclusion = match exclusion {
                Some(exclusion) => exclusion,
                None => crate::hosts::runner::reservations::exclusive(&id.to_string()).await,
            };
            let distributed = match &s.cluster {
                Some(cluster) => match cluster.reserve(&id.to_string(), true).await {
                    Ok(lease) => Some(lease),
                    Err(_) => {
                        orphan_update(&s, id, &mut job).await;
                        return;
                    }
                },
                None => None,
            };
            loop {
                if distributed.as_ref().is_some_and(|lease| !lease.valid()) {
                    orphan_update(&s, id, &mut job).await;
                    return;
                }
                let peer = live().get(&id.to_string()).cloned();
                if let Some(peer) = peer {
                    if peer.managed && peer.ready.load(std::sync::atomic::Ordering::Acquire) {
                        let failed = peer
                            .failed_version
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        let (state, message) = if peer.version == job.version {
                            (
                                "succeeded",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.restartConfirmed",
                                )),
                            )
                        } else if failed.as_deref() == Some(job.version.as_str()) {
                            (
                                "rolled_back",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.restartRollback",
                                )),
                            )
                        } else {
                            (
                                "failed",
                                crate::i18n::store_message(&crate::i18n::Message::new(
                                    "runner.restartRecovered",
                                )),
                            )
                        };
                        transition_update(&s, id, &mut job, state, message).await;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    }
    Ok(())
}
