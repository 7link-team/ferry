//! Application resource ceilings. Missing or null ceilings are unlimited.
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ResourcePolicy {
    #[serde(deserialize_with = "positive_limit")]
    pub http_body_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub websocket_message_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub cluster_message_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub tool_request_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub tool_response_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_native_line_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_event_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_turn_output_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_prompt_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_schema_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_tool_calls: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub agent_tool_concurrency: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub provider_concurrency: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub provider_requests_per_second: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub task_timeout_seconds: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub preview_connections_per_entry: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub preview_connections_per_host: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub preview_body_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub host_task_concurrency: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub pipeline_concurrent_runs: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub conversation_concurrent_runs: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_concurrent_runs: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub cron_concurrent_runs: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub file_upload_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub file_count: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub file_total_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub directory_entries: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub command_output_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub command_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_definition_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_pinned_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_nodes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_node_executions: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_code_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_packet_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_items: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_platform_pages: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub workflow_evaluations: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub sandbox_memory_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub sandbox_process_memory_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub sandbox_timeout_ms: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_input_bytes: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_pages: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_sections: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_metrics: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_concurrency: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub report_timeout_seconds: Option<u64>,
    #[serde(deserialize_with = "positive_limit")]
    pub log_output_bytes: Option<u64>,
}

fn positive_limit<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<u64>::deserialize(deserializer)?;
    if value.is_some_and(|n| n == 0 || n > 9_007_199_254_740_991) {
        return Err(serde::de::Error::custom(
            "resource ceiling must be a positive safe integer",
        ));
    }
    Ok(value)
}

pub fn allows(limit: Option<u64>, actual: usize) -> bool {
    limit.is_none_or(|limit| u64::try_from(actual).is_ok_and(|actual| actual <= limit))
}

pub fn byte_limit(limit: Option<u64>) -> usize {
    limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unlimited_default_and_explicit_boundaries() {
        let policy: ResourcePolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy, ResourcePolicy::default());
        assert!(allows(policy.file_count, usize::MAX));
        assert_eq!(byte_limit(None), usize::MAX);
        assert!(allows(Some(5), 5));
        assert!(!allows(Some(5), 6));
        assert_eq!(byte_limit(Some(5)), 5);
        assert_eq!(byte_limit(Some(u64::MAX)), usize::MAX);
    }
    #[test]
    fn rejects_unknown_zero_unsafe_and_noninteger_limits() {
        for value in [
            r#"{"file_count":0}"#,
            r#"{"file_count":9007199254740992}"#,
            r#"{"file_count":-1}"#,
            r#"{"file_count":1.5}"#,
            r#"{"unknown":1}"#,
        ] {
            assert!(
                serde_json::from_str::<ResourcePolicy>(value).is_err(),
                "{value}"
            );
        }
        let policy: ResourcePolicy =
            serde_json::from_str(r#"{"file_count":9007199254740991,"file_upload_bytes":null}"#)
                .unwrap();
        assert_eq!(policy.file_count, Some(9007199254740991));
        assert_eq!(policy.file_upload_bytes, None);
    }
}
