//! Account-bound TOTP. Password and OTP proofs never create partial sessions.
use super::{
    auth::{self, Account},
    login_guard as guard, platform,
};
use crate::{
    db,
    entities::{login_guards, platform_state, sessions, user_otp, users},
    error::{ApiError, Json, Result},
    snowflake::Id,
    state::AppState,
};
use axum::{
    Extension,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use rand::RngCore;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use totp_rs::{Algorithm, TOTP};

const CHALLENGE_TTL: i64 = 5 * 60 * 1000;
const ENROLL_TTL: i64 = 10 * 60 * 1000;
fn error(status: StatusCode, key: &str) -> ApiError {
    ApiError::localized(status, crate::i18n::Message::new(key))
}
fn invalid() -> ApiError {
    error(StatusCode::BAD_REQUEST, "auth.otpInvalid")
}
fn digest(value: &str) -> String {
    crate::hosts::runner::fingerprint(value)
}
fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn aad(id: i64) -> String {
    format!("account/otp/v1/{id}")
}
fn secret() -> Vec<u8> {
    let mut value = vec![0; 20];
    rand::rngs::OsRng.fill_bytes(&mut value);
    value
}
fn nonce() -> String {
    crate::hosts::runner::secret()
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    secret: Option<Vec<u8>>,
    recovery_hashes: Vec<String>,
    last_step: Option<u64>,
    generation: String,
    pending: Option<Pending>,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    secret: Vec<u8>,
    expires_millis: i64,
    session_id: i64,
    password_tag: String,
}
#[derive(Serialize, Deserialize)]
struct Challenge {
    user_id: i64,
    password_tag: String,
    generation: String,
    ip: String,
}
fn generator(secret: Vec<u8>, issuer: String, email: String) -> Result<TOTP> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some(issuer.replace(':', " ")),
        email.replace(':', " "),
    )
    .map_err(|_| invalid())
}
fn matching_step(secret: &[u8], code: &str, now: u64, last: Option<u64>) -> Result<Option<u64>> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let totp = generator(secret.to_vec(), "Account".into(), "account".into())?;
    let step = now / 30;
    for current in [step.saturating_sub(1), step, step + 1] {
        if last.is_none_or(|used| current > used) && equal(code, &totp.generate(current * 30)) {
            return Ok(Some(current));
        }
    }
    Ok(None)
}
fn recovery_hash(id: i64, value: &str) -> Option<String> {
    let value: String = value
        .chars()
        .filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .collect();
    (value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest(&format!("otp-recovery:{id}:{}", value.to_ascii_lowercase())))
}
fn recovery_codes(id: i64) -> (Vec<String>, Vec<String>) {
    let codes: Vec<String> = (0..10)
        .map(|_| {
            let mut raw = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut raw);
            hex::encode(raw)
                .as_bytes()
                .chunks(8)
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect();
    let hashes = codes
        .iter()
        .map(|code| recovery_hash(id, code).unwrap())
        .collect();
    (codes, hashes)
}
fn consume(data: &mut Stored, id: i64, code: &str, recovery: bool, now: u64) -> Result<bool> {
    if data.secret.is_none() || code.len() > 128 {
        return Ok(false);
    }
    if recovery {
        let Some(hash) = recovery_hash(id, code) else {
            return Ok(false);
        };
        if let Some(index) = data
            .recovery_hashes
            .iter()
            .position(|stored| equal(&hash, stored))
        {
            data.recovery_hashes.remove(index);
            return Ok(true);
        }
    } else if let Some(step) =
        matching_step(data.secret.as_ref().unwrap(), code, now, data.last_step)?
    {
        data.last_step = Some(step);
        return Ok(true);
    }
    Ok(false)
}
async fn load(s: &AppState, conn: &impl ConnectionTrait, id: i64) -> Result<Stored> {
    user_otp::Entity::find_by_id(id)
        .one(conn)
        .await?
        .map(|row| {
            let plain = s.vault.open_secret(&row.payload, &aad(id))?;
            serde_json::from_str(&plain)
                .map_err(std::io::Error::other)
                .map_err(Into::into)
        })
        .unwrap_or_else(|| Ok(Stored::default()))
}
async fn save(s: &AppState, conn: &impl ConnectionTrait, id: i64, data: &Stored) -> Result<()> {
    let payload = s.vault.seal(
        &serde_json::to_string(data).map_err(std::io::Error::other)?,
        &aad(id),
    )?;
    if let Some(row) = user_otp::Entity::find_by_id(id).one(conn).await? {
        let mut model: user_otp::ActiveModel = row.into();
        model.payload = Set(payload);
        model.updated_at = Set(db::now());
        model.updated_by = Set(id);
        model.update(conn).await?;
    } else {
        user_otp::ActiveModel {
            user_id: Set(id),
            payload: Set(payload),
            created_at: Set(db::now()),
            created_by: Set(id),
            updated_at: Set(db::now()),
            updated_by: Set(id),
        }
        .insert(conn)
        .await?;
    }
    Ok(())
}
async fn allowed(s: &AppState, conn: &impl ConnectionTrait) -> Result<bool> {
    let row = platform_state::Entity::find_by_id(1)
        .one(conn)
        .await?
        .ok_or_else(ApiError::missing)?;
    Ok(guard::Configuration::read(s, row.login_security.as_deref())?.otp_enabled)
}
async fn management_rate(s: &AppState, id: i64) -> Result<()> {
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    guard::sweep(&tx, db::now()).await?;
    let result = guard::rate(&tx, format!("otp-manage:{id}"), 10, db::now()).await;
    tx.commit().await?;
    result
}
async fn password(s: &AppState, account: &Account, value: &str) -> Result<String> {
    management_rate(s, i64::from(account.user.id)).await?;
    let row = users::Entity::find_by_id(i64::from(account.user.id))
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if value.len() > 1024 || !auth::verify(value, &row.password_hash) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "auth.current_password_invalid",
        ));
    }
    Ok(digest(&row.password_hash))
}
async fn recheck(
    conn: &impl ConnectionTrait,
    account: &Account,
    password_tag: Option<&str>,
) -> Result<()> {
    let id = i64::from(account.user.id);
    let user = users::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    let session = sessions::Entity::find_by_id(i64::from(account.session_id))
        .filter(sessions::Column::UserId.eq(id))
        .filter(sessions::Column::ExpiresAt.gt(db::now()))
        .one(conn)
        .await?;
    if user.active == 0
        || session.is_none()
        || password_tag.is_some_and(|tag| !equal(tag, &digest(&user.password_hash)))
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}
async fn changed(
    s: &AppState,
    conn: &impl ConnectionTrait,
    account: &Account,
    action: &str,
) -> Result<()> {
    sessions::Entity::delete_many()
        .filter(sessions::Column::UserId.eq(i64::from(account.user.id)))
        .filter(sessions::Column::Id.ne(i64::from(account.session_id)))
        .exec(conn)
        .await?;
    platform::audit(
        s,
        conn,
        &account.user,
        action,
        "users",
        account.user.id,
        json!({"security":"otp"}),
    )
    .await
}

