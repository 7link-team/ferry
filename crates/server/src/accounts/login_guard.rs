//! Shared login limits and one-use, account/IP-bound challenges.
use super::{auth, platform};
use crate::{
    db,
    entities::login_guards as guards,
    error::{ApiError, Json, Result},
    state::AppState,
};
use axum::{
    Extension,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use subtle::ConstantTimeEq;
mod captcha;
#[cfg(test)]
#[path = "login_guard/tests.rs"]
mod tests;

const AAD: &str = "platform/login-guard/v1";
const WINDOW: i64 = 15 * 60 * 1000;
const THRESHOLD: i64 = 5;
const CAPACITY: u64 = 20_000;
fn error(status: StatusCode, key: &str) -> ApiError {
    ApiError::localized(status, crate::i18n::Message::new(key))
}
fn required_error() -> ApiError {
    error(StatusCode::PRECONDITION_REQUIRED, "auth.captchaRequired")
}
fn limited() -> ApiError {
    error(StatusCode::TOO_MANY_REQUESTS, "auth.loginRateLimit")
}
fn local() -> String {
    "local".into()
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Configuration {
    pub mode: String,
    pub otp_enabled: bool,
    site_key: String,
    secret_key: String,
    hostname: String,
}
impl Default for Configuration {
    fn default() -> Self {
        Self {
            mode: local(),
            otp_enabled: false,
            site_key: String::new(),
            secret_key: String::new(),
            hostname: String::new(),
        }
    }
}
impl Configuration {
    pub fn read(s: &AppState, value: Option<&str>) -> Result<Self> {
        value
            .map(|v| {
                s.vault.open_secret(v, AAD).and_then(|plain| {
                    serde_json::from_str(&plain)
                        .map_err(std::io::Error::other)
                        .map_err(Into::into)
                })
            })
            .transpose()
            .map(|v| v.unwrap_or_default())
    }
    pub fn view(&self) -> Value {
        json!({"otp_enabled":self.otp_enabled,"login_guard_mode":self.mode,"login_turnstile_site_key":self.site_key,
        "login_turnstile_hostname":self.hostname,"login_turnstile_has_secret":!self.secret_key.is_empty()})
    }
    pub fn update(
        s: &AppState,
        value: Option<&str>,
        patch: &Map<String, Value>,
    ) -> Result<Option<String>> {
        if !patch.keys().any(|k| {
            k == "otp_enabled" || k.starts_with("login_guard_") || k.starts_with("login_turnstile_")
        }) {
            return Ok(value.map(str::to_owned));
        }
        let mut config = Self::read(s, value)?;
        if let Some(value) = patch.get("otp_enabled") {
            config.otp_enabled = value
                .as_bool()
                .ok_or_else(|| error(StatusCode::BAD_REQUEST, "auth.loginGuardInvalid"))?;
        }
        for (key, target) in [
            ("login_guard_mode", &mut config.mode),
            ("login_turnstile_site_key", &mut config.site_key),
            ("login_turnstile_hostname", &mut config.hostname),
        ] {
            if let Some(value) = patch.get(key) {
                *target = value
                    .as_str()
                    .filter(|v| v.len() <= 255 && !v.chars().any(char::is_control))
                    .ok_or_else(|| error(StatusCode::BAD_REQUEST, "auth.loginGuardInvalid"))?
                    .trim()
                    .to_owned();
            }
        }
        if let Some(value) = patch.get("login_turnstile_secret_key") {
            let key = value
                .as_str()
                .filter(|v| v.len() <= 255 && !v.chars().any(char::is_control))
                .ok_or_else(|| error(StatusCode::BAD_REQUEST, "auth.loginGuardInvalid"))?;
            if !key.trim().is_empty() {
                config.secret_key = key.trim().into();
            }
        }
        config.hostname = config.hostname.to_ascii_lowercase();
        if !matches!(config.mode.as_str(), "local" | "turnstile")
            || (config.mode == "turnstile"
                && (config.site_key.is_empty()
                    || config.secret_key.is_empty()
                    || config.hostname.is_empty()
                    || !config
                        .hostname
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')))
        {
            return Err(error(StatusCode::BAD_REQUEST, "auth.loginGuardInvalid"));
        }
        Ok(Some(s.vault.seal(
            &serde_json::to_string(&config).map_err(std::io::Error::other)?,
            AAD,
        )?))
    }
}

#[derive(Default, Deserialize)]
pub struct Proof {
    #[serde(default)]
    pub login_challenge_id: String,
    #[serde(default)]
    pub login_challenge_answer: String,
    #[serde(default)]
    pub turnstile_token: String,
    #[serde(default)]
    pub login_verification_token: String,
}

fn fingerprint(value: &str) -> String {
    crate::hosts::runner::fingerprint(value)
}
fn key(kind: &str, value: &str) -> String {
    format!("{kind}:{}", fingerprint(value))
}
fn binding(email: &str, ip: &str) -> String {
    fingerprint(&format!("{}:{email}:{ip}", email.len()))
}

pub(super) async fn sweep(conn: &impl ConnectionTrait, now: i64) -> Result<()> {
    let rows = guards::Entity::find()
        .filter(guards::Column::ExpiresAt.lte(now))
        .limit(200)
        .all(conn)
        .await?;
    if !rows.is_empty() {
        guards::Entity::delete_many()
            .filter(guards::Column::Id.is_in(rows.into_iter().map(|r| r.id)))
            .exec(conn)
            .await?;
    }
    Ok(())
}
pub(super) async fn current(
    conn: &impl ConnectionTrait,
    id: &str,
    now: i64,
) -> Result<Option<guards::Model>> {
    Ok(guards::Entity::find_by_id(id)
        .filter(guards::Column::ExpiresAt.gt(now))
        .one(conn)
        .await?)
}
pub(super) async fn save(
    conn: &impl ConnectionTrait,
    id: String,
    count: i64,
    payload: Option<String>,
    expires_at: i64,
) -> Result<()> {
    let exists = guards::Entity::find_by_id(&id).one(conn).await?.is_some();
    if !exists && guards::Entity::find().count(conn).await? >= CAPACITY {
        return Err(limited());
    }
    let row = guards::ActiveModel {
        id: Set(id),
        count: Set(count),
        payload: Set(payload),
        expires_at: Set(expires_at),
    };
    if exists {
        row.update(conn).await?;
    } else {
        row.insert(conn).await?;
    }
    Ok(())
}
pub(super) async fn rate(
    conn: &impl ConnectionTrait,
    id: String,
    limit: i64,
    now: i64,
) -> Result<()> {
    let row = current(conn, &id, now).await?;
    if row.as_ref().is_some_and(|r| r.count >= limit) {
        return Err(limited());
    }
    save(
        conn,
        id,
        row.as_ref().map_or(1, |r| r.count + 1),
        None,
        row.map_or(now + 60_000, |r| r.expires_at),
    )
    .await
}
async fn is_required(conn: &impl ConnectionTrait, email: &str, ip: &str, now: i64) -> Result<bool> {
    for id in [key("failed-account", email), key("failed-ip", ip)] {
        if current(conn, &id, now)
            .await?
            .is_some_and(|r| r.count >= THRESHOLD)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Serialize, Deserialize)]
struct StoredChallenge {
    token_hash: String,
    answer_hash: String,
    kind: String,
}
fn answer_hash(id: &str, answer: &str) -> String {
    fingerprint(&format!("{id}:{}", answer.trim().to_ascii_uppercase()))
}
fn equal(left: &str, right: &str) -> bool {
    bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}
fn valid_answer(stored: &StoredChallenge, proof: &Proof) -> bool {
    if proof.login_challenge_id.len() > 128
        || proof.login_challenge_answer.len() > 32
        || !equal(&stored.token_hash, &fingerprint(&proof.login_challenge_id))
    {
        return false;
    }
    if stored.kind == "slider" {
        let Some(value) = proof
            .login_challenge_answer
            .parse::<i32>()
            .ok()
            .filter(|v| (0..=256).contains(v))
        else {
            return false;
        };
        (value - 3..=value + 3).any(|v| {
            equal(
                &stored.answer_hash,
                &answer_hash(&proof.login_challenge_id, &v.to_string()),
            )
        })
    } else {
        equal(
            &stored.answer_hash,
            &answer_hash(&proof.login_challenge_id, &proof.login_challenge_answer),
        )
    }
}

fn valid_turnstile(value: &Value, config: &Configuration) -> bool {
    value["success"] == true
        && value["action"] == "login"
        && value["hostname"]
            .as_str()
            .is_some_and(|h| h.eq_ignore_ascii_case(&config.hostname))
}
async fn turnstile(config: &Configuration, proof: &Proof, ip: &str) -> Result<()> {
    if proof.turnstile_token.is_empty() || proof.turnstile_token.len() > 2048 {
        return Err(required_error());
    }
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "auth.captchaUnavailable"))?
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&[
            ("secret", config.secret_key.as_str()),
            ("response", proof.turnstile_token.as_str()),
            ("remoteip", ip),
        ])
        .send()
        .await
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "auth.captchaUnavailable"))?;
    if !response.status().is_success() {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth.captchaUnavailable",
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "auth.captchaUnavailable"))?;
    if !valid_turnstile(&value, config) {
        return Err(required_error());
    }
    Ok(())
}

