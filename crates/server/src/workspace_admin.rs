//! Workspace membership and role administration, using the tenant list/form contract.
use crate::{
    auth::User,
    db,
    entities::{users, workspace_members as members, workspace_roles as roles, workspaces},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
    workspace::{self, Scope},
};
use axum::{
    Json,
    extract::{Path, Query, State},
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
async fn authorize(conn: &impl ConnectionTrait, scope: &Scope, permission: &str) -> Result<Scope> {
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
        return Err(ApiError::bad("不能授予或管理超出自身权限的角色。"));
    }
    Ok(())
}
fn role_permissions(role: &roles::Model) -> Result<Vec<String>> {
    serde_json::from_str(&role.permissions).map_err(|_| ApiError::bad("角色权限数据无效。"))
}
fn role_json(row: roles::Model) -> Value {
    json!({"id": Id::from(row.id), "name": row.name, "builtin": row.builtin != 0,
        "permissions": serde_json::from_str::<Value>(&row.permissions).unwrap_or(json!([])), "version": row.version, "created_at": row.created_at})
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
        return Err(ApiError::bad("角色名称需为1–60个字符。"));
    }
    if body
        .permissions
        .iter()
        .any(|p| !workspace::PERMISSIONS.iter().any(|known| known.0 == p))
    {
        return Err(ApiError::bad("存在未知权限，请刷新权限目录。"));
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
        return Err(ApiError::conflict("该角色名称已存在。"));
    }
    Ok(())
}
pub async fn create_role(
    State(s): State<AppState>,
    scope: Scope,
    Json(body): Json<RoleBody>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock_workspace(&tx, scope.workspace_id).await?;
    let scope = authorize(&tx, &scope, "roles.create").await?;
    let (name, permissions) = validate_role(&scope, &body)?;
    unique_name(&tx, scope.workspace_id, &name, None).await?;
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
    .exec(&tx)
    .await?;
    crate::hosts::activity_in(
        &s,
        &tx,
        &scope,
        "create",
        "roles",
        &id,
        json!({"name":name,"permissions":permissions}),
    )
    .await?;
    tx.commit().await?;
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
        return Err(ApiError::bad("内置角色不可修改，请先复制为自定义角色。"));
    }
    if body.version != Some(old.version) {
        return Err(ApiError::conflict("角色已被修改，请刷新后重试。"));
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
        return Err(ApiError::bad("内置角色不可删除。"));
    }
    grant(&scope, &role_permissions(&old)?)?;
    if members::Entity::find()
        .filter(members::Column::WorkspaceId.eq(scope.workspace_id))
        .filter(members::Column::RoleId.eq(id))
        .count(&tx)
        .await?
        > 0
    {
        return Err(ApiError::conflict("仍有成员使用该角色，请先调整成员角色。"));
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
    owner_id: i64,
    state: String,
    version: i64,
    created_at: i64,
}
impl Member {
    fn json(self) -> Value {
        json!({"id":Id::from(self.id),"user_id":Id::from(self.user_id),"role_id":Id::from(self.role_id),"name":self.name,"email":self.email,"role_name":self.role_name,"active":self.active!=0,"owner":self.user_id==self.owner_id,"state":self.state,"version":self.version,"created_at":self.created_at})
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
        return Err(ApiError::bad("不能修改或移除工作空间拥有者。"));
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
        return Err(ApiError::conflict("成员已被修改，请刷新后重试。"));
    }
    let target = role(&tx, scope.workspace_id, body.role_id).await?;
    if target.builtin != 0 && target.name == "拥有者" {
        return Err(ApiError::bad("拥有者身份不能通过成员角色编辑授予。"));
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
