//! Fixed operational keys; only labels, colors and order are configurable.
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
