use ipc::ToolCallResultEnvelope;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone, Debug)]
pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<Value>,
    pub duration_ms: u128,
}

impl ToolExecutionResult {
    pub fn into_envelope(self) -> ToolCallResultEnvelope {
        ToolCallResultEnvelope {
            tool_call_id: self.tool_call_id,
            tool_name: self.tool_name,
            ok: self.ok,
            output: self.output,
            error: self.error,
            metadata: self.metadata,
            duration_ms: self.duration_ms,
        }
    }

    pub fn model_content(&self) -> String {
        if self.ok {
            self.output.clone()
        } else {
            format!(
                "tool execution failed: {}\\n{}",
                self.error.as_deref().unwrap_or("unknown error"),
                self.output
            )
        }
    }
}

#[derive(Clone, Debug)]
pub enum LlmMessage {
    User {
        content: String,
    },
    Assistant {
        content: String,
        tool_calls: Vec<ToolCallRequest>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageMetadata {
    pub reasoning_content: Option<String>,
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub message: LlmMessage,
    pub metadata: MessageMetadata,
}

impl HistoryEntry {
    pub fn plain(message: LlmMessage) -> Self {
        Self {
            message,
            metadata: MessageMetadata::default(),
        }
    }
}

pub fn build_history(messages: &[LlmMessage], cache: &HashMap<usize, String>) -> Vec<HistoryEntry> {
    let mut assistant_index = 0usize;

    messages
        .iter()
        .cloned()
        .map(|message| {
            let reasoning_content = if matches!(message, LlmMessage::Assistant { .. }) {
                let reasoning_content = cache.get(&assistant_index).cloned();
                assistant_index += 1;
                reasoning_content
            } else {
                None
            };

            HistoryEntry {
                message,
                metadata: MessageMetadata { reasoning_content },
            }
        })
        .collect()
}

pub fn rebuild_reasoning_cache_from_history(history: &[HistoryEntry]) -> HashMap<usize, String> {
    let mut cache = HashMap::new();
    let mut assistant_index = 0usize;

    for entry in history {
        if matches!(entry.message, LlmMessage::Assistant { .. }) {
            if let Some(reasoning_content) = entry
                .metadata
                .reasoning_content
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                cache.insert(assistant_index, reasoning_content.clone());
            }
            assistant_index += 1;
        }
    }

    cache
}

#[derive(Clone, Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_history_assigns_reasoning_by_assistant_index_only() {
        let messages = vec![
            LlmMessage::User {
                content: "hello".to_string(),
            },
            LlmMessage::Assistant {
                content: "step1".to_string(),
                tool_calls: vec![],
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "ok".to_string(),
            },
            LlmMessage::Assistant {
                content: "step2".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-2".to_string(),
                    name: "search".to_string(),
                    args: json!({ "q": "rust" }),
                }],
            },
        ];
        let cache = HashMap::from([
            (0usize, "reasoning-0".to_string()),
            (1usize, "reasoning-1".to_string()),
        ]);

        let history = build_history(&messages, &cache);

        assert_eq!(history.len(), 4);
        assert_eq!(
            history[1].metadata.reasoning_content.as_deref(),
            Some("reasoning-0")
        );
        assert_eq!(history[2].metadata.reasoning_content, None);
        assert_eq!(
            history[3].metadata.reasoning_content.as_deref(),
            Some("reasoning-1")
        );
    }

    #[test]
    fn rebuild_reasoning_cache_skips_non_assistant_and_empty_values() {
        let history = vec![
            HistoryEntry::plain(LlmMessage::User {
                content: "hello".to_string(),
            }),
            HistoryEntry {
                message: LlmMessage::Assistant {
                    content: "step1".to_string(),
                    tool_calls: vec![],
                },
                metadata: MessageMetadata {
                    reasoning_content: Some("reasoning-0".to_string()),
                },
            },
            HistoryEntry::plain(LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "tool".to_string(),
            }),
            HistoryEntry {
                message: LlmMessage::Assistant {
                    content: "step2".to_string(),
                    tool_calls: vec![],
                },
                metadata: MessageMetadata {
                    reasoning_content: Some(String::new()),
                },
            },
        ];

        let cache = rebuild_reasoning_cache_from_history(&history);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&0).map(String::as_str), Some("reasoning-0"));
        assert!(cache.get(&1).is_none());
    }
}
