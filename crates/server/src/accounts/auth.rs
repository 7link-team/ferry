//! Accounts and sessions. Global, not workspace-scoped: a user exists before
//! any workspace does, and may belong to several.

#[cfg(test)]
mod tests;

use crate::snowflake::Id;

use crate::error::Json;
use crate::{
    db,
    entities::{
        account_email_verifications, session_refresh_tokens, sessions, users, workspace_members,
        workspace_roles, workspaces,
    },
    error::{ApiError, Result},
    state::AppState,
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    extract::{FromRequestParts, State},
    http::{HeaderMap, HeaderValue, header, request::Parts},
};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, FromQueryResult, JoinType, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub const COOKIE: &str = "ferry_session";
#[derive(Clone, Default)]
pub struct TrustedProxies(pub Vec<ipnet::IpNet>);

pub(crate) fn login_ip(
    peer: std::net::IpAddr,
    headers: &HeaderMap,
    proxies: &TrustedProxies,
) -> std::net::IpAddr {
    let trusted = |ip| proxies.0.iter().any(|network| network.contains(&ip));
    if !trusted(peer) {
        return peer;
    }
    let values = headers.get_all("x-forwarded-for");
    let mut chain = Vec::new();
    for value in values.iter() {
        let Ok(text) = value.to_str() else {
            return peer;
        };
        if text.len() > 4096 {
            return peer;
        }
        for item in text.split(',') {
            let Ok(ip) = item.trim().parse::<std::net::IpAddr>() else {
                return peer;
            };
            chain.push(ip);
            if chain.len() > 32 {
                return peer;
            }
        }
    }
    if chain.is_empty() {
        return headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(peer);
    }
    // The rightmost untrusted hop is the client visible to our trusted proxy;
    // entries supplied to its left by an untrusted client cannot override it.
    let mut ip = peer;
    for forwarded in chain.into_iter().rev() {
        if !trusted(ip) {
            break;
        }
        ip = forwarded;
    }
    ip
}

#[cfg(test)]
mod login_ip_tests {
    use super::*;
    #[test]
    fn only_trusted_proxy_hops_can_supply_client_ips() {
        let peer = "10.0.0.2".parse().unwrap();
        let trusted = TrustedProxies(vec!["10.0.0.0/8".parse().unwrap()]);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "192.0.2.99, 203.0.113.5, 10.0.0.3".parse().unwrap(),
        );
        assert_eq!(login_ip(peer, &headers, &TrustedProxies::default()), peer);
        assert_eq!(
            login_ip(peer, &headers, &trusted).to_string(),
            "203.0.113.5"
        );
        headers.insert("x-forwarded-for", "invalid, 10.0.0.3".parse().unwrap());
        assert_eq!(login_ip(peer, &headers, &trusted), peer);
        headers.remove("x-forwarded-for");
        headers.insert("x-real-ip", "2001:db8::5".parse().unwrap());
        assert_eq!(
            login_ip(peer, &headers, &trusted).to_string(),
            "2001:db8::5"
        );
        assert_eq!(login_ip(peer, &headers, &TrustedProxies::default()), peer);
    }
}
/// Legacy test fixtures retain their original session duration.
#[cfg(test)]
pub(crate) const SESSION_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const REFRESH_COOKIE: &str = "session_refresh";

