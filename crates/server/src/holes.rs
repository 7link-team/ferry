//! Preview holes: a public `/holes/{token}/` link to a loopback port on a
//! Runner host, for remote preview. Design: docs/preview-holes-design.md.
//!
//! This module is the workspace resource (list, create, edit, revoke, delete,
//! probe) and the background reachability monitor; `proxy` is the public
//! route and everything that keeps its connections bounded.

mod owned;
mod proxy;
#[cfg(test)]
mod tests;

pub(crate) use owned::{OwnedRequest, open_owned, revoke_source, wait_ready, wait_review};
pub use proxy::{redact, serve, serve_root};

use crate::{
    accounts::workspace::Scope,
    db,
    entities::{holes, hosts},
    error::{ApiError, Json, Path, Query, Result},
    i18n::Message,
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{workspace}/holes", get(list).post(create))
        .route(
            "/{workspace}/holes/{id}",
            get(detail).patch(update).delete(remove),
        )
        .route("/{workspace}/holes/{id}/revocations", post(revoke))
        .route("/{workspace}/holes/{id}/probes", post(probe_now))
        .route("/{workspace}/holes/{id}/reviews", post(owned::review))
}

const SPEC: Spec = Spec {
    search: &["name"],
    filters: &["host_id", "source_kind", "source_id", "source_ref"],
    sortable: &["name", "created_at", "expires_at"],
    default_sort: "-created_at",
};
/// The validity periods offered; anything else is refused.
const TTL_HOURS: [i64; 4] = [1, 8, 24, 168];
const HOUR_MS: i64 = 3_600_000;
const NAME_LIMIT: usize = 128;
const ERROR_LIMIT: usize = 512;
/// How often active holes are checked for a reachable local service.
const PROBE_EVERY: Duration = Duration::from_secs(15);
const PROBE_BATCH: u64 = 200;
const PROBE_PARALLEL: usize = 16;

fn invalid() -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, Message::new("holes.invalid"))
}
fn changed() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("holes.changed"))
}
fn revoked() -> ApiError {
    ApiError::localized(StatusCode::CONFLICT, Message::new("holes.revoked"))
}

/// Derived, never stored: see design section 7.
pub(crate) fn state(row: &holes::Model, now: i64) -> &'static str {
    if row.revoked_at > 0 {
        "revoked"
    } else if row.expires_at <= now {
        "expired"
    } else {
        "active"
    }
}

fn aad(row: &holes::Model) -> String {
    crate::vault::aad(&row.workspace_id.to_string(), "holes", &row.id.to_string())
}

async fn find(conn: &impl ConnectionTrait, scope: &Scope, id: Id) -> Result<holes::Model> {
    scope.require("holes.read")?;
    holes::Entity::find_by_id(i64::from(id))
        .filter(holes::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

pub(crate) async fn views(
    s: &AppState,
    scope: &Scope,
    rows: Vec<holes::Model>,
) -> Result<Vec<Value>> {
    use crate::resources::names::{self, Kind, of};
    let now = db::now();
    let host_ids = rows.iter().map(|row| row.host_id).collect::<Vec<_>>();
    let online = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Id.is_in(host_ids))
        .all(&s.db)
        .await?
        .into_iter()
        .map(|host| (host.id, host.state == "online"))
        .collect::<HashMap<_, _>>();
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        let url = link(s, &row)?;
        values.push(json!({
            "id": Id::from(row.id),
            "host_id": Id::from(row.host_id),
            "host_online": online.get(&row.host_id).copied().unwrap_or(false),
            "name": row.name,
            "target_port": row.target_port,
            "strip_prefix": row.strip_prefix,
            "url": url,
            "state": state(&row, now),
            "expires_at": row.expires_at,
            "revoked_at": row.revoked_at,
            "probe_state": row.probe_state,
            "probe_error": crate::i18n::render_stored(crate::i18n::current_locale(), &row.probe_error),
            "source_kind": row.source_kind,
            "source_id": Id::from(row.source_id),
            "source_ref": row.source_ref,
            "review_state": row.review_state,
            "reviewed_at": row.reviewed_at,
            "reviewed_by": Id::from(row.reviewed_by),
            "probed_at": row.probed_at,
            "active_connections": proxy::active(row.id),
            "created_at": row.created_at,
            "created_by": Id::from(row.created_by),
            "updated_at": row.updated_at,
            "updated_by": Id::from(row.updated_by),
            "version": row.version,
        }));
    }
    names::attach(
        &s.db,
        scope.workspace_id,
        &mut values,
        &[
            of("host_id", "host_name", Kind::Host),
            of("created_by", "created_by_name", Kind::User),
        ],
    )
    .await?;
    Ok(values)
}

/// Root-relative: only the browser knows the external scheme and host.
pub(crate) fn link(s: &AppState, row: &holes::Model) -> Result<String> {
    let token = s.vault.open_secret(&row.token_sealed, &aad(row))?;
    Ok(format!("/holes/{token}/"))
}

async fn view(s: &AppState, scope: &Scope, id: Id) -> Result<Value> {
    let row = find(&s.db, scope, id).await?;
    Ok(views(s, scope, vec![row]).await?.remove(0))
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_LIMIT {
        return Err(invalid());
    }
    Ok(name.to_owned())
}

