//! Explicit application messages. Provider output, user data, and logs are never translated.
use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Locale {
    #[default]
    EnUs,
    ZhCn,
    ZhTw,
    JaJp,
    KoKr,
    MsMy,
}

impl Locale {
    pub const ALL: [Self; 6] = [
        Self::EnUs,
        Self::ZhCn,
        Self::ZhTw,
        Self::JaJp,
        Self::KoKr,
        Self::MsMy,
    ];

    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
            Self::ZhTw => "zh-TW",
            Self::JaJp => "ja-JP",
            Self::KoKr => "ko-KR",
            Self::MsMy => "ms-MY",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let tag = tag.to_ascii_lowercase();
        let parts: Vec<_> = tag.split('-').collect();
        // Reject malformed ranges rather than accidentally matching arbitrary text.
        if parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_alphanumeric()))
        {
            return None;
        }
        match parts[0] {
            "en" => Some(Self::EnUs),
            "zh" if parts.contains(&"hant")
                || parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) =>
            {
                Some(Self::ZhTw)
            }
            "zh" => Some(Self::ZhCn),
            "ja" => Some(Self::JaJp),
            "ko" => Some(Self::KoKr),
            "ms" => Some(Self::MsMy),
            _ => None,
        }
    }

    pub fn negotiate(headers: &HeaderMap) -> Self {
        let mut candidates = Vec::new();
        for value in headers.get_all(header::ACCEPT_LANGUAGE) {
            let Ok(value) = value.to_str() else { continue };
            for range in value.split(',') {
                let mut parts = range.trim().split(';');
                let tag = parts.next().unwrap_or("").trim();
                let mut quality = 1.0_f32;
                let mut valid = true;
                let mut has_quality = false;
                for parameter in parts {
                    let Some((name, value)) = parameter.trim().split_once('=') else {
                        valid = false;
                        break;
                    };
                    if !name.eq_ignore_ascii_case("q") || has_quality {
                        valid = false;
                        break;
                    }
                    has_quality = true;
                    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
                    if !matches!(whole, "0" | "1")
                        || fraction.len() > 3
                        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
                        || (whole == "1" && fraction.bytes().any(|byte| byte != b'0'))
                    {
                        valid = false;
                        break;
                    }
                    match value.parse::<f32>() {
                        Ok(q) if q.is_finite() && (0.0..=1.0).contains(&q) => quality = q,
                        _ => valid = false,
                    }
                }
                if valid && quality > 0.0 {
                    if let Some(locale) = Self::from_tag(tag) {
                        candidates.push((quality, locale));
                    }
                }
            }
        }
        // Stable sorting preserves the client's order for equal quality values.
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        candidates
            .first()
            .map(|(_, locale)| *locale)
            .unwrap_or_default()
    }
}

tokio::task_local! { static REQUEST_LOCALE: Locale; }

/// Task-local scope follows awaits but is deliberately not inherited by spawned jobs.
pub fn current_locale() -> Locale {
    REQUEST_LOCALE
        .try_with(|locale| *locale)
        .unwrap_or_default()
}

/// Render an application error read from persistent storage for this reader.
pub fn display_stored(value: &str) -> String {
    render_stored(current_locale(), value)
}

/// Explicitly carry a request locale into an upgraded connection or other future.
/// Calling tokio::spawn without this scope still uses the English default.
pub async fn with_locale<F: std::future::Future>(locale: Locale, future: F) -> F::Output {
    REQUEST_LOCALE.scope(locale, future).await
}

/// Kept separate from ApiError's display string so arbitrary diagnostics are never
/// treated as message keys. Render explicitly when crossing an application boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub key: String,
    pub params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, Message>,
}

impl Message {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            params: BTreeMap::new(),
            children: BTreeMap::new(),
        }
    }

    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }
    pub fn with_message(mut self, name: impl Into<String>, message: Message) -> Self {
        self.children.insert(name.into(), message);
        self
    }
}

type Catalog = BTreeMap<String, String>;
static CATALOGS: LazyLock<[Catalog; 6]> = LazyLock::new(|| {
    let mut catalogs: [Catalog; 6] = [
        include_str!("i18n/en-US.json"),
        include_str!("i18n/zh-CN.json"),
        include_str!("i18n/zh-TW.json"),
        include_str!("i18n/ja-JP.json"),
        include_str!("i18n/ko-KR.json"),
        include_str!("i18n/ms-MY.json"),
    ]
    .map(|source| serde_json::from_str(source).expect("valid built-in message catalog"));
    for domain in [
        [
            include_str!("i18n/monitoring-en-US.json"),
            include_str!("i18n/monitoring-zh-CN.json"),
            include_str!("i18n/monitoring-zh-TW.json"),
            include_str!("i18n/monitoring-ja-JP.json"),
            include_str!("i18n/monitoring-ko-KR.json"),
            include_str!("i18n/monitoring-ms-MY.json"),
        ],
        [
            include_str!("i18n/models-en-US.json"),
            include_str!("i18n/models-zh-CN.json"),
            include_str!("i18n/models-zh-TW.json"),
            include_str!("i18n/models-ja-JP.json"),
            include_str!("i18n/models-ko-KR.json"),
            include_str!("i18n/models-ms-MY.json"),
        ],
        [
            include_str!("i18n/conversations-en-US.json"),
            include_str!("i18n/conversations-zh-CN.json"),
            include_str!("i18n/conversations-zh-TW.json"),
            include_str!("i18n/conversations-ja-JP.json"),
            include_str!("i18n/conversations-ko-KR.json"),
            include_str!("i18n/conversations-ms-MY.json"),
        ],
        [
            include_str!("i18n/holes-en-US.json"),
            include_str!("i18n/holes-zh-CN.json"),
            include_str!("i18n/holes-zh-TW.json"),
            include_str!("i18n/holes-ja-JP.json"),
            include_str!("i18n/holes-ko-KR.json"),
            include_str!("i18n/holes-ms-MY.json"),
        ],
        [
            include_str!("i18n/agents-en-US.json"),
            include_str!("i18n/agents-zh-CN.json"),
            include_str!("i18n/agents-zh-TW.json"),
            include_str!("i18n/agents-ja-JP.json"),
            include_str!("i18n/agents-ko-KR.json"),
            include_str!("i18n/agents-ms-MY.json"),
        ],
        [
            include_str!("i18n/auth-en-US.json"),
            include_str!("i18n/auth-zh-CN.json"),
            include_str!("i18n/auth-zh-TW.json"),
            include_str!("i18n/auth-ja-JP.json"),
            include_str!("i18n/auth-ko-KR.json"),
            include_str!("i18n/auth-ms-MY.json"),
        ],
        [
            include_str!("i18n/repository-en-US.json"),
            include_str!("i18n/repository-zh-CN.json"),
            include_str!("i18n/repository-zh-TW.json"),
            include_str!("i18n/repository-ja-JP.json"),
            include_str!("i18n/repository-ko-KR.json"),
            include_str!("i18n/repository-ms-MY.json"),
        ],
        [
            include_str!("i18n/runner-en-US.json"),
            include_str!("i18n/runner-zh-CN.json"),
            include_str!("i18n/runner-zh-TW.json"),
            include_str!("i18n/runner-ja-JP.json"),
            include_str!("i18n/runner-ko-KR.json"),
            include_str!("i18n/runner-ms-MY.json"),
        ],
        [
            include_str!("i18n/execution-en-US.json"),
            include_str!("i18n/execution-zh-CN.json"),
            include_str!("i18n/execution-zh-TW.json"),
            include_str!("i18n/execution-ja-JP.json"),
            include_str!("i18n/execution-ko-KR.json"),
            include_str!("i18n/execution-ms-MY.json"),
        ],
        [
            include_str!("i18n/deployment-en-US.json"),
            include_str!("i18n/deployment-zh-CN.json"),
            include_str!("i18n/deployment-zh-TW.json"),
            include_str!("i18n/deployment-ja-JP.json"),
            include_str!("i18n/deployment-ko-KR.json"),
            include_str!("i18n/deployment-ms-MY.json"),
        ],
        [
            include_str!("i18n/reports-en-US.json"),
            include_str!("i18n/reports-zh-CN.json"),
            include_str!("i18n/reports-zh-TW.json"),
            include_str!("i18n/reports-ja-JP.json"),
            include_str!("i18n/reports-ko-KR.json"),
            include_str!("i18n/reports-ms-MY.json"),
        ],
        [
            include_str!("i18n/reviews-en-US.json"),
            include_str!("i18n/reviews-zh-CN.json"),
            include_str!("i18n/reviews-zh-TW.json"),
            include_str!("i18n/reviews-ja-JP.json"),
            include_str!("i18n/reviews-ko-KR.json"),
            include_str!("i18n/reviews-ms-MY.json"),
        ],
    ] {
        for (catalog, source) in catalogs.iter_mut().zip(domain) {
            let domain: Catalog =
                serde_json::from_str(source).expect("valid account/workspace catalog");
            for (key, value) in domain {
                assert!(
                    catalog.insert(key, value).is_none(),
                    "duplicate message key"
                );
            }
        }
    }
    catalogs
});