#[derive(Clone)]
pub struct User {
    pub id: Id,
    pub email: String,
    pub name: String,
    pub is_superadmin: bool,
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
        let hash = crate::tokens::fingerprint(&token);
        #[derive(FromQueryResult)]
        struct SessionUser {
            session_id: i64,
            id: i64,
            email: String,
            name: String,
            active: i64,
            is_superadmin: i64,
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
                users::Column::IsSuperadmin,
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
        if row.expires_at <= db::now() || row.active == 0 {
            let refresh_alive = row.active != 0
                && session_refresh_tokens::Entity::find_by_id(row.session_id)
                    .filter(session_refresh_tokens::Column::ExpiresAt.gt(db::now()))
                    .one(&s.db)
                    .await?
                    .is_some();
            if !refresh_alive {
                sessions::Entity::delete_many()
                    .filter(sessions::Column::TokenHash.eq(hash))
                    .exec(&s.db)
                    .await?;
            }
            return Err(ApiError::unauthorized());
        }
        let session_id = Id::from(row.session_id);
        if let Some(user) = parts
            .extensions
            .get::<crate::platform_monitor::RequestUser>()
        {
            user.0.store(row.id, std::sync::atomic::Ordering::Relaxed);
        }
        sessions::Entity::update_many()
            .col_expr(sessions::Column::SeenAt, Expr::value(db::now()))
            .filter(sessions::Column::Id.eq(i64::from(session_id)))
            .exec(&s.db)
            .await?;
        Ok(Self {
            user: User {
                id: Id::from(row.id),
                email: row.email,
                name: row.name,
                is_superadmin: row.is_superadmin != 0,
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

pub(crate) fn set_cookie(
    s: &AppState,
    value: &str,
    max_age: i64,
) -> [(header::HeaderName, String); 1] {
    let secure = if s.secure_cookie { "; Secure" } else { "" };
    [(
        header::SET_COOKIE,
        format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax{secure}; Max-Age={max_age}"),
    )]
}

pub(crate) struct SessionCookies {
    pub(crate) token: String,
    refresh_token: String,
    access_max_age: i64,
    refresh_max_age: i64,
}
impl SessionCookies {
    pub(crate) fn headers(&self, s: &AppState) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let [(_, access)] = set_cookie(s, &self.token, self.access_max_age);
        let secure = if s.secure_cookie { "; Secure" } else { "" };
        let refresh = format!(
            "{REFRESH_COOKIE}={}; Path=/api/sessions; HttpOnly; SameSite=Lax{secure}; Max-Age={}",
            self.refresh_token, self.refresh_max_age
        );
        // Values contain only server-generated hex secrets and integer ages.
        // Append, rather than insert: both Set-Cookie headers must survive.
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_str(&access).expect("opaque access cookie"),
        );
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_str(&refresh).expect("opaque refresh cookie"),
        );
        headers
    }
}

fn session_metadata(access: i64, refresh: i64, auto_renew: bool, access_ttl_ms: i64) -> Value {
    let now = db::now();
    json!({"access_expires_at_ms":access,"refresh_expires_at_ms":refresh,"server_time_ms":now,
        "refreshable":refresh>now,"auto_renew":auto_renew&&refresh>now,"access_ttl_ms":access_ttl_ms})
}

#[derive(Deserialize)]
pub struct Credentials {
    pub(crate) email: String,
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) proof: super::login_guard::Proof,
}

/// Argon2 with per-password salt. The cost parameters are the crate defaults,
/// which are the OWASP-recommended ones.
pub(crate) fn hash(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|_| {
            ApiError::localized(
                axum::http::StatusCode::BAD_REQUEST,
                crate::i18n::Message::new("auth.password_save"),
            )
        })
}

pub(crate) fn verify(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// The stored form of a login email: trimmed and lower-cased.
pub(crate) fn email(raw: &str) -> Result<String> {
    let email = raw.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.email_invalid"),
        ));
    }
    Ok(email)
}

fn password(password: &str) -> Result<()> {
    // Length, not a composition rule: composition rules push people toward
    // "Password1!" and are worse than a longer passphrase.
    if password.chars().count() < 8 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.password_short").with("minimum", "8"),
        ));
    }
    if password.len() > 1024 {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.password_long"),
        ));
    }
    Ok(())
}

pub(crate) fn check(credentials: &Credentials, require_name: bool) -> Result<String> {
    let email = email(&credentials.email)?;
    password(&credentials.password)?;
    if require_name && credentials.name.trim().is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.name_required"),
        ));
    }
    Ok(email)
}