fn checked_ttl(hours: i64) -> Result<i64> {
    if TTL_HOURS.contains(&hours) {
        Ok(hours * HOUR_MS)
    } else {
        Err(invalid())
    }
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(mut raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("holes.read")?;
    let now = db::now();
    // State is derived, so it is a condition rather than a column filter.
    let condition = match raw.remove("state").as_deref() {
        None | Some("") => Condition::all(),
        Some("active") => Condition::all()
            .add(holes::Column::RevokedAt.eq(0))
            .add(holes::Column::ExpiresAt.gt(now)),
        Some("expired") => Condition::all()
            .add(holes::Column::RevokedAt.eq(0))
            .add(holes::Column::ExpiresAt.lte(now)),
        Some("revoked") => Condition::all().add(holes::Column::RevokedAt.gt(0)),
        Some(_) => return Err(invalid()),
    };
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::list::<holes::Entity, holes::Model>(
        &s.db,
        &SPEC,
        holes::Entity::find().filter(condition),
        &scope.workspace_id,
        &query,
    )
    .await?;
    Ok(Json(json!({
        "items": views(&s, &scope, rows).await?,
        "total": total,
        "page": query.page,
        "page_size": query.page_size,
    })))
}

pub async fn detail(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    Ok(Json(view(&s, &scope, id).await?))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    host_id: Id,
    target_port: i64,
    #[serde(default = "default_strip")]
    strip_prefix: bool,
    ttl_hours: i64,
}
fn default_strip() -> bool {
    true
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("holes.create")?;
    let name = checked_name(&body.name)?;
    let ttl = checked_ttl(body.ttl_hours)?;
    if !(1..=65535).contains(&body.target_port) {
        return Err(invalid());
    }
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let row = insert(
        &s,
        &tx,
        &scope,
        NewHole {
            host_id: body.host_id,
            name,
            target_port: body.target_port,
            strip_prefix: body.strip_prefix,
            expires_at: db::now() + ttl,
            source: ("manual", 0, String::new()),
            review_state: "none",
        },
    )
    .await?;
    let id = Id::from(row.id);
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "holes",
        &id,
        json!({"name": row.name, "host_id": body.host_id, "target_port": row.target_port,
               "strip_prefix": row.strip_prefix, "ttl_hours": body.ttl_hours}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view(&s, &scope, id).await?)))
}

/// What differs between a manual preview and one a pipeline or workflow opens.
pub(crate) struct NewHole {
    pub host_id: Id,
    pub name: String,
    pub target_port: i64,
    pub strip_prefix: bool,
    pub expires_at: i64,
    /// kind, id, ref: see design section 3.
    pub source: (&'static str, i64, String),
    pub review_state: &'static str,
}

/// The one place a preview row is created: the host must be a runner host in
/// the caller's workspace, and only a hash and a sealed copy of the token are kept.
async fn insert(
    s: &AppState,
    conn: &impl ConnectionTrait,
    scope: &Scope,
    new: NewHole,
) -> Result<holes::Model> {
    hosts::Entity::find_by_id(i64::from(new.host_id))
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::Connection.eq("runner"))
        .one(conn)
        .await?
        .ok_or_else(invalid)?;
    let id = s.next_id()?;
    let now = db::now();
    let token = crate::hosts::runner::secret();
    let (source_kind, source_id, source_ref) = new.source;
    let mut row = holes::Model {
        id: id.into(),
        workspace_id: scope.workspace_id.into(),
        host_id: new.host_id.into(),
        name: new.name,
        target_port: new.target_port,
        strip_prefix: new.strip_prefix,
        token_hash: crate::hosts::runner::fingerprint(&token),
        token_sealed: String::new(),
        expires_at: new.expires_at,
        revoked_at: 0,
        revoked_by: 0,
        probe_state: "unknown".into(),
        probe_error: String::new(),
        probed_at: 0,
        probed_by: 0,
        created_at: now,
        created_by: scope.user.id.into(),
        updated_at: now,
        updated_by: scope.user.id.into(),
        version: 1,
        source_kind: source_kind.into(),
        source_id,
        source_ref,
        review_state: new.review_state.into(),
        reviewed_at: 0,
        reviewed_by: 0,
    };
    row.token_sealed = s.vault.seal(&token, &aad(&row))?;
    holes::Entity::insert(row.clone().into_active_model())
        .exec(conn)
        .await?;
    Ok(row)
}

