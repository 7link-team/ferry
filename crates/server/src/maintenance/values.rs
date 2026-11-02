//! Typed dictionary/configuration values shared by both resource APIs.
use crate::error::{ApiError, Result};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const TYPES: &[&str] = &[
    "text",
    "num",
    "tab",
    "selection",
    "image",
    "video",
    "file",
    "code",
    "markdown",
    "html",
    "richtext",
    "json",
];

pub struct TypedValue {
    pub value: Value,
    pub schema: Value,
}

fn invalid(key: &'static str) -> ApiError {
    ApiError::localized(
        axum::http::StatusCode::BAD_REQUEST,
        crate::i18n::Message::new(key),
    )
}

pub fn validate(kind: &str, raw: &str, schema: &str) -> Result<TypedValue> {
    if !TYPES.contains(&kind) {
        return Err(invalid("typed.kindUnsupported"));
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| invalid("typed.valueJsonRequired"))?;
    let schema: Value =
        serde_json::from_str(schema).map_err(|_| invalid("typed.schemaJsonRequired"))?;
    let object = schema
        .as_object()
        .ok_or_else(|| invalid("typed.schemaObjectRequired"))?;
    for (key, v) in object {
        match key.as_str() {
            "maxLength" | "maxFiles" | "minSelect" | "maxSelect" | "decimals" => {
                let n = v
                    .as_u64()
                    .ok_or_else(|| invalid("typed.countConstraintInvalid"))?;
                if key == "decimals" && n > 10 {
                    return Err(invalid("typed.decimalsInvalid"));
                }
            }
            "min" | "max" | "maxSizeMb" => {
                let n = v
                    .as_f64()
                    .ok_or_else(|| invalid("typed.numberConstraintInvalid"))?;
                if key == "maxSizeMb" && n < 0.0 {
                    return Err(invalid("typed.sizeConstraintInvalid"));
                }
            }
            "lang" | "accept" => {
                if !v.is_string() {
                    return Err(invalid("typed.textConstraintInvalid"));
                }
            }
            _ => return Err(invalid("typed.unknownConstraint")),
        }
    }
    if let (Some(min), Some(max)) = (schema["min"].as_f64(), schema["max"].as_f64()) {
        if min > max {
            return Err(invalid("typed.boundsInvalid"));
        }
    }
    if let (Some(min), Some(max)) = (schema["minSelect"].as_u64(), schema["maxSelect"].as_u64()) {
        if max > 0 && min > max {
            return Err(invalid("typed.selectionBoundsInvalid"));
        }
    }
    match kind {
        "text" | "code" | "markdown" | "html" | "richtext" => {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("typed.textRequired"))?;
            if text.contains('\0') {
                return Err(invalid("typed.textNulInvalid"));
            }
            if schema["maxLength"]
                .as_u64()
                .is_some_and(|n| n > 0 && text.chars().count() as u64 > n)
            {
                return Err(invalid("typed.textTooLong"));
            }
        }
        "num" => {
            let n = value
                .as_f64()
                .ok_or_else(|| invalid("typed.numberRequired"))?;
            if schema["min"].as_f64().is_some_and(|v| n < v)
                || schema["max"].as_f64().is_some_and(|v| n > v)
            {
                return Err(invalid("typed.numberOutOfRange"));
            }
            if let Some(decimals) = schema["decimals"].as_u64() {
                let scaled = n * 10_f64.powi(decimals as i32);
                if (scaled - scaled.round()).abs() > 1e-7 * scaled.abs().max(1.0) {
                    return Err(invalid("typed.precisionExceeded"));
                }
            }
        }
        "tab" | "selection" => {
            let options = value
                .as_array()
                .ok_or_else(|| invalid("typed.optionsArrayRequired"))?;
            let mut keys = HashSet::new();
            for item in options {
                let key = item
                    .get("value")
                    .or_else(|| item.get("key"))
                    .and_then(Value::as_str);
                let label = item.get("label").and_then(Value::as_str);
                if key.is_none_or(|v| {
                    v.trim().is_empty() || v.chars().count() > 100 || v.contains('\0')
                }) || label.is_none_or(|v| {
                    v.trim().is_empty() || v.chars().count() > 100 || v.contains('\0')
                }) || !keys.insert(key.unwrap_or_default())
                {
                    return Err(invalid("typed.optionsInvalid"));
                }
            }
            if options.len() > 200 {
                return Err(invalid("typed.optionsTooMany"));
            }
        }
        "image" | "video" | "file" => {
            let files = value
                .as_array()
                .ok_or_else(|| invalid("typed.filesArrayRequired"))?;
            if schema["maxFiles"]
                .as_u64()
                .is_some_and(|n| n > 0 && files.len() as u64 > n)
            {
                return Err(invalid("typed.filesTooMany"));
            }
            for file in files {
                let url = file
                    .as_str()
                    .or_else(|| file.get("url").and_then(Value::as_str))
                    .or_else(|| file.get("imageFileUrl").and_then(Value::as_str))
                    .ok_or_else(|| invalid("typed.fileUrlRequired"))?;
                if !(url.starts_with("https://")
                    || url.starts_with("http://")
                    || (url.starts_with('/') && !url.starts_with("//")))
                    || url.chars().any(|c| c.is_control() || c == '\\')
                {
                    return Err(invalid("typed.fileUrlInvalid"));
                }
                if let (Some(limit), Some(size)) = (
                    schema["maxSizeMb"].as_f64(),
                    file.get("size").and_then(Value::as_f64),
                ) {
                    if size < 0.0 || (limit > 0.0 && size > limit * 1024.0 * 1024.0) {
                        return Err(invalid("typed.fileTooLarge"));
                    }
                }
                let content_type = file
                    .get("content_type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if (kind == "image" && !content_type.starts_with("image/"))
                    || (kind == "video" && !content_type.starts_with("video/"))
                {
                    return Err(invalid("typed.mediaTypeInvalid"));
                }
                if let Some(accept) = schema["accept"].as_str().filter(|s| !s.trim().is_empty()) {
                    let name = file
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(url)
                        .to_ascii_lowercase();
                    let allowed = accept.split(',').map(str::trim).any(|pattern| {
                        let pattern = pattern.to_ascii_lowercase();
                        pattern == "*/*"
                            || (pattern.starts_with('.') && name.ends_with(&pattern))
                            || pattern == content_type
                            || pattern.strip_suffix("/*").is_some_and(|prefix| {
                                content_type.starts_with(&format!("{prefix}/"))
                            })
                    });
                    if !allowed {
                        return Err(invalid("typed.fileTypeInvalid"));
                    }
                }
            }
        }
        "json" => {}
        _ => unreachable!(),
    }
    Ok(TypedValue { value, schema })
}