#[derive(Deserialize)]
pub struct Registration {
    #[serde(flatten)]
    credentials: Credentials,
    #[serde(default)]
    verification_code: String,
}
pub async fn register(
    State(s): State<AppState>,
    Json(request): Json<Registration>,
) -> Result<impl axum::response::IntoResponse> {
    let body = request.credentials;
    let email = check(&body, true)?;
    use sea_orm::TransactionTrait;
    let tx = s.db.begin().await?;
    let platform = crate::accounts::platform::lock(&tx).await?;
    if platform.initialized == 0 || platform.registration_enabled == 0 {
        return Err(ApiError::forbidden());
    }
    let mail = super::mail::Configuration::read(&s, platform.mail_config.as_deref())?;
    if !super::mail::consume(&tx, &mail, &email, &request.verification_code).await? {
        // Failed attempts must survive the rejected registration transaction.
        tx.commit().await?;
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("mail.codeInvalid"),
        ));
    }
    if users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&tx)
        .await?
        .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("auth.email_registered"),
        ));
    }
    let (id, now) = (s.next_id()?, db::now());
    users::Entity::insert(users::ActiveModel {
        id: Set(i64::from(id)),
        email: Set(email.clone()),
        name: Set(body.name.trim().to_owned()),
        password_hash: Set(hash(&body.password)?),
        last_login_at: Set(None),
        last_login_ip: Set(None),
        active: Set(1),
        is_superadmin: Set(0),
        version: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&tx)
    .await?;
    if mail.required() {
        account_email_verifications::Entity::insert(account_email_verifications::ActiveModel {
            user_id: Set(i64::from(id)),
            email: Set(email.clone()),
            verified_at: Set(now),
            verified_by: Set(i64::from(id)),
        })
        .exec(&tx)
        .await?;
    }
    let session = start_session(&s, &id, &tx).await?;
    tx.commit().await?;
    Ok((session.headers(&s), Json(json!({"id": id, "email": email}))))
}

