//! Provider-specific configuration, endpoint validation and request signing.
use super::webhook::{Robot, robot};
use crate::i18n::Message;
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub signing_secret: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub bot_token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub chat_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thread_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub bearer_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_provider_fields_and_preserves_legacy_channels() {
        let legacy = Options::default();
        assert!(legacy.resolve("https://notify.example.invalid/old").is_ok());
        assert_eq!(serde_json::to_value(&legacy).unwrap(), json!({}));
        let mut options = Options {
            provider: "Telegram".into(),
            bot_token: "123:fixture".into(),
            chat_id: "-100".into(),
            thread_id: "42".into(),
            ..Default::default()
        };
        let mut url = options.resolve("").unwrap();
        assert_eq!(url.host_str(), Some("api.telegram.org"));
        assert_eq!(
            url.query_pairs()
                .find(|(key, _)| key == "chat_id")
                .unwrap()
                .1,
            "-100"
        );
        let mut body = json!({});
        options.prepare(&mut url, &mut body, 0).unwrap();
        assert_eq!(body["message_thread_id"], 42);
        options.thread_id = "18446744073709551616".into();
        assert!(options.resolve("").is_err());
        options.bot_token = "123:token/other".into();
        assert!(options.resolve("").is_err());
        let explicit = Options {
            provider: "WeCom".into(),
            ..Default::default()
        };
        assert!(
            explicit
                .resolve("https://qyapi.weixin.qq.com.evil.invalid/cgi-bin/webhook/send")
                .is_err()
        );
        assert!(
            explicit
                .resolve("https://hooks.slack.com/services/T/B/X")
                .is_err()
        );
        let generic = Options {
            provider: "Webhook".into(),
            bearer_token: "fixture".into(),
            ..Default::default()
        };
        assert!(
            generic
                .resolve("https://api.telegram.org/bot123:fixture/sendMessage")
                .is_ok()
        );
        let bad = Options {
            signing_secret: "secret".into(),
            ..generic
        };
        assert!(bad.resolve("https://oapi.dingtalk.com/robot/send").is_err());
        for address in [
            "file:///etc/passwd",
            "https://user:secret@example.invalid",
            "https://example.invalid/#fragment",
        ] {
            assert!(legacy.resolve(address).is_err());
        }
    }

    #[test]
    fn signs_dingtalk_and_feishu_against_independent_hmac_vectors() {
        let secret = Options {
            signing_secret: "fixture-secret".into(),
            ..Default::default()
        };
        let mut ding = Url::parse(
            "https://oapi.dingtalk.com/robot/send?access_token=fixture&sign=old&timestamp=old",
        )
        .unwrap();
        let mut body = json!({"msg_type":"post"});
        secret.prepare(&mut ding, &mut body, 1700000000123).unwrap();
        let pairs: Vec<_> = ding.query_pairs().collect();
        assert_eq!(pairs.iter().filter(|(key, _)| key == "sign").count(), 1);
        assert!(
            pairs.iter().any(|(key, value)| key == "sign"
                && value == "TnHSn+d438zHLYhSm0b2RE8ShfqQC1dUowaH8QLO2Go=")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "timestamp" && value == "1700000000123")
        );
        let mut feishu =
            Url::parse("https://open.feishu.cn/open-apis/bot/v2/hook/fixture").unwrap();
        secret
            .prepare(&mut feishu, &mut body, 1700000000123)
            .unwrap();
        assert_eq!(body["sign"], "eSSBpPPdWfBl7avPl9BWSBfQOnLGZ91Xn8G5oZis0sc=");
        assert_eq!(body["timestamp"], "1700000000");
        assert_eq!(body["msg_type"], "post");
    }

    #[test]
    fn redacts_secrets_without_changing_delivery_preferences() {
        let mut options = Options {
            provider: "Telegram".into(),
            signing_secret: "sign".into(),
            bot_token: "bot".into(),
            bearer_token: "bearer".into(),
            chat_id: "-100".into(),
            thread_id: "2".into(),
        };
        options.redact();
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            json!({"provider":"Telegram","chat_id":"-100","thread_id":"2"})
        );
    }

    #[tokio::test]
    async fn generic_webhook_keeps_json_and_supports_bearer_without_leaking_it() {
        use axum::{Json, Router, http::HeaderMap, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/notify", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/notify",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer fixture-secret");
                assert_eq!(body, json!({"event":"push","sha":"fixture"}));
                axum::http::StatusCode::NO_CONTENT
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let notice = super::super::webhook::Notice {
            title: "Push",
            fields: vec![],
            generic: json!({"event":"push","sha":"fixture"}),
        };
        let options = Options {
            provider: "Webhook".into(),
            bearer_token: "fixture-secret".into(),
            ..Default::default()
        };
        let result = super::super::webhook::deliver_configured(
            &reqwest::Client::new(),
            &url,
            &options,
            &notice,
        )
        .await;
        assert!(crate::i18n::display_stored(&result).contains("204"));
        assert!(!result.contains("fixture-secret"));
        task.abort();
    }
}

