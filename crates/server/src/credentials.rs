//! 授权认证 — the credential store shared by SSH hosts and Git repositories.
//!
//! The secret half never leaves the server. List and detail report only
//! whether one is set; there is no endpoint that returns a stored secret, not
//! even to an owner. Reading one happens inside the server, at the moment it
//! is used, and that use is audited.

use crate::entities::{credentials, hosts, repositories};
use crate::snowflake::Id;

use crate::{
    db,
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    state::AppState,
    vault,
    workspace::Scope,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QuerySelect, Select,
    Set,
    sea_query::{Expr, ExprTrait},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const SPEC: Spec = Spec {
    search: &["name", "username"],
    filters: &["kind"],
    sortable: &["name", "created_at", "updated_at"],
    default_sort: "name",
};

pub const KINDS: [&str; 3] = ["password", "ssh-key", "token"];

// List/detail never select the sealed secret, even inside the server.
#[derive(FromQueryResult)]
struct Summary {
    id: i64,
    name: String,
    kind: String,
    username: String,
    description: String,
    has_secret: bool,
    created_at: i64,
    updated_at: i64,
    version: i64,
}

fn summary_query() -> Select<credentials::Entity> {
    use credentials::Column;
    credentials::Entity::find()
        .select_only()
        .columns([
            Column::Id,
            Column::Name,
            Column::Kind,
            Column::Username,
            Column::Description,
            Column::CreatedAt,
            Column::UpdatedAt,
            Column::Version,
        ])
        .expr_as(Column::Secret.ne(""), "has_secret")
}

fn to_json(row: &Summary) -> Value {
    json!({
        "id": Id::from(row.id),
        "name": row.name,
        "kind": row.kind,
        "username": row.username,
        "description": row.description,
        // Whether a secret exists, never the secret.
        "has_secret": row.has_secret,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "version": row.version,
    })
}

#[derive(Deserialize)]
pub struct Body {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    description: String,
    /// Absent on an edit means "leave the stored secret alone", which is what
    /// lets the edit form round-trip without ever holding the real value.
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    version: i64,
}

fn check(body: &Body, creating: bool) -> Result<String> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写名称（60 字以内）。"));
    }
    if creating {
        if !KINDS.contains(&body.kind.trim()) {
            return Err(ApiError::bad("请选择凭据类型：密码、SSH 私钥或令牌。"));
        }
        match body.secret.as_deref().map(str::trim) {
            None | Some("") => return Err(ApiError::bad("请填写凭据内容。")),
            Some(secret) if secret.len() > 64 * 1024 => {
                return Err(ApiError::bad("凭据内容过长（上限 64KB）。"));
            }
            Some(_) => {}
        }
    }
    Ok(name.to_owned())
}

pub async fn list(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("credentials.read")?;
    let query = ListQuery::parse(&SPEC, &raw)?;
    let (rows, total) =
        resource::list(&s.db, &SPEC, summary_query(), &scope.workspace_id, &query).await?;
    Ok(Json(json!({
        "items": rows.iter().map(to_json).collect::<Vec<_>>(),
        "total": total, "page": query.page, "page_size": query.page_size,
    })))
}

pub async fn get(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("credentials.read")?;
    let row = summary_query()
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(credentials::Column::Id.eq(id))
        .into_model::<Summary>()
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(to_json(&row)))
}

