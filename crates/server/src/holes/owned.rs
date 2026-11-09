//! Previews opened by pipelines and workflows (docs/preview-automation-design.md):
//! an optional human review, and a lifetime bound to the run or execution that
//! opened them. Nothing here is edited by hand; it is revoked when its source
//! ends, by the source's own terminal write or by `sweep` as a backstop.

use super::*;
use crate::entities::{pipeline_runs, workflow_executions};
use sea_orm::sea_query::{Expr, ExprTrait};
use std::time::Duration;

#[cfg(not(test))]
const POLL: Duration = Duration::from_secs(1);
#[cfg(test)]
const POLL: Duration = Duration::from_millis(100);
/// Previews that nobody reviews still end with their source; this is a cap.
const UNREVIEWED_TTL_MS: i64 = 24 * HOUR_MS;
const REVIEW_MINUTES: std::ops::RangeInclusive<i64> = 1..=1440;
const TERMINAL: [&str; 3] = ["succeeded", "failed", "cancelled"];
const SWEEP_BATCH: u64 = 500;

pub(crate) struct OwnedRequest {
    pub host_id: Id,
    pub port: i64,
    pub name: String,
    pub strip_prefix: bool,
    /// "pipeline" or "workflow".
    pub source_kind: &'static str,
    /// Pipeline run id or workflow execution id.
    pub source_id: i64,
    /// Job key or node id.
    pub source_ref: String,
    /// `Some(minutes)` waits for a person to approve or reject.
    pub review_minutes: Option<i64>,
}

fn review_closed() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("holes.reviewClosed"))
}

/// Opens a preview on behalf of `scope`, which is the identity that started the
/// run or execution: it needs `holes.create` like a person would.
pub(crate) async fn open_owned(
    s: &AppState,
    scope: &Scope,
    request: OwnedRequest,
) -> Result<holes::Model> {
    scope.require("holes.create")?;
    crate::accounts::addresses::Configuration::load(s, &s.db)
        .await?
        .require_preview()?;
    let name = checked_name(if request.name.trim().is_empty() {
        &request.source_ref
    } else {
        &request.name
    })?;
    if !(1..=65535).contains(&request.port)
        || request
            .review_minutes
            .is_some_and(|minutes| !REVIEW_MINUTES.contains(&minutes))
        || request.source_ref.chars().count() > 128
    {
        return Err(invalid());
    }
    let expires_at = db::now()
        + request
            .review_minutes
            .map_or(UNREVIEWED_TTL_MS, |minutes| minutes * 60_000);
    let tx = crate::variables::transaction(s, scope.workspace_id).await?;
    let row = insert(
        s,
        &tx,
        scope,
        NewHole {
            host_id: request.host_id,
            name,
            target_port: request.port,
            strip_prefix: request.strip_prefix,
            expires_at,
            source: (
                request.source_kind,
                request.source_id,
                request.source_ref.clone(),
            ),
            review_state: if request.review_minutes.is_some() {
                "waiting"
            } else {
                "none"
            },
        },
    )
    .await?;
    crate::hosts::activity_in(
        s,
        &tx,
        scope,
        "create",
        "holes",
        &Id::from(row.id),
        json!({"name": row.name, "host_id": request.host_id, "target_port": row.target_port,
               "source_kind": request.source_kind, "source_id": Id::from(request.source_id),
               "source_ref": request.source_ref, "review_minutes": request.review_minutes}),
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Waits until the local service accepts a connection. On timeout the preview
/// is revoked and the last failure is returned, so callers fail their step.
pub(crate) async fn wait_ready(s: &AppState, row: &holes::Model, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let outcome = proxy::reach(s, row).await;
        let late = tokio::time::Instant::now() >= deadline;
        if outcome.is_ok() || late {
            proxy::record_reach(s, row.id, &outcome).await;
        }
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if late => {
                revoke_ids(s, &[row.id]).await?;
                return Err(ApiError::localized(
                    StatusCode::BAD_GATEWAY,
                    Message::new("holes.notReady")
                        .with("detail", error.render(crate::i18n::current_locale())),
                ));
            }
            Err(_) => tokio::time::sleep(POLL).await,
        }
    }
}

