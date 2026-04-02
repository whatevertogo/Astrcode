use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ToolOutputStream, UserMessageOrigin};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    Auto,
    Manual,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StorageEvent {
    SessionStart {
        session_id: String,
        timestamp: DateTime<Utc>,
        working_dir: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_storage_seq: Option<u64>,
    },
    UserMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        timestamp: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "is_default_user_message_origin")]
        origin: UserMessageOrigin,
    },
    AssistantDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        token: String,
    },
    ThinkingDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        token: String,
    },
    AssistantFinal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<DateTime<Utc>>,
    },
    ToolCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolCallDelta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        stream: ToolOutputStream,
        delta: String,
    },
    ToolResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        output: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
        duration_ms: u64,
    },
    PromptMetrics {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        step_index: u32,
        estimated_tokens: u32,
        context_window: u32,
        effective_window: u32,
        threshold_tokens: u32,
        truncated_tool_results: u32,
    },
    CompactApplied {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        trigger: CompactTrigger,
        summary: String,
        preserved_recent_turns: u32,
        pre_tokens: u32,
        post_tokens_estimate: u32,
        messages_removed: u32,
        tokens_freed: u32,
        timestamp: DateTime<Utc>,
    },
    TurnDone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        timestamp: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<DateTime<Utc>>,
    },
}

impl StorageEvent {
    pub fn turn_id(&self) -> Option<&str> {
        match self {
            Self::UserMessage { turn_id, .. }
            | Self::AssistantDelta { turn_id, .. }
            | Self::ThinkingDelta { turn_id, .. }
            | Self::AssistantFinal { turn_id, .. }
            | Self::ToolCall { turn_id, .. }
            | Self::ToolCallDelta { turn_id, .. }
            | Self::ToolResult { turn_id, .. }
            | Self::PromptMetrics { turn_id, .. }
            | Self::CompactApplied { turn_id, .. }
            | Self::TurnDone { turn_id, .. }
            | Self::Error { turn_id, .. } => turn_id.as_deref(),
            Self::SessionStart { .. } => None,
        }
    }
}

fn is_default_user_message_origin(origin: &UserMessageOrigin) -> bool {
    matches!(origin, UserMessageOrigin::User)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub storage_seq: u64,
    #[serde(flatten)]
    pub event: StorageEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StoredEventLine {
    Stored(StoredEvent),
    Legacy(StorageEvent),
}

impl StoredEventLine {
    pub fn into_stored(self, fallback_seq: u64) -> StoredEvent {
        match self {
            Self::Stored(stored) => stored,
            Self::Legacy(event) => StoredEvent {
                storage_seq: fallback_seq,
                event,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::Value;

    use super::{CompactTrigger, StorageEvent};

    #[test]
    fn tool_result_deserializes_legacy_lines_without_error_or_metadata() {
        let event: StorageEvent = serde_json::from_str(
            r#"{"type":"toolResult","turn_id":"turn-1","tool_call_id":"call-1","tool_name":"readFile","output":"hello","success":true,"duration_ms":12}"#,
        )
        .expect("legacy tool result should deserialize");

        match event {
            StorageEvent::ToolResult {
                error, metadata, ..
            } => {
                assert_eq!(error, None);
                assert_eq!(metadata, None);
            }
            other => panic!("expected tool result, got {other:?}"),
        }
    }

    #[test]
    fn turn_done_deserializes_legacy_lines_without_reason() {
        let event: StorageEvent = serde_json::from_str(
            r#"{"type":"turnDone","turn_id":"turn-1","timestamp":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("legacy turn done should deserialize");

        match event {
            StorageEvent::TurnDone { reason, .. } => {
                assert_eq!(reason, None);
            }
            other => panic!("expected turn done, got {other:?}"),
        }
    }

    #[test]
    fn prompt_metrics_round_trip_preserves_all_fields() {
        let event = StorageEvent::PromptMetrics {
            turn_id: Some("turn-1".to_string()),
            step_index: 2,
            estimated_tokens: 1_234,
            context_window: 128_000,
            effective_window: 108_000,
            threshold_tokens: 97_200,
            truncated_tool_results: 3,
        };

        let encoded = serde_json::to_value(&event).expect("event should serialize");
        let decoded: StorageEvent =
            serde_json::from_value(encoded.clone()).expect("event should deserialize");

        match decoded {
            StorageEvent::PromptMetrics {
                turn_id,
                step_index,
                estimated_tokens,
                context_window,
                effective_window,
                threshold_tokens,
                truncated_tool_results,
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-1"));
                assert_eq!(step_index, 2);
                assert_eq!(estimated_tokens, 1_234);
                assert_eq!(context_window, 128_000);
                assert_eq!(effective_window, 108_000);
                assert_eq!(threshold_tokens, 97_200);
                assert_eq!(truncated_tool_results, 3);
            }
            other => panic!("expected prompt metrics, got {other:?}"),
        }

        let expected: Value = serde_json::json!({
            "type": "promptMetrics",
            "turn_id": "turn-1",
            "step_index": 2,
            "estimated_tokens": 1234,
            "context_window": 128000,
            "effective_window": 108000,
            "threshold_tokens": 97200,
            "truncated_tool_results": 3,
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn compact_applied_round_trip_preserves_all_fields() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("timestamp should build");
        let event = StorageEvent::CompactApplied {
            turn_id: Some("turn-2".to_string()),
            trigger: CompactTrigger::Manual,
            summary: "condensed work".to_string(),
            preserved_recent_turns: 2,
            pre_tokens: 2_000,
            post_tokens_estimate: 600,
            messages_removed: 5,
            tokens_freed: 1_400,
            timestamp,
        };

        let encoded = serde_json::to_value(&event).expect("event should serialize");
        let decoded: StorageEvent =
            serde_json::from_value(encoded.clone()).expect("event should deserialize");

        match decoded {
            StorageEvent::CompactApplied {
                turn_id,
                trigger,
                summary,
                preserved_recent_turns,
                pre_tokens,
                post_tokens_estimate,
                messages_removed,
                tokens_freed,
                timestamp: decoded_timestamp,
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-2"));
                assert_eq!(trigger, CompactTrigger::Manual);
                assert_eq!(summary, "condensed work");
                assert_eq!(preserved_recent_turns, 2);
                assert_eq!(pre_tokens, 2_000);
                assert_eq!(post_tokens_estimate, 600);
                assert_eq!(messages_removed, 5);
                assert_eq!(tokens_freed, 1_400);
                assert_eq!(decoded_timestamp, timestamp);
            }
            other => panic!("expected compact applied, got {other:?}"),
        }

        assert_eq!(
            encoded,
            serde_json::json!({
                "type": "compactApplied",
                "turn_id": "turn-2",
                "trigger": "manual",
                "summary": "condensed work",
                "preserved_recent_turns": 2,
                "pre_tokens": 2000,
                "post_tokens_estimate": 600,
                "messages_removed": 5,
                "tokens_freed": 1400,
                "timestamp": "2026-01-02T03:04:05Z",
            })
        );
    }
}
