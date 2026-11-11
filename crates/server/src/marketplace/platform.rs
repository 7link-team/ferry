//! Platform-level public template sources: superadmin only, no credentials, stored with workspace_id 0.
use super::Source;
use crate::{
    accounts::platform::{Admin, check_admin},
    db,
    entities::{marketplace_entries as entries, marketplace_sources as sources},
    error::{ApiError, Json, Path, Query, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{extract::State, http::StatusCode};
use sea_orm::{
    ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait, sea_query::Expr,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) const PLATFORM: i64 = 0;
const SPEC: Spec = Spec {
    search: &["name"],
    filters: &["state", "trust"],
    sortable: &["name", "updated_at"],
    default_sort: "name",
};

#[derive(Deserialize)]
pub struct PlatformBody {
    name: String,
    trust: String,
    #[serde(default = "enabled")]
    enabled: bool,
    sync_interval_hours: i64,
    #[serde(flatten)]
    source: Source,
    #[serde(default)]
    version: i64,
}
fn enabled() -> bool {
    true
}

fn bad(key: &str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new(key))
}

fn validate(b: &PlatformBody) -> Result<()> {
    if b.name.trim().is_empty() || b.name.chars().count() > 60 {
        return Err(bad("marketplace.nameInvalid"));
    }
    if !matches!(b.trust.as_str(), "official" | "community") {
        return Err(bad("marketplace.trustInvalid"));
    }
    if !(1..=168).contains(&b.sync_interval_hours) {
        return Err(bad("marketplace.syncIntervalInvalid"));
    }
    let url = b.source.url()?;
    if url.scheme() != "https" && !test_loopback_http(&url) {
        return Err(bad("marketplace.platformHttpsRequired"));
    }
    Ok(())
}

/// Debug builds only: acceptance scripts serve fixtures over plain HTTP on loopback when
/// FERRY_TEST_MARKETPLACE_HTTP=1. Release builds always require HTTPS.
fn test_loopback_http(url: &reqwest::Url) -> bool {
    cfg!(debug_assertions)
        && std::env::var("FERRY_TEST_MARKETPLACE_HTTP").is_ok_and(|v| v == "1")
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

pub(crate) async fn scoped(conn: &impl sea_orm::ConnectionTrait, id: Id) -> Result<sources::Model> {
    sources::Entity::find_by_id(i64::from(id))
        .filter(sources::Column::WorkspaceId.eq(PLATFORM))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}

async fn source_json(conn: &impl sea_orm::ConnectionTrait, row: sources::Model) -> Result<Value> {
    let entry_count = entries::Entity::find()
        .filter(entries::Column::WorkspaceId.eq(PLATFORM))
        .filter(entries::Column::SourceId.eq(row.id))
        .count(conn)
        .await?;
    let mut value: Value =
        serde_json::from_str(&row.config).map_err(|_| bad("marketplace.sourceConfigCorrupt"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| bad("marketplace.configCorrupt"))?;
    for (k, v) in json!({
        "id": Id::from(row.id), "name": row.name, "trust": row.trust, "enabled": row.enabled != 0,
        "sync_interval_hours": row.sync_interval / 3600, "next_sync_at": row.next_sync_at,
        "state": row.state, "last_error": crate::i18n::display_stored(&row.last_error),
        "synced_at": row.synced_at, "version": row.version, "updated_at": row.updated_at, "entry_count": entry_count
    })
    .as_object()
    .unwrap()
    {
        object.insert(k.clone(), v.clone());
    }
    Ok(value)
}

pub async fn list(
    State(s): State<AppState>,
    admin: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    check_admin(&s.db, &admin).await?;
    let q = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) = resource::global_list::<sources::Entity, sources::Model>(
        &s.db,
        &admin,
        &SPEC,
        sources::Entity::find().filter(sources::Column::WorkspaceId.eq(PLATFORM)),
        &q,
    )
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(source_json(&s.db, row).await?);
    }
    Ok(Json(
        json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}

pub async fn get(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    check_admin(&s.db, &admin).await?;
    Ok(Json(source_json(&s.db, scoped(&s.db, id).await?).await?))
}

pub async fn create(
    State(s): State<AppState>,
    admin: Admin,
    Json(b): Json<PlatformBody>,
) -> Result<(StatusCode, Json<Value>)> {
    let tx = s.db.begin().await?;
    check_admin(&tx, &admin).await?;
    validate(&b)?;
    let id = s.next_id()?;
    let now = db::now();
    let actor = i64::from(admin.0.user.id);
    sources::Entity::insert(sources::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(PLATFORM),
        name: Set(b.name.trim().into()),
        repository_id: Set(0),
        config: Set(serde_json::to_string(&b.source).unwrap()),
        state: Set("idle".into()),
        last_error: Set(String::new()),
        synced_at: Set(0),
        run_as: Set(actor),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
        trust: Set(b.trust.clone()),
        enabled: Set(i64::from(b.enabled)),
        sync_interval: Set(b.sync_interval_hours * 3600),
        next_sync_at: Set(0),
        sync_owner: Set(String::new()),
        sync_lease_until: Set(0),
        created_by: Set(actor),
        updated_by: Set(actor),
        synced_by: Set(0),
    })
    .exec(&tx)
    .await?;
    crate::accounts::platform::audit(
        &s,
        &tx,
        &admin.0.user,
        "create",
        "marketplace-sources",
        id,
        json!({"name":b.name,"trust":b.trust}),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}

pub async fn update(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
    Json(b): Json<PlatformBody>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    check_admin(&tx, &admin).await?;
    validate(&b)?;
    scoped(&tx, id).await?;
    let now = db::now();
    let n = sources::Entity::update_many()
        .col_expr(sources::Column::Name, Expr::value(b.name.trim()))
        .col_expr(
            sources::Column::Config,
            Expr::value(serde_json::to_string(&b.source).unwrap()),
        )
        .col_expr(sources::Column::Trust, Expr::value(b.trust.clone()))
        .col_expr(sources::Column::Enabled, Expr::value(i64::from(b.enabled)))
        .col_expr(
            sources::Column::SyncInterval,
            Expr::value(b.sync_interval_hours * 3600),
        )
        .col_expr(
            sources::Column::UpdatedBy,
            Expr::value(i64::from(admin.0.user.id)),
        )
        .col_expr(sources::Column::UpdatedAt, Expr::value(now))
        .col_expr(sources::Column::Version, Expr::value(b.version + 1))
        .filter(sources::Column::Id.eq(id))
        .filter(sources::Column::WorkspaceId.eq(PLATFORM))
        .filter(sources::Column::Version.eq(b.version))
        .filter(sources::Column::SyncLeaseUntil.lte(now))
        .exec(&tx)
        .await?
        .rows_affected;
    if n == 0 {
        let key = if scoped(&tx, id).await?.sync_lease_until > now {
            "marketplace.waitForSync"
        } else {
            "marketplace.sourceChanged"
        };
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new(key),
        ));
    }
    crate::accounts::platform::audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "marketplace-sources",
        id,
        json!({"name":b.name,"trust":b.trust,"enabled":b.enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":b.version+1})))
}

pub async fn delete(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
) -> Result<StatusCode> {
    let tx = s.db.begin().await?;
    check_admin(&tx, &admin).await?;
    scoped(&tx, id).await?;
    let removed = sources::Entity::delete_many()
        .filter(sources::Column::Id.eq(id))
        .filter(sources::Column::WorkspaceId.eq(PLATFORM))
        .filter(sources::Column::SyncLeaseUntil.lte(db::now()))
        .exec(&tx)
        .await?
        .rows_affected;
    if removed == 0 {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("marketplace.waitForSync"),
        ));
    }
    entries::Entity::delete_many()
        .filter(entries::Column::WorkspaceId.eq(PLATFORM))
        .filter(entries::Column::SourceId.eq(id))
        .exec(&tx)
        .await?;
    crate::accounts::platform::audit(
        &s,
        &tx,
        &admin.0.user,
        "delete",
        "marketplace-sources",
        id,
        json!({"imported_templates_preserved":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

const LEASE_MS: i64 = 60_000;

/// Takes the sync lease for one platform source; true only for the single winner.
pub(crate) async fn claim(s: &AppState, id: i64) -> Result<bool> {
    let now = db::now();
    Ok(sources::Entity::update_many()
        .col_expr(
            sources::Column::SyncOwner,
            Expr::value(s.instance_id.clone()),
        )
        .col_expr(sources::Column::SyncLeaseUntil, Expr::value(now + LEASE_MS))
        .col_expr(sources::Column::State, Expr::value("syncing"))
        .filter(sources::Column::Id.eq(id))
        .filter(sources::Column::WorkspaceId.eq(PLATFORM))
        .filter(sources::Column::SyncLeaseUntil.lte(now))
        .exec(&s.db)
        .await?
        .rows_affected
        == 1)
}

/// Reads a public bundle without credentials and replaces this source's catalog; failure keeps it.
/// Replaces the catalog and marks the source idle, but only while this instance still owns the lease.
pub(crate) async fn commit_sync(
    s: &AppState,
    id: i64,
    actor: i64,
    packages: Vec<super::index::PackageVersion>,
    interval: i64,
    deadline: std::time::Instant,
) -> Result<usize> {
    let tx = s.db.begin().await?;
    let total =
        super::store::replace_entries(s, &tx, PLATFORM, id, actor, packages, deadline).await?;
    let now = db::now();
    let owned = sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("idle"))
        .col_expr(sources::Column::LastError, Expr::value(""))
        .col_expr(sources::Column::SyncedAt, Expr::value(now))
        .col_expr(sources::Column::SyncedBy, Expr::value(actor))
        .col_expr(sources::Column::NextSyncAt, Expr::value(now + interval))
        .col_expr(sources::Column::SyncOwner, Expr::value(""))
        .col_expr(sources::Column::SyncLeaseUntil, Expr::value(0))
        .filter(sources::Column::Id.eq(id))
        .filter(sources::Column::SyncOwner.eq(s.instance_id.clone()))
        .exec(&tx)
        .await?
        .rows_affected;
    if owned == 0 {
        // Lease expired and another instance took over: dropping `tx` rolls back the replaced entries.
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("marketplace.syncing"),
        ));
    }
    tx.commit().await?;
    Ok(total)
}

pub(crate) async fn synchronize_platform(s: &AppState, id: i64, actor: i64) -> Result<usize> {
    if !claim(s, id).await? {
        return Err(ApiError::localized(
            StatusCode::CONFLICT,
            crate::i18n::Message::new("marketplace.syncing"),
        ));
    }
    let timeout = std::time::Duration::from_secs(20);
    let deadline = std::time::Instant::now() + timeout;
    let row = scoped(&s.db, id.into()).await?;
    let interval = row.sync_interval.max(3600) * 1000;
    let result: Result<usize> = async {
        let config: Source =
            serde_json::from_str(&row.config).map_err(|_| bad("marketplace.configCorrupt"))?;
        let text = super::fetch(&config, None, false, timeout).await?;
        let packages = super::index::parse_bundle(&text, &super::ferry_version(), "en-US")?;
        commit_sync(s, id, actor, packages, interval, deadline).await
    }
    .await;
    if let Err(error) = &result {
        let recorded = sources::Entity::update_many()
            .col_expr(sources::Column::State, Expr::value("failed"))
            .col_expr(sources::Column::LastError, Expr::value(error.stored()))
            .col_expr(
                sources::Column::NextSyncAt,
                Expr::value(db::now() + interval),
            )
            .col_expr(sources::Column::SyncOwner, Expr::value(""))
            .col_expr(sources::Column::SyncLeaseUntil, Expr::value(0))
            .filter(sources::Column::Id.eq(id))
            .filter(sources::Column::SyncOwner.eq(s.instance_id.clone()))
            .exec(&s.db)
            .await;
        if let Err(write) = recorded {
            tracing::warn!(source = id, %write, sync_error = %error, "记录平台模板来源同步失败状态失败");
        }
    }
    result
}

pub async fn sync(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    check_admin(&s.db, &admin).await?;
    scoped(&s.db, id).await?;
    let total = synchronize_platform(&s, id.into(), admin.0.user.id.into()).await?;
    crate::accounts::platform::audit(
        &s,
        &s.db,
        &admin.0.user,
        "sync",
        "marketplace-sources",
        id,
        json!({"templates":total}),
    )
    .await?;
    Ok(Json(json!({"templates": total})))
}

/// A crashed instance leaves `syncing` with a lease that eventually expires; mark those failed so
/// the source can be synced again. Rows with a live lease belong to a running instance.
pub(crate) async fn release_expired(s: &AppState) -> Result<()> {
    sources::Entity::update_many()
        .col_expr(sources::Column::State, Expr::value("failed"))
        .col_expr(
            sources::Column::LastError,
            Expr::value(crate::i18n::store_message(&crate::i18n::Message::new(
                "marketplace.interrupted",
            ))),
        )
        .col_expr(sources::Column::SyncOwner, Expr::value(""))
        .col_expr(sources::Column::SyncLeaseUntil, Expr::value(0))
        .filter(sources::Column::WorkspaceId.eq(PLATFORM))
        .filter(sources::Column::State.eq("syncing"))
        .filter(sources::Column::SyncLeaseUntil.lte(db::now()))
        .exec(&s.db)
        .await?;
    Ok(())
}

/// Background worker: synchronizes enabled platform sources whose interval elapsed.
pub async fn scheduler(s: AppState, mut drain: tokio::sync::watch::Receiver<bool>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = drain.changed() => break, _ = tick.tick() => {} }
        if let Err(error) = release_expired(&s).await {
            tracing::warn!(%error, "释放过期的平台模板来源同步失败");
        }
        let now = db::now();
        let due = match sources::Entity::find()
            .filter(sources::Column::WorkspaceId.eq(PLATFORM))
            .filter(sources::Column::Enabled.eq(1))
            .filter(sources::Column::NextSyncAt.lte(now))
            .filter(sources::Column::SyncLeaseUntil.lte(now))
            .all(&s.db)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "读取平台模板来源失败");
                continue;
            }
        };
        for row in due {
            match synchronize_platform(&s, row.id, 0).await {
                Ok(total) => tracing::info!(source = row.id, total, "平台模板来源已同步"),
                Err(error) if error.0 == StatusCode::CONFLICT => {}
                Err(error) => tracing::warn!(source = row.id, %error, "平台模板来源同步失败"),
            }
        }
    }
}