pub fn options(value: &Value) -> Value {
    json!(value.as_array().into_iter().flatten().map(|item| json!({
        "value":item.get("value").or_else(|| item.get("key")).and_then(Value::as_str).unwrap_or_default(),
        "label":item["label"].as_str().unwrap_or_default()
    })).collect::<Vec<_>>())
}

pub fn summary(raw: &str) -> String {
    let value = serde_json::from_str::<Value>(raw).unwrap_or(Value::Null);
    let summary = match value {
        Value::String(text) => text,
        Value::Array(items) => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new(if items.len() == 1 {
                "typed.itemSummary"
            } else {
                "typed.itemsSummary"
            })
            .with("count", items.len().to_string()),
        ),
        Value::Object(items) => crate::i18n::render(
            crate::i18n::current_locale(),
            &crate::i18n::Message::new(if items.len() == 1 {
                "typed.fieldSummary"
            } else {
                "typed.fieldsSummary"
            })
            .with("count", items.len().to_string()),
        ),
        other => other.to_string(),
    };
    let short: String = summary.chars().take(120).collect();
    if summary.chars().count() > 120 {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn summaries_follow_reader_locale_without_interpreting_user_text() {
        use crate::i18n::{Locale, with_locale};
        let user_text = r#"{"$system_message":{"key":"error.forbidden"}}"#;
        for (locale, items, fields) in [
            (Locale::EnUs, "2 items", "1 field"),
            (Locale::ZhCn, "2 项", "1 个字段"),
            (Locale::ZhTw, "2 項", "1 個欄位"),
            (Locale::JaJp, "2 件", "1 フィールド"),
            (Locale::KoKr, "2개 항목", "1개 필드"),
            (Locale::MsMy, "2 item", "1 medan"),
        ] {
            with_locale(locale, async {
                assert_eq!(summary("[1,2]"), items);
                assert_eq!(summary(r#"{"a":1}"#), fields);
                assert_eq!(summary(&json!(user_text).to_string()), user_text);
                assert_eq!(summary("0"), "0");
                let error = validate("text", "42", "{}").err().unwrap();
                assert_eq!(error.2.as_ref().unwrap().key, "typed.textRequired");
                assert_eq!(
                    crate::i18n::render_stored(locale, &error.stored()),
                    error.render(locale)
                );
                if locale != Locale::EnUs {
                    assert_ne!(error.render(locale), error.1);
                }
            })
            .await;
        }
    }

    #[test]
    fn zero_bounds_and_decimal_precision_are_enforced() {
        assert!(validate("num", "-1", r#"{"min":0}"#).is_err());
        assert!(validate("num", "1", r#"{"max":0}"#).is_err());
        assert!(validate("num", "0", r#"{"min":0,"max":0}"#).is_ok());
        assert!(validate("num", "1.25", r#"{"decimals":1}"#).is_err());
        assert!(validate("num", "1.25", r#"{"decimals":2}"#).is_ok());
    }

    #[test]
    fn all_types_validate_and_malformed_values_fail() {
        for kind in ["text", "code", "markdown", "html", "richtext"] {
            assert!(validate(kind, r#""中文""#, r#"{"maxLength":2}"#).is_ok());
            assert!(validate(kind, r#""中文好""#, r#"{"maxLength":2}"#).is_err());
            assert!(validate(kind, "{}", "{}").is_err());
        }
        for kind in ["tab", "selection"] {
            assert!(validate(kind, r#"[{"key":"a","label":"A"}]"#, "{}").is_ok());
            assert!(
                validate(
                    kind,
                    r#"[{"key":"a","label":"A"},{"value":"a","label":"B"}]"#,
                    "{}"
                )
                .is_err()
            );
        }
        for kind in ["image", "video", "file"] {
            let content_type = match kind {
                "image" => "image/png",
                "video" => "video/mp4",
                _ => "text/plain",
            };
            assert!(
                validate(
                    kind,
                    &json!([{"url":"https://example.com/file","content_type":content_type}])
                        .to_string(),
                    "{}"
                )
                .is_ok()
            );
            assert!(validate(kind, r#"[{"url":"javascript:alert(1)"}]"#, "{}").is_err());
            assert!(validate(kind, r#"["//example.com/file"]"#, "{}").is_err());
            assert!(validate(kind, r#"["/a","/b"]"#, r#"{"maxFiles":1}"#).is_err());
        }
        assert!(validate("json", r#"{"any":[1,true,null]}"#, "{}").is_ok());
        assert!(validate("json", "{broken", "{}").is_err());
        assert!(validate("json", "null", "[]").is_err());
        assert!(
            validate(
                "image",
                r#"[{"url":"/f","content_type":"text/html"}]"#,
                "{}"
            )
            .is_err()
        );
        assert!(
            validate(
                "video",
                r#"[{"url":"/f","content_type":"image/png"}]"#,
                "{}"
            )
            .is_err()
        );
        assert!(
            validate(
                "file",
                r#"[{"url":"/f","name":"a.txt","content_type":"text/plain","size":2}]"#,
                r#"{"accept":".txt","maxSizeMb":1}"#
            )
            .is_ok()
        );
        assert!(
            validate(
                "file",
                r#"[{"url":"/f","name":"a.exe","content_type":"application/octet-stream"}]"#,
                r#"{"accept":".txt,text/plain"}"#
            )
            .is_err()
        );
        assert!(
            validate(
                "file",
                r#"[{"url":"/f","name":"a.txt","size":2097152}]"#,
                r#"{"maxSizeMb":1}"#
            )
            .is_err()
        );
    }
}
