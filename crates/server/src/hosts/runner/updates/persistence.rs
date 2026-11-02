//! Fenced, durable update transitions and orphan handling.
use super::*;

pub(super) async fn persist_update(s: &AppState, id: Id, job: &UpdateJob) -> Result<()> {
    let value = serde_json::to_string(job).map_err(|_| {
        ApiError::localized(
            StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("runner.statusEncode"),
        )
    })?;
    let mut update = hosts::Entity::update_many()
        .col_expr(hosts::Column::AgentUpdate, Expr::value(value))
        .filter(hosts::Column::Id.eq(id));
    {
        let row = hosts::Entity::find_by_id(i64::from(id))
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?;
        if let Some(previous) = row.agent_update {
            let old: UpdateJob = serde_json::from_str(&previous).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.invalidState"),
                )
            })?;
            if matches!(old.state.as_str(), "draining" | "updating" | "unknown")
                && (old.request != job.request
                    || s.cluster
                        .as_ref()
                        .is_some_and(|cluster| old.owner.as_deref() != Some(cluster.node.as_str())))
            {
                return Err(ApiError::localized(
                    StatusCode::CONFLICT,
                    crate::i18n::Message::new("runner.ownershipChanged"),
                ));
            }
            update = update.filter(hosts::Column::AgentUpdate.eq(previous));
        } else {
            update = update.filter(hosts::Column::AgentUpdate.is_null());
        }
    }
    if update.exec(&s.db).await?.rows_affected != 1 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("runner.ownershipChanged"),
        ));
    }
    Ok(())
}

pub(super) async fn orphan_update(s: &AppState, id: Id, job: &mut UpdateJob) {
    job.state = "unknown".into();
    job.message = crate::i18n::store_message(&crate::i18n::Message::new("runner.lockLost"));
    // Keep admission closed, but permit the current connection owner to recover.
    loop {
        let result: Result<()> = async {
            let row = hosts::Entity::find_by_id(i64::from(id))
                .one(&s.db)
                .await?
                .ok_or_else(ApiError::missing)?;
            let previous = row.agent_update.ok_or_else(ApiError::missing)?;
            let old: UpdateJob = serde_json::from_str(&previous).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.invalidState"),
                )
            })?;
            if old.request != job.request || old.owner != job.owner {
                return Err(ApiError::localized(
                    StatusCode::CONFLICT,
                    crate::i18n::Message::new("runner.ownershipChanged"),
                ));
            }
            let mut orphan = job.clone();
            orphan.owner = None;
            let value = serde_json::to_string(&orphan).map_err(|_| {
                ApiError::localized(
                    StatusCode::BAD_REQUEST,
                    crate::i18n::Message::new("runner.invalidState"),
                )
            })?;
            if hosts::Entity::update_many()
                .col_expr(hosts::Column::AgentUpdate, Expr::value(value))
                .filter(hosts::Column::Id.eq(id))
                .filter(hosts::Column::AgentUpdate.eq(previous))
                .exec(&s.db)
                .await?
                .rows_affected
                != 1
            {
                return Err(ApiError::localized(
                    StatusCode::CONFLICT,
                    crate::i18n::Message::new("runner.ownershipChanged"),
                ));
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => return,
            Err(error) if error.0 == StatusCode::CONFLICT || error.0 == StatusCode::NOT_FOUND => {
                return;
            }
            Err(error) => {
                tracing::error!(host=%id,error=%error.1,"Cannot release orphaned update ownership")
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// The caller retains its exclusive reservation throughout persistence retries.
// A failed state write must never release a host with an uncertain update.
pub(super) async fn transition_update(
    s: &AppState,
    id: Id,
    job: &mut UpdateJob,
    state: &str,
    message: impl AsRef<str>,
) -> bool {
    job.state = state.into();
    job.message = message.as_ref().to_owned();
    if let Some(retry) = &mut job.auto {
        retry.next_retry_at = if state == "failed" {
            db::now() + retry_delay(retry.attempts)
        } else {
            0
        };
    }
    loop {
        match persist_update(s, id, job).await {
            Ok(()) => break,
            Err(error) => {
                if error.0 == StatusCode::CONFLICT || error.0 == StatusCode::NOT_FOUND {
                    return false;
                }
                tracing::error!(host=%id,error=%error.1,"无法保存代理升级状态，继续保留主机升级锁")
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    UPDATES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), job.clone());
    true
}
