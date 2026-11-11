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
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set, TransactionTrait, sea_query::Expr};
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
    if b.source.url()?.scheme() != "https" {
        return Err(bad("marketplace.platformHttpsRequired"));
    }
    Ok(())
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
    let mut value: Value = serde_json::from_str(&row.config).map_err(|_| bad("marketplace.sourceConfigCorrupt"))?;
    let object = value.as_object_mut().ok_or_else(|| bad("marketplace.configCorrupt"))?;
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

pub async fn list(State(s): State<AppState>, admin: Admin, Query(raw): Query<HashMap<String, String>>) -> Result<Json<Value>> {
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
    Ok(Json(json!({"items":items,"total":total,"page":q.page,"page_size":q.page_size})))
}

pub async fn get(State(s): State<AppState>, admin: Admin, Path(id): Path<Id>) -> Result<Json<Value>> {
    check_admin(&s.db, &admin).await?;
    Ok(Json(source_json(&s.db, scoped(&s.db, id).await?).await?))
}

pub async fn create(State(s): State<AppState>, admin: Admin, Json(b): Json<PlatformBody>) -> Result<(StatusCode, Json<Value>)> {
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
    crate::accounts::platform::audit(&s, &tx, &admin.0.user, "create", "marketplace-sources", id, json!({"name":b.name,"trust":b.trust})).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"version":1}))))
}

pub async fn update(State(s): State<AppState>, admin: Admin, Path(id): Path<Id>, Json(b): Json<PlatformBody>) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    check_admin(&tx, &admin).await?;
    validate(&b)?;
    scoped(&tx, id).await?;
    let now = db::now();
    let n = sources::Entity::update_many()
        .col_expr(sources::Column::Name, Expr::value(b.name.trim()))
        .col_expr(sources::Column::Config, Expr::value(serde_json::to_string(&b.source).unwrap()))
        .col_expr(sources::Column::Trust, Expr::value(b.trust.clone()))
        .col_expr(sources::Column::Enabled, Expr::value(i64::from(b.enabled)))
        .col_expr(sources::Column::SyncInterval, Expr::value(b.sync_interval_hours * 3600))
        .col_expr(sources::Column::UpdatedBy, Expr::value(i64::from(admin.0.user.id)))
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
        return Err(ApiError::localized(StatusCode::CONFLICT, crate::i18n::Message::new("marketplace.sourceChanged")));
    }
    crate::accounts::platform::audit(&s, &tx, &admin.0.user, "update", "marketplace-sources", id, json!({"name":b.name,"trust":b.trust,"enabled":b.enabled})).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":b.version+1})))
}

pub async fn delete(State(s): State<AppState>, admin: Admin, Path(id): Path<Id>) -> Result<StatusCode> {
    let tx = s.db.begin().await?;
    check_admin(&tx, &admin).await?;
    let row = scoped(&tx, id).await?;
    if row.sync_lease_until > db::now() {
        return Err(ApiError::localized(StatusCode::CONFLICT, crate::i18n::Message::new("marketplace.waitForSync")));
    }
    entries::Entity::delete_many()
        .filter(entries::Column::WorkspaceId.eq(PLATFORM))
        .filter(entries::Column::SourceId.eq(id))
        .exec(&tx)
        .await?;
    sources::Entity::delete_by_id(i64::from(id)).exec(&tx).await?;
    crate::accounts::platform::audit(&s, &tx, &admin.0.user, "delete", "marketplace-sources", id, json!({"imported_templates_preserved":true})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
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
            user: User { id: id.into(), email: format!("user{id}@example.invalid"), name: format!("Owner {id}"), is_superadmin },
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
        let denied = create(State(s.clone()), account(2, true), Json(body("Official"))).await.err().unwrap();
        assert_eq!(denied.0, StatusCode::FORBIDDEN);
        let (status, Json(created)) = create(State(s.clone()), superadmin(&s).await, Json(body("Official"))).await.unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let Json(row) = get(State(s.clone()), account(1, true), Path(serde_json::from_value(created["id"].clone()).unwrap())).await.unwrap();
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
            let error = create(State(s.clone()), account(1, true), Json(with("Official", patch.clone()))).await.err().unwrap_or_else(|| panic!("{patch} accepted"));
            assert_eq!(error.0, StatusCode::BAD_REQUEST, "{patch}");
        }
        drop(admin);
    }

    #[tokio::test]
    async fn platform_source_update_version_conflict() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let (_, Json(created)) = create(State(s.clone()), superadmin(&s).await, Json(body("Official"))).await.unwrap();
        let id: Id = serde_json::from_value(created["id"].clone()).unwrap();
        let mut b = with("Renamed", json!({"version":1}));
        let Json(updated) = update(State(s.clone()), account(1, true), Path(id), Json(b)).await.unwrap();
        assert_eq!(updated["version"], 2);
        b = with("Stale", json!({"version":1}));
        assert_eq!(update(State(s.clone()), account(1, true), Path(id), Json(b)).await.err().unwrap().0, StatusCode::CONFLICT);
        assert_eq!(delete(State(s.clone()), account(1, true), Path(id)).await.unwrap(), StatusCode::NO_CONTENT);
        assert_eq!(get(State(s.clone()), account(1, true), Path(id)).await.err().unwrap().0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn platform_sources_not_listed_in_workspace_sources() {
        let (_dir, s) = crate::agents::tests::setup().await;
        let (_, Json(created)) = create(State(s.clone()), superadmin(&s).await, Json(body("Official"))).await.unwrap();
        let Json(list_all) = list(State(s.clone()), account(1, true), Query(HashMap::new())).await.unwrap();
        assert_eq!(list_all["total"], 1);
        let Json(workspace) = super::super::list_sources(State(s.clone()), crate::agents::tests::admin(1), Query(HashMap::new())).await.unwrap();
        assert!(workspace["items"].as_array().unwrap().iter().all(|i| i["id"] != created["id"]));
    }
}
