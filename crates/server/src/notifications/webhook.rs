//! Outbound notification webhooks. A URL of a recognised chat robot receives
//! that platform's native readable message and is judged by the platform's own
//! result: several robots answer HTTP 200 even when they reject a message.
//! Every other URL keeps the generic JSON event and HTTP status semantics.
use crate::i18n::Message;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Robot {
    WeCom,
    DingTalk,
    Feishu,
    Telegram,
    Slack,
    Discord,
    GoogleChat,
}

impl Robot {
    fn name(self) -> &'static str {
        match self {
            Robot::WeCom => "企业微信",
            Robot::DingTalk => "钉钉",
            Robot::Feishu => "飞书",
            Robot::Telegram => "Telegram",
            Robot::Slack => "Slack",
            Robot::Discord => "Discord",
            Robot::GoogleChat => "Google Chat",
        }
    }
}

/// Only official HTTPS robot endpoints are recognised; a look-alike host or
/// path stays a generic webhook.
fn robot(url: &reqwest::Url) -> Option<Robot> {
    if url.scheme() != "https" || url.port().is_some() {
        return None;
    }
    let path = url.path();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match url.host_str()? {
        "qyapi.weixin.qq.com" if path == "/cgi-bin/webhook/send" => Some(Robot::WeCom),
        "oapi.dingtalk.com" if path == "/robot/send" => Some(Robot::DingTalk),
        "open.feishu.cn" | "open.larksuite.com" if path.starts_with("/open-apis/bot/v2/hook/") => {
            Some(Robot::Feishu)
        }
        "api.telegram.org"
            if segments.len() == 2
                && segments[0].len() > 3
                && segments[0].starts_with("bot")
                && segments[1] == "sendMessage" =>
        {
            Some(Robot::Telegram)
        }
        "hooks.slack.com" if path.starts_with("/services/") => Some(Robot::Slack),
        "discord.com" | "discordapp.com" if path.starts_with("/api/webhooks/") => {
            Some(Robot::Discord)
        }
        "chat.googleapis.com"
            if segments.len() == 4
                && segments[0] == "v1"
                && segments[1] == "spaces"
                && segments[3] == "messages" =>
        {
            Some(Robot::GoogleChat)
        }
        _ => None,
    }
}

/// A notification: readable title and fields for chat robots, and the generic
/// JSON event for every other webhook.
pub struct Notice<'a> {
    pub title: &'a str,
    pub fields: Vec<(String, String)>,
    pub generic: Value,
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn lines<'a>(notice: &'a Notice<'_>) -> impl Iterator<Item = (&'a str, String)> {
    notice
        .fields
        .iter()
        .map(|(name, value)| (name.as_str(), clean(value, 300)))
        .filter(|(_, value)| !value.is_empty())
}

