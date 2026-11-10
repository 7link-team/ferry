//! Outbound notification webhooks. A URL of a recognised chat robot receives
//! that platform's native readable message and is judged by the platform's own
//! result: several robots answer HTTP 200 even when they reject a message.
//! Every other URL keeps the generic JSON event and HTTP status semantics.
use crate::i18n::Message;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Robot {
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
pub(super) fn robot(url: &reqwest::Url) -> Option<Robot> {
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
    pub accent: Option<Accent>,
    pub field_links: Vec<(String, String)>,
    pub field_accents: Vec<(String, Accent)>,
    pub fields: Vec<(String, String)>,
    pub generic: Value,
}

#[derive(Clone, Copy)]
pub enum Accent {
    Blue,
    Green,
    Purple,
    Red,
}

impl Accent {
    fn feishu(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Purple => "purple",
            Self::Red => "red",
        }
    }

    fn rgb(self) -> u32 {
        match self {
            Self::Blue => 0x2563eb,
            Self::Green => 0x15803d,
            Self::Purple => 0x7e22ce,
            Self::Red => 0xb91c1c,
        }
    }
}

/// Uses the delivery payload builder without credentials or IO.
pub fn preview(provider: &str, notice: &Notice<'_>) -> Result<Value, Message> {
    let kind = match provider {
        "WeCom" => Some(Robot::WeCom),
        "DingTalk" => Some(Robot::DingTalk),
        "Feishu" | "Lark" => Some(Robot::Feishu),
        "Telegram" => Some(Robot::Telegram),
        "Slack" => Some(Robot::Slack),
        "Discord" => Some(Robot::Discord),
        "Google Chat" => Some(Robot::GoogleChat),
        "Webhook" => None,
        _ => return Err(Message::new("webhook.channelInvalid")),
    };
    let url = reqwest::Url::parse("https://example.invalid/?chat_id=preview").unwrap();
    let payload = match kind {
        Some(kind) => body(kind, &url, notice)?,
        None => notice.generic.clone(),
    };
    let fields: Vec<Value> = lines(notice)
        .map(|(name, value)| {
            let accent = notice
                .field_accents
                .iter()
                .find(|(label, _)| label == name)
                .map(|(_, accent)| match kind {
                    Some(Robot::Feishu) => accent.feishu(),
                    Some(Robot::WeCom) => match accent {
                        Accent::Green => "green",
                        Accent::Blue | Accent::Red => "orange",
                        _ => "gray",
                    },
                    _ => "inherit",
                })
                .unwrap_or("inherit");
            let url = field_url(notice, name, &value);
            json!({"name":name,"value":value,"accent":accent,"url":url})
        })
        .collect();
    Ok(json!({"title":notice.title,"provider":provider,"fields":fields,"payload":payload}))
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
        .map(|(name, value)| {
            (
                name.as_str(),
                if name.is_empty() {
                    value
                        .chars()
                        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                        .take(48000)
                        .collect()
                } else {
                    link_url(value).unwrap_or_else(|| clean(value, 300))
                },
            )
        })
        .filter(|(_, value)| !value.is_empty())
}