pub async fn admit(s: &AppState, email: &str, ip: &str, proof: &Proof) -> Result<()> {
    check(s, email, ip, proof, false).await
}

async fn check(
    s: &AppState,
    email: &str,
    ip: &str,
    proof: &Proof,
    verification: bool,
) -> Result<()> {
    let tx = s.db.begin().await?;
    let row = platform::lock(&tx).await?;
    let config = Configuration::read(s, row.login_security.as_deref())?;
    let now = db::now();
    sweep(&tx, now).await?;
    if let Err(error) = rate(
        &tx,
        key(
            if verification {
                "verify-ip"
            } else {
                "login-ip"
            },
            ip,
        ),
        if verification { 20 } else { 30 },
        now,
    )
    .await
    {
        tx.commit().await?;
        return Err(error);
    }
    if let Err(error) = rate(
        &tx,
        key(
            if verification {
                "verify-account"
            } else {
                "login-account"
            },
            email,
        ),
        10,
        now,
    )
    .await
    {
        tx.commit().await?;
        return Err(error);
    }
    if !verification && !proof.login_verification_token.is_empty() {
        if proof.login_verification_token.len() > 128 {
            tx.commit().await?;
            return Err(required_error());
        }
        let id = key("verified", &proof.login_verification_token);
        let stored = current(&tx, &id, now).await?;
        guards::Entity::delete_by_id(id).exec(&tx).await?;
        let valid = stored
            .and_then(|r| r.payload)
            .is_some_and(|value| equal(&value, &binding(email, ip)));
        tx.commit().await?;
        return if valid { Ok(()) } else { Err(required_error()) };
    }
    let required = verification || is_required(&tx, email, ip, now).await?;
    if required && config.mode == "local" {
        let id = format!(
            "captcha:{}:{}",
            binding(email, ip),
            fingerprint(&proof.login_challenge_id)
        );
        let stored = current(&tx, &id, now)
            .await?
            .and_then(|r| r.payload)
            .and_then(|v| serde_json::from_str::<StoredChallenge>(&v).ok());
        guards::Entity::delete_by_id(id).exec(&tx).await?;
        let valid = stored.as_ref().is_some_and(|v| valid_answer(v, proof));
        tx.commit().await?;
        if !valid {
            return Err(required_error());
        }
    } else {
        tx.commit().await?;
        if required {
            turnstile(&config, proof, ip).await?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    email: String,
    #[serde(flatten)]
    proof: Proof,
}

pub async fn verify(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Extension(proxies): Extension<auth::TrustedProxies>,
    headers: HeaderMap,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<Value>> {
    let email = body.email.trim().to_lowercase();
    if email.len() > 320 {
        return Err(error(StatusCode::BAD_REQUEST, "auth.login_invalid"));
    }
    let ip = auth::login_ip(peer.ip(), &headers, &proxies).to_string();
    check(&s, &email, &ip, &body.proof, true).await?;
    let token = crate::hosts::runner::secret();
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    let expires_at = db::now() + 120_000;
    save(
        &tx,
        key("verified", &token),
        0,
        Some(binding(&email, &ip)),
        expires_at,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"login_verification_token":token,"expires_at":expires_at}),
    ))
}