fn plain(notice: &Notice<'_>) -> String {
    std::iter::once(clean(notice.title, 100))
        .chain(lines(notice).map(|(name, value)| format!("{name}：{value}")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn body(robot: Robot, url: &reqwest::Url, notice: &Notice<'_>) -> Result<Value, Message> {
    let title = clean(notice.title, 100);
    let markdown = std::iter::once(format!("### {title}"))
        .chain(lines(notice).map(|(name, value)| format!("> **{name}**：{value}")))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(match robot {
        Robot::WeCom => json!({"msgtype":"markdown","markdown":{"content":markdown}}),
        Robot::DingTalk => json!({"msgtype":"markdown","markdown":{"title":title,"text":markdown}}),
        Robot::Feishu => json!({"msg_type":"text","content":{"text":plain(notice)}}),
        Robot::Telegram => {
            // The Bot API needs a destination; it travels in the configured URL.
            let chat = url
                .query_pairs()
                .find(|(key, value)| key == "chat_id" && !value.is_empty())
                .map(|(_, value)| value.into_owned())
                .ok_or_else(|| Message::new("webhook.chatMissing"))?;
            json!({"chat_id":chat,"text":plain(notice),"disable_web_page_preview":true})
        }
        Robot::Slack | Robot::GoogleChat => json!({"text":plain(notice)}),
        // Discord rejects content over 2000 characters; lines are already clean.
        Robot::Discord => json!({"content":plain(notice).chars().take(1900).collect::<String>()}),
    })
}

/// The platform's verdict on an HTTP response.
fn judge(robot: Robot, status: u16, response: &str) -> Result<(), Message> {
    let success = (200..300).contains(&status);
    let value: Option<Value> = serde_json::from_str(response).ok();
    let text = |key: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .map(|v| clean(v, 200))
            .unwrap_or_default()
    };
    match robot {
        Robot::WeCom | Robot::DingTalk | Robot::Feishu => {
            if !success {
                return Err(Message::new("webhook.http").with("status", status.to_string()));
            }
            let value = value
                .as_ref()
                .ok_or_else(|| Message::new("webhook.responseInvalid"))?;
            // Current Feishu answers {code,msg}; older robots answered {StatusCode}.
            let (code, message) = match robot {
                Robot::Feishu if value.get("code").is_none() => ("StatusCode", "StatusMessage"),
                Robot::Feishu => ("code", "msg"),
                _ => ("errcode", "errmsg"),
            };
            match value.get(code).and_then(Value::as_i64) {
                Some(0) => Ok(()),
                Some(code) => Err(Message::new("webhook.code")
                    .with("code", code.to_string())
                    .with("detail", text(message))),
                None => Err(Message::new("webhook.responseInvalid")),
            }
        }
        Robot::Telegram => match value
            .as_ref()
            .and_then(|v| v.get("ok"))
            .and_then(Value::as_bool)
        {
            Some(true) if success => Ok(()),
            _ => Err(Message::new("webhook.httpDetail")
                .with("status", status.to_string())
                .with("detail", text("description"))),
        },
        // Slack answers the plain text "ok", or an error code such as invalid_payload.
        Robot::Slack if success && response.trim() == "ok" => Ok(()),
        Robot::Slack => Err(Message::new("webhook.httpDetail")
            .with("status", status.to_string())
            .with("detail", clean(response, 200))),
        Robot::Discord | Robot::GoogleChat if success => Ok(()),
        Robot::Discord => Err(Message::new("webhook.httpDetail")
            .with("status", status.to_string())
            .with("detail", text("message"))),
        Robot::GoogleChat => Err(Message::new("webhook.httpDetail")
            .with("status", status.to_string())
            .with(
                "detail",
                value
                    .as_ref()
                    .and_then(|v| v.pointer("/error/message"))
                    .and_then(Value::as_str)
                    .map(|v| clean(v, 200))
                    .unwrap_or_default(),
            )),
    }
}

/// Deliver once and describe the outcome for the event's delivery column.
pub async fn deliver(client: &reqwest::Client, url: &str, notice: &Notice<'_>) -> String {
    let parsed = reqwest::Url::parse(url).ok();
    let Some((robot, parsed)) = parsed.as_ref().and_then(|u| robot(u).map(|r| (r, u))) else {
        return match client.post(url).json(&notice.generic).send().await {
            Ok(response) if response.status().is_success() => crate::i18n::store_message(
                &Message::new("webhook.deliveredHttp")
                    .with("status", response.status().as_u16().to_string()),
            ),
            Ok(response) => crate::i18n::store_message(
                &Message::new("webhook.failedHttp")
                    .with("status", response.status().as_u16().to_string()),
            ),
            Err(_) => crate::i18n::store_message(&Message::new("webhook.failedTimeout")),
        };
    };
    let name = robot.name();
    let message = match body(robot, parsed, notice) {
        Ok(message) => message,
        Err(reason) => {
            return crate::i18n::store_message(
                &Message::new("webhook.notSent")
                    .with("provider", name)
                    .with_message("reason", reason),
            );
        }
    };
    match client.post(url).json(&message).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            match judge(robot, status, &text) {
                Ok(()) => crate::i18n::store_message(
                    &Message::new("webhook.delivered").with("provider", name),
                ),
                Err(reason) => crate::i18n::store_message(
                    &Message::new("webhook.failed")
                        .with("provider", name)
                        .with_message("reason", reason),
                ),
            }
        }
        Err(_) => crate::i18n::store_message(
            &Message::new("webhook.providerTimeout").with("provider", name),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Existing provider-verdict goldens also verify legacy Chinese rendering.
    fn judge(robot: Robot, status: u16, response: &str) -> Result<(), String> {
        super::judge(robot, status, response)
            .map_err(|message| crate::i18n::render(crate::i18n::Locale::ZhCn, &message))
    }

    fn detect(url: &str) -> Option<Robot> {
        reqwest::Url::parse(url).ok().as_ref().and_then(robot)
    }

    #[test]
    fn recognises_only_official_https_robot_endpoints() {
        for (url, expected) in [
            (
                "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=k",
                Some(Robot::WeCom),
            ),
            (
                "https://oapi.dingtalk.com/robot/send?access_token=t",
                Some(Robot::DingTalk),
            ),
            (
                "https://open.feishu.cn/open-apis/bot/v2/hook/abc",
                Some(Robot::Feishu),
            ),
            (
                "https://open.larksuite.com/open-apis/bot/v2/hook/abc",
                Some(Robot::Feishu),
            ),
            (
                "https://api.telegram.org/bot123:ABC/sendMessage?chat_id=-100",
                Some(Robot::Telegram),
            ),
            ("https://hooks.slack.com/services/T/B/X", Some(Robot::Slack)),
            (
                "https://discord.com/api/webhooks/1/token",
                Some(Robot::Discord),
            ),
            (
                "https://discordapp.com/api/webhooks/1/token",
                Some(Robot::Discord),
            ),
            (
                "https://chat.googleapis.com/v1/spaces/AAA/messages?key=k&token=t",
                Some(Robot::GoogleChat),
            ),
            (
                "http://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=k",
                None,
            ),
            (
                "https://qyapi.weixin.qq.com.example.com/cgi-bin/webhook/send",
                None,
            ),
            (
                "https://qyapi.weixin.qq.com:8443/cgi-bin/webhook/send",
                None,
            ),
            ("https://qyapi.weixin.qq.com/cgi-bin/other", None),
            ("https://api.telegram.org/bot123:ABC/getUpdates", None),
            ("https://api.telegram.org/bot/sendMessage", None),
            ("https://hooks.slack.com/triggers/T/1/x", None),
            ("https://chat.googleapis.com/v1/spaces/AAA/members", None),
            ("https://example.com/hook", None),
        ] {
            assert_eq!(detect(url), expected, "{url}");
        }
    }

    #[test]
    fn builds_each_platforms_readable_message_without_empty_or_control_fields() {
        let notice = Notice {
            title: "仓库事件：push",
            fields: vec![
                ("仓库".into(), "group/app".into()),
                ("引用".into(), "main\nx".into()),
                ("操作人".into(), String::new()),
            ],
            generic: json!({}),
        };
        let url = |u: &str| reqwest::Url::parse(u).unwrap();
        let any = url("https://example.com");
        let text = "仓库事件：push\n仓库：group/app\n引用：main x";
        assert_eq!(
            body(Robot::WeCom, &any, &notice).unwrap(),
            json!({"msgtype":"markdown","markdown":{"content":"### 仓库事件：push\n> **仓库**：group/app\n> **引用**：main x"}})
        );
        assert_eq!(
            body(Robot::DingTalk, &any, &notice).unwrap()["markdown"]["title"],
            "仓库事件：push"
        );
        assert_eq!(
            body(Robot::Feishu, &any, &notice).unwrap(),
            json!({"msg_type":"text","content":{"text":text}})
        );
        assert_eq!(
            body(
                Robot::Telegram,
                &url("https://api.telegram.org/bot1:A/sendMessage?chat_id=-100123"),
                &notice
            )
            .unwrap(),
            json!({"chat_id":"-100123","text":text,"disable_web_page_preview":true})
        );
        assert_eq!(
            body(
                Robot::Telegram,
                &url("https://api.telegram.org/bot1:A/sendMessage"),
                &notice
            )
            .map_err(|message| crate::i18n::render(crate::i18n::Locale::ZhCn, &message)),
            Err("地址缺少 chat_id 参数".into())
        );
        assert_eq!(
            body(Robot::Slack, &any, &notice).unwrap(),
            json!({"text":text})
        );
        assert_eq!(
            body(Robot::GoogleChat, &any, &notice).unwrap(),
            json!({"text":text})
        );
        assert_eq!(
            body(Robot::Discord, &any, &notice).unwrap(),
            json!({"content":text})
        );
    }

    #[test]
    fn judges_delivery_by_the_platform_result() {
        assert_eq!(
            judge(Robot::WeCom, 200, r#"{"errcode":0,"errmsg":"ok"}"#),
            Ok(())
        );
        assert_eq!(
            judge(
                Robot::WeCom,
                200,
                r#"{"errcode":93000,"errmsg":"invalid webhook url"}"#
            ),
            Err("错误码 93000：invalid webhook url".into())
        );
        assert!(
            judge(
                Robot::DingTalk,
                200,
                r#"{"errcode":310000,"errmsg":"keywords not in content"}"#
            )
            .is_err()
        );
        assert_eq!(
            judge(
                Robot::Feishu,
                200,
                r#"{"code":0,"msg":"success","data":{}}"#
            ),
            Ok(())
        );
        assert_eq!(judge(Robot::Feishu, 200, r#"{"StatusCode":0}"#), Ok(()));
        assert!(
            judge(
                Robot::Feishu,
                200,
                r#"{"code":19021,"msg":"sign match fail"}"#
            )
            .is_err()
        );
        assert_eq!(judge(Robot::WeCom, 500, "{}"), Err("HTTP 500".into()));
        assert_eq!(
            judge(Robot::WeCom, 200, "<html>"),
            Err("无法识别的响应".into())
        );
        assert_eq!(judge(Robot::WeCom, 200, "{}"), Err("无法识别的响应".into()));
        assert_eq!(
            judge(Robot::Telegram, 200, r#"{"ok":true,"result":{}}"#),
            Ok(())
        );
        assert_eq!(
            judge(
                Robot::Telegram,
                400,
                r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#
            ),
            Err("HTTP 400：Bad Request: chat not found".into())
        );
        assert_eq!(judge(Robot::Slack, 200, "ok"), Ok(()));
        assert_eq!(
            judge(Robot::Slack, 400, "invalid_payload"),
            Err("HTTP 400：invalid_payload".into())
        );
        assert_eq!(judge(Robot::Discord, 204, ""), Ok(()));
        assert_eq!(
            judge(
                Robot::Discord,
                404,
                r#"{"message":"Unknown Webhook","code":10015}"#
            ),
            Err("HTTP 404：Unknown Webhook".into())
        );
        assert_eq!(
            judge(Robot::GoogleChat, 200, r#"{"name":"spaces/A/messages/B"}"#),
            Ok(())
        );
        assert_eq!(
            judge(
                Robot::GoogleChat,
                400,
                r#"{"error":{"code":400,"message":"Invalid JSON payload"}}"#
            ),
            Err("HTTP 400：Invalid JSON payload".into())
        );
    }
}
