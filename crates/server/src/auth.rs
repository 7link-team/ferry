//! Accounts and sessions. Global, not workspace-scoped: a user exists before
//! any workspace does, and may belong to several.

use crate::{
    db::{self, all, execute, one, text},
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
use serde::Deserialize;
use serde_json::{Value, json};

pub const COOKIE: &str = "ferry_session";
/// Idle timeout. Every authenticated request pushes it out.
const SESSION_MS: i64 = 14 * 24 * 60 * 60 * 1000;

#[derive(Clone)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
}

/// An authenticated request with no workspace in scope yet — used by the
/// endpoints that choose or create one. Workspace resources take `Scope`.
pub struct Account {
    pub user: User,
    pub session_id: String,
}

impl FromRequestParts<AppState> for Account {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self> {
        let token = cookie(&parts.headers, COOKIE).ok_or_else(ApiError::unauthorized)?;
        let row = one(
            &s.db,
            "SELECT s.id AS session_id, u.id, u.email, u.name, u.active, s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.id = ?",
            vec![token.clone().into()],
        )
        .await?
        .ok_or_else(ApiError::unauthorized)?;
        if db::int(&row, "expires_at") < db::now() || db::int(&row, "active") == 0 {
            execute(
                &s.db,
                "DELETE FROM sessions WHERE id = ?",
                vec![token.into()],
            )
            .await?;
            return Err(ApiError::unauthorized());
        }
        let session_id = text(&row, "session_id");
        execute(
            &s.db,
            "UPDATE sessions SET seen_at = ?, expires_at = ? WHERE id = ?",
            vec![
                db::now().into(),
                (db::now() + SESSION_MS).into(),
                session_id.clone().into(),
            ],
        )
        .await?;
        Ok(Self {
            user: User {
                id: text(&row, "id"),
                email: text(&row, "email"),
                name: text(&row, "name"),
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
    if one(
        &s.db,
        "SELECT id FROM users WHERE email = ?",
        vec![email.clone().into()],
    )
    .await?
    .is_some()
    {
        return Err(ApiError::conflict("该邮箱已注册，请直接登录。"));
    }
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO users (id,email,name,password_hash,active,created_at,updated_at)
         VALUES (?,?,?,?,1,?,?)",
        vec![
            id.clone().into(),
            email.clone().into(),
            body.name.trim().into(),
            hash(&body.password)?.into(),
            now.into(),
            now.into(),
        ],
    )
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
    let row = one(
        &s.db,
        "SELECT id, password_hash, active FROM users WHERE email = ?",
        vec![email.clone().into()],
    )
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
    if db::int(&row, "active") == 0 || !verify(&body.password, &text(&row, "password_hash")) {
        return Err(refused());
    }
    let id = text(&row, "id");
    let session = start_session(&s, &id).await?;
    Ok((
        set_cookie(&s, &session, SESSION_MS / 1000),
        Json(json!({"id": id, "email": email})),
    ))
}

async fn start_session(s: &AppState, user_id: &str) -> Result<String> {
    let (id, now) = (db::id(), db::now());
    execute(
        &s.db,
        "INSERT INTO sessions (id,user_id,workspace_id,created_at,seen_at,expires_at)
         VALUES (?,?,'',?,?,?)",
        vec![
            id.clone().into(),
            user_id.into(),
            now.into(),
            now.into(),
            (now + SESSION_MS).into(),
        ],
    )
    .await?;
    Ok(id)
}

pub async fn logout(
    State(s): State<AppState>,
    account: Account,
) -> Result<impl axum::response::IntoResponse> {
    execute(
        &s.db,
        "DELETE FROM sessions WHERE id = ?",
        vec![account.session_id.into()],
    )
    .await?;
    Ok((set_cookie(&s, "", 0), Json(json!({"ok": true}))))
}

/// Who am I, and which workspaces can I open. The shell calls this on load.
pub async fn me(State(s): State<AppState>, account: Account) -> Result<Json<Value>> {
    let rows = all(
        &s.db,
        "SELECT w.id, w.slug, w.name, r.name AS role_name, r.permissions
         FROM workspace_members m
         JOIN workspaces w ON w.id = m.workspace_id
         JOIN workspace_roles r ON r.id = m.role_id
         WHERE m.user_id = ? AND m.state = 'active' ORDER BY w.name ASC",
        vec![account.user.id.clone().into()],
    )
    .await?;
    let workspaces: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": text(row, "id"),
                "slug": text(row, "slug"),
                "name": text(row, "name"),
                "role_name": text(row, "role_name"),
                "permissions": serde_json::from_str::<Value>(&text(row, "permissions")).unwrap_or(json!([])),
            })
        })
        .collect();
    Ok(Json(json!({
        "user": {"id": account.user.id, "email": account.user.email, "name": account.user.name},
        "workspaces": workspaces,
    })))
}