pub async fn login(
    State(s): State<AppState>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    axum::Extension(proxies): axum::Extension<TrustedProxies>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    let email = body.email.trim().to_lowercase();
    let ip = login_ip(peer.ip(), &headers, &proxies).to_string();
    if email.len() > 320 || body.password.len() > 1024 {
        return Err(ApiError::localized(
            axum::http::StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("auth.login_invalid"),
        ));
    }
    super::login_guard::admit(&s, &email, &ip, &body.proof).await?;
    let row = users::Entity::find()
        .filter(users::Column::Email.eq(email.clone()))
        .one(&s.db)
        .await?;
    // One message and one code path for "no such user" and "wrong password",
    // so the response does not say which emails are registered.
    let refused = || {
        ApiError::localized(
            axum::http::StatusCode::UNAUTHORIZED,
            crate::i18n::Message::new("auth.login_invalid"),
        )
    };
    if row
        .as_ref()
        .is_none_or(|row| row.active == 0 || !verify(&body.password, &row.password_hash))
    {
        if super::login_guard::failed(&s, &email, &ip).await? {
            return Err(ApiError::localized(
                axum::http::StatusCode::PRECONDITION_REQUIRED,
                crate::i18n::Message::new("auth.captchaRequired"),
            ));
        }
        return Err(refused());
    }
    let row = row.ok_or_else(refused)?;
    let id = Id::from(row.id);
    use sea_orm::TransactionTrait;
    let tx = s.db.begin().await?;
    super::platform::lock(&tx).await?;
    let current = users::Entity::find_by_id(row.id)
        .one(&tx)
        .await?
        .ok_or_else(refused)?;
    if current.active == 0 || current.password_hash != row.password_hash {
        return Err(refused());
    }
    if let Some(challenge) = super::otp::issue(&s, &tx, &current, &ip).await? {
        tx.commit().await?;
        return Ok(([(header::CACHE_CONTROL, "no-store")], Json(challenge)).into_response());
    }
    super::login_guard::succeeded(&tx, &email).await?;
    let session = start_session(&s, &id, &tx).await?;
    users::Entity::update_many()
        .col_expr(users::Column::LastLoginAt, Expr::value(db::now()))
        .col_expr(
            users::Column::LastLoginIp,
            Expr::value(login_ip(peer.ip(), &headers, &proxies).to_string()),
        )
        .filter(users::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok((session.headers(&s), Json(json!({"id": id, "email": email}))).into_response())
}

/// Both credentials are hashed at rest; callers include both cookies only after
/// their login/OTP transaction commits. No existing session row is rewritten.
pub(crate) async fn start_session(
    s: &AppState,
    user_id: &Id,
    conn: &impl sea_orm::ConnectionTrait,
) -> Result<SessionCookies> {
    let (id, now) = (s.next_id()?, db::now());
    let policy = super::session_policy::Configuration::load(conn).await?;
    let (access_ms, refresh_ms) = policy.lifetimes(now)?;
    let token = crate::tokens::secret();
    let refresh_token = crate::tokens::secret();
    sessions::Entity::insert(sessions::ActiveModel {
        id: Set(i64::from(id)),
        token_hash: Set(crate::tokens::fingerprint(&token)),
        user_id: Set(i64::from(*user_id)),
        workspace_id: Set(0),
        created_at: Set(now),
        seen_at: Set(now),
        expires_at: Set(now + access_ms),
    })
    .exec(conn)
    .await?;
    session_refresh_tokens::Entity::insert(session_refresh_tokens::ActiveModel {
        session_id: Set(id.into()),
        token_hash: Set(crate::tokens::fingerprint(&refresh_token)),
        previous_hash: Set(None),
        access_ttl_ms: Set(access_ms),
        created_at: Set(now),
        created_by: Set((*user_id).into()),
        rotated_at: Set(now),
        rotated_by: Set((*user_id).into()),
        expires_at: Set(now + refresh_ms),
        expires_by: Set((*user_id).into()),
    })
    .exec(conn)
    .await?;
    Ok(SessionCookies {
        token,
        refresh_token,
        access_max_age: access_ms / 1000,
        refresh_max_age: refresh_ms / 1000,
    })
}

pub async fn logout(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<impl axum::response::IntoResponse> {
    let mut ids = std::collections::BTreeSet::new();
    if let Some(token) = cookie(&headers, COOKIE) {
        if let Some(session) = sessions::Entity::find()
            .filter(sessions::Column::TokenHash.eq(crate::tokens::fingerprint(&token)))
            .one(&s.db)
            .await?
        {
            ids.insert(session.id);
        }
    }
    if let Some(token) = cookie(&headers, REFRESH_COOKIE) {
        let hash = crate::tokens::fingerprint(&token);
        if let Some(grant) = session_refresh_tokens::Entity::find()
            .filter(
                Condition::any()
                    .add(session_refresh_tokens::Column::TokenHash.eq(hash.clone()))
                    .add(session_refresh_tokens::Column::PreviousHash.eq(hash)),
            )
            .one(&s.db)
            .await?
        {
            ids.insert(grant.session_id);
        }
    }
    if !ids.is_empty() {
        let tx = s.db.begin().await?;
        super::platform::lock(&tx).await?;
        // Authorize the stable session id before taking the rotation fence.
        // One previous refresh hash is logout-only: it cannot mint credentials.
        sessions::Entity::delete_many()
            .filter(sessions::Column::Id.is_in(ids))
            .exec(&tx)
            .await?;
        tx.commit().await?;
    }
    let cleared = SessionCookies {
        token: String::new(),
        refresh_token: String::new(),
        access_max_age: 0,
        refresh_max_age: 0,
    };
    Ok((cleared.headers(&s), Json(json!({"ok": true}))))
}

/// A refresh credential can only rotate its original session. Neither a user
/// id nor a workspace is accepted from the caller, and a failed CAS emits no
/// Set-Cookie header that could erase another tab's successful rotation.
pub async fn refresh(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Json<Value>)> {
    let token = cookie(&headers, REFRESH_COOKIE).ok_or_else(ApiError::unauthorized)?;
    let hash = crate::tokens::fingerprint(&token);
    // Invalid guesses never acquire the platform mutation fence. Recheck the
    // current hash inside the transaction after acquiring it.
    session_refresh_tokens::Entity::find()
        .filter(session_refresh_tokens::Column::TokenHash.eq(hash.clone()))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let tx = s.db.begin().await?;
    let platform = super::platform::lock(&tx).await?;
    let policy = super::session_policy::Configuration::read(platform.session_config.as_deref())?;
    if !policy.auto_renew {
        return Err(ApiError::localized(
            axum::http::StatusCode::FORBIDDEN,
            crate::i18n::Message::new("auth.sessionAutoRenewDisabled"),
        ));
    }
    let grant = session_refresh_tokens::Entity::find()
        .filter(session_refresh_tokens::Column::TokenHash.eq(hash.clone()))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let access = sessions::Entity::find_by_id(grant.session_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    users::Entity::update_many()
        .col_expr(users::Column::Active, Expr::col(users::Column::Active))
        .filter(users::Column::Id.eq(access.user_id))
        .exec(&tx)
        .await?;
    let user = users::Entity::find_by_id(access.user_id)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let now = db::now();
    if user.active == 0 || grant.expires_at <= now || grant.access_ttl_ms <= 0 {
        sessions::Entity::delete_by_id(grant.session_id)
            .exec(&tx)
            .await?;
        tx.commit().await?;
        return Err(ApiError::unauthorized());
    }
    let token = crate::tokens::secret();
    let refresh_token = crate::tokens::secret();
    let expires_at = now
        .saturating_add(grant.access_ttl_ms)
        .min(grant.expires_at);
    let changed = session_refresh_tokens::Entity::update_many()
        .col_expr(
            session_refresh_tokens::Column::PreviousHash,
            Expr::value(hash.clone()),
        )
        .col_expr(
            session_refresh_tokens::Column::TokenHash,
            Expr::value(crate::tokens::fingerprint(&refresh_token)),
        )
        .col_expr(session_refresh_tokens::Column::RotatedAt, Expr::value(now))
        .col_expr(
            session_refresh_tokens::Column::RotatedBy,
            Expr::value(user.id),
        )
        .filter(session_refresh_tokens::Column::SessionId.eq(grant.session_id))
        .filter(session_refresh_tokens::Column::TokenHash.eq(hash))
        .filter(session_refresh_tokens::Column::ExpiresAt.gt(now))
        .exec(&tx)
        .await?;
    if changed.rows_affected != 1 {
        return Err(ApiError::unauthorized());
    }
    let updated = sessions::Entity::update_many()
        .col_expr(
            sessions::Column::TokenHash,
            Expr::value(crate::tokens::fingerprint(&token)),
        )
        .col_expr(sessions::Column::SeenAt, Expr::value(now))
        .col_expr(sessions::Column::ExpiresAt, Expr::value(expires_at))
        .filter(sessions::Column::Id.eq(grant.session_id))
        .filter(sessions::Column::UserId.eq(user.id))
        .exec(&tx)
        .await?;
    if updated.rows_affected != 1 || db::now() >= grant.expires_at {
        return Err(ApiError::unauthorized());
    }
    tx.commit().await?;
    let cookies = SessionCookies {
        token,
        refresh_token,
        access_max_age: (expires_at - now) / 1000,
        refresh_max_age: (grant.expires_at - now) / 1000,
    };
    Ok((
        cookies.headers(&s),
        Json(session_metadata(
            expires_at,
            grant.expires_at,
            true,
            grant.access_ttl_ms,
        )),
    ))
}

/// Who am I, and which workspaces can I open. The shell calls this on load.
pub async fn me(State(s): State<AppState>, account: Account) -> Result<Json<Value>> {
    #[derive(FromQueryResult)]
    struct WorkspaceRole {
        id: i64,
        slug: String,
        name: String,
        timezone: Option<String>,
        owner_id: i64,
        role_name: String,
        role_builtin: i64,
        permissions: String,
    }
    let rows = workspace_members::Entity::find()
        .select_only()
        .columns([
            workspaces::Column::Id,
            workspaces::Column::Slug,
            workspaces::Column::Name,
            workspaces::Column::Timezone,
            workspaces::Column::OwnerId,
        ])
        .column_as(workspace_roles::Column::Name, "role_name")
        .column_as(workspace_roles::Column::Builtin, "role_builtin")
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
    let mut workspaces: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": Id::from(row.id),
                "slug": row.slug,
                "name": row.name,
                "timezone":row.timezone,
                "role_name": crate::accounts::workspace::role_name(&row.role_name, row.role_builtin != 0),
                "relation": if row.owner_id == i64::from(account.user.id) { "owner" } else { "member" },
                "permissions": serde_json::from_str::<Value>(&row.permissions).unwrap_or(json!([])),
            })
        })
        .collect();
    if account.user.is_superadmin {
        // Every workspace with full permissions, but the role still says
        // whether the administrator is the owner, a member, or neither.
        let roles: std::collections::HashMap<i64, String> = rows
            .iter()
            .map(|row| {
                (
                    row.id,
                    crate::accounts::workspace::role_name(&row.role_name, row.role_builtin != 0),
                )
            })
            .collect();
        workspaces = workspaces::Entity::find().order_by_asc(workspaces::Column::Name).all(&s.db).await?
            .into_iter().map(|row| json!({"id": Id::from(row.id), "slug": row.slug, "name": row.name,"timezone":row.timezone,
                "role_name": roles.get(&row.id).cloned().unwrap_or_else(|| crate::i18n::message("auth.platform_superadmin")),
                "relation": if row.owner_id == i64::from(account.user.id) { "owner" } else if roles.contains_key(&row.id) { "member" } else { "none" },
                "permissions": crate::accounts::workspace::PERMISSIONS.iter().map(|p| p.0).collect::<Vec<_>>() })).collect();
    }
    let session = if let Some(access) = sessions::Entity::find_by_id(account.session_id)
        .one(&s.db)
        .await?
    {
        let grant = session_refresh_tokens::Entity::find_by_id(access.id)
            .one(&s.db)
            .await?;
        let renewable = if grant.is_some() {
            super::session_policy::Configuration::load(&s.db)
                .await?
                .auto_renew
        } else {
            false
        };
        session_metadata(
            access.expires_at,
            grant.as_ref().map_or(0, |grant| grant.expires_at),
            renewable,
            grant.map_or(0, |grant| grant.access_ttl_ms),
        )
    } else {
        Value::Null
    };
    Ok(Json(json!({
        "user": {"avatar": super::preferences::avatars(&s.db, vec![i64::from(account.user.id)]).await?.remove(&i64::from(account.user.id)), "id": account.user.id, "email": account.user.email, "name": account.user.name, "is_superadmin": account.user.is_superadmin},
        "workspaces": workspaces,
        "session": session,
    })))
}