pub async fn failed(s: &AppState, email: &str, ip: &str) -> Result<bool> {
    let tx = s.db.begin().await?;
    platform::lock(&tx).await?;
    let now = db::now();
    for id in [key("failed-account", email), key("failed-ip", ip)] {
        let row = current(&tx, &id, now).await?;
        save(
            &tx,
            id,
            row.as_ref()
                .map_or(1, |r| r.count.saturating_add(1).min(1000)),
            None,
            row.map_or(now + WINDOW, |r| r.expires_at),
        )
        .await?;
    }
    let required = is_required(&tx, email, ip, now).await?;
    tx.commit().await?;
    Ok(required)
}
pub async fn succeeded(conn: &impl ConnectionTrait, email: &str) -> Result<()> {
    guards::Entity::delete_by_id(key("failed-account", email))
        .exec(conn)
        .await?;
    // IP failures expire independently: a successful account cannot reset spraying.
    Ok(())
}

#[derive(Deserialize)]
pub struct ChallengeRequest {
    email: String,
}
pub async fn challenge(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Extension(proxies): Extension<auth::TrustedProxies>,
    headers: HeaderMap,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<Value>> {
    let email = body.email.trim().to_lowercase();
    if email.len() > 320 {
        return Err(error(StatusCode::BAD_REQUEST, "auth.login_invalid"));
    }
    let ip = auth::login_ip(peer.ip(), &headers, &proxies).to_string();
    let tx = s.db.begin().await?;
    let row = platform::lock(&tx).await?;
    let now = db::now();
    sweep(&tx, now).await?;
    rate(&tx, key("challenge-ip", &ip), 20, now).await?;
    if !is_required(&tx, &email, &ip, now).await? {
        tx.commit().await?;
        return Ok(Json(json!({"required":false})));
    }
    let config = Configuration::read(&s, row.login_security.as_deref())?;
    if config.mode == "turnstile" {
        tx.commit().await?;
        return Ok(Json(
            json!({"required":true,"kind":"turnstile","site_key":config.site_key}),
        ));
    }
    let generated = captcha::generate();
    let token = crate::hosts::runner::secret();
    let stored = StoredChallenge {
        token_hash: fingerprint(&token),
        answer_hash: answer_hash(&token, &generated.answer),
        kind: generated.kind.into(),
    };
    save(
        &tx,
        format!("captcha:{}:{}", binding(&email, &ip), fingerprint(&token)),
        0,
        Some(serde_json::to_string(&stored).map_err(std::io::Error::other)?),
        now + 300_000,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"required":true,"kind":generated.kind,"id":token,"image":generated.image,"choices":generated.choices,"slider_path":generated.slider_path,"expires_at":now+300_000}),
    ))
}
