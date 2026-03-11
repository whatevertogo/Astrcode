use serde::{Deserialize, Serialize};
use serde_json::Value;

use astrcode_core::StorageEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionMessage {
    User {
        content: String,
        timestamp: String,
    },
    Assistant {
        content: String,
        timestamp: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        output: Option<String>,
        success: Option<bool>,
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
            StorageEvent::AssistantFinal { content } => {
                if !content.is_empty() {
                    messages.push(SessionMessage::Assistant {
                        content: content.clone(),
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
            },
        ];

        let messages = convert_events_to_messages(&events);
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0], SessionMessage::User { content, .. } if content == "hello"));
        assert!(
            matches!(&messages[1], SessionMessage::Assistant { content, .. } if content == "hi there")
        );
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
}
