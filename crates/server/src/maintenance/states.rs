//! Fixed operational keys; only translated labels, colors and order are configurable.
use crate::i18n::{Locale, Message};
use serde_json::{Value, json};

fn label_key(dictionary: &str, value: &str) -> String {
    if dictionary == "host-state" {
        format!("maintenance.{value}")
    } else {
        format!("maintenance.dictionary.{dictionary}.{value}")
    }
}

/// Localize legacy seeded labels at read time, without rewriting workspace data.
pub(super) fn presentation(dictionary: &str, mut options: Value) -> Value {
    let Some(expected) = controlled_values(dictionary) else {
        return options;
    };
    for option in options.as_array_mut().into_iter().flatten() {
        let value = option["value"].as_str().unwrap_or_default();
        if !expected.contains(&value) {
            continue;
        }
        let key = label_key(dictionary, value);
        let message = Message::new(key);
        let canonical = Locale::ALL.iter().any(|locale| {
            option["label"].as_str() == Some(crate::i18n::render(*locale, &message).as_str())
        });
        let mut labels = if canonical {
            Locale::ALL
                .iter()
                .map(|locale| {
                    (
                        locale.tag().to_owned(),
                        json!(crate::i18n::render(*locale, &message)),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
        } else {
            serde_json::Map::new()
        };
        if let Some(overrides) = option["labels"].as_object() {
            labels.extend(overrides.clone());
        }
        if !labels.is_empty() {
            option["labels"] = Value::Object(labels);
        }
    }
    options
}

/// Generated translations are presentation metadata, not permanent user overrides.
pub(super) fn normalize(dictionary: &str, options: &mut Value) {
    if controlled_values(dictionary).is_none() {
        return;
    }
    for option in options.as_array_mut().into_iter().flatten() {
        let message = Message::new(label_key(
            dictionary,
            option["value"].as_str().unwrap_or_default(),
        ));
        if let Some(labels) = option["labels"].as_object_mut() {
            labels.retain(|locale, label| {
                Locale::from_tag(locale).is_some_and(|locale| {
                    label.as_str() != Some(crate::i18n::render(locale, &message).as_str())
                })
            });
            if labels.is_empty() {
                option.as_object_mut().unwrap().remove("labels");
            }
        }
    }
}
const DEFINITIONS: &[(&str, &[&str])] = &[
    (
        "pipeline-state",
        &[
            "queued",
            "cancelling",
            "running",
            "waiting",
            "pending",
            "approved",
            "succeeded",
            "failed",
            "cancelled",
            "timed_out",
            "skipped",
        ],
    ),
    (
        "deployment-state",
        &[
            "pending",
            "skipped",
            "idle",
            "queued",
            "running",
            "succeeded",
            "failed",
            "needs-review",
        ],
    ),
    (
        "cron-state",
        &[
            "queued",
            "running",
            "succeeded",
            "failed",
            "skipped",
            "needs-review",
        ],
    ),
    ("alert-state", &["firing", "recovered"]),
    (
        "repository-state",
        &["connected", "refreshing", "reauthorize"],
    ),
    (
        "insight-state",
        &[
            "pending",
            "queued",
            "running",
            "complete",
            "failed",
            "partial",
            "unsupported",
            "blocked",
            "retry_wait",
            "unavailable",
        ],
    ),
];
pub(super) fn controlled_values(key: &str) -> Option<&'static [&'static str]> {
    if key == "host-state" {
        return Some(&crate::hosts::STATES);
    }
    DEFINITIONS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, values)| *values)
}
pub(super) fn definitions() -> Vec<(&'static str, String, Vec<(&'static str, String)>)> {
    DEFINITIONS
        .iter()
        .map(|(key, values)| {
            (
                *key,
                crate::i18n::render(
                    crate::i18n::current_locale(),
                    &crate::i18n::Message::new(format!("maintenance.dictionary.{key}")),
                ),
                values
                    .iter()
                    .map(|value| {
                        (
                            *value,
                            crate::i18n::render(
                                crate::i18n::current_locale(),
                                &crate::i18n::Message::new(format!(
                                    "maintenance.dictionary.{key}.{value}"
                                )),
                            ),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_controlled_state_keeps_machine_values_and_localizes_legacy_seeds_in_all_locales() {
        for dictionary in
            std::iter::once("host-state").chain(DEFINITIONS.iter().map(|(key, _)| *key))
        {
            let expected = controlled_values(dictionary).unwrap();
            for seeded_locale in Locale::ALL {
                let raw = json!(expected.iter().rev().map(|value| json!({"value":value,"label":crate::i18n::render(seeded_locale, &Message::new(label_key(dictionary, value))),"color":"#123456"})).collect::<Vec<_>>());
                let view = presentation(dictionary, raw.clone());
                for (index, value) in expected.iter().rev().enumerate() {
                    assert_eq!(view[index]["value"], *value);
                    assert_eq!(view[index]["color"], "#123456");
                    for reader in Locale::ALL {
                        assert_eq!(
                            view[index]["labels"][reader.tag()],
                            crate::i18n::render(
                                reader,
                                &Message::new(label_key(dictionary, value))
                            )
                        );
                    }
                }
                let mut saved = view;
                normalize(dictionary, &mut saved);
                assert_eq!(
                    saved, raw,
                    "generated labels are not frozen into workspace data"
                );
            }
        }
    }

    #[test]
    fn custom_names_and_locale_overrides_do_not_change_state_values_or_colors() {
        let raw = json!([{"value":"online","label":"Online","color":"#123456","labels":{"ja-JP":"Custom Japanese"}}]);
        let view = presentation("host-state", raw.clone());
        assert_eq!(view[0]["labels"]["ja-JP"], "Custom Japanese");
        assert_eq!(view[0]["labels"]["zh-CN"], "在线");
        let mut saved = view;
        normalize("host-state", &mut saved);
        assert_eq!(saved, raw);
        let custom = json!([{"value":"online","label":"User's literal alias","labels":{"ko-KR":"Custom Korean"}}]);
        assert_eq!(presentation("host-state", custom.clone()), custom);
        assert_eq!(presentation("user-defined", raw.clone()), raw);
    }
}