pub fn render(locale: Locale, message: &Message) -> String {
    render_message(locale, message, 1)
}

fn render_message(locale: Locale, message: &Message, depth: usize) -> String {
    if depth > MAX_MESSAGE_DEPTH {
        return message.key.clone();
    }
    let index = Locale::ALL
        .iter()
        .position(|candidate| *candidate == locale)
        .unwrap();
    let template = CATALOGS[index]
        .get(&message.key)
        .or_else(|| CATALOGS[0].get(&message.key))
        .map(String::as_str)
        .unwrap_or(&message.key);
    let mut params = message.params.clone();
    for (name, child) in &message.children {
        params.insert(name.clone(), render_message(locale, child, depth + 1));
    }
    interpolate(template, &params)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredEnvelope {
    #[serde(rename = "$system_message")]
    system_message: StoredMessage,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredMessage {
    version: u32,
    message: Message,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    fallback: String,
}

const MAX_STORED_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_MESSAGE_DEPTH: usize = 16;
const MAX_MESSAGE_NODES: usize = 256;
const RAW_MESSAGE_PREFIX: &str = "\u{001e}system-message-raw:v1:";

/// Escape only raw strings that use a reserved storage namespace. Decoding this
/// wrapper once returns the original string, even if it resembles another envelope.
pub fn store_raw(value: &str) -> String {
    let collision = value.starts_with(RAW_MESSAGE_PREFIX)
        || (value.len() <= MAX_STORED_TEXT_BYTES
            && serde_json::from_str::<serde_json::Value>(value)
                .ok()
                .is_some_and(|value| {
                    value.as_object().is_some_and(|object| {
                        object.contains_key("$system_message")
                            || object.contains_key("$system_text")
                    })
                }));
    if !collision {
        return value.to_owned();
    }
    format!("{RAW_MESSAGE_PREFIX}{value}")
}

fn known_message(message: &Message) -> bool {
    validate_message(message, 1, &mut 0)
}

fn validate_message(message: &Message, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if depth > MAX_MESSAGE_DEPTH
        || *nodes > MAX_MESSAGE_NODES
        || message
            .children
            .keys()
            .any(|name| message.params.contains_key(name))
    {
        return false;
    }
    let Some(template) = CATALOGS[0].get(&message.key) else {
        return false;
    };
    let expected: std::collections::BTreeSet<_> = template
        .split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect();
    let actual: std::collections::BTreeSet<_> = message
        .params
        .keys()
        .chain(message.children.keys())
        .map(String::as_str)
        .collect();
    expected == actual
        && message
            .children
            .values()
            .all(|child| validate_message(child, depth + 1, nodes))
}

/// Explicit compact storage for an application-generated message. Older envelopes
/// with an English fallback remain readable. Never wrap raw diagnostics or user data.
pub fn store_message(message: &Message) -> String {
    let fallback = render(Locale::EnUs, message);
    if !known_message(message) {
        return store_raw(&fallback);
    }
    let stored = serde_json::to_string(&StoredEnvelope {
        system_message: StoredMessage {
            version: 1,
            message: message.clone(),
            fallback: String::new(),
        },
    })
    .expect("system message serializes");
    if stored.len() > MAX_STORED_MESSAGE_BYTES {
        return store_raw(&render(Locale::EnUs, message));
    }
    stored
}

/// Only opt in for columns that hold application errors, never arbitrary JSON or
/// user data. Legacy strings and unrecognized versions/identities stay untouched.
pub fn decode_stored(value: &str) -> Option<Message> {
    if value.len() > MAX_STORED_MESSAGE_BYTES {
        return None;
    }
    let envelope: StoredEnvelope = serde_json::from_str(value).ok()?;
    let stored = envelope.system_message;
    (stored.version == 1 && known_message(&stored.message)).then_some(stored.message)
}

pub fn render_stored(locale: Locale, value: &str) -> String {
    if let Some(raw) = value.strip_prefix(RAW_MESSAGE_PREFIX) {
        return raw.to_owned();
    }
    if let Some(text) = StoredText::decode(value) {
        return text.render(locale);
    }
    decode_stored(value)
        .map(|message| render(locale, &message))
        .unwrap_or_else(|| value.to_owned())
}

const MAX_TEXT_RAW_BYTES: usize = 1_048_576;
const MAX_TEXT_SEGMENTS: usize = 4096;
// Raw control characters can expand sixfold when encoded as JSON. Metadata is
// bounded independently so a large literal log does not consume its budget.
const MAX_STORED_TEXT_BYTES: usize = 8 * 1_048_576;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum TextSegment {
    Raw(String),
    Message(Message),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTextEnvelope {
    #[serde(rename = "$system_text")]
    text: TextContent,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TextContent {
    version: u32,
    segments: Vec<TextSegment>,
    truncated: bool,
}

/// Mixed application output. Only explicitly supplied messages are translated;
/// stdout, stderr, notes and legacy output are literal segments. This type does
/// not capture a locale and can therefore cross background/storage boundaries.
#[derive(Clone, Debug, Default)]
pub struct StoredText {
    segments: Vec<TextSegment>,
    raw_bytes: usize,
    metadata_bytes: usize,
    truncated: bool,
}

impl StoredText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_message(message: Message) -> Self {
        let mut text = Self::new();
        text.push_message(message);
        text
    }

    /// Log columns only recognize the new text envelope. An old raw log that
    /// resembles a single-message envelope must remain literal, byte for byte.
    pub fn from_stored(value: &str) -> Self {
        if let Some(raw) = value.strip_prefix(RAW_MESSAGE_PREFIX) {
            return Self::legacy(raw);
        }
        Self::decode(value).unwrap_or_else(|| Self::legacy(value))
    }

    fn legacy(value: &str) -> Self {
        Self {
            segments: vec![TextSegment::Raw(value.to_owned())],
            raw_bytes: value.len(),
            ..Self::default()
        }
    }

    fn decode(value: &str) -> Option<Self> {
        if value.len() > MAX_STORED_TEXT_BYTES || !value.starts_with('{') {
            return None;
        }
        let envelope: StoredTextEnvelope = serde_json::from_str(value).ok()?;
        let content = envelope.text;
        if content.version != 1 || content.segments.len() > MAX_TEXT_SEGMENTS {
            return None;
        }
        let mut raw_bytes = 0usize;
        let mut metadata_bytes = 0usize;
        for segment in &content.segments {
            match segment {
                TextSegment::Raw(raw) => raw_bytes = raw_bytes.checked_add(raw.len())?,
                TextSegment::Message(message) => {
                    if !known_message(message) {
                        return None;
                    }
                    metadata_bytes =
                        metadata_bytes.checked_add(serde_json::to_vec(message).ok()?.len())?;
                }
            }
            if raw_bytes > MAX_TEXT_RAW_BYTES || metadata_bytes > MAX_STORED_MESSAGE_BYTES {
                return None;
            }
        }
        Some(Self {
            segments: content.segments,
            raw_bytes,
            metadata_bytes,
            truncated: content.truncated,
        })
    }

    pub fn push_raw(&mut self, raw: impl Into<String>) {
        let mut raw = raw.into();
        let remaining = MAX_TEXT_RAW_BYTES.saturating_sub(self.raw_bytes);
        if raw.len() > remaining {
            truncate_utf8(&mut raw, remaining);
            self.truncated = true;
        }
        if raw.is_empty() {
            return;
        }
        if let Some(TextSegment::Raw(previous)) = self.segments.last_mut() {
            self.raw_bytes += raw.len();
            previous.push_str(&raw);
        } else if self.segments.len() < MAX_TEXT_SEGMENTS {
            self.raw_bytes += raw.len();
            self.segments.push(TextSegment::Raw(raw));
        } else {
            self.truncated = true;
        }
    }

    pub fn push_message(&mut self, message: Message) {
        if !known_message(&message) {
            self.push_raw(render(Locale::EnUs, &message));
            return;
        }
        let size = serde_json::to_vec(&message)
            .expect("message serializes")
            .len();
        if size > MAX_STORED_MESSAGE_BYTES.saturating_sub(self.metadata_bytes)
            || self.segments.len() >= MAX_TEXT_SEGMENTS
        {
            self.truncated = true;
            return;
        }
        self.metadata_bytes += size;
        self.segments.push(TextSegment::Message(message));
    }

    /// Append another explicitly decoded output without rendering its messages.
    pub fn extend(&mut self, text: Self) {
        self.truncated |= text.truncated;
        for segment in text.segments {
            match segment {
                TextSegment::Raw(raw) => self.push_raw(raw),
                TextSegment::Message(message) => self.push_message(message),
            }
        }
    }

    /// Apply an existing, possibly smaller log budget to literal bytes only.
    /// Messages are never truncated. A translated limit notice is added at
    /// render time; callers need not append a second notice.
    pub fn truncate_raw(&mut self, max_bytes: usize) -> bool {
        let mut remaining = max_bytes.min(MAX_TEXT_RAW_BYTES);
        self.raw_bytes = 0;
        for segment in &mut self.segments {
            if let TextSegment::Raw(raw) = segment {
                if raw.len() > remaining {
                    truncate_utf8(raw, remaining);
                    self.truncated = true;
                }
                remaining -= raw.len();
                self.raw_bytes += raw.len();
            }
        }
        self.truncated
    }

    pub fn stored(&self) -> String {
        let mut bounded = self.clone();
        bounded.truncate_raw(MAX_TEXT_RAW_BYTES);
        serde_json::to_string(&StoredTextEnvelope {
            text: TextContent {
                version: 1,
                segments: bounded.segments,
                truncated: bounded.truncated,
            },
        })
        .expect("mixed output serializes")
    }

    pub fn render(&self, locale: Locale) -> String {
        let mut output = String::new();
        for segment in &self.segments {
            match segment {
                TextSegment::Raw(raw) => output.push_str(raw),
                TextSegment::Message(message) => output.push_str(&render(locale, message)),
            }
        }
        if self.truncated {
            output.push_str(&render(locale, &Message::new("output.truncated")));
        }
        output
    }
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

pub fn render_text_stored(locale: Locale, value: &str) -> String {
    StoredText::from_stored(value).render(locale)
}

pub fn display_text_stored(value: &str) -> String {
    render_text_stored(current_locale(), value)
}

// Replace tokens in a single pass: substituted data is never interpreted as a template.
fn interpolate(template: &str, params: &BTreeMap<String, String>) -> String {
    let mut result = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        result.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find('}').map(|end| start + end) else {
            result.push_str(&remaining[start..]);
            return result;
        };
        let name = &remaining[start + 1..end];
        result.push_str(
            params
                .get(name)
                .map(String::as_str)
                .unwrap_or(&remaining[start..=end]),
        );
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    result
}

pub fn message(key: &'static str) -> String {
    render(current_locale(), &Message::new(key))
}

fn request_locale(request: &Request) -> Locale {
    let header_locale = Locale::negotiate(request.headers());
    // Browser WebSocket constructors cannot set Accept-Language. The exception
    // is restricted to this browser terminal route, never regular HTTP APIs.
    if request.method() != axum::http::Method::GET
        || !request
            .headers()
            .get(header::UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    {
        return header_locale;
    }
    let segments: Vec<_> = request
        .uri()
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect();
    if !matches!(segments.as_slice(), ["api", "workspaces", workspace, "nodes", node, "terminal-sessions"] if !workspace.is_empty() && !node.is_empty())
    {
        return header_locale;
    }
    axum::extract::Query::<std::collections::HashMap<String, String>>::try_from_uri(request.uri())
        .ok()
        .and_then(|query| query.get("locale").and_then(|tag| Locale::from_tag(tag)))
        .unwrap_or(header_locale)
}

pub async fn middleware(mut request: Request, next: Next) -> Response {
    let locale = request_locale(&request);
    request.extensions_mut().insert(locale);
    let mut response = with_locale(locale, next.run(request)).await;
    response.headers_mut().insert(
        header::CONTENT_LANGUAGE,
        HeaderValue::from_static(locale.tag()),
    );
    let varies_by_language = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim() == "*" || value.trim().eq_ignore_ascii_case("accept-language"));
    if !varies_by_language {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Accept-Language"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_output_retains_large_literal_logs_and_renders_only_explicit_messages() {
        let raw = "stdout 中文 한국어 日本語 {detail}\n\"\\\0".repeat(10_000);
        assert!(raw.len() > MAX_STORED_MESSAGE_BYTES);
        assert!(raw.len() < MAX_TEXT_RAW_BYTES);
        let message = Message::new("alerts.noticeTriggered").with("name", "user 原文 {name}");
        let mut text = StoredText::new();
        text.push_message(message.clone());
        text.push_raw(raw.clone());
        text.extend(StoredText::from_message(Message::new("error.forbidden")));
        let stored = text.stored();
        assert!(stored.len() > MAX_STORED_MESSAGE_BYTES);
        assert!(stored.len() < MAX_STORED_TEXT_BYTES);
        assert!(StoredText::decode(&stored).is_some());
        for locale in Locale::ALL {
            let expected = format!(
                "{}{}{}",
                render(locale, &message),
                raw,
                render(locale, &Message::new("error.forbidden"))
            );
            assert_eq!(render_text_stored(locale, &stored), expected);
            assert_eq!(render_stored(locale, &stored), expected);
            assert_eq!(StoredText::from_stored(&stored).stored(), stored);
        }
    }

    #[test]
    fn mixed_output_bounds_raw_bytes_metadata_and_segments_without_cutting_envelopes() {
        let mut exact = StoredText::from_message(Message::new("error.forbidden"));
        exact.push_raw("a".repeat(MAX_TEXT_RAW_BYTES));
        assert!(!exact.truncate_raw(MAX_TEXT_RAW_BYTES));
        let exact_stored = exact.stored();
        assert_eq!(
            StoredText::decode(&exact_stored).unwrap().raw_bytes,
            MAX_TEXT_RAW_BYTES
        );
        exact.push_raw("界");
        let stored = exact.stored();
        for locale in Locale::ALL {
            assert_eq!(
                render_text_stored(locale, &stored),
                format!(
                    "{}{}{}",
                    render(locale, &Message::new("error.forbidden")),
                    "a".repeat(MAX_TEXT_RAW_BYTES),
                    render(locale, &Message::new("output.truncated"))
                )
            );
        }
        let mut boundary = StoredText::new();
        boundary.push_raw("界".repeat(MAX_TEXT_RAW_BYTES / 3 + 1));
        assert_eq!(boundary.raw_bytes, MAX_TEXT_RAW_BYTES / 3 * 3);
        assert!(boundary.truncated);
        assert!(boundary.truncate_raw(5));
        assert_eq!(boundary.raw_bytes, 3);
        assert!(boundary.render(Locale::EnUs).starts_with("界\n"));
        assert!(StoredText::decode(&boundary.stored()).is_some());

        let mut metadata = StoredText::new();
        metadata.push_raw("literal log");
        for _ in 0..10 {
            metadata
                .push_message(Message::new("alerts.hostState").with("state", "x".repeat(40_000)));
        }
        assert!(metadata.truncated);
        assert!(metadata.metadata_bytes <= MAX_STORED_MESSAGE_BYTES);
        assert_eq!(metadata.raw_bytes, "literal log".len());
        assert!(StoredText::decode(&metadata.stored()).is_some());
        let mut segments = StoredText::new();
        for _ in 0..MAX_TEXT_SEGMENTS + 1 {
            segments.push_message(Message::new("error.forbidden"));
        }
        assert!(segments.truncated);
        assert!(segments.segments.len() <= MAX_TEXT_SEGMENTS);
        assert!(StoredText::decode(&segments.stored()).is_some());
        // Legacy logs are fully readable even if larger than current write limits.
        let legacy = "a".repeat(MAX_TEXT_RAW_BYTES + 10);
        assert_eq!(render_text_stored(Locale::JaJp, &legacy), legacy);
    }

    #[test]
    fn mixed_output_rejects_unknown_or_malformed_metadata_and_decodes_raw_collisions_once() {
        let stored = StoredText::from_message(Message::new("error.forbidden")).stored();
        let valid: serde_json::Value = serde_json::from_str(&stored).unwrap();
        let mut unknown_version = valid.clone();
        unknown_version["$system_text"]["version"] = serde_json::json!(2);
        let mut unknown_message = valid.clone();
        unknown_message["$system_text"]["segments"][0]["Message"]["key"] =
            serde_json::json!("not.a.product.key");
        let mut invalid_params = valid.clone();
        invalid_params["$system_text"]["segments"][0]["Message"]["params"] =
            serde_json::json!({"injected":"value"});
        let mut unknown_field = valid.clone();
        unknown_field["$system_text"]["unexpected"] = serde_json::json!(true);
        let mut too_many_segments = valid.clone();
        too_many_segments["$system_text"]["segments"] =
            serde_json::json!(vec![serde_json::json!({"Raw":""}); MAX_TEXT_SEGMENTS + 1]);
        let mut oversized_raw = valid.clone();
        oversized_raw["$system_text"]["segments"] =
            serde_json::json!([{"Raw":"x".repeat(MAX_TEXT_RAW_BYTES + 1)}]);
        for invalid in [
            unknown_version,
            unknown_message,
            invalid_params,
            unknown_field,
            too_many_segments,
            oversized_raw,
        ] {
            let invalid = invalid.to_string();
            assert!(StoredText::decode(&invalid).is_none());
            for locale in Locale::ALL {
                assert_eq!(render_text_stored(locale, &invalid), invalid);
            }
        }
        for raw in [
            stored,
            store_message(&Message::new("error.forbidden")),
            format!("{RAW_MESSAGE_PREFIX}literal"),
            "{\"$system_text\":".into(),
        ] {
            for locale in Locale::ALL {
                let escaped = store_raw(&raw);
                assert_eq!(render_text_stored(locale, &escaped), raw);
                assert_eq!(StoredText::from_stored(&escaped).render(locale), raw);
                let mut text = StoredText::new();
                text.push_raw(raw.clone());
                assert_eq!(render_text_stored(locale, &text.stored()), raw);
            }
        }
        // Single-message JSON was previously ordinary log content.
        let legacy = store_message(&Message::new("error.forbidden"));
        assert_eq!(render_text_stored(Locale::JaJp, &legacy), legacy);
    }

    #[tokio::test]
    async fn persisted_mixed_output_reads_in_six_locales_without_rewriting_storage() {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE output_fixture (id INTEGER PRIMARY KEY, output TEXT NOT NULL)",
        )
        .await
        .unwrap();
        let mut text = StoredText::from_message(Message::new("alerts.executionSucceeded"));
        let raw = "user stdout {name}\n".repeat(20_000);
        text.push_raw(raw.clone());
        let stored = text.stored();
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO output_fixture (id, output) VALUES (1, ?)",
            [stored.clone().into()],
        ))
        .await
        .unwrap();
        for locale in Locale::ALL {
            let row = db
                .query_one_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    "SELECT output FROM output_fixture WHERE id = 1",
                ))
                .await
                .unwrap()
                .unwrap();
            let output: String = row.try_get("", "output").unwrap();
            assert_eq!(
                with_locale(locale, async { display_text_stored(&output) }).await,
                format!(
                    "{}{}",
                    render(locale, &Message::new("alerts.executionSucceeded")),
                    raw
                )
            );
            assert_eq!(output, stored);
        }
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT output FROM output_fixture WHERE id = 1",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "output").unwrap(), stored);
    }

    #[test]
    fn negotiates_language_ranges_and_weights() {
        for (value, expected) in [
            ("fr-FR, zh-Hant-HK;q=0.9, en;q=0.5", Locale::ZhTw),
            ("zh-CN;q=0.2, ja-JP;q=0.9", Locale::JaJp),
            ("ko;q=0, ms;q=bogus, en-GB;q=0.3", Locale::EnUs),
            ("ZH-hk,zh-CN", Locale::ZhTw),
            ("zh-SG", Locale::ZhCn),
            ("ms", Locale::MsMy),
            ("ko", Locale::KoKr),
            ("it,*;q=0.9", Locale::EnUs),
            ("ja;q=NaN,ko;q=1.1,zh;q=-1", Locale::EnUs),
            ("ja;q=0.5;q=1,ko;q=0.4", Locale::KoKr),
            ("ja;q=1e0,ko;q=.9,zh;q=0.0001", Locale::EnUs),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_LANGUAGE, value.parse().unwrap());
            assert_eq!(Locale::negotiate(&headers), expected, "{value}");
        }
        assert_eq!(Locale::negotiate(&HeaderMap::new()), Locale::EnUs);
    }

    fn placeholders(template: &str) -> Vec<&str> {
        template
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(key, _)| key))
            .collect()
    }

    #[test]
    fn catalogs_have_identical_keys_and_named_parameters() {
        let english = &CATALOGS[0];
        assert!(!english.is_empty());
        for (index, catalog) in CATALOGS.iter().enumerate() {
            assert_eq!(
                catalog.keys().collect::<Vec<_>>(),
                english.keys().collect::<Vec<_>>(),
                "{}",
                Locale::ALL[index].tag()
            );
            for (key, value) in catalog {
                assert!(!value.trim().is_empty(), "{key}");
                assert!(
                    !value.contains("???"),
                    "corrupted translation {} {key}",
                    Locale::ALL[index].tag()
                );
                // Protocol notation/composition contains no prose to translate.
                // "Platform" and "item" also use the same spelling in Malay.
                let language_neutral = matches!(
                    key.as_str(),
                    "webhook.http" | "webhook.httpDetail" | "repository.monitor.notes"
                ) || (Locale::ALL[index] == Locale::MsMy
                    && matches!(
                        key.as_str(),
                        "repository.monitor.providerLabel" | "typed.itemSummary"
                    ));
                if index != 0 && !language_neutral {
                    assert_ne!(
                        value,
                        &english[key],
                        "untranslated {} {key}",
                        Locale::ALL[index].tag()
                    );
                }
                let mut actual = placeholders(value);
                let mut expected = placeholders(&english[key]);
                actual.sort();
                expected.sort();
                assert_eq!(actual, expected, "{} {key}", Locale::ALL[index].tag());
            }
        }
    }

    #[test]
    fn parameters_are_literal_data() {
        let params = Message::new("test")
            .with("name", "{other}")
            .with("other", "changed")
            .params;
        assert_eq!(
            interpolate("Hello {name}, {missing}", &params),
            "Hello {other}, {missing}"
        );
    }

    #[tokio::test]
    async fn request_locales_are_isolated_and_spawned_tasks_default_to_english() {
        let (chinese, japanese) = tokio::join!(
            REQUEST_LOCALE.scope(Locale::ZhCn, async {
                tokio::task::yield_now().await;
                assert_eq!(
                    tokio::spawn(async { current_locale() }).await.unwrap(),
                    Locale::EnUs
                );
                message("error.unauthorized")
            }),
            REQUEST_LOCALE.scope(Locale::JaJp, async {
                tokio::task::yield_now().await;
                message("error.unauthorized")
            }),
        );
        assert_eq!(chinese, "请登录后继续操作。");
        assert_eq!(japanese, "ログインして操作を続けてください。");
        assert_eq!(current_locale(), Locale::EnUs);
        let upgraded = tokio::spawn(with_locale(Locale::KoKr, async {
            tokio::task::yield_now().await;
            message("error.unauthorized")
        }));
        assert_eq!(upgraded.await.unwrap(), "계속하려면 로그인하세요.");
        assert_eq!(current_locale(), Locale::EnUs);
    }

    #[tokio::test]
    async fn shared_errors_keep_status_and_tuple_display_compatibility() {
        use crate::error::ApiError;
        use axum::http::StatusCode;

        for locale in Locale::ALL {
            REQUEST_LOCALE
                .scope(locale, async {
                    let errors = [
                        (
                            ApiError::unauthorized(),
                            StatusCode::UNAUTHORIZED,
                            "error.unauthorized",
                        ),
                        (
                            ApiError::forbidden(),
                            StatusCode::FORBIDDEN,
                            "error.forbidden",
                        ),
                        (ApiError::missing(), StatusCode::NOT_FOUND, "error.missing"),
                        (
                            ApiError::from(sea_orm::DbErr::Custom("provider diagnostic".into())),
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "error.database",
                        ),
                        (
                            ApiError::from(std::io::Error::other("private filename")),
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "error.file",
                        ),
                    ];
                    for (error, status, key) in errors {
                        assert_eq!(error.0, status);
                        assert_eq!(error.1, render(Locale::EnUs, &Message::new(key)));
                        assert_eq!(error.render(locale), render(locale, &Message::new(key)));
                        assert_eq!(error.2.as_ref(), Some(&Message::new(key)));
                        assert_eq!(error.to_string(), error.1);
                        assert!(!error.1.contains("provider diagnostic"));
                        assert!(!error.1.contains("private filename"));
                    }
                })
                .await;
        }
    }

    #[tokio::test]
    async fn error_identity_survives_background_transport_and_reader_locale_changes() {
        use crate::error::ApiError;
        use axum::{http::StatusCode, response::IntoResponse};

        let error = with_locale(Locale::ZhCn, async {
            ApiError::localized(
                StatusCode::BAD_REQUEST,
                Message::new(String::from("resource.positive_integer"))
                    .with("field", "用户字段 {minimum}\n\"quoted\""),
            )
        })
        .await;
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            error.1,
            "用户字段 {minimum}\n\"quoted\" must be a positive integer."
        );
        let identity = serde_json::to_vec(error.2.as_ref().unwrap()).unwrap();
        let stored = tokio::spawn(async move {
            assert_eq!(current_locale(), Locale::EnUs);
            let message: Message = serde_json::from_slice(&identity).unwrap();
            let background = ApiError::localized(StatusCode::BAD_REQUEST, message);
            assert_eq!(background.to_string(), background.1);
            background.stored()
        })
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(value["$system_message"]["version"], 1);
        assert!(value["$system_message"].get("fallback").is_none());
        let mut legacy = value.clone();
        legacy["$system_message"]["fallback"] = serde_json::json!(error.1);
        assert_eq!(decode_stored(&legacy.to_string()), error.2);
        let restored = decode_stored(&stored).unwrap();
        assert_eq!(restored, error.2.clone().unwrap());
        assert_eq!(
            render_stored(Locale::JaJp, &stored),
            "用户字段 {minimum}\n\"quoted\"は正の整数で指定してください。"
        );
        assert_eq!(
            render_stored(Locale::ZhCn, &stored),
            "用户字段 {minimum}\n\"quoted\" 必须是正整数。"
        );
        assert_eq!(render_stored(Locale::EnUs, &stored), error.1);

        // The error was built under another locale; response rendering uses its reader.
        let response = with_locale(Locale::JaJp, async { error.into_response() }).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error": "用户字段 {minimum}\n\"quoted\"は正の整数で指定してください。"})
        );
    }

    #[test]
    fn stored_messages_do_not_interpret_legacy_unknown_or_malformed_data() {
        let valid: serde_json::Value =
            serde_json::from_str(&store_message(&Message::new("error.unauthorized"))).unwrap();
        let mut invalid = Vec::new();
        let mut changed = valid.clone();
        changed["$system_message"]["version"] = serde_json::json!(2);
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["$system_message"]["message"]["key"] = serde_json::json!("future.unknown");
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["$system_message"]["message"]["params"] =
            serde_json::json!({"unexpected":"user data"});
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["$system_message"]["message"]["key"] =
            serde_json::json!("resource.positive_integer");
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["$system_message"]["message"]["params"] = serde_json::json!({"field":42});
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["$system_message"]["fallback"] = serde_json::json!(null);
        invalid.push(changed.to_string());
        let mut changed = valid.clone();
        changed["user_data"] = serde_json::json!("preserve me");
        invalid.push(changed.to_string());
        let mut changed = valid;
        changed["$system_message"]["message"]["user_data"] = serde_json::json!("preserve me");
        invalid.push(changed.to_string());
        invalid.extend(
            [
                "用户原文 {field}",
                "error.unauthorized",
                "The record does not exist or is no longer available.",
                "{\"key\":\"error.unauthorized\",\"params\":{}}",
                "{\"error\":\"provider diagnostic\",\"status\":401}",
                "{\"$system_message\":",
                "",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        for raw in invalid {
            assert!(decode_stored(&raw).is_none(), "{raw}");
            assert_eq!(render_stored(Locale::JaJp, &raw), raw);
            assert_eq!(render_stored(Locale::ZhCn, &raw), raw);
        }
        assert_eq!(
            store_message(&Message::new("future.unknown")),
            "future.unknown"
        );
    }

    #[tokio::test]
    async fn raw_errors_preserve_tuple_display_storage_status_and_response() {
        use crate::error::ApiError;
        use axum::{http::StatusCode, response::IntoResponse};
        let raw = "用户原文 {field} / provider diagnostic";
        let error = ApiError::raw(StatusCode::IM_A_TEAPOT, raw.into());
        assert_eq!(error.0, StatusCode::IM_A_TEAPOT);
        assert_eq!(error.1, raw);
        assert!(error.2.is_none());
        assert_eq!(error.to_string(), raw);
        assert_eq!(error.stored(), raw);
        assert_eq!(error.render(Locale::JaJp), raw);
        let response = with_locale(Locale::JaJp, async { error.into_response() }).await;
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error":raw})
        );
    }

    #[test]
    fn raw_envelope_collisions_are_escaped_only_for_storage_and_decoded_once() {
        use crate::error::ApiError;
        let envelope = store_message(&Message::new("error.unauthorized"));
        let error = ApiError::bad(envelope.clone());
        assert_eq!(error.render(Locale::JaJp), envelope);
        let stored = error.stored();
        assert_ne!(stored, envelope);
        assert!(decode_stored(&stored).is_none());
        assert_eq!(render_stored(Locale::JaJp, &stored), envelope);
        assert_eq!(render_stored(Locale::ZhCn, &stored), envelope);
        let twice = store_raw(&stored);
        assert_eq!(render_stored(Locale::JaJp, &twice), stored);
        assert_ne!(render_stored(Locale::JaJp, &twice), envelope);
        for raw in [
            "用户原文 {error}",
            "",
            "error.unauthorized",
            "{\"key\":\"error.unauthorized\"}",
        ] {
            assert_eq!(store_raw(raw), raw);
        }
    }

    #[test]
    fn explicit_nested_messages_render_per_reader_while_raw_parameters_stay_literal() {
        let message = Message::new("repository.analysis.scheduledNotRun")
            .with_message("error", Message::new("error.unauthorized"));
        let stored = store_message(&message);
        assert_eq!(decode_stored(&stored), Some(message.clone()));
        assert_eq!(
            render_stored(Locale::ZhCn, &stored),
            "定时更新未执行：请登录后继续操作。"
        );
        assert_eq!(
            render_stored(Locale::JaJp, &stored),
            "定期更新を実行できませんでした：ログインして操作を続けてください。"
        );
        let raw = store_message(&Message::new("error.unauthorized"));
        let literal =
            Message::new("repository.analysis.scheduledNotRun").with("error", raw.clone());
        assert_eq!(
            render(Locale::JaJp, &literal),
            format!("定期更新を実行できませんでした：{raw}")
        );
        let ambiguous = message.with("error", "raw");
        assert!(!known_message(&ambiguous));
    }

    #[test]
    fn stored_message_decoding_bounds_depth_and_size_without_truncating_raw_data() {
        let mut message = Message::new("error.unauthorized");
        for _ in 0..MAX_MESSAGE_DEPTH {
            message =
                Message::new("repository.analysis.scheduledNotRun").with_message("error", message);
        }
        let deep = serde_json::to_string(&StoredEnvelope {
            system_message: StoredMessage {
                version: 1,
                message,
                fallback: "Legacy fallback".into(),
            },
        })
        .unwrap();
        assert!(decode_stored(&deep).is_none());
        assert_eq!(render_stored(Locale::JaJp, &deep), deep);
        let large = Message::new("resource.positive_integer")
            .with("field", "x".repeat(MAX_STORED_MESSAGE_BYTES));
        let oversized = serde_json::to_string(&StoredEnvelope {
            system_message: StoredMessage {
                version: 1,
                message: large.clone(),
                fallback: "Legacy fallback".into(),
            },
        })
        .unwrap();
        assert!(decode_stored(&oversized).is_none());
        assert_eq!(render_stored(Locale::JaJp, &oversized), oversized);
        let fallback = store_message(&large);
        assert_eq!(fallback, render(Locale::EnUs, &large));
        assert!(fallback.contains(&"x".repeat(MAX_STORED_MESSAGE_BYTES)));
    }

    #[test]
    fn account_and_workspace_keys_are_available_in_every_catalog() {
        for source in [
            include_str!("accounts/auth.rs"),
            include_str!("resources/resource.rs"),
            include_str!("accounts/workspace.rs"),
            include_str!("accounts/workspace_admin.rs"),
            include_str!("repositories/auth.rs"),
            include_str!("repositories/analysis.rs"),
            include_str!("repositories/analysis/insights.rs"),
            include_str!("repositories/analysis/insight_identity.rs"),
            include_str!("repositories/analysis/identity.rs"),
            include_str!("repositories/analysis/sync.rs"),
            include_str!("repositories/analysis/query.rs"),
            include_str!("repositories/analysis/sync/pipeline.rs"),
            include_str!("repositories/analysis/sync/provider.rs"),
            include_str!("repositories/monitor.rs"),
            include_str!("repositories/monitor/detail.rs"),
            include_str!("repositories/monitor/polling.rs"),
            include_str!("repositories/monitor/git.rs"),
            include_str!("notifications/webhook.rs"),
            include_str!("credentials.rs"),
            include_str!("repositories/mod.rs"),
            include_str!("variables.rs"),
            include_str!("hosts/channel.rs"),
            include_str!("resources/transfers.rs"),
            include_str!("hosts/ssh.rs"),
            include_str!("pipelines/contexts.rs"),
            include_str!("hosts/windows_shell.rs"),
            include_str!("hosts/monitor.rs"),
            include_str!("pipelines/concurrency.rs"),
            include_str!("accounts/platform.rs"),
            include_str!("hosts/mod.rs"),
            include_str!("notifications/alerts.rs"),
            include_str!("maintenance/values.rs"),
            include_str!("maintenance/files.rs"),
            include_str!("maintenance/mod.rs"),
            include_str!("pipelines/mod.rs"),
            include_str!("pipelines/artifacts.rs"),
            include_str!("cron_jobs.rs"),
            include_str!("marketplace.rs"),
            include_str!("templates.rs"),
            include_str!("pipelines/actions.rs"),
            include_str!("pipelines/actions_run.rs"),
            include_str!("pipelines/actions_run/context.rs"),
            include_str!("pipelines/actions_run/scheduler.rs"),
            include_str!("pipelines/actions_run/job.rs"),
            include_str!("pipelines/actions_run/checkout.rs"),
            include_str!("pipelines/actions_run/events.rs"),
            include_str!("repositories/auth/provider.rs"),
            include_str!("repositories/auth/oauth.rs"),
            include_str!("repositories/auth/connection.rs"),
            include_str!("hosts/runner/mod.rs"),
            include_str!("hosts/runner/install.rs"),
            include_str!("hosts/runner/reservations.rs"),
            include_str!("hosts/runner/enrollment.rs"),
            include_str!("hosts/runner/transport/mod.rs"),
            include_str!("hosts/runner/transport/control.rs"),
            include_str!("hosts/runner/transport/channel.rs"),
            include_str!("hosts/runner/transport/pty.rs"),
            include_str!("hosts/runner/updates/mod.rs"),
            include_str!("hosts/runner/updates/policy.rs"),
            include_str!("hosts/runner/updates/jobs.rs"),
            include_str!("hosts/runner/updates/persistence.rs"),
            include_str!("hosts/runner/updates/recovery.rs"),
        ] {
            for part in source.split("Message::new(\"").skip(1) {
                let key = part.split('"').next().unwrap();
                for catalog in CATALOGS.iter() {
                    assert!(
                        catalog.contains_key(key),
                        "missing application message {key}"
                    );
                }
            }
        }
        for (_, group, action) in crate::accounts::workspace::PERMISSIONS {
            for catalog in CATALOGS.iter() {
                assert!(
                    catalog.contains_key(*group),
                    "missing permission label {group}"
                );
                assert!(
                    catalog.contains_key(*action),
                    "missing permission label {action}"
                );
            }
        }
    }

    #[tokio::test]
    async fn account_and_workspace_http_localization_preserves_stored_and_user_data() {
        use crate::entities::{users, workspace_roles};
        use axum::{
            Router,
            http::StatusCode,
            middleware,
            routing::{get, post},
        };
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

        let (dir, db) = crate::lock::tests::database().await;
        let s = std::sync::Arc::new(crate::state::State {
            ids: std::sync::Arc::new(
                crate::snowflake::Generator::new(1, std::time::Instant::now()).unwrap(),
            ),
            db,
            data_dir: dir.path().into(),
            instance_id: "locale-account-test".into(),
            server_mode: false,
            secure_cookie: false,
            bind: "127.0.0.1:0".parse().unwrap(),
            vault: crate::vault::Vault::open(dir.path(), false).unwrap(),
            host_monitor: Default::default(),
            monitor: Default::default(),
            cluster: None,
        });
        let user_id = crate::snowflake::Id::from(10);
        users::Entity::insert(users::ActiveModel {
            id: Set(user_id.into()),
            email: Set("owner@example.test".into()),
            name: Set("用户原文".into()),
            password_hash: Set(crate::accounts::auth::hash("valid-password").unwrap()),
            last_login_at: Set(None),
            last_login_ip: Set(None),
            active: Set(1),
            is_superadmin: Set(0),
            version: Set(1),
            created_at: Set(0),
            updated_at: Set(0),
        })
        .exec(&s.db)
        .await
        .unwrap();
        let workspace = crate::accounts::workspace::create_in(
            &s,
            user_id,
            serde_json::from_value(
                serde_json::json!({"name":"工作空间原文", "slug":"locale-test"}),
            )
            .unwrap(),
            &s.db,
        )
        .await
        .unwrap();
        let workspace_id = workspace["id"].as_str().unwrap();
        let token = crate::accounts::auth::start_session(&s, &user_id, &s.db)
            .await
            .unwrap();
        let app = Router::new()
            .route("/api/users", post(crate::accounts::auth::register))
            .route("/api/sessions", post(crate::accounts::auth::login))
            .route("/api/me", get(crate::accounts::auth::me))
            .route("/api/workspaces", post(crate::accounts::workspace::create))
            .route(
                "/api/permissions",
                get(crate::accounts::workspace::permission_catalogue),
            )
            .route(
                "/api/workspaces/{workspace}/roles",
                get(crate::accounts::workspace_admin::roles),
            )
            .route(
                "/api/workspaces/{workspace}/members",
                get(crate::accounts::workspace_admin::members),
            )
            .layer(middleware::from_fn(super::middleware))
            .layer(axum::Extension(
                crate::accounts::auth::TrustedProxies::default(),
            ))
            .with_state(s.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap()
        });
        let client = reqwest::Client::new();
        for (locale, email_error, login_error, owner, dashboard) in [
            (
                None,
                "Please enter a valid email address.",
                "The email or password is incorrect.",
                "Owner",
                "Dashboard",
            ),
            (
                Some("zh-CN"),
                "请填写有效的邮箱地址。",
                "邮箱或密码不正确。",
                "拥有者",
                "仪表盘",
            ),
            (
                Some("ja-JP"),
                "有効なメールアドレスを入力してください。",
                "メールアドレスまたはパスワードが正しくありません。",
                "所有者",
                "ダッシュボード",
            ),
        ] {
            let request = |method, path: &str| {
                let mut request = client.request(method, format!("{url}{path}")).header(
                    header::COOKIE,
                    format!("{}={token}", crate::accounts::auth::COOKIE),
                );
                if let Some(locale) = locale {
                    request = request.header(header::ACCEPT_LANGUAGE, locale);
                }
                request
            };
            let invalid = request(reqwest::Method::POST, "/api/users")
                .json(&serde_json::json!({"email":"invalid", "password":"valid-password", "name":"用户原文"}))
                .send().await.unwrap();
            assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                invalid.headers()[header::CONTENT_LANGUAGE],
                locale.unwrap_or("en-US")
            );
            assert_eq!(
                invalid.json::<serde_json::Value>().await.unwrap()["error"],
                email_error
            );
            let short_password = request(reqwest::Method::POST, "/api/users")
                .json(&serde_json::json!({"email":"valid@example.test", "password":"short", "name":"用户原文"}))
                .send().await.unwrap();
            assert_eq!(short_password.status(), StatusCode::BAD_REQUEST);
            let parsed_locale = locale.and_then(Locale::from_tag).unwrap_or_default();
            assert_eq!(
                short_password.json::<serde_json::Value>().await.unwrap()["error"],
                render(
                    parsed_locale,
                    &Message::new("auth.password_short").with("minimum", "8")
                )
            );
            let invalid_login = request(reqwest::Method::POST, "/api/sessions")
                .json(&serde_json::json!({"email":"missing@example.test", "password":"valid-password"}))
                .send().await.unwrap();
            assert_eq!(invalid_login.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                invalid_login.json::<serde_json::Value>().await.unwrap()["error"],
                login_error
            );
            let invalid_workspace = request(reqwest::Method::POST, "/api/workspaces")
                .json(&serde_json::json!({"name":"用户原文", "slug":"api"}))
                .send()
                .await
                .unwrap();
            assert_eq!(invalid_workspace.status(), StatusCode::BAD_REQUEST);
            let parsed_locale = locale.and_then(Locale::from_tag).unwrap_or_default();
            assert_eq!(
                invalid_workspace.json::<serde_json::Value>().await.unwrap()["error"],
                render(parsed_locale, &Message::new("workspace.error.slug_invalid"))
            );
            let query_field = "用户{minimum}";
            let invalid_sort = request(
                reqwest::Method::GET,
                &format!("/api/workspaces/{workspace_id}/roles"),
            )
            .query(&[("sort", query_field)])
            .send()
            .await
            .unwrap();
            assert_eq!(invalid_sort.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                invalid_sort.json::<serde_json::Value>().await.unwrap()["error"],
                render(
                    parsed_locale,
                    &Message::new("resource.sort_unsupported").with("field", query_field)
                )
            );
            let roles = request(
                reqwest::Method::GET,
                &format!("/api/workspaces/{workspace_id}/roles"),
            )
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
            assert!(
                roles["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|role| role["name"] == owner && role["builtin"] == true)
            );
            let me = request(reqwest::Method::GET, "/api/me")
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            assert_eq!(me["user"]["name"], "用户原文");
            assert_eq!(me["workspaces"][0]["name"], "工作空间原文");
            assert_eq!(me["workspaces"][0]["role_name"], owner);
            let permissions = request(reqwest::Method::GET, "/api/permissions")
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            let permission = permissions["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|permission| permission["key"] == "dashboard.read")
                .unwrap();
            assert_eq!(permission["group"], dashboard);
            let members = request(
                reqwest::Method::GET,
                &format!("/api/workspaces/{workspace_id}/members"),
            )
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
            assert_eq!(members["items"][0]["role_name"], owner);
            assert_eq!(members["items"][0]["name"], "用户原文");
        }
        let stored_owner = workspace_roles::Entity::find()
            .filter(workspace_roles::Column::Name.eq("拥有者"))
            .one(&s.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_owner.builtin, 1);
        assert_eq!(
            crate::accounts::workspace::role_name("拥有者", false),
            "拥有者"
        );
        server.abort();
        let _ = server.await;
        s.db.clone().close().await.unwrap();
    }

    #[tokio::test]
    async fn http_responses_localize_errors_and_preserve_diagnostics_and_headers() {
        use axum::{Router, http::StatusCode, middleware, response::IntoResponse, routing::get};

        let app = Router::new()
            .route(
                "/auth",
                get(|| async { crate::error::ApiError::unauthorized() }),
            )
            .route(
                "/api/workspaces/{workspace}/nodes/{node}/terminal-sessions",
                get(|| async { crate::error::ApiError::unauthorized() }),
            )
            .route(
                "/plain",
                get(|| async { crate::error::ApiError::bad("用户内容 {error.missing}") }),
            )
            .route(
                "/vary",
                get(|| async { ([(header::VARY, "Accept-Encoding")], "ok").into_response() }),
            )
            .layer(middleware::from_fn(|request, next| async {
                crate::guard(request, next).await
            }))
            .layer(middleware::from_fn(super::middleware));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let url = format!("http://{address}");

        let english = client.get(format!("{url}/auth")).send().await.unwrap();
        assert_eq!(english.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(english.headers()[header::CONTENT_LANGUAGE], "en-US");
        assert_eq!(
            english.json::<serde_json::Value>().await.unwrap(),
            serde_json::json!({"error":"Please sign in to continue."})
        );
        for (path, upgrade, expected_locale) in [
            (
                "/api/workspaces/1/nodes/2/terminal-sessions?locale=ja-JP",
                Some("websocket"),
                Locale::JaJp,
            ),
            (
                "/api/workspaces/1/nodes/2/terminal-sessions?locale=zh-Hant",
                Some("WebSocket"),
                Locale::ZhTw,
            ),
            (
                "/api/workspaces/1/nodes/2/terminal-sessions?locale=ja-JP",
                None,
                Locale::EnUs,
            ),
            (
                "/api/workspaces/1/nodes/2/terminal-sessions?locale=ja-JP",
                Some("h2c"),
                Locale::EnUs,
            ),
            (
                "/api/workspaces/1/nodes/2/terminal-sessions?locale=unsupported",
                Some("websocket"),
                Locale::EnUs,
            ),
            ("/auth?locale=ja-JP", Some("websocket"), Locale::EnUs),
        ] {
            let mut request = client
                .get(format!("{url}{path}"))
                .header(header::ACCEPT_LANGUAGE, "en-US");
            if let Some(upgrade) = upgrade {
                request = request
                    .header(header::UPGRADE, upgrade)
                    .header(header::CONNECTION, "Upgrade");
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.headers()[header::CONTENT_LANGUAGE],
                expected_locale.tag()
            );
            assert_eq!(
                response.json::<serde_json::Value>().await.unwrap()["error"],
                render(expected_locale, &Message::new("error.unauthorized"))
            );
        }

        let (chinese, traditional) = tokio::join!(
            client
                .get(format!("{url}/auth"))
                .header(header::ACCEPT_LANGUAGE, "zh-CN")
                .send(),
            client
                .get(format!("{url}/auth"))
                .header(header::ACCEPT_LANGUAGE, "zh-Hant")
                .send(),
        );
        let chinese = chinese.unwrap();
        let traditional = traditional.unwrap();
        assert_eq!(chinese.headers()[header::CONTENT_LANGUAGE], "zh-CN");
        assert_eq!(traditional.headers()[header::CONTENT_LANGUAGE], "zh-TW");
        assert_eq!(
            chinese.json::<serde_json::Value>().await.unwrap()["error"],
            "请登录后继续操作。"
        );
        assert_eq!(
            traditional.json::<serde_json::Value>().await.unwrap()["error"],
            "請登入後繼續操作。"
        );

        let denied = client
            .get(format!("{url}/auth"))
            .header(header::HOST, "invalid/host")
            .header(header::ACCEPT_LANGUAGE, "zh-CN")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert_eq!(denied.headers()[header::CONTENT_LANGUAGE], "zh-CN");
        assert_eq!(denied.headers()[header::VARY], "Accept-Language");
        assert_eq!(denied.text().await.unwrap(), "请求的主机地址无效。");

        let plain = client
            .get(format!("{url}/plain"))
            .header(header::ACCEPT_LANGUAGE, "ja-JP")
            .send()
            .await
            .unwrap();
        assert_eq!(plain.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            plain.json::<serde_json::Value>().await.unwrap()["error"],
            "用户内容 {error.missing}"
        );

        let vary = client.get(format!("{url}/vary")).send().await.unwrap();
        let vary_values = vary
            .headers()
            .get_all(header::VARY)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(vary_values.contains(&"Accept-Encoding"));
        assert!(vary_values.contains(&"Accept-Language"));
        server.abort();
    }
}
