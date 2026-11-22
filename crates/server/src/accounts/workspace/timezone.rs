//! Workspace business-time configuration. UTC instants and SQL sessions stay UTC.
use super::*;
use axum::extract::State;
use sea_orm::{DatabaseTransaction, TransactionTrait, sea_query::Expr};

pub(crate) fn validate(value: &str) -> Result<chrono_tz::Tz> {
    value.trim().parse().map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.timezone_invalid"),
        )
    })
}

fn view(row: &workspaces::Model) -> Value {
    json!({"id":"1","name":row.name,"timezone":row.timezone,"version":row.updated_at})
}

pub async fn read(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    if i64::from(id) != 1 {
        return Err(ApiError::missing());
    }
    let row = workspaces::Entity::find_by_id(i64::from(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(view(&row)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    timezone: String,
    version: i64,
    // Shared resource forms resend these read-only fields. Neither chooses the tenant.
    #[serde(rename = "id")]
    _id: Option<Id>,
    #[serde(rename = "name")]
    _name: Option<String>,
}

/// Changes workspace wall-clock semantics and recomputes future schedules atomically.
pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<Update>,
) -> Result<Json<Value>> {
    if i64::from(id) != 1 {
        return Err(ApiError::missing());
    }
    scope.require("settings.update")?;
    let tx = s.db.begin().await?;
    let row = super::super::workspace_admin::lock_workspace(&tx, scope.workspace_id).await?;
    let scope = super::super::workspace_admin::authorize(&tx, &scope, "settings.update").await?;
    if row.updated_at != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.recordChangedOrDeleted"),
        ));
    }
    let saved = update_in(&tx, &row, &row.name, Some(&body.timezone)).await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "workspace-settings",
        &scope.workspace_id,
        json!({"timezone":{"from":row.timezone,"to":saved.timezone}}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(view(&saved)))
}

/// Both platform administration and workspace administration use the same write path.
/// None preserves the saved zone for older clients; a confirmed zone cannot be cleared.
pub(crate) async fn update_in(
    tx: &DatabaseTransaction,
    row: &workspaces::Model,
    name: &str,
    timezone: Option<&str>,
) -> Result<workspaces::Model> {
    let timezone = timezone
        .map(validate)
        .transpose()?
        .map(|zone| zone.name().to_owned())
        .or_else(|| row.timezone.clone());
    let now = db::now();
    let version = now.max(row.updated_at.saturating_add(1));
    let changed = workspaces::Entity::update_many()
        .col_expr(workspaces::Column::Name, Expr::value(name))
        .col_expr(workspaces::Column::Timezone, Expr::value(timezone.clone()))
        .col_expr(workspaces::Column::UpdatedAt, Expr::value(version))
        .filter(workspaces::Column::Id.eq(row.id))
        .filter(workspaces::Column::UpdatedAt.eq(row.updated_at))
        .exec(tx)
        .await?;
    if changed.rows_affected != 1 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("platform.recordChangedOrDeleted"),
        ));
    }
    if timezone != row.timezone {
        let zone = validate(timezone.as_deref().ok_or_else(ApiError::missing)?)?;
        crate::cron_jobs::recompute_workspace_schedules(tx, row.id.into(), zone, now).await?;
    }
    workspaces::Entity::find_by_id(row.id)
        .one(tx)
        .await?
        .ok_or_else(ApiError::missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::{admin, setup};
    use axum::http::StatusCode;

    fn body(value: Value) -> Json<Update> {
        Json(serde_json::from_value(value).unwrap())
    }

    #[tokio::test]
    async fn workspace_timezone_is_explicit_audited_scoped_and_versioned() {
        let (_dir, s) = setup().await;
        crate::entities::workspace_roles::Entity::insert(
            crate::entities::workspace_roles::ActiveModel {
                id: Set(1),
                workspace_id: Set(1),
                name: Set("Owner".into()),
                permissions: Set(serde_json::to_string(
                    &PERMISSIONS.iter().map(|p| p.0).collect::<Vec<_>>(),
                )
                .unwrap()),
                builtin: Set(1),
                created_at: Set(0),
                updated_at: Set(0),
                version: Set(1),
            },
        )
        .exec(&s.db)
        .await
        .unwrap();
        let scope = admin(1);
        let path = || Path(("1".into(), 1.into()));
        let Json(old) = read(State(s.clone()), scope.clone(), path()).await.unwrap();
        assert!(old["timezone"].is_null());
        assert!(business_timezone(&s.db, 1.into()).await.unwrap().is_none());
        let mut forbidden = scope.clone();
        forbidden.permissions.remove("settings.update");
        assert_eq!(
            update(
                State(s.clone()),
                forbidden,
                path(),
                body(json!({"timezone":"UTC","version":old["version"]}))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
        for timezone in ["", "system", "+08:00", "Not/AZone"] {
            assert_eq!(
                update(
                    State(s.clone()),
                    scope.clone(),
                    path(),
                    body(json!({"timezone":timezone,"version":old["version"]}))
                )
                .await
                .unwrap_err()
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        let Json(saved) = update(State(s.clone()), scope.clone(), path(),
            body(json!({"id":"2","name":"ignored tenant name","timezone":"Asia/Kuala_Lumpur","version":old["version"]})))
            .await.unwrap();
        assert_eq!(saved["name"], old["name"]);
        assert_eq!(saved["timezone"], "Asia/Kuala_Lumpur");
        assert_ne!(saved["version"], old["version"]);
        assert_eq!(
            business_timezone(&s.db, 1.into())
                .await
                .unwrap()
                .unwrap()
                .name(),
            "Asia/Kuala_Lumpur"
        );
        assert!(business_timezone(&s.db, 2.into()).await.unwrap().is_none());
        assert_eq!(
            update(
                State(s.clone()),
                scope.clone(),
                path(),
                body(json!({"timezone":"UTC","version":old["version"]}))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            read(State(s.clone()), scope, Path(("1".into(), 2.into())))
                .await
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );
        let audits = crate::entities::activity::Entity::find()
            .filter(crate::entities::activity::Column::WorkspaceId.eq(1_i64))
            .filter(crate::entities::activity::Column::Resource.eq("workspace-settings"))
            .all(&s.db)
            .await
            .unwrap();
        assert_eq!(audits.len(), 1);
        crate::entities::workspace_roles::Entity::update_many()
            .col_expr(
                crate::entities::workspace_roles::Column::Permissions,
                Expr::value("[]"),
            )
            .filter(crate::entities::workspace_roles::Column::Id.eq(1_i64))
            .exec(&s.db)
            .await
            .unwrap();
        // The stale in-memory scope cannot outlive the role revocation.
        assert_eq!(
            update(
                State(s.clone()),
                admin(1),
                path(),
                body(json!({"timezone":"UTC","version":saved["version"]}))
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn workspace_creation_accepts_confirmed_zone_and_preserves_legacy_api_compatibility() {
        let (_dir, s) = setup().await;
        for (slug, timezone) in [
            ("zone-confirmed", Some("Pacific/Auckland")),
            ("legacy-client", None),
        ] {
            let tx = s.db.begin().await.unwrap();
            let request =
                serde_json::from_value(json!({"name":slug,"slug":slug,"timezone":timezone}))
                    .unwrap();
            let value = create_in(&s, admin(1).user.id, request, &tx).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(value["timezone"], json!(timezone));
            let id = serde_json::from_value(value["id"].clone()).unwrap();
            assert_eq!(
                business_timezone(&s.db, id)
                    .await
                    .unwrap()
                    .map(|zone| zone.name()),
                timezone
            );
        }
    }
}