pub async fn status(State(s): State<AppState>, account: Account) -> Result<impl IntoResponse> {
    let data = load(&s, &s.db, i64::from(account.user.id)).await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(
            json!({"enabled":data.secret.is_some(),"enrollment_enabled":allowed(&s,&s.db).await?,"recovery_remaining":data.recovery_hashes.len()}),
        ),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Password {
    current_password: String,
}
pub async fn enroll(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<Password>,
) -> Result<impl IntoResponse> {
    let verified = password(&s, &account, &body.current_password).await?;
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    recheck(&tx, &account, Some(&verified)).await?;
    if !allowed(&s, &tx).await? {
        return Err(error(StatusCode::CONFLICT, "auth.otpUnavailable"));
    }
    let id = i64::from(account.user.id);
    let mut data = load(&s, &tx, id).await?;
    if data.secret.is_some() {
        return Err(error(StatusCode::CONFLICT, "auth.otpAlreadyEnabled"));
    }
    let raw = secret();
    let config = platform_state::Entity::find_by_id(1)
        .one(&tx)
        .await?
        .ok_or_else(ApiError::missing)?;
    let site = super::site::Configuration::read(config.site_config.as_deref())?;
    let issuer = if site.site_name.is_empty() {
        "AI Workspace".to_owned()
    } else {
        site.site_name
    };
    let totp = generator(raw.clone(), issuer, account.user.email.clone())?;
    let uri = totp.get_url();
    let qr = qrcode::QrCode::new(uri.as_bytes())
        .map_err(|_| invalid())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    data.pending = Some(Pending {
        secret: raw.clone(),
        expires_millis: db::now() + ENROLL_TTL,
        session_id: i64::from(account.session_id),
        password_tag: verified,
    });
    save(&s, &tx, id, &data).await?;
    tx.commit().await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(
            json!({"secret":data_encoding::BASE32_NOPAD.encode(&raw),"uri":uri,"qr_svg":qr,"expires_in":ENROLL_TTL/1000}),
        ),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    code: String,
}
pub async fn confirm(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<Code>,
) -> Result<impl IntoResponse> {
    let id = i64::from(account.user.id);
    management_rate(&s, id).await?;
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    if !allowed(&s, &tx).await? {
        return Err(error(StatusCode::CONFLICT, "auth.otpUnavailable"));
    }
    let mut data = load(&s, &tx, id).await?;
    if data.secret.is_some() {
        return Err(error(StatusCode::CONFLICT, "auth.otpAlreadyEnabled"));
    }
    let pending = data
        .pending
        .take()
        .filter(|p| p.expires_millis > db::now())
        .ok_or_else(|| error(StatusCode::GONE, "auth.otpExpired"))?;
    if pending.session_id != i64::from(account.session_id) {
        return Err(invalid());
    }
    recheck(&tx, &account, Some(&pending.password_tag)).await?;
    let step = matching_step(&pending.secret, &body.code, (db::now() / 1000) as u64, None)?
        .ok_or_else(invalid)?;
    let (codes, hashes) = recovery_codes(id);
    data.secret = Some(pending.secret);
    data.last_step = Some(step);
    data.recovery_hashes = hashes;
    data.generation = nonce();
    save(&s, &tx, id, &data).await?;
    changed(&s, &tx, &account, "otp-enable").await?;
    tx.commit().await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"recovery_codes":codes})),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manage {
    current_password: String,
    code: String,
    #[serde(default)]
    use_recovery: bool,
}
pub async fn regenerate(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<Manage>,
) -> Result<impl IntoResponse> {
    let verified = password(&s, &account, &body.current_password).await?;
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    let id = i64::from(account.user.id);
    let mut data = load(&s, &tx, id).await?;
    recheck(&tx, &account, Some(&verified)).await?;
    if !consume(
        &mut data,
        id,
        &body.code,
        body.use_recovery,
        (db::now() / 1000) as u64,
    )? {
        return Err(invalid());
    }
    let (codes, hashes) = recovery_codes(id);
    data.recovery_hashes = hashes;
    data.generation = nonce();
    save(&s, &tx, id, &data).await?;
    changed(&s, &tx, &account, "otp-recovery-reset").await?;
    tx.commit().await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({"recovery_codes":codes})),
    ))
}
pub async fn disable(
    State(s): State<AppState>,
    account: Account,
    Json(body): Json<Manage>,
) -> Result<Json<Value>> {
    let verified = password(&s, &account, &body.current_password).await?;
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    let id = i64::from(account.user.id);
    let mut data = load(&s, &tx, id).await?;
    recheck(&tx, &account, Some(&verified)).await?;
    if !consume(
        &mut data,
        id,
        &body.code,
        body.use_recovery,
        (db::now() / 1000) as u64,
    )? {
        return Err(invalid());
    }
    user_otp::Entity::delete_by_id(id).exec(&tx).await?;
    changed(&s, &tx, &account, "otp-disable").await?;
    tx.commit().await?;
    Ok(Json(json!({"enabled":false})))
}
/// Called only after password verification, with the platform transaction lock held.
pub(super) async fn issue(
    s: &AppState,
    conn: &impl ConnectionTrait,
    user: &users::Model,
    ip: &str,
) -> Result<Option<Value>> {
    let data = load(s, conn, user.id).await?;
    if data.secret.is_none() {
        return Ok(None);
    }
    guard::sweep(conn, db::now()).await?;
    let token = nonce();
    let payload = Challenge {
        user_id: user.id,
        password_tag: digest(&user.password_hash),
        generation: data.generation,
        ip: ip.into(),
    };
    let key = format!("otp-login:{}", digest(&token));
    let sealed = s.vault.seal(
        &serde_json::to_string(&payload).map_err(std::io::Error::other)?,
        &key,
    )?;
    guard::save(conn, key, 0, Some(sealed), db::now() + CHALLENGE_TTL).await?;
    Ok(Some(
        json!({"otp_required":true,"otp_token":token,"expires_in":CHALLENGE_TTL/1000}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    token: String,
    code: String,
    #[serde(default)]
    use_recovery: bool,
}
pub async fn login(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Extension(proxies): Extension<auth::TrustedProxies>,
    headers: HeaderMap,
    Json(body): Json<Login>,
) -> Result<impl IntoResponse> {
    let ip = auth::login_ip(peer.ip(), &headers, &proxies).to_string();
    let now = db::now();
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    guard::sweep(&tx, now).await?;
    let limit = guard::rate(&tx, format!("otp-ip:{}", digest(&ip)), 20, now).await;
    if let Err(e) = limit {
        tx.commit().await?;
        return Err(e);
    }
    let key = format!("otp-login:{}", digest(&body.token));
    let Some(row) = guard::current(&tx, &key, now).await? else {
        tx.commit().await?;
        return Err(error(StatusCode::GONE, "auth.otpExpired"));
    };
    if row.count >= 5 {
        tx.commit().await?;
        return Err(error(StatusCode::TOO_MANY_REQUESTS, "auth.otpAttempts"));
    }
    guard::save(
        &tx,
        key.clone(),
        row.count + 1,
        row.payload.clone(),
        row.expires_at,
    )
    .await?;
    let plain = s
        .vault
        .open_secret(row.payload.as_deref().ok_or_else(invalid)?, &key)?;
    let challenge: Challenge = serde_json::from_str(&plain).map_err(std::io::Error::other)?;
    let user = users::Entity::find_by_id(challenge.user_id)
        .one(&tx)
        .await?;
    let Some(user) = user.filter(|u| {
        u.active != 0
            && equal(&digest(&u.password_hash), &challenge.password_tag)
            && equal(&ip, &challenge.ip)
    }) else {
        tx.commit().await?;
        return Err(invalid());
    };
    if let Err(e) = guard::rate(&tx, format!("otp-user:{}", user.id), 10, now).await {
        tx.commit().await?;
        return Err(e);
    }
    let mut data = load(&s, &tx, user.id).await?;
    if body.token.len() > 256
        || !equal(&data.generation, &challenge.generation)
        || !consume(
            &mut data,
            user.id,
            &body.code,
            body.use_recovery,
            (now / 1000) as u64,
        )?
    {
        tx.commit().await?;
        return Err(invalid());
    }
    save(&s, &tx, user.id, &data).await?;
    login_guards::Entity::delete_by_id(key).exec(&tx).await?;
    guard::succeeded(&tx, &user.email).await?;
    let id = Id::from(user.id);
    let session = auth::start_session(&s, &id, &tx).await?;
    users::Entity::update_many()
        .col_expr(
            users::Column::LastLoginAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .col_expr(
            users::Column::LastLoginIp,
            sea_orm::sea_query::Expr::value(ip),
        )
        .filter(users::Column::Id.eq(user.id))
        .exec(&tx)
        .await?;
    tx.commit().await?;
    Ok((
        auth::set_cookie(&s, &session, auth::SESSION_MS / 1000),
        Json(json!({"id":id,"email":user.email})),
    ))
}

#[cfg(test)]
mod tests;
