//! Global administration. Admin extraction is separate from tenant Scope;
//! every privileged mutation rechecks the actor under the singleton lock.
use crate::{
    auth::{self, Account},
    db,
    entities::{platform_state, sessions, users, workspaces},
    error::{ApiError, Result},
    resource::{self, ListQuery, Spec},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Json,
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts},
};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, io::Write};

pub struct Admin(pub Account);
impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        let account = Account::from_request_parts(parts, s).await?;
        if !account.user.is_superadmin {
            return Err(ApiError::forbidden());
        }
        Ok(Self(account))
    }
}

pub async fn lock(conn: &impl ConnectionTrait) -> Result<platform_state::Model> {
    // A real write serializes across SQLite, MySQL and PostgreSQL. Do not
    // inspect affected rows: MySQL may report zero for a no-op update.
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::Initialized,
            Expr::col(platform_state::Column::Initialized).into(),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(conn)
        .await?;
    platform_state::Entity::find_by_id(1)
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)
}
async fn check_admin(conn: &impl ConnectionTrait, admin: &Admin) -> Result<()> {
    let allowed = users::Entity::find_by_id(i64::from(admin.0.user.id))
        .filter(users::Column::Active.eq(1))
        .filter(users::Column::IsSuperadmin.eq(1))
        .count(conn)
        .await?;
    if allowed == 0 {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// Generate only while setup is pending. No token or password enters logs.
pub async fn prepare(s: &AppState) -> Result<()> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let path = s.data_dir.join("setup-token");
    if row.initialized != 0 {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(crate::runner::secret().as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    tracing::info!(path = %path.display(), "platform initialization required; operator token file");
    Ok(())
}

pub async fn status(State(s): State<AppState>) -> Result<impl axum::response::IntoResponse> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "required": row.initialized == 0,
            "has_users": users::Entity::find().count(&s.db).await? > 0,
            "registration_enabled": row.initialized != 0 && row.registration_enabled != 0,
        })),
    ))
}
#[derive(Deserialize)]
pub struct Setup {
    token: String,
    #[serde(flatten)]
    credentials: auth::Credentials,
}
pub async fn initialize(
    State(s): State<AppState>,
    Json(body): Json<Setup>,
) -> Result<impl axum::response::IntoResponse> {
    let email = auth::check(&body.credentials, false)?;
    let tx = s.db.begin().await?;
    let state = lock(&tx).await?;
    if state.initialized != 0 {
        return Err(ApiError::conflict("平台已初始化，请登录。"));
    }
    let token = std::fs::read_to_string(s.data_dir.join("setup-token"))?;
    // Compare fixed-length digests without data-dependent early exits.
    let expected = crate::runner::fingerprint(token.trim());
    let supplied = crate::runner::fingerprint(body.token.trim());
    if token.trim().is_empty()
        || expected
            .bytes()
            .zip(supplied.bytes())
            .fold(0u8, |n, (a, b)| n | (a ^ b))
            != 0
    {
        return Err(ApiError::forbidden());
    }
    let existing = users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&tx)
        .await?;
    let (id, name) = if let Some(row) = existing {
        if row.active == 0 || !auth::verify(&body.credentials.password, &row.password_hash) {
            return Err(ApiError::bad("已有账号的邮箱或密码不正确，或账号已停用。"));
        }
        users::Entity::update_many()
            .col_expr(users::Column::IsSuperadmin, Expr::value(1))
            .col_expr(
                users::Column::Version,
                Expr::col(users::Column::Version).add(1),
            )
            .col_expr(users::Column::UpdatedAt, Expr::value(db::now()))
            .filter(users::Column::Id.eq(row.id))
            .exec(&tx)
            .await?;
        sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(row.id))
            .exec(&tx)
            .await?;
        (Id::from(row.id), row.name)
    } else {
        auth::check(&body.credentials, true)?;
        let (id, now) = (s.next_id()?, db::now());
        let name = body.credentials.name.trim().to_owned();
        users::Entity::insert(users::ActiveModel {
            id: Set(i64::from(id)),
            email: Set(email.clone()),
            name: Set(name.clone()),
            password_hash: Set(auth::hash(&body.credentials.password)?),
            active: Set(1),
            is_superadmin: Set(1),
            version: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .exec(&tx)
        .await?;
        (id, name)
    };
    platform_state::Entity::update_many()
        .col_expr(platform_state::Column::Initialized, Expr::value(1))
        .filter(platform_state::Column::Id.eq(1))
        .exec(&tx)
        .await?;
    let session = auth::start_session(&s, &id, &tx).await?;
    let actor = auth::User {
        id,
        email: email.clone(),
        name,
        is_superadmin: true,
    };
    audit(&s, &tx, &actor, "initialize", "platform", id, json!({})).await?;
    tx.commit().await?;
    let _ = std::fs::remove_file(s.data_dir.join("setup-token"));
    Ok((
        auth::set_cookie(&s, &session, auth::SESSION_MS / 1000),
        Json(json!({"id": id, "email": email})),
    ))
}

async fn audit(
    s: &AppState,
    conn: &impl ConnectionTrait,
    actor: &auth::User,
    action: &str,
    resource: &str,
    id: Id,
    details: Value,
) -> Result<()> {
    crate::hosts::activity_in(
        s,
        conn,
        &crate::workspace::Scope {
            workspace_id: Id::from(0),
            user: actor.clone(),
            permissions: Default::default(),
        },
        action,
        resource,
        &id,
        details,
    )
    .await
}
const USER_SPEC: Spec = Spec {
    search: &["name", "email"],
    filters: &[],
    sortable: &["name", "email", "created_at"],
    default_sort: "-created_at",
};
#[derive(FromQueryResult)]
struct PublicUser {
    id: i64,
    email: String,
    name: String,
    active: i64,
    is_superadmin: i64,
    version: i64,
    created_at: i64,
}
impl PublicUser {
    fn json(self) -> Value {
        json!({"id": Id::from(self.id), "email": self.email, "name": self.name, "active": self.active != 0, "is_superadmin": self.is_superadmin != 0, "version": self.version, "created_at": self.created_at})
    }
}
fn public_users() -> sea_orm::Select<users::Entity> {
    users::Entity::find().select_only().columns([
        users::Column::Id,
        users::Column::Email,
        users::Column::Name,
        users::Column::Active,
        users::Column::IsSuperadmin,
        users::Column::Version,
        users::Column::CreatedAt,
    ])
}
pub async fn list_users(
    State(s): State<AppState>,
    admin: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let q = ListQuery::parse(&USER_SPEC, &raw)?;
    let (rows, total) = resource::global_list::<users::Entity, PublicUser>(
        &s.db,
        &admin,
        &USER_SPEC,
        public_users(),
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items": rows.into_iter().map(PublicUser::json).collect::<Vec<_>>(), "total": total, "page": q.page, "page_size": q.page_size}),
    ))
}
pub async fn get_user(
    State(s): State<AppState>,
    _: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    let row = public_users()
        .filter(users::Column::Id.eq(i64::from(id)))
        .into_model::<PublicUser>()
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(row.json()))
}
#[derive(Deserialize)]
pub struct UserUpdate {
    version: i64,
    active: bool,
    is_superadmin: bool,
}
pub async fn update_user(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
    Json(body): Json<UserUpdate>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let old = public_users()
        .filter(users::Column::Id.eq(i64::from(id)))
        .into_model::<PublicUser>()
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    if old.version != body.version {
        return Err(ApiError::conflict("记录已被修改，请刷新后重试。"));
    }
    if old.active != 0
        && old.is_superadmin != 0
        && !(body.active && body.is_superadmin)
        && users::Entity::find()
            .filter(users::Column::Active.eq(1))
            .filter(users::Column::IsSuperadmin.eq(1))
            .count(&tx)
            .await?
            <= 1
    {
        return Err(ApiError::conflict("必须保留至少一个启用的平台超级管理员。"));
    }
    users::Entity::update_many()
        .col_expr(users::Column::Active, Expr::value(i64::from(body.active)))
        .col_expr(
            users::Column::IsSuperadmin,
            Expr::value(i64::from(body.is_superadmin)),
        )
        .col_expr(users::Column::Version, Expr::value(old.version + 1))
        .col_expr(users::Column::UpdatedAt, Expr::value(db::now()))
        .filter(users::Column::Id.eq(i64::from(id)))
        .exec(&tx)
        .await?;
    if old.active != i64::from(body.active) || old.is_superadmin != i64::from(body.is_superadmin) {
        sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(i64::from(id)))
            .exec(&tx)
            .await?;
    }
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-users",
        id,
        json!({"active": body.active, "is_superadmin": body.is_superadmin}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": id, "version": old.version + 1})))
}
const WORKSPACE_SPEC: Spec = Spec {
    search: &["name", "slug"],
    filters: &[],
    sortable: &["name", "slug", "created_at"],
    default_sort: "-created_at",
};
#[derive(FromQueryResult)]
struct PublicWorkspace {
    id: i64,
    name: String,
    slug: String,
    updated_at: i64,
    created_at: i64,
}
impl PublicWorkspace {
    fn json(self) -> Value {
        json!({"id": Id::from(self.id), "name": self.name, "slug": self.slug, "version": self.updated_at, "created_at": self.created_at})
    }
}
fn public_workspaces() -> sea_orm::Select<workspaces::Entity> {
    workspaces::Entity::find().select_only().columns([
        workspaces::Column::Id,
        workspaces::Column::Name,
        workspaces::Column::Slug,
        workspaces::Column::UpdatedAt,
        workspaces::Column::CreatedAt,
    ])
}
pub async fn list_workspaces(
    State(s): State<AppState>,
    admin: Admin,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<Value>> {
    let q = ListQuery::parse(&WORKSPACE_SPEC, &raw)?;
    let (rows, total) = resource::global_list::<workspaces::Entity, PublicWorkspace>(
        &s.db,
        &admin,
        &WORKSPACE_SPEC,
        public_workspaces(),
        &q,
    )
    .await?;
    Ok(Json(
        json!({"items": rows.into_iter().map(PublicWorkspace::json).collect::<Vec<_>>(), "total": total, "page": q.page, "page_size": q.page_size}),
    ))
}
pub async fn get_workspace(
    State(s): State<AppState>,
    _: Admin,
    Path(id): Path<Id>,
) -> Result<Json<Value>> {
    Ok(Json(
        public_workspaces()
            .filter(workspaces::Column::Id.eq(i64::from(id)))
            .into_model::<PublicWorkspace>()
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::missing)?
            .json(),
    ))
}
pub async fn create_workspace(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<crate::workspace::NewWorkspace>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let value = crate::workspace::create_in(&s, admin.0.user.id, body, &tx).await?;
    let id: Id = serde_json::from_value(value["id"].clone())
        .map_err(|_| ApiError::bad("工作空间标识无效。"))?;
    audit(
        &s,
        &tx,
        &admin.0.user,
        "create",
        "platform-workspaces",
        id,
        json!({"name": value["name"], "slug": value["slug"]}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[derive(Deserialize)]
pub struct WorkspaceUpdate {
    name: String,
    version: i64,
}
pub async fn update_workspace(
    State(s): State<AppState>,
    admin: Admin,
    Path(id): Path<Id>,
    Json(body): Json<WorkspaceUpdate>,
) -> Result<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("请填写工作空间名称（60 字以内）。"));
    }
    let tx = s.db.begin().await?;
    lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    let version = std::cmp::max(db::now(), body.version.saturating_add(1));
    let changed = workspaces::Entity::update_many()
        .col_expr(workspaces::Column::Name, Expr::value(name))
        .col_expr(workspaces::Column::UpdatedAt, Expr::value(version))
        .filter(workspaces::Column::Id.eq(i64::from(id)))
        .filter(workspaces::Column::UpdatedAt.eq(body.version))
        .exec(&tx)
        .await?;
    if changed.rows_affected != 1 {
        return Err(ApiError::conflict("记录已被修改或删除，请刷新后重试。"));
    }
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-workspaces",
        id,
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": id, "version": version})))
}
pub async fn get_settings(State(s): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(Json(
        json!({"id": "1", "registration_enabled": row.registration_enabled != 0, "version": row.version}),
    ))
}
#[derive(Deserialize)]
pub struct SettingsUpdate {
    version: i64,
    registration_enabled: bool,
}
pub async fn update_settings(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<SettingsUpdate>,
) -> Result<Json<Value>> {
    let tx = s.db.begin().await?;
    let row = lock(&tx).await?;
    check_admin(&tx, &admin).await?;
    if row.version != body.version {
        return Err(ApiError::conflict("设置已被修改，请刷新后重试。"));
    }
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::RegistrationEnabled,
            Expr::value(i64::from(body.registration_enabled)),
        )
        .col_expr(
            platform_state::Column::Version,
            Expr::value(row.version + 1),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&tx)
        .await?;
    audit(
        &s,
        &tx,
        &admin.0.user,
        "update",
        "platform-settings",
        Id::from(1),
        json!({"registration_enabled": body.registration_enabled}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": "1", "version": row.version + 1})))
}