fn invalid() -> Message {
    Message::new("webhook.channelInvalid")
}

pub fn provider(url: &Url) -> &'static str {
    match robot(url) {
        Some(Robot::WeCom) => "WeCom",
        Some(Robot::DingTalk) => "DingTalk",
        Some(Robot::Feishu) if url.host_str() == Some("open.larksuite.com") => "Lark",
        Some(Robot::Feishu) => "Feishu",
        Some(Robot::Telegram) => "Telegram",
        Some(Robot::Slack) => "Slack",
        Some(Robot::Discord) => "Discord",
        Some(Robot::GoogleChat) => "Google Chat",
        None => "Webhook",
    }
}

impl Options {
    pub fn redact(&mut self) {
        self.signing_secret.clear();
        self.bot_token.clear();
        self.bearer_token.clear();
    }

    pub fn resolve(&self, webhook: &str) -> Result<Url, Message> {
        if [
            &self.signing_secret,
            &self.bot_token,
            &self.chat_id,
            &self.thread_id,
            &self.bearer_token,
        ]
        .iter()
        .any(|value| value.len() > 4096 || value.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        let mut url = if self.provider == "Telegram" && !self.bot_token.is_empty() {
            let (id, token) = self.bot_token.split_once(':').ok_or_else(invalid)?;
            if id.is_empty()
                || !id.bytes().all(|c| c.is_ascii_digit())
                || token.is_empty()
                || !token
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(invalid());
            }
            Url::parse(&format!(
                "https://api.telegram.org/bot{}/sendMessage",
                self.bot_token
            ))
            .map_err(|_| invalid())?
        } else {
            Url::parse(webhook).map_err(|_| invalid())?
        };
        if !matches!(url.scheme(), "http" | "https")
            || !url.has_host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.as_str().len() > 4096
        {
            return Err(invalid());
        }
        let detected = if self.provider == "Webhook" {
            "Webhook"
        } else {
            provider(&url)
        };
        if !matches!(self.provider.as_str(), "" | "auto" | "Webhook") && self.provider != detected {
            return Err(invalid());
        }
        if !self.signing_secret.is_empty() && !matches!(detected, "DingTalk" | "Feishu" | "Lark") {
            return Err(invalid());
        }
        if !self.bearer_token.is_empty() && !(self.provider == "Webhook" || detected == "Webhook") {
            return Err(invalid());
        }
        if (!self.bot_token.is_empty() || !self.chat_id.is_empty()) && detected != "Telegram" {
            return Err(invalid());
        }
        if detected == "Telegram" {
            if !self.chat_id.is_empty() {
                let pairs: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(key, _)| key != "chat_id")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                url.set_query(None);
                url.query_pairs_mut()
                    .extend_pairs(pairs)
                    .append_pair("chat_id", &self.chat_id);
            }
            if !url
                .query_pairs()
                .any(|(key, value)| key == "chat_id" && !value.is_empty())
            {
                return Err(Message::new("webhook.chatMissing"));
            }
        }
        if !self.thread_id.is_empty()
            && (!matches!(detected, "Telegram" | "Discord")
                || self
                    .thread_id
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .is_none())
        {
            return Err(invalid());
        }
        Ok(url)
    }

    pub fn prepare(
        &self,
        url: &mut Url,
        body: &mut Value,
        timestamp_ms: i64,
    ) -> Result<(), Message> {
        let detected = provider(url);
        if !self.signing_secret.is_empty() {
            let ding = detected == "DingTalk";
            let timestamp = if ding {
                timestamp_ms
            } else {
                timestamp_ms / 1000
            };
            let text = format!("{timestamp}\n{}", self.signing_secret);
            let mut mac = Hmac::<Sha256>::new_from_slice(if ding {
                self.signing_secret.as_bytes()
            } else {
                text.as_bytes()
            })
            .map_err(|_| invalid())?;
            if ding {
                mac.update(text.as_bytes());
            }
            let sign =
                base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
            if ding {
                let pairs: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(key, _)| key != "sign" && key != "timestamp")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                url.set_query(None);
                url.query_pairs_mut()
                    .extend_pairs(pairs)
                    .append_pair("timestamp", &timestamp.to_string())
                    .append_pair("sign", &sign);
            } else {
                body["timestamp"] = Value::String(timestamp.to_string());
                body["sign"] = Value::String(sign);
            }
        }
        if !self.thread_id.is_empty() {
            if detected == "Telegram" {
                body["message_thread_id"] =
                    Value::from(self.thread_id.parse::<u64>().map_err(|_| invalid())?);
            }
            if detected == "Discord" {
                let pairs: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(key, _)| key != "thread_id")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                url.set_query(None);
                url.query_pairs_mut()
                    .extend_pairs(pairs)
                    .append_pair("thread_id", &self.thread_id);
            }
        }
        Ok(())
    }
}
