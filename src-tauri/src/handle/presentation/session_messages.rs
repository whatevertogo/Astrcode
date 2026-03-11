use serde::{Deserialize, Serialize};
use serde_json::Value;

use astrcode_core::{action::split_assistant_content, StorageEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionMessage {
    User {
        content: String,
        timestamp: String,
    },
    Assistant {
        content: String,
        #[serde(rename = "reasoningContent")]
        reasoning_content: Option<String>,
        timestamp: String,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        output: Option<String>,
        success: Option<bool>,
        #[serde(rename = "durationMs")]
        duration_ms: Option<u64>,
    },
}

pub fn convert_events_to_messages(events: &[StorageEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<(String, String, Value)> = Vec::new();

    for event in events {
        match event {
            StorageEvent::UserMessage { content, timestamp } => {
                messages.push(SessionMessage::User {
                    content: content.clone(),
                    timestamp: timestamp.to_rfc3339(),
                });
            }
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
            } => {
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                if !parts.visible_content.is_empty() || parts.reasoning_content.is_some() {
                    messages.push(SessionMessage::Assistant {
                        content: parts.visible_content,
                        reasoning_content: parts.reasoning_content,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
            } => {
                pending_tool_calls.push((tool_call_id.clone(), tool_name.clone(), args.clone()));
            }
            StorageEvent::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                duration_ms,
            } => {
                // Remove from pending if present (to get args)
                let args = pending_tool_calls
                    .iter()
                    .find(|(pending_id, _, _)| pending_id == tool_call_id)
                    .map(|(_, _, args)| args.clone())
                    .unwrap_or(serde_json::Value::Null);

                // Remove the pending tool call
                pending_tool_calls.retain(|(pending_id, _, _)| pending_id != tool_call_id);

                messages.push(SessionMessage::ToolCall {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    args,
                    output: Some(output.clone()),
                    success: Some(*success),
                    duration_ms: Some(*duration_ms),
                });
            }
            _ => {}
        }
    }

    for (tool_call_id, tool_name, args) in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output: None,
            success: None,
            duration_ms: None,
        });
    }

    messages
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;

    #[test]
    fn convert_events_to_user_and_assistant_messages() {
        let events = vec![
            StorageEvent::UserMessage {
                content: "hello".to_string(),
                timestamp: Utc::now(),
            },
            StorageEvent::AssistantFinal {
                content: "hi there".to_string(),
                reasoning_content: None,
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], SessionMessage::User { content, .. } if content == "hello"));
        assert!(matches!(
            &messages[1],
            SessionMessage::Assistant {
                content,
                reasoning_content,
                ..
            } if content == "hi there" && reasoning_content.is_none()
        ));
    }

    #[test]
    fn convert_events_merges_tool_call_and_result() {
        let events = vec![
            StorageEvent::ToolCall {
                tool_call_id: "tc-1".to_string(),
                tool_name: "listDir".to_string(),
                args: json!({ "path": "." }),
            },
            StorageEvent::ToolResult {
                tool_call_id: "tc-1".to_string(),
                tool_name: "listDir".to_string(),
                output: "files listed".to_string(),
                success: true,
                duration_ms: 100,
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            SessionMessage::ToolCall {
                tool_name,
                output,
                success,
                duration_ms,
                ..
            } => {
                assert_eq!(tool_name, "listDir");
                assert_eq!(output, &Some("files listed".to_string()));
                assert_eq!(success, &Some(true));
                assert_eq!(duration_ms, &Some(100));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn convert_events_preserves_pending_tool_call_without_result() {
        let events = vec![StorageEvent::ToolCall {
            tool_call_id: "tc-pending".to_string(),
            tool_name: "readFile".to_string(),
            args: json!({ "path": "README.md" }),
        }];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            SessionMessage::ToolCall {
                tool_call_id,
                tool_name,
                output,
                success,
                duration_ms,
                ..
            } => {
                assert_eq!(tool_call_id, "tc-pending");
                assert_eq!(tool_name, "readFile");
                assert_eq!(output, &None);
                assert_eq!(success, &None);
                assert_eq!(duration_ms, &None);
            }
            _ => panic!("expected pending ToolCall"),
        }
    }

    #[test]
    fn convert_events_ignores_transient_events() {
        let events = vec![
            StorageEvent::SessionStart {
                session_id: "s-1".to_string(),
                timestamp: Utc::now(),
                working_dir: "/tmp".to_string(),
            },
            StorageEvent::AssistantDelta {
                token: "partial".to_string(),
            },
            StorageEvent::TurnDone {
                timestamp: Utc::now(),
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert!(messages.is_empty(), "transient events should be ignored");
    }

    #[test]
    fn convert_events_preserves_reasoning_only_assistant_messages() {
        let events = vec![StorageEvent::AssistantFinal {
            content: "<think>private steps</think>".to_string(),
            reasoning_content: None,
        }];

        let messages = convert_events_to_messages(&events);
        assert!(matches!(
            &messages[0],
            SessionMessage::Assistant {
                content,
                reasoning_content,
                ..
            } if content.is_empty() && reasoning_content.as_deref() == Some("private steps")
        ));
    }

    #[test]
    fn assistant_session_message_serializes_reasoning_content_as_camel_case() {
        let payload = serde_json::to_value(SessionMessage::Assistant {
            content: "visible".to_string(),
            reasoning_content: Some("private steps".to_string()),
            timestamp: "2026-03-11T00:00:00Z".to_string(),
        })
        .expect("session message should serialize");

        assert_eq!(payload.get("kind").and_then(serde_json::Value::as_str), Some("assistant"));
        assert_eq!(
            payload
                .get("reasoningContent")
                .and_then(serde_json::Value::as_str),
            Some("private steps")
        );
        assert!(payload.get("reasoning_content").is_none());
    }

    #[test]
    fn tool_call_session_message_serializes_tool_fields_as_camel_case() {
        let payload = serde_json::to_value(SessionMessage::ToolCall {
            tool_call_id: "call-1".to_string(),
            tool_name: "shell".to_string(),
            args: json!({ "command": "echo ok" }),
            output: Some("ok".to_string()),
            success: Some(true),
            duration_ms: Some(12),
        })
        .expect("tool call message should serialize");

        assert_eq!(
            payload.get("toolCallId").and_then(serde_json::Value::as_str),
            Some("call-1")
        );
        assert_eq!(
            payload.get("toolName").and_then(serde_json::Value::as_str),
            Some("shell")
        );
        assert_eq!(
            payload.get("durationMs").and_then(serde_json::Value::as_u64),
            Some(12)
        );
        assert!(payload.get("tool_call_id").is_none());
        assert!(payload.get("tool_name").is_none());
        assert!(payload.get("duration_ms").is_none());
    }
}