pub async fn create(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<Body>,
) -> Result<(StatusCode, Json<Value>)> {
    scope.require("credentials.create")?;
    let name = check(&body, true)?;
    let (id, now) = (s.next_id()?, db::now());
    // The row's own identity is the associated data, so the id has to exist
    // before the secret is sealed.
    let sealed = s.vault.seal(
        body.secret.as_deref().unwrap_or_default().trim(),
        &vault::aad(
            &scope.workspace_id.to_string(),
            "credentials",
            &id.to_string(),
        ),
    )?;
    credentials::Entity::insert(credentials::ActiveModel {
        id: Set(i64::from(id)),
        workspace_id: Set(i64::from(scope.workspace_id)),
        name: Set(name.clone()),
        kind: Set(body.kind.trim().to_owned()),
        username: Set(body.username.trim().to_owned()),
        description: Set(body.description.trim().to_owned()),
        secret: Set(sealed),
        created_at: Set(now),
        updated_at: Set(now),
        version: Set(1),
        ..Default::default()
    })
    .exec(&s.db)
    .await?;
    // The details of an audit entry are readable by anyone with activity.read,
    // so they carry the name and never the secret.
    crate::hosts::activity(
        &s,
        &scope,
        "create",
        "credentials",
        &id,
        json!({"name": name}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

pub async fn update(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
    Json(body): Json<Body>,
) -> Result<Json<Value>> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("credentials.update")?;
    let managed = crate::entities::repository_connections::Entity::find()
        .filter(crate::entities::repository_connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::repository_connections::Column::CredentialId.eq(id))
        .one(&s.db)
        .await?
        .is_some();
    if managed
        && (body.kind != "token" || body.secret.as_ref().is_some_and(|v| !v.trim().is_empty()))
    {
        return Err(ApiError::bad(
            "网页授权凭据的令牌由系统维护，请通过仓库授权页面重新授权。",
        ));
    }
    let name = check(&body, false)?;
    let rotated = match body
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(secret) => Some(s.vault.seal(
            secret,
            &vault::aad(
                &scope.workspace_id.to_string(),
                "credentials",
                &id.to_string(),
            ),
        )?),
        None => None,
    };
    let mut update = credentials::Entity::update_many()
        .col_expr(credentials::Column::Name, Expr::value(name.clone()))
        .col_expr(
            credentials::Column::Username,
            Expr::value(body.username.trim()),
        )
        .col_expr(
            credentials::Column::Description,
            Expr::value(body.description.trim()),
        )
        .col_expr(credentials::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(
            credentials::Column::Version,
            Expr::col(credentials::Column::Version).add(1),
        )
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(credentials::Column::Id.eq(id))
        .filter(credentials::Column::Version.eq(body.version));
    if let Some(sealed) = &rotated {
        update = update.col_expr(credentials::Column::Secret, Expr::value(sealed.clone()));
    }
    let affected = update.exec(&s.db).await?.rows_affected;
    if affected == 0 {
        return Err(crate::hosts::stale(
            &s,
            credentials::Entity::find()
                .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
                .filter(credentials::Column::Id.eq(id)),
        )
        .await?);
    }
    crate::hosts::activity(
        &s,
        &scope,
        "update",
        "credentials",
        &id,
        json!({"name": name, "secret_rotated": rotated.is_some()}),
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn remove(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let id = id.parse::<Id>().map_err(|_| ApiError::missing())?;
    scope.require("credentials.delete")?;
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    // Refuse rather than leave anything pointing at a credential that is gone:
    // the failure would surface later as an unexplained connection error, far
    // from the deletion that caused it. Every table that can reference a
    // credential has to be listed here — a new one that is forgotten silently
    // reintroduces the dangling reference.
    let used = hosts::Entity::find()
        .filter(hosts::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(hosts::Column::CredentialId.eq(id))
        .count(&tx)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "还有 {used} 台主机在用这条凭据，请先改掉再删除。"
        )));
    }
    let used = repositories::Entity::find()
        .filter(repositories::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(repositories::Column::CredentialId.eq(id))
        .count(&tx)
        .await?;
    if used > 0 {
        return Err(ApiError::conflict(format!(
            "还有 {used} 个仓库在用这条凭据，请先改掉再删除。"
        )));
    }
    let affected = credentials::Entity::delete_many()
        .filter(credentials::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(credentials::Column::Id.eq(id))
        .exec(&tx)
        .await?
        .rows_affected;
    if affected == 0 {
        return Err(ApiError::missing());
    }
    crate::entities::repository_connections::Entity::delete_many()
        .filter(crate::entities::repository_connections::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(crate::entities::repository_connections::Column::CredentialId.eq(id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "credentials", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Decrypts a credential for the server's own use. There is no HTTP route to
/// this; the caller is a channel that is about to connect somewhere.
pub async fn unseal(s: &AppState, workspace_id: &Id, id: &Id) -> Result<(String, String, String)> {
    crate::repository_auth::ensure_fresh(s, *workspace_id, *id).await?;
    let row = credentials::Entity::find()
        .filter(credentials::Column::WorkspaceId.eq(*workspace_id))
        .filter(credentials::Column::Id.eq(*id))
        .one(&s.db)
        .await?
        .ok_or_else(|| ApiError::bad("这台主机引用的凭据已不存在，请重新选择。"))?;
    let secret = s.vault.open_secret(
        &row.secret,
        &vault::aad(&workspace_id.to_string(), "credentials", &id.to_string()),
    )?;
    Ok((row.kind, row.username, secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn summary_does_not_fetch_the_sealed_secret() {
        struct Projection;
        impl FromQueryResult for Projection {
            fn from_query_result(
                row: &sea_orm::QueryResult,
                prefix: &str,
            ) -> std::result::Result<Self, sea_orm::DbErr> {
                assert!(row.try_get::<String>(prefix, "secret").is_err());
                assert!(row.try_get::<bool>(prefix, "has_secret")?);
                Ok(Self)
            }
        }
        let (_dir, db) = crate::lock::tests::database().await;
        credentials::Entity::insert(credentials::ActiveModel {
            id: Set(1),
            workspace_id: Set(1),
            name: Set("credential".into()),
            kind: Set("password".into()),
            username: Set(String::new()),
            description: Set(String::new()),
            secret: Set("sealed-value-must-stay-in-database".into()),
            created_at: Set(1),
            updated_at: Set(1),
            version: Set(1),
        })
        .exec(&db)
        .await
        .unwrap();
        assert!(
            summary_query()
                .into_model::<Projection>()
                .one(&db)
                .await
                .unwrap()
                .is_some()
        );
    }
}