/// Waits for the review: `approved`, `rejected`, `timed_out`, or `revoked`
/// when the preview stopped existing first. Only `approved` lets a flow go on.
pub(crate) async fn wait_review(s: &AppState, hole: i64) -> Result<String> {
    loop {
        let Some(row) = holes::Entity::find_by_id(hole).one(&s.db).await? else {
            return Ok("revoked".into());
        };
        if row.review_state != "waiting" {
            return Ok(row.review_state);
        }
        if row.revoked_at > 0 {
            return Ok("revoked".into());
        }
        if row.expires_at <= db::now() {
            time_out(&s.db, Some(hole)).await?;
            continue;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Closes reviews whose deadline passed; `None` means all of them.
async fn time_out(conn: &impl ConnectionTrait, hole: Option<i64>) -> Result<u64> {
    let now = db::now();
    let mut update = holes::Entity::update_many()
        .col_expr(holes::Column::ReviewState, Expr::value("timed_out"))
        .col_expr(holes::Column::ReviewedAt, Expr::value(now))
        .col_expr(holes::Column::ReviewedBy, Expr::value(0_i64))
        .filter(holes::Column::ReviewState.eq("waiting"))
        .filter(holes::Column::ExpiresAt.lte(now));
    if let Some(hole) = hole {
        update = update.filter(holes::Column::Id.eq(hole));
    }
    Ok(update.exec(conn).await?.rows_affected)
}

/// Revokes the live previews a run or execution (optionally one job or node of
/// it) opened. Runs inside the caller's transaction; the caller cuts the
/// returned holes' connections with `close_all` once it has committed.
pub(crate) async fn revoke_source(
    conn: &impl ConnectionTrait,
    kind: &str,
    id: i64,
    source_ref: Option<&str>,
) -> Result<Vec<i64>> {
    let mut find = holes::Entity::find()
        .filter(holes::Column::SourceKind.eq(kind))
        .filter(holes::Column::SourceId.eq(id))
        .filter(holes::Column::RevokedAt.eq(0));
    if let Some(source_ref) = source_ref {
        find = find.filter(holes::Column::SourceRef.eq(source_ref));
    }
    let ids = find
        .all(conn)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    revoke_rows(conn, &ids).await?;
    Ok(ids)
}

async fn revoke_rows(conn: &impl ConnectionTrait, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = db::now();
    holes::Entity::update_many()
        .col_expr(holes::Column::RevokedAt, Expr::value(now))
        .col_expr(holes::Column::RevokedBy, Expr::value(0_i64))
        .col_expr(holes::Column::UpdatedAt, Expr::value(now))
        .col_expr(holes::Column::UpdatedBy, Expr::value(0_i64))
        .col_expr(
            holes::Column::Version,
            Expr::col(holes::Column::Version).add(1),
        )
        .filter(holes::Column::Id.is_in(ids.to_vec()))
        .filter(holes::Column::RevokedAt.eq(0))
        .exec(conn)
        .await?;
    Ok(())
}

async fn revoke_ids(s: &AppState, ids: &[i64]) -> Result<()> {
    revoke_rows(&s.db, ids).await?;
    close_all(ids);
    Ok(())
}

#[derive(Deserialize)]
pub struct ReviewBody {
    decision: String,
}

/// `POST .../holes/{id}/reviews`: approve or reject a preview that waits for
/// review. Reviewing a run's preview needs the same right as running it.
pub async fn review(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<ReviewBody>,
) -> Result<Json<Value>> {
    if !matches!(body.decision.as_str(), "approved" | "rejected") {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = find(&tx, &scope, id).await?;
    match row.source_kind.as_str() {
        "pipeline" => scope.require("pipelines.run")?,
        "workflow" => scope.require("workflows.run")?,
        _ => return Err(review_closed()),
    }
    let now = db::now();
    if row.review_state != "waiting" || state(&row, now) != "active" {
        return Err(review_closed());
    }
    let changed = holes::Entity::update_many()
        .col_expr(
            holes::Column::ReviewState,
            Expr::value(body.decision.clone()),
        )
        .col_expr(holes::Column::ReviewedAt, Expr::value(now))
        .col_expr(
            holes::Column::ReviewedBy,
            Expr::value(i64::from(scope.user.id)),
        )
        .col_expr(holes::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            holes::Column::UpdatedBy,
            Expr::value(i64::from(scope.user.id)),
        )
        .col_expr(
            holes::Column::Version,
            Expr::col(holes::Column::Version).add(1),
        )
        .filter(holes::Column::Id.eq(row.id))
        .filter(holes::Column::ReviewState.eq("waiting"))
        .exec(&tx)
        .await?
        .rows_affected;
    if changed == 0 {
        return Err(review_closed());
    }
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "review",
        "holes",
        &id,
        json!({"name": row.name, "decision": body.decision}),
    )
    .await?;
    tx.commit().await?;
    crate::workflows::preview::nudge(&s.db, &row).await?;
    Ok(Json(view(&s, &scope, id).await?))
}

/// Backstop for the terminal hooks (design section 4): closes overdue reviews
/// and revokes previews whose run or execution ended or no longer exists.
pub(super) async fn sweep(s: &AppState) -> Result<()> {
    time_out(&s.db, None).await?;
    let rows = holes::Entity::find()
        .filter(holes::Column::RevokedAt.eq(0))
        .filter(holes::Column::SourceKind.ne("manual"))
        .limit(SWEEP_BATCH)
        .all(&s.db)
        .await?;
    let sources = |kind: &str| {
        rows.iter()
            .filter(|row| row.source_kind == kind)
            .map(|row| row.source_id)
            .collect::<Vec<_>>()
    };
    let live_runs = pipeline_runs::Entity::find()
        .filter(pipeline_runs::Column::Id.is_in(sources("pipeline")))
        .filter(pipeline_runs::Column::State.is_not_in(TERMINAL))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|run| run.id)
        .collect::<std::collections::HashSet<_>>();
    let live_executions = workflow_executions::Entity::find()
        .filter(workflow_executions::Column::Id.is_in(sources("workflow")))
        .filter(workflow_executions::Column::State.is_not_in(TERMINAL))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|execution| execution.id)
        .collect::<std::collections::HashSet<_>>();
    let ended = rows
        .iter()
        .filter(|row| match row.source_kind.as_str() {
            "pipeline" => !live_runs.contains(&row.source_id),
            "workflow" => !live_executions.contains(&row.source_id),
            _ => false,
        })
        .map(|row| row.id)
        .collect::<Vec<_>>();
    revoke_ids(s, &ended).await
}
