//! Provider protocols only. Authorization, encryption and the durable send ledger
//! belong to the notification center. Never retry an ambiguous SMS request.
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) fn is_sms(provider: &str) -> bool {
    matches!(provider, "Aliyun SMS" | "Tencent SMS")
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    pub sms_access_key: String,
    pub sms_secret_key: String,
    pub sms_region: String,
    pub sms_app_id: String,
    pub sms_sign_name: String,
    pub sms_template_code: String,
    /// Tencent parameters are positional: preserve this order, never sort it.
    pub sms_parameter_names: Vec<String>,
}
pub(crate) fn phone(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('+') else {
        return false;
    };
    (7..=15).contains(&digits.len())
        && !digits.starts_with('0')
        && digits.bytes().all(|b| b.is_ascii_digit())
}
impl Config {
    pub fn validate(&self, provider: &str, recipients: &[String]) -> bool {
        let token = |s: &str, max: usize| {
            !s.is_empty()
                && s.len() <= max
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        };
        token(&self.sms_access_key, 128)
            && !self.sms_secret_key.is_empty()
            && self.sms_secret_key.len() <= 256
            && self.sms_sign_name.chars().count() <= 50
            && !self.sms_sign_name.trim().is_empty()
            && !self.sms_sign_name.chars().any(char::is_control)
            && token(&self.sms_template_code, 128)
            && (provider != "Tencent SMS"
                || (token(&self.sms_region, 64)
                    && !self.sms_app_id.is_empty()
                    && self.sms_app_id.len() <= 32
                    && self.sms_app_id.bytes().all(|b| b.is_ascii_digit())))
            && self.sms_parameter_names.len() <= 20
            && self.sms_parameter_names.iter().all(|s| token(s, 64))
            && self
                .sms_parameter_names
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.sms_parameter_names.len()
            && recipients.len() <= 20
            && recipients.iter().all(|s| {
                phone(s) && (provider != "Aliyun SMS" || s.starts_with("+86") && s.len() == 14)
            })
    }
    pub fn parameters(
        &self,
        variables: &BTreeMap<String, String>,
    ) -> Option<BTreeMap<String, String>> {
        self.sms_parameter_names
            .iter()
            .map(|name| {
                let value = variables.get(name)?;
                if value.is_empty()
                    || value.chars().count() > 500
                    || value.chars().any(char::is_control)
                {
                    return None;
                }
                Some((name.clone(), value.clone()))
            })
            .collect()
    }
}
fn sha(value: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(value.as_ref()))
}
fn mac(key: &[u8], value: &str) -> Vec<u8> {
    let mut hmac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key size");
    hmac.update(value.as_bytes());
    hmac.finalize().into_bytes().to_vec()
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
struct Request {
    url: String,
    headers: BTreeMap<String, String>,
    body: String,
}
fn request(
    provider: &str,
    config: &Config,
    to: &[String],
    parameters: &BTreeMap<String, String>,
    timestamp: i64,
    nonce: &str,
) -> Request {
    if provider == "Aliyun SMS" {
        let host = "dysmsapi.aliyuncs.com";
        let query = BTreeMap::from([
            (
                "PhoneNumbers",
                to.iter()
                    .map(|s| s.trim_start_matches("+86"))
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ("SignName", config.sms_sign_name.clone()),
            ("TemplateCode", config.sms_template_code.clone()),
            (
                "TemplateParam",
                serde_json::to_string(parameters).expect("string map"),
            ),
        ])
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
        let mut headers = BTreeMap::from([
            ("host".into(), host.into()),
            ("x-acs-action".into(), "SendSms".into()),
            ("x-acs-version".into(), "2017-05-25".into()),
            (
                "x-acs-date".into(),
                chrono::DateTime::from_timestamp(timestamp, 0)
                    .expect("current timestamp")
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            ),
            ("x-acs-signature-nonce".into(), nonce.into()),
            ("x-acs-content-sha256".into(), sha("")),
        ]);
        let names = headers.keys().cloned().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect::<String>();
        let canonical = format!(
            "POST\n/\n{query}\n{canonical_headers}\n{names}\n{}",
            sha("")
        );
        let signature = hex::encode(mac(
            config.sms_secret_key.as_bytes(),
            &format!("ACS3-HMAC-SHA256\n{}", sha(canonical)),
        ));
        headers.insert(
            "authorization".into(),
            format!(
                "ACS3-HMAC-SHA256 Credential={},SignedHeaders={names},Signature={signature}",
                config.sms_access_key
            ),
        );
        Request {
            url: format!("https://{host}/?{query}"),
            headers,
            body: String::new(),
        }
    } else {
        let host = "sms.tencentcloudapi.com";
        let body = json!({"PhoneNumberSet":to,"SmsSdkAppId":config.sms_app_id,"SignName":config.sms_sign_name,"TemplateId":config.sms_template_code,"TemplateParamSet":config.sms_parameter_names.iter().map(|name| &parameters[name]).collect::<Vec<_>>()}).to_string();
        let date = chrono::DateTime::from_timestamp(timestamp, 0)
            .expect("current timestamp")
            .format("%Y-%m-%d")
            .to_string();
        let scope = format!("{date}/sms/tc3_request");
        let canonical = format!(
            "POST\n/\n\ncontent-type:application/json; charset=utf-8\nhost:{host}\n\ncontent-type;host\n{}",
            sha(&body)
        );
        let signing = mac(
            &mac(
                &mac(format!("TC3{}", config.sms_secret_key).as_bytes(), &date),
                "sms",
            ),
            "tc3_request",
        );
        let signature = hex::encode(mac(
            &signing,
            &format!("TC3-HMAC-SHA256\n{timestamp}\n{scope}\n{}", sha(canonical)),
        ));
        let headers = BTreeMap::from([
            ("host".into(), host.into()),
            (
                "content-type".into(),
                "application/json; charset=utf-8".into(),
            ),
            ("x-tc-action".into(), "SendSms".into()),
            ("x-tc-version".into(), "2021-01-11".into()),
            ("x-tc-timestamp".into(), timestamp.to_string()),
            ("x-tc-region".into(), config.sms_region.clone()),
            (
                "authorization".into(),
                format!(
                    "TC3-HMAC-SHA256 Credential={}/{scope}, SignedHeaders=content-type;host, Signature={signature}",
                    config.sms_access_key
                ),
            ),
        ]);
        Request {
            url: format!("https://{host}/"),
            headers,
            body,
        }
    }
}
// Acceptance by the provider is not confirmation of delivery to the handset.
fn outcome(provider: &str, value: &Value, to: &[String]) -> &'static str {
    if provider == "Aliyun SMS" {
        return match value.get("Code").and_then(Value::as_str) {
            Some("OK") => "succeeded",
            Some(_) => "failed",
            None => "unknown",
        };
    }
    if value
        .pointer("/Response/Error/Code")
        .and_then(Value::as_str)
        .is_some()
    {
        return "failed";
    }
    let Some(statuses) = value
        .pointer("/Response/SendStatusSet")
        .and_then(Value::as_array)
    else {
        return "unknown";
    };
    let codes = statuses
        .iter()
        .filter_map(|s| s["Code"].as_str())
        .collect::<Vec<_>>();
    let returned = statuses
        .iter()
        .filter_map(|s| s["PhoneNumber"].as_str())
        .collect::<std::collections::HashSet<_>>();
    if statuses.len() != to.len()
        || codes.len() != to.len()
        || returned.len() != to.len()
        || !to.iter().all(|s| returned.contains(s.as_str()))
    {
        return "unknown";
    }
    if codes.iter().all(|s| *s == "Ok") {
        "succeeded"
    } else if codes.iter().all(|s| *s != "Ok") {
        "failed"
    } else {
        "unknown"
    }
}
async fn execute(
    client: &reqwest::Client,
    request: Request,
    provider: &str,
    to: &[String],
) -> &'static str {
    let mut builder = client.post(&request.url).body(request.body);
    for (key, value) in request.headers {
        builder = builder.header(key, value);
    }
    let Ok(mut response) = builder.send().await else {
        return "unknown";
    };
    // Bound replies without logging provider bodies, phone numbers or secrets.
    if response.status().is_redirection() {
        return "unknown";
    }
    let accepted_http = response.status().is_success();
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= 65536 => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            _ => return "unknown",
        }
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => match outcome(provider, &value, to) {
            "succeeded" if !accepted_http => "unknown",
            state => state,
        },
        Err(_) => "unknown",
    }
}
pub(crate) async fn send(
    provider: &str,
    config: &Config,
    to: &[String],
    parameters: &BTreeMap<String, String>,
) -> (String, &'static str) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return (
            crate::i18n::store_message(&crate::i18n::Message::new("notification.sendFailed")),
            "failed",
        );
    };
    let request = request(
        provider,
        config,
        to,
        parameters,
        chrono::Utc::now().timestamp(),
        &uuid::Uuid::new_v4().to_string(),
    );
    let state = execute(&client, request, provider, to).await;
    let key = match state {
        "succeeded" => "notification.smsAccepted",
        "failed" => "notification.sendFailed",
        _ => "notification.unknown",
    };
    (
        crate::i18n::store_message(&crate::i18n::Message::new(key)),
        state,
    )
}

#[cfg(test)]
mod tests;