/// The signed-in user's own account, as a single-record form (`/api/profile/1`,
/// like the platform settings). Password inputs are write-only and always empty.
pub async fn profile(State(s): State<AppState>, account: Account) -> Result<Json<Value>> {
    let row = users::Entity::find_by_id(i64::from(account.user.id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let verified = account_email_verifications::Entity::find_by_id(row.id)
        .one(&s.db)
        .await?
        .is_some_and(|evidence| evidence.email == row.email);
    Ok(Json(json!({
        "id": "1", "name": row.name, "email": row.email, "version": row.version,
        "email_verified": verified,
        "new_password": "", "current_password": "",
        "avatar": super::preferences::avatars(&s.db, vec![row.id]).await?.remove(&row.id),
    })))
}

#[derive(Deserialize)]
pub struct ProfileUpdate {
    #[serde(default)]
    avatar: Option<crate::avatar::Avatar>,
    version: i64,
    name: String,
    email: String,
    #[serde(default)]
    new_password: String,
    #[serde(default)]
    current_password: String,
}

/// Name, login email and password of the signed-in user. Changing the email or
/// the password needs the current password and signs out every other session;
/// nobody else can change them, so an administrator cannot take an account over.
pub async fn update_profile(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<ProfileUpdate>,
) -> Result<Json<Value>> {
    use sea_orm::TransactionTrait;
    if let Some(avatar) = &body.avatar {
        avatar.validate()?;
    }
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.name_required"),
        ));
    }
    let new_email = email(&body.email)?;
    if !body.new_password.is_empty() {
        password(&body.new_password)?;
    }
    let tx = s.db.begin().await?;
    // Serialises email changes with registration, like the uniqueness check there.
    crate::accounts::platform::lock(&tx).await?;
    let row = users::Entity::find_by_id(i64::from(account.user.id))
        .one(&tx)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if row.version != body.version {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("auth.profile_changed"),
        ));
    }
    let email_changed = new_email != row.email;
    let password_changed = !body.new_password.is_empty();
    if (email_changed || password_changed) && !verify(&body.current_password, &row.password_hash) {
        return Err(ApiError::localized(
            axum::http::StatusCode::BAD_REQUEST,
            crate::i18n::Message::new("auth.current_password_invalid"),
        ));
    }
    if email_changed
        && users::Entity::find()
            .filter(users::Column::Email.eq(new_email.clone()))
            .one(&tx)
            .await?
            .is_some()
    {
        return Err(ApiError::localized(
            axum::http::StatusCode::CONFLICT,
            crate::i18n::Message::new("auth.email_used"),
        ));
    }
    let mut update = users::Entity::update_many()
        .col_expr(users::Column::Name, Expr::value(name.clone()))
        .col_expr(users::Column::Email, Expr::value(new_email.clone()))
        .col_expr(users::Column::Version, Expr::value(row.version + 1))
        .col_expr(users::Column::UpdatedAt, Expr::value(db::now()));
    if password_changed {
        update = update.col_expr(
            users::Column::PasswordHash,
            Expr::value(hash(&body.new_password)?),
        );
    }
    update
        .filter(users::Column::Id.eq(row.id))
        .exec(&tx)
        .await?;
    if email_changed {
        account_email_verifications::Entity::delete_by_id(row.id)
            .exec(&tx)
            .await?;
    }
    if email_changed || password_changed {
        sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(row.id))
            .filter(sessions::Column::Id.ne(i64::from(account.session_id)))
            .exec(&tx)
            .await?;
    }
    let mut changed = Vec::new();
    if let Some(avatar) = &body.avatar {
        super::preferences::save_avatar(&tx, row.id, avatar).await?;
        changed.push("avatar");
    }
    if name != row.name {
        changed.push("name");
    }
    if email_changed {
        changed.push("email");
    }
    if password_changed {
        changed.push("password");
    }
    // Never the password; the email trail is what an account takeover would hide.
    let details = if email_changed {
        json!({"changed": changed, "email": {"from": row.email, "to": new_email}})
    } else {
        json!({"changed": changed})
    };
    crate::accounts::platform::audit(
        &s,
        &tx,
        &account.user,
        "update-profile",
        "users",
        account.user.id,
        details,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": "1", "version": row.version + 1})))
}
