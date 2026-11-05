//! Platform SMTP configuration and shared, single-use registration challenges.
use super::{
    auth,
    platform::{self, Admin},
};
use crate::{
    db,
    entities::{email_codes, platform_state},
    error::{ApiError, Json, Result},
    state::AppState,
};
use axum::{
    Extension,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use rand::Rng;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

const AAD: &str = "platform/mail/v1";
fn error(key: &str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new(key))
}
#[derive(Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Configuration {
    pub enabled: bool,
    host: String,
    port: u16,
    security: String,
    username: String,
    password: String,
    from: String,
    pub verified_at: Option<i64>,
    tag: String,
    code_subject: String,
    code_body: String,
    test_subject: String,
    test_body: String,
}
impl Configuration {
    pub fn read(s: &AppState, stored: Option<&str>) -> Result<Self> {
        match stored {
            Some(value) => serde_json::from_str(&s.vault.open_secret(value, AAD)?)
                .map_err(|_| error("mail.invalid")),
            None => Ok(Self {
                port: 587,
                security: "starttls".into(),
                ..Self::default()
            }),
        }
    }
    pub fn required(&self) -> bool {
        self.enabled && self.verified_at.is_some()
    }
    pub fn view(&self) -> Value {
        json!({"mail_enabled":self.enabled,"mail_host":self.host,"mail_port":self.port,"mail_security":self.security,"mail_username":self.username,"mail_from":self.from,"mail_has_password":!self.password.is_empty(),"mail_verified_at":self.verified_at,"mail_code_subject":self.code_subject,"mail_code_body":self.code_body,"mail_test_subject":self.test_subject,"mail_test_body":self.test_body})
    }
    fn seal(&self, s: &AppState) -> Result<String> {
        s.vault.seal(
            &serde_json::to_string(self).map_err(std::io::Error::other)?,
            AAD,
        )
    }
    pub fn update(
        s: &AppState,
        stored: Option<&str>,
        patch: &Map<String, Value>,
    ) -> Result<Option<String>> {
        let mut config = Self::read(s, stored)?;
        let previous = serde_json::to_value(&config).map_err(std::io::Error::other)?;
        let mut fields = previous.clone();
        for key in [
            "enabled",
            "host",
            "port",
            "security",
            "username",
            "from",
            "code_subject",
            "code_body",
            "test_subject",
            "test_body",
        ] {
            if let Some(value) = patch.get(&format!("mail_{key}")) {
                fields[key] = value.clone();
            }
        }
        if let Some(value) = patch.get("mail_password") {
            if !value.is_string() {
                return Err(error("mail.invalid"));
            }
            if !value.as_str().unwrap().is_empty() {
                fields["password"] = value.clone();
            }
        }
        config = serde_json::from_value(fields).map_err(|_| error("mail.invalid"))?;
        config.host = config.host.trim().to_owned();
        config.username = config.username.trim().to_owned();
        config.from = config.from.trim().to_owned();
        validate_template(&config.code_subject, &config.code_body, true)?;
        validate_template(&config.test_subject, &config.test_body, false)?;
        if config.host.len() > 253
            || config
                .host
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b".:-[]".contains(&b))
            || config.port == 0
            || !["tls", "starttls", "none"].contains(&config.security.as_str())
            || config.username.len() > 320
            || config.password.len() > 4096
            || config.from.len() > 320
            || (!config.from.is_empty() && config.from.parse::<Mailbox>().is_err())
            || (config.enabled && (config.host.is_empty() || config.from.is_empty()))
        {
            return Err(error("mail.invalid"));
        }
        let changed = ["host", "port", "security", "username", "password", "from"]
            .iter()
            .any(|key| previous[*key] != json!(&config)[*key]);
        if changed {
            config.verified_at = None;
            config.tag = crate::hosts::runner::secret();
        }
        if json!(&config) == previous {
            return Ok(stored.map(str::to_owned));
        }
        Ok(Some(config.seal(s)?))
    }
    async fn send(&self, to: &str, subject: String, body: String) -> Result<()> {
        let from = self
            .from
            .parse::<Mailbox>()
            .map_err(|_| error("mail.invalid"))?;
        let to = to
            .parse::<Mailbox>()
            .map_err(|_| error("auth.email_invalid"))?;
        let message = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .body(body)
            .map_err(|_| error("mail.invalid"))?;
        if self.host.is_empty() {
            return Err(error("mail.invalid"));
        }
        let builder = match self.security.as_str() {
            "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&self.host)
                .map_err(|_| error("mail.invalid"))?,
            "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)
                .map_err(|_| error("mail.invalid"))?,
            "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.host),
            _ => return Err(error("mail.invalid")),
        }
        .port(self.port)
        .timeout(Some(std::time::Duration::from_secs(15)));
        let builder = if self.username.is_empty() {
            builder
        } else {
            builder.credentials(Credentials::new(
                self.username.clone(),
                self.password.clone(),
            ))
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            builder.build().send(message),
        )
        .await
        .map_err(|_| error("mail.sendFailed"))?
        .map_err(|_| error("mail.sendFailed"))?;
        Ok(())
    }
}
fn validate_template(subject: &str, body: &str, code: bool) -> Result<()> {
    if subject.chars().count() > 200
        || subject.contains(['\r', '\n', '\0'])
        || body.len() > 16_384
        || body.contains('\0')
    {
        return Err(error("mail.invalid"));
    }
    for value in [subject, body] {
        let mut rest = value;
        while let Some(start) = rest.find("{{") {
            let Some(end) = rest[start + 2..].find("}}") else {
                return Err(error("mail.invalid"));
            };
            let key = &rest[start + 2..start + 2 + end];
            if !matches!(key, "email" | "expires_minutes") && !(code && key == "code") {
                return Err(error("mail.invalid"));
            }
            rest = &rest[start + 2 + end + 2..];
        }
        if rest.contains("}}") {
            return Err(error("mail.invalid"));
        }
    }
    if code && !body.trim().is_empty() && !body.contains("{{code}}") {
        return Err(error("mail.invalid"));
    }
    Ok(())
}
fn render_template(template: &str, email: &str, code: &str) -> String {
    // Single pass: inserted user data is never treated as another template.
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(match &rest[start + 2..start + 2 + end] {
            "email" => email,
            "code" => code,
            "expires_minutes" => "10",
            _ => "",
        });
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    out
}
impl Configuration {
    fn message(&self, email: &str, code: Option<&str>) -> (String, String) {
        let (subject, body, subject_key, body_key) = if code.is_some() {
            (
                &self.code_subject,
                &self.code_body,
                "mail.codeSubject",
                "mail.codeBody",
            )
        } else {
            (
                &self.test_subject,
                &self.test_body,
                "mail.testSubject",
                "mail.testBody",
            )
        };
        let subject = if subject.trim().is_empty() {
            crate::i18n::message(subject_key)
        } else {
            render_template(subject, email, code.unwrap_or(""))
        };
        let body = if body.trim().is_empty() {
            crate::i18n::render(
                crate::i18n::current_locale(),
                &crate::i18n::Message::new(body_key).with("code", code.unwrap_or("")),
            )
        } else {
            render_template(body, email, code.unwrap_or(""))
        };
        (subject, body)
    }
}
#[derive(Deserialize)]
pub struct Recipient {
    email: String,
}
pub async fn test(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<Recipient>,
) -> Result<Json<Value>> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut config = Configuration::read(&s, row.mail_config.as_deref())?;
    let to = auth::email(&body.email)?;
    let (subject, body) = config.message(&to, None);
    config.send(&to, subject, body).await?;
    let tx = s.db.begin().await?;
    let latest = platform::lock(&tx).await?;
    platform::check_admin(&tx, &admin).await?;
    if latest.mail_config != row.mail_config {
        return Err(error("platform.settingsChanged"));
    }
    config.verified_at = Some(db::now());
    platform_state::Entity::update_many()
        .col_expr(
            platform_state::Column::MailConfig,
            Expr::value(config.seal(&s)?),
        )
        .col_expr(
            platform_state::Column::Version,
            Expr::value(latest.version + 1),
        )
        .filter(platform_state::Column::Id.eq(1))
        .exec(&tx)
        .await?;
    platform::audit(
        &s,
        &tx,
        &admin.0.user,
        "test",
        "platform-mail",
        1.into(),
        json!({"success":true}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"mail_verified_at":config.verified_at,"version":latest.version+1}),
    ))
}
async fn rate(conn: &impl ConnectionTrait, id: String, limit: i64, now: i64) -> Result<()> {
    if let Some(row) = email_codes::Entity::find_by_id(&id).one(conn).await? {
        if row.count >= limit {
            return Err(ApiError::localized(
                StatusCode::TOO_MANY_REQUESTS,
                crate::i18n::Message::new("mail.rateLimit"),
            ));
        }
        email_codes::Entity::update_many()
            .col_expr(email_codes::Column::Count, Expr::value(row.count + 1))
            .filter(email_codes::Column::Id.eq(id))
            .exec(conn)
            .await?;
    } else {
        email_codes::ActiveModel {
            id: Set(id),
            code_hash: Set(String::new()),
            count: Set(1),
            expires_at: Set(now + 3600000),
            updated_at: Set(now),
            config_tag: Set(String::new()),
        }
        .insert(conn)
        .await?;
    }
    Ok(())
}
pub async fn request_code(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Extension(proxies): Extension<auth::TrustedProxies>,
    headers: HeaderMap,
    Json(body): Json<Recipient>,
) -> Result<Json<Value>> {
    let email = auth::email(&body.email)?;
    if email.parse::<Mailbox>().is_err() {
        return Err(error("auth.email_invalid"));
    }
    let tx = s.db.begin().await?;
    let row = platform::lock(&tx).await?;
    let config = Configuration::read(&s, row.mail_config.as_deref())?;
    if row.initialized == 0 || row.registration_enabled == 0 || !config.required() {
        return Err(ApiError::forbidden());
    }
    let now = db::now();
    email_codes::Entity::delete_many()
        .filter(email_codes::Column::ExpiresAt.lt(now))
        .exec(&tx)
        .await?;
    let id = format!("code:{email}");
    if email_codes::Entity::find_by_id(&id)
        .one(&tx)
        .await?
        .is_some_and(|r| r.updated_at > now - 60000)
    {
        return Err(ApiError::localized(
            StatusCode::TOO_MANY_REQUESTS,
            crate::i18n::Message::new("mail.rateLimit"),
        ));
    }
    let ip = auth::login_ip(peer.ip(), &headers, &proxies);
    rate(&tx, format!("rate:ip:{ip}:{}", now / 3600000), 20, now).await?;
    rate(
        &tx,
        format!("rate:email:{email}:{}", now / 3600000),
        10,
        now,
    )
    .await?;
    let code = format!("{:06}", rand::thread_rng().gen_range(0..1000000));
    let hash = auth::hash(&code)?;
    email_codes::Entity::delete_by_id(&id).exec(&tx).await?;
    email_codes::ActiveModel {
        id: Set(id.clone()),
        code_hash: Set(hash.clone()),
        count: Set(0),
        expires_at: Set(now + 600000),
        updated_at: Set(now),
        config_tag: Set(config.tag.clone()),
    }
    .insert(&tx)
    .await?;
    tx.commit().await?;
    let (subject, body) = config.message(&email, Some(&code));
    if let Err(e) = config.send(&email, subject, body).await {
        email_codes::Entity::delete_many()
            .filter(email_codes::Column::Id.eq(id))
            .filter(email_codes::Column::CodeHash.eq(hash))
            .exec(&s.db)
            .await?;
        return Err(e);
    }
    Ok(Json(json!({"retry_after":60,"expires_in":600})))
}
/// Called under the same singleton lock/transaction as user creation.
pub async fn consume(
    conn: &impl ConnectionTrait,
    config: &Configuration,
    email: &str,
    code: &str,
) -> Result<bool> {
    if !config.required() {
        return Ok(true);
    }
    let id = format!("code:{email}");
    let Some(row) = email_codes::Entity::find_by_id(&id).one(conn).await? else {
        return Ok(false);
    };
    if row.expires_at <= db::now() || row.count >= 5 || row.config_tag != config.tag {
        return Ok(false);
    }
    if code.len() != 6
        || !code.bytes().all(|b| b.is_ascii_digit())
        || !auth::verify(code, &row.code_hash)
    {
        email_codes::Entity::update_many()
            .col_expr(email_codes::Column::Count, Expr::value(row.count + 1))
            .filter(email_codes::Column::Id.eq(id))
            .exec(conn)
            .await?;
        return Ok(false);
    }
    email_codes::Entity::delete_by_id(id).exec(conn).await?;
    Ok(true)
}

#[cfg(test)]
mod template_tests {
    use super::*;
    #[test]
    fn templates_require_code_and_reject_header_injection_or_unknown_variables() {
        assert!(validate_template("Subject", "Your code: {{code}}", true).is_ok());
        assert!(validate_template("Subject", "Missing code", true).is_err());
        assert!(validate_template("Subject\r\nBcc: injected", "{{code}}", true).is_err());
        assert!(validate_template("Subject", "{{code}} {{password}}", true).is_err());
        assert!(validate_template("Subject", "{{code}}", false).is_err());
        assert!(validate_template("Subject", &"x".repeat(16385), false).is_err());
        assert_eq!(
            render_template(
                "{{email}} {{code}} {{expires_minutes}}",
                "{{code}}@example.com",
                "123456"
            ),
            "{{code}}@example.com 123456 10"
        );
    }
}