#[derive(Deserialize)]
pub struct UpdateBody {
    version: i64,
    name: Option<String>,
    strip_prefix: Option<bool>,
    /// Present means "extend": the new expiry counts from now.
    ttl_hours: Option<i64>,
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<UpdateBody>,
) -> Result<Json<Value>> {
    scope.require("holes.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = find(&tx, &scope, id).await?;
    if old.revoked_at > 0 {
        return Err(revoked());
    }
    // A pipeline or workflow owns its preview's lifetime; it can only be revoked.
    if old.source_kind != "manual" {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("holes.automatic"),
        ));
    }
    if old.version != body.version {
        return Err(changed());
    }
    let now = db::now();
    let mut row = old.clone().into_active_model();
    let mut details = serde_json::Map::new();
    if let Some(name) = &body.name {
        let name = checked_name(name)?;
        details.insert("name".into(), json!(name));
        row.name = Set(name);
    }
    if let Some(strip) = body.strip_prefix {
        details.insert("strip_prefix".into(), json!(strip));
        row.strip_prefix = Set(strip);
    }
    if let Some(hours) = body.ttl_hours {
        details.insert("ttl_hours".into(), json!(hours));
        row.expires_at = Set(now + checked_ttl(hours)?);
    }
    row.updated_at = Set(now);
    row.updated_by = Set(scope.user.id.into());
    row.version = Set(old.version.checked_add(1).ok_or_else(changed)?);
    row.update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "holes",
        &id,
        Value::Object(details),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(view(&s, &scope, id).await?))
}

pub async fn revoke(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("holes.update")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = find(&tx, &scope, id).await?;
    if old.revoked_at > 0 {
        return Err(revoked());
    }
    let now = db::now();
    let mut row = old.clone().into_active_model();
    row.revoked_at = Set(now);
    row.revoked_by = Set(scope.user.id.into());
    row.updated_at = Set(now);
    row.updated_by = Set(scope.user.id.into());
    row.version = Set(old.version.checked_add(1).ok_or_else(changed)?);
    row.update(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "revoke",
        "holes",
        &id,
        json!({"name": old.name}),
    )
    .await?;
    tx.commit().await?;
    proxy::close(old.id);
    crate::workflows::preview::nudge(&s.db, &old).await?;
    Ok(Json(view(&s, &scope, id).await?))
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    scope.require("holes.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let old = find(&tx, &scope, id).await?;
    holes::Entity::delete_by_id(old.id).exec(&tx).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "holes",
        &id,
        json!({"name": old.name}),
    )
    .await?;
    tx.commit().await?;
    proxy::close(old.id);
    crate::workflows::preview::nudge(&s.db, &old).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn probe_now(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("holes.update")?;
    let row = find(&s.db, &scope, id).await?;
    if state(&row, db::now()) != "active" {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            Message::new("holes.notActive"),
        ));
    }
    proxy::probe(&s, &row).await;
    crate::hosts::activity(&s, &scope, "probe", "holes", &id, json!({"name": row.name})).await?;
    Ok(Json(view(&s, &scope, id).await?))
}

/// Deleting a host deletes its holes in the same transaction. Returns the ids
/// so the caller can cut their live connections once the delete is committed.
pub(crate) async fn delete_for_host(
    s: &AppState,
    conn: &impl ConnectionTrait,
    scope: &Scope,
    host_id: Id,
) -> Result<Vec<i64>> {
    let rows = holes::Entity::find()
        .filter(holes::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(holes::Column::HostId.eq(host_id))
        .all(conn)
        .await?;
    for row in &rows {
        holes::Entity::delete_by_id(row.id).exec(conn).await?;
        crate::hosts::activity_in(
            s,
            conn,
            scope,
            "delete",
            "holes",
            &Id::from(row.id),
            json!({"name": row.name, "reason": "host-deleted"}),
        )
        .await?;
    }
    Ok(rows.into_iter().map(|row| row.id).collect())
}

/// Cuts the live connections of holes that no longer exist.
pub(crate) fn close_all(ids: &[i64]) {
    for id in ids {
        proxy::close(*id);
    }
}

/// Background reachability monitor (design section 9). One replica at a time.
pub async fn monitor(s: AppState, mut drain: tokio::sync::watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(PROBE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let owner = format!("holes-probe:{}", s.instance_id);
    loop {
        tokio::select! {
            _ = drain.changed() => break,
            _ = tick.tick() => {
                if let Err(error) = probe_round(&s, &owner).await {
                    tracing::warn!(%error, "预览穿透探测失败");
                }
                if let Err(error) = owned::sweep(&s).await {
                    tracing::warn!(%error, "预览穿透来源回收失败");
                }
                proxy::forget_stale();
            }
        }
    }
}

async fn probe_round(s: &AppState, owner: &str) -> Result<()> {
    use futures::StreamExt;
    let now = db::now();
    let ttl = i64::try_from(PROBE_EVERY.as_millis()).unwrap_or(i64::MAX) * 2;
    if !crate::lock::acquire(&s.db, "holes-probe", owner, now, ttl).await? {
        return Ok(());
    }
    let rows = holes::Entity::find()
        .filter(holes::Column::RevokedAt.eq(0))
        .filter(holes::Column::ExpiresAt.gt(now))
        .order_by_asc(holes::Column::ProbedAt)
        .limit(PROBE_BATCH)
        .all(&s.db)
        .await?;
    futures::stream::iter(rows)
        .for_each_concurrent(PROBE_PARALLEL, |row| async move {
            proxy::probe(s, &row).await;
        })
        .await;
    Ok(())
}

/// Stored probe error text is bounded by the column.
fn bounded(text: &str) -> String {
    text.chars().take(ERROR_LIMIT).collect()
}
