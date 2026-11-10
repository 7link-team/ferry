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
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
};
use rand::Rng;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const AAD: &str = "platform/mail/v1";
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CodeTemplate {
    subject: String,
    body: String,
    html: String,
}
impl CodeTemplate {
    fn validate(&self) -> Result<()> {
        validate_template(&self.subject, &self.body, true)?;
        validate_template("", &self.html, true)
    }
}
fn error(key: &str) -> ApiError {
    ApiError::localized(StatusCode::BAD_REQUEST, crate::i18n::Message::new(key))
}
#[derive(Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Configuration {
    pub enabled: bool,
    code_enabled: Option<bool>,
    code_valid_minutes: Option<u32>,
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
    code_html: String,
    test_html: String,
    code_templates: BTreeMap<String, CodeTemplate>,
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
        self.enabled && self.verified_at.is_some() && self.code_enabled.unwrap_or(true)
    }
    fn code_minutes(&self) -> u32 {
        self.code_valid_minutes.unwrap_or(10)
    }
    pub fn view(&self) -> Value {
        json!({"mail_enabled":self.enabled,"mail_code_enabled":self.code_enabled.unwrap_or(true),"mail_code_valid_minutes":self.code_minutes(),"mail_host":self.host,"mail_port":self.port,"mail_security":self.security,"mail_username":self.username,"mail_from":self.from,"mail_has_password":!self.password.is_empty(),"mail_verified_at":self.verified_at,"mail_code_subject":self.code_subject,"mail_code_body":self.code_body,"mail_test_subject":self.test_subject,"mail_test_body":self.test_body,"mail_code_html":self.code_html,"mail_test_html":self.test_html,"mail_code_templates":self.code_templates})
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
            "code_enabled",
            "code_valid_minutes",
            "host",
            "port",
            "security",
            "username",
            "from",
            "code_subject",
            "code_body",
            "test_subject",
            "test_body",
            "code_html",
            "test_html",
            "code_templates",
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
        if !(1..=60).contains(&config.code_minutes()) {
            return Err(error("mail.invalid"));
        }
        config.host = config.host.trim().to_owned();
        config.username = config.username.trim().to_owned();
        config.from = config.from.trim().to_owned();
        validate_template(&config.code_subject, &config.code_body, true)?;
        validate_template(&config.test_subject, &config.test_body, false)?;
        validate_template("", &config.code_html, true)?;
        validate_template("", &config.test_html, false)?;
        for (locale, template) in &config.code_templates {
            if !crate::i18n::Locale::ALL
                .iter()
                .any(|item| item.tag() == locale)
            {
                return Err(error("mail.invalid"));
            }
            template.validate()?;
        }
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
            .multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(html_text(&body)))
                    .singlepart(SinglePart::html(body)),
            )
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
fn render_template(template: &str, email: &str, code: &str, minutes: &str) -> String {
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
            "expires_minutes" => minutes,
            _ => "",
        });
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    out
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn plain_html(value: &str) -> String {
    value
        .lines()
        .map(|line| format!("<p>{}</p>", escape_html(line)))
        .collect::<String>()
}
pub(crate) fn html_text(value: &str) -> String {
    let mut text = String::new();
    let mut inside = false;
    let mut head = false;
    let mut tag = String::new();
    for c in value.chars() {
        if c == '<' {
            inside = true;
            tag.clear();
        } else if inside && c == '>' {
            inside = false;
            if tag.eq_ignore_ascii_case("head") {
                head = true;
            }
            if tag.eq_ignore_ascii_case("/head") {
                head = false;
            }
            if !head && ["/p", "/h1", "/div", "br", "br/", "/tr"].contains(&tag.as_str()) {
                text.push('\n');
            }
        } else if inside {
            tag.push(c);
        } else if !head {
            text.push(c);
        }
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}
impl Configuration {
    fn message(&self, email: &str, code: Option<&str>) -> (String, String) {
        self.message_in(email, code, crate::i18n::current_locale(), None)
    }
    fn message_in(
        &self,
        email: &str,
        code: Option<&str>,
        locale: crate::i18n::Locale,
        sample: Option<&CodeTemplate>,
    ) -> (String, String) {
        let minutes = self.code_minutes().to_string();
        let selected = sample.or_else(|| self.code_templates.get(locale.tag()));
        let (subject, body, subject_key, body_key) = if code.is_some() {
            (
                selected.map_or(&self.code_subject, |template| &template.subject),
                selected.map_or(&self.code_body, |template| &template.body),
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
            crate::i18n::render(locale, &crate::i18n::Message::new(subject_key))
        } else {
            render_template(subject, email, code.unwrap_or(""), &minutes)
        };
        let body = if body.trim().is_empty() {
            crate::i18n::render(
                locale,
                &crate::i18n::Message::new(body_key)
                    .with("code", code.unwrap_or(""))
                    .with("expires_minutes", &minutes),
            )
        } else {
            render_template(body, email, code.unwrap_or(""), &minutes)
        };
        let html = if code.is_some() {
            selected.map_or(&self.code_html, |template| &template.html)
        } else {
            &self.test_html
        };
        let content = if html.trim().is_empty() {
            let content = plain_html(&body);
            if let Some(code) = code.filter(|code| !code.is_empty()) {
                content.replacen(&escape_html(code), &format!("<strong style=\"font-size:28px;letter-spacing:6px;color:#2563eb\">{}</strong>", escape_html(code)), 1)
            } else {
                content
            }
        } else {
            render_template(
                html,
                &escape_html(email),
                &escape_html(code.unwrap_or("")),
                &minutes,
            )
        };
        let document = include_str!("../../../../src/shared/mail/layout.html")
            .replacen("{{title}}", &escape_html(&subject), 1)
            .replacen("{{content}}", &content, 1);
        (subject, document)
    }
}
#[derive(Deserialize)]
pub struct Recipient {
    email: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestRecipient {
    email: String,
    locale: Option<String>,
    template: Option<CodeTemplate>,
}
pub async fn test(
    State(s): State<AppState>,
    admin: Admin,
    Json(body): Json<TestRecipient>,
) -> Result<Json<Value>> {
    let row = platform_state::Entity::find_by_id(1)
        .one(&s.db)
        .await?
        .ok_or_else(ApiError::missing)?;
    let mut config = Configuration::read(&s, row.mail_config.as_deref())?;
    let to = auth::email(&body.email)?;
    let locale = match body.locale {
        Some(tag) => crate::i18n::Locale::ALL
            .into_iter()
            .find(|item| item.tag() == tag)
            .ok_or_else(|| error("mail.invalid"))?,
        None => crate::i18n::current_locale(),
    };
    if let Some(template) = &body.template {
        template.validate()?;
    }
    // A sample never creates a usable registration challenge or a template type.
    let (subject, body) = config.message_in(&to, Some("123456"), locale, body.template.as_ref());
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
        expires_at: Set(now + i64::from(config.code_minutes()) * 60000),
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
    Ok(Json(
        json!({"retry_after":60,"expires_in":config.code_minutes() * 60}),
    ))
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
    fn registration_switch_and_validity_keep_legacy_defaults_and_render_current_configuration() {
        let mut config = Configuration {
            enabled: true,
            verified_at: Some(1),
            ..Configuration::default()
        };
        assert!(config.required());
        assert_eq!(config.code_minutes(), 10);
        config.code_enabled = Some(false);
        assert!(!config.required());
        config.code_enabled = Some(true);
        config.code_valid_minutes = Some(17);
        assert!(config.required());
        for locale in crate::i18n::Locale::ALL {
            let html = config
                .message_in("receiver@example.invalid", Some("123456"), locale, None)
                .1;
            assert!(html.contains("17"));
            assert!(!html.contains("expires_minutes"));
        }
        config.code_html = "<p>{{expires_minutes}}</p>".into();
        assert!(
            config
                .message("receiver@example.invalid", Some("123456"))
                .1
                .contains("<p>17</p>")
        );
        config.verified_at = None;
        assert!(!config.required());
    }

    #[test]
    fn language_templates_are_independent_and_legacy_content_survives() {
        use crate::i18n::Locale;
        let mut config = Configuration {
            code_subject: "Legacy".into(),
            code_body: "Legacy {{code}}".into(),
            ..Configuration::default()
        };
        config.code_templates.insert(
            "zh-CN".into(),
            CodeTemplate {
                subject: "Chinese {{email}}".into(),
                html: "<p>CN {{code}}</p>".into(),
                ..CodeTemplate::default()
            },
        );
        assert_eq!(
            config
                .message_in("a@example.invalid", Some("123456"), Locale::ZhCn, None)
                .0,
            "Chinese a@example.invalid"
        );
        assert!(
            config
                .message_in("a@example.invalid", Some("123456"), Locale::ZhCn, None)
                .1
                .contains("CN 123456")
        );
        assert_eq!(
            config
                .message_in("a@example.invalid", Some("654321"), Locale::EnUs, None)
                .0,
            "Legacy"
        );
        config
            .code_templates
            .insert("en-US".into(), CodeTemplate::default());
        let reset = config.message_in("a@example.invalid", Some("654321"), Locale::EnUs, None);
        assert_eq!(
            reset.0,
            crate::i18n::render(Locale::EnUs, &crate::i18n::Message::new("mail.codeSubject"))
        );
        assert!(!reset.1.contains("Legacy"));
        let sample = CodeTemplate {
            subject: "Unsaved".into(),
            html: "<p>{{email}} {{code}} {{expires_minutes}}</p>".into(),
            ..CodeTemplate::default()
        };
        let sent = config.message_in(
            "<unsafe>@example.invalid",
            Some("123456"),
            Locale::ZhCn,
            Some(&sample),
        );
        assert_eq!(sent.0, "Unsaved");
        assert!(sent.1.contains("&lt;unsafe&gt;@example.invalid 123456 10"));
        assert_eq!(config.code_templates["zh-CN"].subject, "Chinese {{email}}");
        let old: Configuration =
            serde_json::from_str(r#"{"code_subject":"Old","code_body":"{{code}}"}"#).unwrap();
        assert!(old.code_templates.is_empty());
        assert_eq!(old.code_subject, "Old");
    }
    #[test]
    fn html_mail_has_default_design_plain_fallback_and_escaped_variables() {
        let config = Configuration::default();
        let (_, html) = config.message("recipient@example.invalid", Some("123456"));
        assert!(html.contains("max-width:600px"));
        assert!(html.contains("123456"));
        assert!(!html.contains("{{content}}"));
        assert!(html_text(&html).contains("123456"));
        assert!(!html_text(&html).contains("<table"));
        let custom = Configuration {
            test_subject: "<title>".into(),
            test_html: "<h2>Hello {{email}}</h2>".into(),
            ..Configuration::default()
        };
        let (_, html) = custom.message("<unsafe>@example.invalid", None);
        assert!(html.contains("&lt;title&gt;"));
        assert!(html.contains("Hello &lt;unsafe&gt;@example.invalid"));
        assert!(!html.contains("Hello <unsafe>"));
        let legacy = Configuration {
            test_body: "<literal> old plain text".into(),
            ..Configuration::default()
        };
        assert!(
            legacy
                .message("a@example.invalid", None)
                .1
                .contains("&lt;literal&gt; old plain text")
        );
    }
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
                "123456",
                "10"
            ),
            "{{code}}@example.com 123456 10"
        );
    }
}