#[cfg(test)]
mod sync_tests {
    use super::tests::{account, body, superadmin};
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves each response in order, one connection per response.
    async fn fixture(responses: Vec<(u16, String)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut part = [0u8; 1024];
                    let n = stream.read(&mut part).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&part[..n]);
                }
                let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            }
        });
        address
    }
    fn wrapped(index: &Value) -> String {
        use base64::Engine;
        json!({"content": base64::engine::general_purpose::STANDARD.encode(index.to_string()), "encoding": "base64"}).to_string()
    }
    fn bundle() -> Value {
        let app = json!({"apiVersion":"ferry/v1","kind":"Application","metadata":{"name":"web"},"services":[{"name":"web","image":"busybox:1.36"}]});
        json!({"schemaVersion":2,"packages":[{"key":"web","kind":"app","meta":{"name":{"en-US":"Website"},"category":"web","topology":"single","drivers":["docker"]},"versions":[{"version":"1.0.0","blueprint":app},{"version":"1.1.0","blueprint":app}]}]})
    }
    async fn platform_source(s: &AppState, api_base: &str) -> i64 {
        let (_, Json(created)) = create(
            State(s.clone()),
            superadmin(s).await,
            Json(body("Official")),
        )
        .await
        .unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let mut config: Value =
            serde_json::from_str(&scoped(&s.db, id).await.unwrap().config).unwrap();
        config["api_base"] = json!(api_base);
        sources::Entity::update_many()
            .col_expr(sources::Column::Config, Expr::value(config.to_string()))
            .filter(sources::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
        id.into()
    }

    #[tokio::test]
    async fn lease_claim_is_exclusive() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        assert!(claim(&s, id).await.unwrap());
        assert!(!claim(&s, id).await.unwrap());
        sources::Entity::update_many()
            .col_expr(sources::Column::SyncLeaseUntil, Expr::value(db::now() - 1))
            .filter(sources::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
        assert!(claim(&s, id).await.unwrap());
    }

    #[tokio::test]
    async fn platform_sync_success_and_failure_keep_previous_entries() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let api = fixture(vec![(200, wrapped(&bundle())), (500, "{}".into())]).await;
        let id = platform_source(&s, &api).await;
        assert_eq!(synchronize_platform(&s, id, 1).await.unwrap(), 2);
        let row = scoped(&s.db, id.into()).await.unwrap();
        assert_eq!(row.state, "idle");
        assert_eq!(row.synced_by, 1);
        assert_eq!(row.sync_owner, "");
        assert!(row.next_sync_at > db::now());
        assert!(synchronize_platform(&s, id, 0).await.is_err());
        let row = scoped(&s.db, id.into()).await.unwrap();
        assert_eq!(row.state, "failed");
        assert!(!row.last_error.is_empty());
        assert!(row.next_sync_at > db::now());
        assert_eq!(row.sync_lease_until, 0);
        let kept = entries::Entity::find()
            .filter(entries::Column::SourceId.eq(id))
            .count(&s.db)
            .await
            .unwrap();
        assert_eq!(kept, 2);
        let Json(listed) = get(State(s.clone()), account(1, true), Path(id.into()))
            .await
            .unwrap();
        assert_eq!(listed["entry_count"], 2);
    }

    #[tokio::test]
    async fn manual_sync_rejected_while_leased() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        assert!(claim(&s, id).await.unwrap());
        assert_eq!(
            synchronize_platform(&s, id, 1).await.err().unwrap().0,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn recover_ignores_platform_rows() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        assert!(claim(&s, id).await.unwrap());
        super::super::recover(&s).await.unwrap();
        assert_eq!(scoped(&s.db, id.into()).await.unwrap().state, "syncing");
    }

    #[tokio::test]
    async fn commit_aborts_when_lease_was_taken_over() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        sources::Entity::update_many()
            .col_expr(sources::Column::SyncOwner, Expr::value("other-instance"))
            .col_expr(
                sources::Column::SyncLeaseUntil,
                Expr::value(db::now() + 60_000),
            )
            .filter(sources::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
        let packages = super::super::index::parse_bundle(
            &bundle().to_string(),
            &semver::Version::new(0, 3, 0),
            "en-US",
        )
        .unwrap();
        let error = commit_sync(
            &s,
            id,
            1,
            packages,
            3_600_000,
            std::time::Instant::now() + std::time::Duration::from_secs(20),
        )
        .await
        .err()
        .expect("lease lost must abort");
        assert_eq!(error.0, StatusCode::CONFLICT);
        assert_eq!(
            entries::Entity::find()
                .filter(entries::Column::SourceId.eq(id))
                .count(&s.db)
                .await
                .unwrap(),
            0,
            "no entries committed"
        );
    }

    #[tokio::test]
    async fn delete_and_edit_wait_for_running_sync() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        assert!(claim(&s, id).await.unwrap());
        let deleted = delete(State(s.clone()), account(1, true), Path(id.into()))
            .await
            .err()
            .unwrap();
        assert_eq!(
            deleted.2.as_ref().map(|m| m.key.as_str()),
            Some("marketplace.waitForSync")
        );
        let version = scoped(&s.db, id.into()).await.unwrap().version;
        let mut b = body("Renamed");
        b.version = version;
        let edited = update(State(s.clone()), account(1, true), Path(id.into()), Json(b))
            .await
            .err()
            .unwrap();
        assert_eq!(edited.0, StatusCode::CONFLICT);
        assert_eq!(
            edited.2.as_ref().map(|m| m.key.as_str()),
            Some("marketplace.waitForSync")
        );
        assert!(scoped(&s.db, id.into()).await.is_ok());
    }

    #[tokio::test]
    async fn expired_lease_releases_stuck_syncing_source() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let id = platform_source(&s, "http://127.0.0.1:9").await;
        assert!(claim(&s, id).await.unwrap());
        sources::Entity::update_many()
            .col_expr(sources::Column::SyncLeaseUntil, Expr::value(db::now() - 1))
            .filter(sources::Column::Id.eq(id))
            .exec(&s.db)
            .await
            .unwrap();
        super::super::recover(&s).await.unwrap();
        let row = scoped(&s.db, id.into()).await.unwrap();
        assert_eq!(row.state, "failed");
        assert!(!row.last_error.is_empty());
        assert_eq!(row.sync_owner, "");
        let Json(listed) = get(State(s.clone()), account(1, true), Path(id.into()))
            .await
            .unwrap();
        assert_eq!(listed["state"], "failed");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::accounts::auth::{Account, User};

    pub(crate) async fn superadmin(s: &AppState) -> Admin {
        crate::entities::users::Entity::update_many()
            .col_expr(crate::entities::users::Column::IsSuperadmin, Expr::value(1))
            .filter(crate::entities::users::Column::Id.eq(1))
            .exec(&s.db)
            .await
            .unwrap();
        account(1, true)
    }
    pub(crate) fn account(id: i64, is_superadmin: bool) -> Admin {
        Admin(Account {
            user: User {
                id: id.into(),
                email: format!("user{id}@example.invalid"),
                name: format!("Owner {id}"),
                is_superadmin,
            },
            session_id: Id::from(1),
        })
    }
    pub(crate) fn body(name: &str) -> PlatformBody {
        serde_json::from_value(json!({
            "name": name, "trust": "official", "enabled": true, "sync_interval_hours": 6,
            "provider": "github", "api_base": "https://api.github.com", "project": "ferry/templates",
            "branch": "main", "index_path": "index.json"
        }))
        .unwrap()
    }
    fn with(name: &str, patch: Value) -> PlatformBody {
        let mut v = json!({
            "name": name, "trust": "official", "enabled": true, "sync_interval_hours": 6,
            "provider": "github", "api_base": "https://api.github.com", "project": "ferry/templates",
            "branch": "main", "index_path": "index.json"
        });
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn platform_sources_require_superadmin() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let denied = create(State(s.clone()), account(2, true), Json(body("Official")))
            .await
            .err()
            .unwrap();
        assert_eq!(denied.0, StatusCode::FORBIDDEN);
        let (status, Json(created)) = create(
            State(s.clone()),
            superadmin(&s).await,
            Json(body("Official")),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let Json(row) = get(
            State(s.clone()),
            account(1, true),
            Path(serde_json::from_value(created["id"].clone()).unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(row["trust"], "official");
        assert_eq!(row["sync_interval_hours"], 6);
        assert_eq!(row["enabled"], true);
        assert_eq!(row["project"], "ferry/templates");
        assert_eq!(row["entry_count"], 0);
    }

    #[tokio::test]
    async fn platform_source_validation() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let admin = superadmin(&s).await;
        for patch in [
            json!({"trust":"workspace"}),
            json!({"sync_interval_hours":0}),
            json!({"sync_interval_hours":169}),
            json!({"api_base":"http://api.github.com"}),
            json!({"name":""}),
            json!({"name":"x".repeat(61)}),
        ] {
            let error = create(
                State(s.clone()),
                account(1, true),
                Json(with("Official", patch.clone())),
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("{patch} accepted"));
            assert_eq!(error.0, StatusCode::BAD_REQUEST, "{patch}");
        }
        drop(admin);
    }

    #[tokio::test]
    async fn platform_source_update_version_conflict() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let (_, Json(created)) = create(
            State(s.clone()),
            superadmin(&s).await,
            Json(body("Official")),
        )
        .await
        .unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let mut b = with("Renamed", json!({"version":1}));
        let Json(updated) = update(State(s.clone()), account(1, true), Path(id), Json(b))
            .await
            .unwrap();
        assert_eq!(updated["version"], 2);
        b = with("Stale", json!({"version":1}));
        assert_eq!(
            update(State(s.clone()), account(1, true), Path(id), Json(b))
                .await
                .err()
                .unwrap()
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            delete(State(s.clone()), account(1, true), Path(id))
                .await
                .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            get(State(s.clone()), account(1, true), Path(id))
                .await
                .err()
                .unwrap()
                .0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn platform_sources_not_listed_in_workspace_sources() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let (_, Json(created)) = create(
            State(s.clone()),
            superadmin(&s).await,
            Json(body("Official")),
        )
        .await
        .unwrap();
        let Json(list_all) = list(State(s.clone()), account(1, true), Query(HashMap::new()))
            .await
            .unwrap();
        assert!(
            list_all["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == created["id"])
        );
        assert!(
            list_all["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == "1" && i["trust"] == "official"),
            "built-in official source is listed"
        );
        let Json(workspace) = super::super::list_sources(
            State(s.clone()),
            crate::agents::tests::admin(1),
            Query(HashMap::new()),
        )
        .await
        .unwrap();
        assert!(
            workspace["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["id"] != created["id"])
        );
    }
}
