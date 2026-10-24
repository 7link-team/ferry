//! Accounts and sessions. Global, not workspace-scoped: a user exists before
//! any workspace does, and may belong to several.

use crate::snowflake::Id;

use crate::{
    db,
    entities::{sessions, users, workspace_members, workspace_roles, workspaces},
    error::{ApiError, Result},
    state::AppState,
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, header, request::Parts},
};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub const COOKIE: &str = "ferry_session";
/// Idle timeout. Every authenticated request pushes it out.
const SESSION_MS: i64 = 14 * 24 * 60 * 60 * 1000;

#[derive(Clone)]
pub struct User {
    pub id: Id,
    pub email: String,
    pub name: String,
}

/// An authenticated request with no workspace in scope yet — used by the
/// endpoints that choose or create one. Workspace resources take `Scope`.
pub struct Account {
    pub user: User,
    pub session_id: Id,
}

impl FromRequestParts<AppState> for Account {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        // The cookie is a random secret, never the row's primary key: a
        // snowflake carries its own creation time, so using it here would let
        // anyone who knows roughly when a user signed in enumerate the few
        // thousand candidates. Only its hash is stored, so a database leak
        // does not hand over live sessions either.
        let token = cookie(&parts.headers, COOKIE).ok_or_else(ApiError::unauthorized)?;
        let hash = crate::runner::fingerprint(&token);
        #[derive(FromQueryResult)]
        struct SessionUser {
            session_id: i64,
            id: i64,
            email: String,
            name: String,
            active: i64,
            expires_at: i64,
        }
        let row = sessions::Entity::find()
            .select_only()
            .column_as(sessions::Column::Id, "session_id")
            .columns([sessions::Column::ExpiresAt])
            .columns([
                users::Column::Id,
                users::Column::Email,
                users::Column::Name,
                users::Column::Active,
            ])
            .join(
                JoinType::InnerJoin,
                sessions::Entity::belongs_to(users::Entity)
                    .from(sessions::Column::UserId)
                    .to(users::Column::Id)
                    .into(),
            )
            .filter(sessions::Column::TokenHash.eq(hash.clone()))
            .into_model::<SessionUser>()
            .one(&s.db)
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        if row.expires_at < db::now() || row.active == 0 {
            sessions::Entity::delete_many()
                .filter(sessions::Column::TokenHash.eq(hash))
                .exec(&s.db)
                .await?;
            return Err(ApiError::unauthorized());
        }
        let session_id = Id::from(row.session_id);
        sessions::Entity::update_many()
            .col_expr(sessions::Column::SeenAt, Expr::value(db::now()))
            .col_expr(
                sessions::Column::ExpiresAt,
                Expr::value(db::now() + SESSION_MS),
            )
            .filter(sessions::Column::Id.eq(i64::from(session_id)))
            .exec(&s.db)
            .await?;
        Ok(Self {
            user: User {
                id: Id::from(row.id),
                email: row.email,
                name: row.name,
            },
            session_id,
        })
    }
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

fn set_cookie(s: &AppState, value: &str, max_age: i64) -> [(header::HeaderName, String); 1] {
    let secure = if s.secure_cookie { "; Secure" } else { "" };
    [(
        header::SET_COOKIE,
        format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax{secure}; Max-Age={max_age}"),
    )]
}

#[derive(Deserialize)]
pub struct Credentials {
    email: String,
    password: String,
    #[serde(default)]
    name: String,
}

/// Argon2 with per-password salt. The cost parameters are the crate defaults,
/// which are the OWASP-recommended ones.
fn hash(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|_| ApiError::bad("无法保存密码，请重试。"))
}

fn verify(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

fn check(credentials: &Credentials, require_name: bool) -> Result<String> {
    let email = credentials.email.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(ApiError::bad("请填写有效的邮箱地址。"));
    }
    // Length, not a composition rule: composition rules push people toward
    // "Password1!" and are worse than a longer passphrase.
    if credentials.password.chars().count() < 10 {
        return Err(ApiError::bad("密码至少 10 个字符。"));
    }
    if credentials.password.len() > 1024 {
        return Err(ApiError::bad("密码过长。"));
    }
    if require_name && credentials.name.trim().is_empty() {
        return Err(ApiError::bad("请填写姓名。"));
    }
    Ok(email)
}

