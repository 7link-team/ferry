//! Workspace membership and role administration, using the tenant list/form contract.
use crate::error::{Json, Path, Query};
use crate::{
    accounts::auth::User,
    accounts::workspace::{self, Scope},
    db,
    entities::{users, workspace_members as members, workspace_roles as roles, workspaces},
    error::{ApiError, Result},
    resources::resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    extract::State,
    http::{StatusCode, header},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, JoinType, PaginatorTrait,
    QueryFilter, QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, ExprTrait, Func},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// The existing workspace row is the portable lock for membership, roles and invitations.
pub(crate) async fn lock_workspace(
    conn: &impl ConnectionTrait,
    id: Id,
) -> Result<workspaces::Model> {
    workspaces::Entity::update_many()
        .col_expr(
            workspaces::Column::PipelineRunNumber,
            Expr::col(workspaces::Column::PipelineRunNumber),
        )
        .filter(workspaces::Column::Id.eq(id))
        .exec(conn)
        .await?;
    workspaces::Entity::find_by_id(i64::from(id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
pub(crate) async fn authorize(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    permission: &str,
) -> Result<Scope> {
    // Re-read after acquiring the lock: a queued request must not use a role
    // or platform privilege that another administrator has just revoked.
    let (active, admin) = users::Entity::find_by_id(i64::from(scope.user.id))
        .select_only()
        .columns([users::Column::Active, users::Column::IsSuperadmin])
        .into_tuple::<(i64, i64)>()
        .one(conn)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if active == 0 {
        return Err(ApiError::unauthorized());
    }
    let permissions: HashSet<String> = if admin != 0 {
        workspace::PERMISSIONS
            .iter()
            .map(|p| p.0.to_owned())
            .collect()
    } else {
        let text = members::Entity::find()
            .select_only()
            .column(roles::Column::Permissions)
            .join(
                JoinType::InnerJoin,
                members::Entity::belongs_to(roles::Entity)
                    .from(members::Column::RoleId)
                    .to(roles::Column::Id)
                    .into(),
            )
            .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(roles::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(members::Column::UserId.eq(scope.user.id))
            .filter(members::Column::State.eq("active"))
            .into_tuple::<String>()
            .one(conn)
            .await?
            .ok_or_else(ApiError::forbidden)?;
        serde_json::from_str(&text).map_err(|_| ApiError::forbidden())?
    };
    let fresh = Scope {
        workspace_id: scope.workspace_id,
        user: User {
            is_superadmin: admin != 0,
            ..scope.user.clone()
        },
        permissions,
    };
    fresh.require(permission)?;
    Ok(fresh)
}
fn grant(scope: &Scope, permissions: &[String]) -> Result<()> {
    if permissions.iter().any(|p| !scope.permissions.contains(p)) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.grant_exceeded"),
        ));
    }
    Ok(())
}
fn role_permissions(role: &roles::Model) -> Result<Vec<String>> {
    serde_json::from_str(&role.permissions).map_err(|_| {
        ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.role_permissions_invalid"),
        )
    })
}
fn role_json(row: roles::Model) -> Value {
    json!({"id": Id::from(row.id), "name": workspace::role_name(&row.name, row.builtin != 0), "builtin": row.builtin != 0,
        "permissions": serde_json::from_str::<Vec<String>>(&row.permissions).unwrap_or_default().into_iter().filter(|p| !p.starts_with("code-reviews.")).collect::<Vec<_>>(), "version": row.version, "created_at": row.created_at})
}
async fn role(conn: &impl ConnectionTrait, workspace: Id, id: Id) -> Result<roles::Model> {
    roles::Entity::find_by_id(i64::from(id))
        .filter(roles::Column::WorkspaceId.eq(workspace))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
const ROLE_SPEC: Spec = Spec {
    search: &["name"],
    filters: &[],
    sortable: &["name", "created_at"],
    default_sort: "name",
};
pub async fn roles(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("roles.read")?;
    let q = ListQuery::parse(&ROLE_SPEC, &raw)?;
    let (rows, total) = resource::list::<roles::Entity, roles::Model>(
        &s.db,
        &ROLE_SPEC,
        roles::Entity::find(),
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items": rows.into_iter().map(role_json).collect::<Vec<_>>(), "total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_role(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("roles.read")?;
    Ok(Json(role_json(role(&s.db, scope.workspace_id, id).await?)))
}
#[derive(Deserialize)]
pub struct RoleBody {
    name: String,
    permissions: Vec<String>,
    version: Option<i64>,
}
fn validate_role(scope: &Scope, body: &RoleBody) -> Result<(String, Vec<String>)> {
    let name = body.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.role_name_invalid"),
        ));
    }
    if body
        .permissions
        .iter()
        .any(|p| !workspace::PERMISSIONS.iter().any(|known| known.0 == p))
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.permission_unknown"),
        ));
    }
    let permissions = workspace::expand(&body.permissions);
    grant(scope, &permissions)?;
    Ok((name, permissions))
}
async fn unique_name(
    conn: &impl ConnectionTrait,
    workspace: Id,
    name: &str,
    except: Option<Id>,
) -> Result<()> {
    let mut query = roles::Entity::find()
        .filter(roles::Column::WorkspaceId.eq(workspace))
        .filter(roles::Column::Name.eq(name));
    if let Some(id) = except {
        query = query.filter(roles::Column::Id.ne(id));
    }
    if query.count(conn).await? != 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.role_name_used"),
        ));
    }
    Ok(())
}
pub async fn create_role(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<RoleBody>,
) -> Result<Json<Value>> {
    let tx = crate::variables::transaction(&s, scope.workspace_id).await?;
    let result = create_role_in(&s, &tx, &scope, body).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn create_role_in(
    s: &AppState,
    tx: &sea_orm::DatabaseTransaction,
    scope: &Scope,
    body: RoleBody,
) -> Result<Json<Value>> {
    lock_workspace(tx, scope.workspace_id).await?;
    let scope = authorize(tx, &scope, "roles.create").await?;
    let (name, permissions) = validate_role(&scope, &body)?;
    unique_name(tx, scope.workspace_id, &name, None).await?;
    let (id, now) = (s.next_id()?, db::now());
    roles::Entity::insert(roles::ActiveModel {
        id: Set(id.into()),
        workspace_id: Set(scope.workspace_id.into()),
        name: Set(name.clone()),
        permissions: Set(json!(permissions).to_string()),
        builtin: Set(0),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        tx,
        &scope,
        "create",
        "roles",
        &id,
        json!({"name":name,"permissions":permissions}),
    )
    .await?;
    Ok(Json(json!({"id":id,"version":1})))
}
pub async fn update_role(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<RoleBody>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "roles.update").await?;
    let old = role(&tx, scope.workspace_id, id).await?;
    if old.builtin != 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.builtin_immutable"),
        ));
    }
    if body.version != Some(old.version) {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.role_changed"),
        ));
    }
    grant(&scope, &role_permissions(&old)?)?;
    let (name, permissions) = validate_role(&scope, &body)?;
    unique_name(&tx, scope.workspace_id, &name, Some(id)).await?;
    roles::Entity::update_many()
        .col_expr(roles::Column::Name, Expr::value(name.clone()))
        .col_expr(
            roles::Column::Permissions,
            Expr::value(json!(permissions).to_string()),
        )
        .col_expr(roles::Column::UpdatedAt, Expr::value(db::now()))
        .col_expr(roles::Column::Version, Expr::value(old.version + 1))
        .filter(roles::Column::Id.eq(id))
        .filter(roles::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "roles",
        &id,
        json!({"name":name,"permissions":permissions}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":old.version+1})))
}
pub async fn delete_role(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    let tx = s.db.begin().await?;
    lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "roles.delete").await?;
    let old = role(&tx, scope.workspace_id, id).await?;
    if old.builtin != 0 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.builtin_delete"),
        ));
    }
    grant(&scope, &role_permissions(&old)?)?;
    if members::Entity::find()
        .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(members::Column::RoleId.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.role_in_use"),
        ));
    }
    roles::Entity::delete_many()
        .filter(roles::Column::Id.eq(id))
        .filter(roles::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "delete",
        "roles",
        &id,
        json!({"name":old.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(FromQueryResult)]
struct Member {
    id: i64,
    user_id: i64,
    role_id: i64,
    name: String,
    email: String,
    active: i64,
    role_name: String,
    role_builtin: i64,
    owner_id: i64,
    state: String,
    version: i64,
    created_at: i64,
}
impl Member {
    fn json(self) -> Value {
        json!({"id":Id::from(self.id),"user_id":Id::from(self.user_id),"role_id":Id::from(self.role_id),"name":self.name,"email":self.email,"role_name":workspace::role_name(&self.role_name, self.role_builtin != 0),"active":self.active!=0,"owner":self.user_id==self.owner_id,"state":self.state,"version":self.version,"created_at":self.created_at})
    }
}
fn member_select() -> sea_orm::Select<members::Entity> {
    members::Entity::find()
        .select_only()
        .columns([
            members::Column::Id,
            members::Column::UserId,
            members::Column::RoleId,
            members::Column::State,
            members::Column::Version,
            members::Column::CreatedAt,
        ])
        .columns([
            users::Column::Name,
            users::Column::Email,
            users::Column::Active,
        ])
        .column_as(roles::Column::Name, "role_name")
        .column_as(roles::Column::Builtin, "role_builtin")
        .column(workspaces::Column::OwnerId)
        .join(
            JoinType::InnerJoin,
            members::Entity::belongs_to(users::Entity)
                .from(members::Column::UserId)
                .to(users::Column::Id)
                .into(),
        )
        .join(
            JoinType::InnerJoin,
            members::Entity::belongs_to(roles::Entity)
                .from(members::Column::RoleId)
                .to(roles::Column::Id)
                .into(),
        )
        .join(
            JoinType::InnerJoin,
            members::Entity::belongs_to(workspaces::Entity)
                .from(members::Column::WorkspaceId)
                .to(workspaces::Column::Id)
                .into(),
        )
}
const MEMBER_SPEC: Spec = Spec {
    search: &[],
    filters: &["role_id", "state"],
    sortable: &["created_at"],
    default_sort: "created_at",
};
pub async fn members(
    State(s): State<AppState>,
    scope: Scope,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    scope.require("members.read")?;
    let q = ListQuery::parse(&MEMBER_SPEC, &raw)?;
    let mut select = member_select().filter(roles::Column::WorkspaceId.eq(scope.workspace_id));
    // Search the joined public profile, still using bound values and the
    // shared tenant-scoped pagination/count implementation.
    if let Some(text) = raw.get("q").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let pattern = format!("%{}%", text.to_lowercase());
        select = select.filter(
            Expr::expr(Func::lower(Expr::col((users::Entity, users::Column::Name))))
                .like(pattern.clone())
                .or(Expr::expr(Func::lower(Expr::col((
                    users::Entity,
                    users::Column::Email,
                ))))
                .like(pattern)),
        );
    }
    let (rows, total) = resource::list::<members::Entity, Member>(
        &s.db,
        &MEMBER_SPEC,
        select,
        &scope.workspace_id,
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(Member::json).collect::<Vec<_>>(),"total":total,"page":q.page,"page_size":q.page_size}),
    ))
}
pub async fn get_member(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<Json<Value>> {
    scope.require("members.read")?;
    Ok(Json(
        member_select()
            .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(roles::Column::WorkspaceId.eq(scope.workspace_id))
            .filter(members::Column::Id.eq(id))
            .into_model::<Member>()
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?
            .json(),
    ))
}
async fn mutable_member(
    conn: &impl ConnectionTrait,
    scope: &Scope,
    workspace: &workspaces::Model,
    id: Id,
) -> Result<members::Model> {
    let row = members::Entity::find_by_id(i64::from(id))
        .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    if row.user_id == workspace.owner_id {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.owner_immutable"),
        ));
    }
    grant(
        scope,
        &role_permissions(&role(conn, scope.workspace_id, Id::from(row.role_id)).await?)?,
    )?;
    Ok(row)
}
#[derive(Deserialize)]
pub struct MemberUpdate {
    role_id: Id,
    version: i64,
}
pub async fn update_member(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
    Json(body): Json<MemberUpdate>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    let workspace = lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "members.update").await?;
    let old = mutable_member(&tx, &scope, &workspace, id).await?;
    if old.version != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("workspace.error.member_changed"),
        ));
    }
    let target = role(&tx, scope.workspace_id, body.role_id).await?;
    if target.builtin != 0 && target.name == "拥有者" {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("workspace.error.owner_role_grant"),
        ));
    }
    grant(&scope, &role_permissions(&target)?)?;
    members::Entity::update_many()
        .col_expr(
            members::Column::RoleId,
            Expr::value(i64::from(body.role_id)),
        )
        .col_expr(members::Column::Version, Expr::value(old.version + 1))
        .col_expr(members::Column::UpdatedAt, Expr::value(db::now()))
        .filter(members::Column::Id.eq(id))
        .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "update",
        "members",
        &id,
        json!({"role_id":body.role_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"version":old.version+1})))
}
pub async fn delete_member(
    State(s): State<AppState>,
    scope: Scope,
    Path((_, id)): Path<(String, Id)>,
) -> Result<StatusCode> {
    let tx = s.db.begin().await?;
    let workspace = lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "members.delete").await?;
    mutable_member(&tx, &scope, &workspace, id).await?;
    members::Entity::delete_many()
        .filter(members::Column::Id.eq(id))
        .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(&s, &tx, &scope, "delete", "members", &id, json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn role_options(State(s): State<AppState>, scope: Scope) -> Result<Json<Value>> {
    scope.require("members.update")?;
    let rows = roles::Entity::find()
        .filter(roles::Column::WorkspaceId.eq(scope.workspace_id))
        .all(&s.db)
        .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .filter(|r| !(r.builtin != 0 && r.name == "拥有者"))
        .filter(|r| {
            role_permissions(r)
                .map(|p| grant(&scope, &p).is_ok())
                .unwrap_or(false)
        })
        .map(role_json)
        .collect();
    Ok(Json(json!({"items":items})))
}
pub async fn invitation(
    State(s): State<AppState>,
    scope: Scope,
) -> Result<impl axum::response::IntoResponse> {
    scope.require("members.create")?;
    let row = workspaces::Entity::find_by_id(i64::from(scope.workspace_id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"code":row.join_code})),
    ))
}
pub async fn rotate_invitation(
    State(s): State<AppState>,
    scope: Scope,
) -> Result<impl axum::response::IntoResponse> {
    let tx = s.db.begin().await?;
    lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "members.create").await?;
    let code = workspace::join_code();
    workspaces::Entity::update_many()
        .col_expr(workspaces::Column::JoinCode, Expr::value(code.clone()))
        .filter(workspaces::Column::Id.eq(scope.workspace_id))
        .exec(&tx)
        .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "rotate-invitation",
        "members",
        &Id::from(0),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"code":code})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope {
            workspace_id: Id::from(1),
            user: User {
                id: Id::from(2),
                email: "member@example.test".into(),
                name: "Member".into(),
                is_superadmin: false,
            },
            permissions: ["hosts.read".into()].into_iter().collect(),
        }
    }

    #[test]
    fn role_validation_preserves_constraints_and_defaults_to_english() {
        let scope = scope();
        let error = validate_role(
            &scope,
            &RoleBody {
                name: " ".into(),
                permissions: vec![],
                version: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.1, "The role name must contain 1–60 characters.");
        let error = validate_role(
            &scope,
            &RoleBody {
                name: "Role".into(),
                permissions: vec!["unknown.read".into()],
                version: None,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.1,
            "Unknown permissions were found. Please refresh the permission catalog."
        );
        let error = grant(&scope, &["hosts.delete".into()]).unwrap_err();
        assert_eq!(
            error.1,
            "You cannot grant or manage roles beyond your own permissions."
        );
        let valid = validate_role(
            &scope,
            &RoleBody {
                name: " 用户原文 ".into(),
                permissions: vec!["hosts.read".into()],
                version: None,
            },
        )
        .unwrap();
        assert_eq!(valid, ("用户原文".into(), vec!["hosts.read".into()]));
    }

    #[test]
    fn role_presentation_translates_only_flagged_builtins() {
        let row = roles::Model {
            id: 1,
            workspace_id: 1,
            name: "拥有者".into(),
            permissions: "[]".into(),
            builtin: 1,
            created_at: 0,
            updated_at: 0,
            version: 1,
        };
        assert_eq!(role_json(row.clone())["name"], "Owner");
        let custom = roles::Model { builtin: 0, ..row };
        assert_eq!(role_json(custom)["name"], "拥有者");
    }
}