fn link_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = reqwest::Url::parse(value.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn field_url(notice: &Notice<'_>, name: &str, value: &str) -> Option<String> {
    notice
        .field_links
        .iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, url)| link_url(url))
        .or_else(|| link_url(value))
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn markdown(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if "\\`*_[]()!<>#".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

fn chat_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape(robot: Robot, value: &str) -> String {
    match robot {
        Robot::Telegram => html(value),
        Robot::Slack => chat_text(value),
        Robot::GoogleChat => value.replace('<', "‹").replace('>', "›"),
        _ => markdown(value),
    }
}

fn formatted_field(robot: Robot, name: &str, value: &str, url: Option<String>) -> String {
    if name.is_empty() {
        return escape(robot, value);
    }
    let name = escape(robot, name);
    if let Some(url) = url {
        let text = escape(robot, value);
        let link = match robot {
            Robot::Telegram => format!("<a href=\"{}\">{text}</a>", html(&url)),
            Robot::Slack => format!(
                "<{}|{}>",
                chat_text(&url.replace('|', "%7C")),
                text.replace('|', "∣")
            ),
            Robot::GoogleChat => format!(
                "<{}|{}>",
                url.replace('|', "%7C")
                    .replace('<', "%3C")
                    .replace('>', "%3E"),
                text.replace('|', "∣")
            ),
            _ => format!("[{text}]({})", url.replace('(', "%28").replace(')', "%29")),
        };
        return format!("{name}：{link}");
    }
    format!("{name}：{}", escape(robot, value))
}

fn formatted(robot: Robot, notice: &Notice<'_>) -> String {
    std::iter::once(escape(robot, &clean(notice.title, 100)))
        .chain(lines(notice).map(|(name, value)| styled_field(robot, notice, name, &value)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn styled_field(robot: Robot, notice: &Notice<'_>, name: &str, value: &str) -> String {
    let text = formatted_field(robot, name, value, field_url(notice, name, value));
    let accent = notice
        .field_accents
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, accent)| accent);
    match (robot, accent) {
        (Robot::Feishu, Some(accent)) => format!("<font color='{}'>{text}</font>", accent.feishu()),
        (Robot::WeCom, Some(accent)) => format!(
            "<font color=\"{}\">{text}</font>",
            match accent {
                Accent::Green => "info",
                Accent::Blue | Accent::Red => "warning",
                // Use native orange for emphasis: comment gray is too faint.
                _ => "comment",
            }
        ),
        _ => text,
    }
}

fn body(robot: Robot, url: &reqwest::Url, notice: &Notice<'_>) -> Result<Value, Message> {
    let title = clean(notice.title, 100);
    let heading = if robot == Robot::WeCom {
        notice.accent.map(|accent| {
            format!(
                "### <font color=\"{}\">{}</font>",
                match accent {
                    Accent::Red => "warning",
                    Accent::Green => "info",
                    _ => "comment",
                },
                markdown(&title)
            )
        })
    } else {
        None
    }
    .unwrap_or_else(|| format!("### {}", markdown(&title)));
    let markdown = std::iter::once(heading)
        .chain(lines(notice).map(|(name, value)| {
            if name.is_empty() {
                markdown(&value)
            } else if field_url(notice, name, &value).is_some()
                || notice.field_accents.iter().any(|(key, _)| key == name)
            {
                format!("> {}", styled_field(robot, notice, name, &value))
            } else {
                format!("> **{}**：{}", markdown(name), markdown(&value))
            }
        }))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(match robot {
        Robot::WeCom => json!({"msgtype":"markdown","markdown":{"content":markdown}}),
        Robot::DingTalk => json!({"msgtype":"markdown","markdown":{"title":title,"text":markdown}}),
        Robot::Feishu => {
            if let Some(accent) = notice.accent {
                let elements: Vec<Value> = lines(notice)
                    .map(|(name, value)| json!({"tag":"div","text":{"tag":"lark_md","content":styled_field(robot,notice,name,&value)}}))
                    .collect();
                return Ok(json!({"msg_type":"interactive","card":{
                    "header":{"template":accent.feishu(),"title":{"tag":"plain_text","content":title}},
                    "elements":elements
                }}));
            }
            let content: Vec<Value> = lines(notice)
                .map(|(name, value)| match field_url(notice,name,&value) {
                    Some(url) => json!([{"tag":"text","text":format!("{name}：")},{"tag":"a","text":value,"href":url}]),
                    None => json!([{"tag":"text","text":if name.is_empty() { value } else { format!("{name}：{value}") }}]),
                })
                .collect();
            json!({"msg_type":"post","content":{"post":{"en_us":{"title":title,"content":content}}}})
        }
        Robot::Telegram => {
            // The Bot API needs a destination; it travels in the configured URL.
            let chat = url
                .query_pairs()
                .find(|(key, value)| key == "chat_id" && !value.is_empty())
                .map(|(_, value)| value.into_owned())
                .ok_or_else(|| Message::new("webhook.chatMissing"))?;
            json!({"chat_id":chat,"text":formatted(robot, notice),"parse_mode":"HTML","disable_web_page_preview":true})
        }
        Robot::Slack if notice.accent.is_some() => json!({"text":title,"attachments":[{
            "color":format!("#{:06x}",notice.accent.unwrap().rgb()),"text":formatted(robot,notice),"mrkdwn_in":["text"]
        }]}),
        Robot::Slack | Robot::GoogleChat => json!({"text":formatted(robot, notice)}),
        // Keep whole lines: cutting the message could break a link's destination.
        Robot::Discord => {
            let text = formatted(robot, notice);
            let mut content = String::new();
            for line in text.lines() {
                if content.chars().count() + usize::from(!content.is_empty()) + line.chars().count()
                    > 1900
                {
                    break;
                }
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(line);
            }
            if let Some(accent) = notice.accent {
                json!({"embeds":[{"title":title,"color":accent.rgb(),"description":content}],"allowed_mentions":{"parse":[]}})
            } else {
                json!({"content":content,"allowed_mentions":{"parse":[]}})
            }
        }
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
    deliver_configured(client, url, &super::channel::Options::default(), notice).await
}

pub async fn deliver_configured(
    client: &reqwest::Client,
    url: &str,
    options: &super::channel::Options,
    notice: &Notice<'_>,
) -> String {
    let mut parsed = match options.resolve(url) {
        Ok(url) => url,
        Err(reason) => {
            return crate::i18n::store_message(
                &Message::new("webhook.notSent")
                    .with("provider", &options.provider)
                    .with_message("reason", reason),
            );
        }
    };
    let detected = if options.provider == "Webhook" {
        None
    } else {
        robot(&parsed)
    };
    let Some(robot) = detected else {
        let mut request = client.post(parsed).json(&notice.generic);
        if !options.bearer_token.is_empty() {
            request = request.bearer_auth(&options.bearer_token);
        }
        return match request.send().await {
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
    let mut message = match body(robot, &parsed, notice) {
        Ok(message) => message,
        Err(reason) => {
            return crate::i18n::store_message(
                &Message::new("webhook.notSent")
                    .with("provider", name)
                    .with_message("reason", reason),
            );
        }
    };
    if let Err(reason) = options.prepare(&mut parsed, &mut message, crate::db::now()) {
        return crate::i18n::store_message(
            &Message::new("webhook.notSent")
                .with("provider", name)
                .with_message("reason", reason),
        );
    }
    match client.post(parsed).json(&message).send().await {
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
    #[test]
    fn previews_share_native_delivery_payloads_and_reject_unknown_providers() {
        let notice = Notice {
            title: "Test",
            accent: Some(Accent::Blue),
            fields: vec![("Message".into(), "<sample>".into())],
            field_links: vec![],
            field_accents: vec![("Message".into(), Accent::Blue)],
            generic: json!({"test":true}),
        };
        let url = reqwest::Url::parse("https://example.invalid/?chat_id=preview").unwrap();
        for (provider, kind) in [
            ("WeCom", Robot::WeCom),
            ("DingTalk", Robot::DingTalk),
            ("Feishu", Robot::Feishu),
            ("Lark", Robot::Feishu),
            ("Telegram", Robot::Telegram),
            ("Slack", Robot::Slack),
            ("Discord", Robot::Discord),
            ("Google Chat", Robot::GoogleChat),
        ] {
            let preview = preview(provider, &notice).unwrap();
            assert_eq!(preview["payload"], body(kind, &url, &notice).unwrap());
            assert_eq!(preview["fields"][0]["value"], "<sample>");
        }
        assert_eq!(
            preview("Webhook", &notice).unwrap()["payload"],
            notice.generic
        );
        assert!(preview("unknown", &notice).is_err());
        assert_eq!(
            preview("Feishu", &notice).unwrap()["fields"][0]["accent"],
            "blue"
        );
        assert_eq!(
            preview("WeCom", &notice).unwrap()["fields"][0]["accent"],
            "orange"
        );
    }

    #[test]
    fn repository_cards_color_titles_stats_and_link_the_repository_name() {
        let notice = Notice {
            title: "Code push",
            accent: Some(Accent::Blue),
            fields: vec![
                ("Repository".into(), "team/app".into()),
                ("Commit message".into(), "fix: <unsafe> & [message]".into()),
                ("Added".into(), "12 lines".into()),
                ("Deleted".into(), "3 lines".into()),
            ],
            field_links: vec![(
                "Repository".into(),
                "https://example.invalid/team/app".into(),
            )],
            field_accents: vec![
                ("Commit message".into(), Accent::Blue),
                ("Added".into(), Accent::Green),
                ("Deleted".into(), Accent::Red),
            ],
            generic: json!({}),
        };
        let url = reqwest::Url::parse("https://example.invalid").unwrap();
        let card = body(Robot::Feishu, &url, &notice).unwrap();
        assert_eq!(card["card"]["header"]["template"], "blue");
        assert_eq!(card["card"]["header"]["title"]["content"], "Code push");
        let elements = card["card"]["elements"].to_string();
        assert!(elements.contains("[team/app](https://example.invalid/team/app)"));
        assert!(elements.contains("<font color='green'>Added：12 lines</font>"));
        assert!(elements.contains("<font color='red'>Deleted：3 lines</font>"));
        assert!(elements.contains("<font color='blue'>Commit message：fix:"));
        assert!(!elements.contains("fix: <unsafe>"));
        let wecom = body(Robot::WeCom, &url, &notice).unwrap();
        let content = wecom["markdown"]["content"].as_str().unwrap();
        assert!(content.contains("<font color=\"warning\">Commit message：fix:"));
        assert!(content.contains("<font color=\"info\">Added：12 lines</font>"));
        assert!(content.contains("<font color=\"warning\">Deleted：3 lines</font>"));

        assert_eq!(
            body(Robot::Slack, &url, &notice).unwrap()["attachments"][0]["color"],
            "#2563eb"
        );
        let discord = body(Robot::Discord, &url, &notice).unwrap();
        assert_eq!(discord["embeds"][0]["color"], 0x2563eb);
        for robot in [
            Robot::WeCom,
            Robot::DingTalk,
            Robot::Slack,
            Robot::Discord,
            Robot::GoogleChat,
        ] {
            let payload = body(robot, &url, &notice).unwrap().to_string();
            assert!(payload.contains("team/app"));
            assert!(payload.contains("12 lines"));
            assert!(!payload.contains("!["));
        }
        let mut invalid = notice;
        invalid.field_links[0].1 = "javascript:alert(1)".into();
        assert!(
            !body(Robot::Feishu, &url, &invalid)
                .unwrap()
                .to_string()
                .contains("javascript:")
        );
    }

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
            field_links: vec![],
            field_accents: vec![],
            accent: None,
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
            json!({"msg_type":"post","content":{"post":{"en_us":{"title":"仓库事件：push","content":[[{"tag":"text","text":"仓库：group/app"}],[{"tag":"text","text":"引用：main x"}]]}}}})
        );
        assert_eq!(
            body(
                Robot::Telegram,
                &url("https://api.telegram.org/bot1:A/sendMessage?chat_id=-100123"),
                &notice
            )
            .unwrap(),
            json!({"chat_id":"-100123","text":text,"parse_mode":"HTML","disable_web_page_preview":true})
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
            json!({"content":text,"allowed_mentions":{"parse":[]}})
        );
    }

    #[test]
    fn links_use_native_platform_formats_and_escape_untrusted_text() {
        let url = "https://example.invalid/commit/abc?left=1&right=2";
        let notice = Notice {
            title: "Push <tag>",
            field_links: vec![],
            field_accents: vec![],
            accent: None,
            fields: vec![
                ("Commit".into(), url.into()),
                ("Author".into(), "<script>*name*".into()),
            ],
            generic: json!({}),
        };
        let any = reqwest::Url::parse("https://example.invalid").unwrap();
        for platform in [Robot::WeCom, Robot::DingTalk, Robot::Discord] {
            assert!(
                body(platform, &any, &notice)
                    .unwrap()
                    .to_string()
                    .contains(&format!("Commit：[{url}]({url})"))
            );
        }
        let google = body(Robot::GoogleChat, &any, &notice).unwrap();
        assert!(
            google["text"]
                .as_str()
                .unwrap()
                .contains(&format!("Commit：<{url}|{url}>"))
        );
        let slack = body(Robot::Slack, &any, &notice).unwrap();
        assert!(
            slack["text"]
                .as_str()
                .unwrap()
                .contains("Commit：<https://example.invalid/commit/abc?left=1&amp;right=2|https://example.invalid/commit/abc?left=1&amp;right=2>")
        );
        let telegram_url =
            reqwest::Url::parse("https://api.telegram.org/bot1:A/sendMessage?chat_id=-1").unwrap();
        let telegram = body(Robot::Telegram, &telegram_url, &notice).unwrap();
        assert_eq!(telegram["parse_mode"], "HTML");
        assert!(telegram["text"].as_str().unwrap().contains(
            "Commit：<a href=\"https://example.invalid/commit/abc?left=1&amp;right=2\">https://example.invalid/commit/abc?left=1&amp;right=2</a>"
        ));
        assert!(
            telegram["text"]
                .as_str()
                .unwrap()
                .contains("&lt;script&gt;")
        );
        let feishu = body(Robot::Feishu, &any, &notice).unwrap();
        assert_eq!(
            feishu["content"]["post"]["en_us"]["content"][0][1]["href"],
            url
        );
        assert_eq!(
            feishu["content"]["post"]["en_us"]["content"][0][1]["tag"],
            "a"
        );
        assert_eq!(
            feishu["content"]["post"]["en_us"]["content"][0][0]["text"],
            "Commit："
        );
        assert_eq!(
            feishu["content"]["post"]["en_us"]["content"][0][1]["text"],
            url
        );
        assert_eq!(
            formatted_field(Robot::WeCom, "链接", url, link_url(url)),
            format!("链接：[{url}]({url})")
        );
        assert!(link_url("javascript:alert(1)").is_none());
        assert!(link_url("https://user:password@example.invalid/").is_none());
        let long = format!("https://example.invalid/{}", "a".repeat(500));
        assert!(formatted_field(Robot::DingTalk, "Commit", &long, link_url(&long)).contains(&long));
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