pub async fn register(
    State(s): State<AppState>,
    Json(body): Json<Credentials>,
) -> Result<impl axum::response::IntoResponse> {
    let email = check(&body, true)?;
    if users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&s.db)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("该邮箱已注册，请直接登录。"));
    }
    let (id, now) = (s.next_id()?, db::now());
    users::Entity::insert(users::ActiveModel {
        id: Set(i64::from(id)),
        email: Set(email.clone()),
        name: Set(body.name.trim().to_owned()),
        password_hash: Set(hash(&body.password)?),
        active: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&s.db)
    .await?;
    let session = start_session(&s, &id).await?;
    Ok((
        set_cookie(&s, &session, SESSION_MS / 1000),
        Json(json!({"id": id, "email": email})),
    ))
}

pub async fn login(
    State(s): State<AppState>,
    Json(body): Json<Credentials>,
) -> Result<impl axum::response::IntoResponse> {
    let email = body.email.trim().to_lowercase();
    let row = users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&s.db)
        .await?;
    // One message and one code path for "no such user" and "wrong password",
    // so the response does not say which emails are registered.
    let refused = || {
        ApiError(
            axum::http::StatusCode::UNAUTHORIZED,
            "邮箱或密码不正确。".into(),
        )
    };
    let row = row.ok_or_else(refused)?;
    if row.active == 0 || !verify(&body.password, &row.password_hash) {
        return Err(refused());
    }
    let id = Id::from(row.id);
    let session = start_session(&s, &id).await?;
    Ok((
        set_cookie(&s, &session, SESSION_MS / 1000),
        Json(json!({"id": id, "email": email})),
    ))
}

/// Returns the cookie value. The row keeps only a hash of it, so the secret
/// exists in exactly one place: the browser that was handed it.
async fn start_session(s: &AppState, user_id: &Id) -> Result<String> {
    let (id, now) = (s.next_id()?, db::now());
    let token = crate::runner::secret();
    sessions::Entity::insert(sessions::ActiveModel {
        id: Set(i64::from(id)),
        token_hash: Set(crate::runner::fingerprint(&token)),
        user_id: Set(i64::from(*user_id)),
        workspace_id: Set(0),
        created_at: Set(now),
        seen_at: Set(now),
        expires_at: Set(now + SESSION_MS),
    })
    .exec(&s.db)
    .await?;
    Ok(token)
}

pub async fn logout(
    State(s): State<AppState>,
    account: Account,
) -> Result<impl axum::response::IntoResponse> {
    sessions::Entity::delete_by_id(i64::from(account.session_id))
        .exec(&s.db)
        .await?;
    Ok((set_cookie(&s, "", 0), Json(json!({"ok": true}))))
}

/// Who am I, and which workspaces can I open. The shell calls this on load.
pub async fn me(State(s): State<AppState>, account: Account) -> Result<Json<Value>> {
    #[derive(FromQueryResult)]
    struct WorkspaceRole {
        id: i64,
        slug: String,
        name: String,
        role_name: String,
        permissions: String,
    }
    let rows = workspace_members::Entity::find()
        .select_only()
        .columns([
            workspaces::Column::Id,
            workspaces::Column::Slug,
            workspaces::Column::Name,
        ])
        .column_as(workspace_roles::Column::Name, "role_name")
        .column(workspace_roles::Column::Permissions)
        .join(
            JoinType::InnerJoin,
            workspace_members::Entity::belongs_to(workspaces::Entity)
                .from(workspace_members::Column::WorkspaceId)
                .to(workspaces::Column::Id)
                .into(),
        )
        .join(
            JoinType::InnerJoin,
            workspace_members::Entity::belongs_to(workspace_roles::Entity)
                .from(workspace_members::Column::RoleId)
                .to(workspace_roles::Column::Id)
                .into(),
        )
        .filter(workspace_members::Column::UserId.eq(i64::from(account.user.id)))
        .filter(workspace_members::Column::State.eq("active"))
        .order_by_asc(workspaces::Column::Name)
        .into_model::<WorkspaceRole>()
        .all(&s.db)
        .await?;
    let workspaces: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": Id::from(row.id),
                "slug": row.slug,
                "name": row.name,
                "role_name": row.role_name,
                "permissions": serde_json::from_str::<Value>(&row.permissions).unwrap_or(json!([])),
            })
        })
        .collect();
    Ok(Json(json!({
        "user": {"id": account.user.id, "email": account.user.email, "name": account.user.name},
        "workspaces": workspaces,
    })))
}
