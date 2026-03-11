use ipc::ToolCallResultEnvelope;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::events::StorageEvent;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantContentParts {
    pub visible_content: String,
    pub reasoning_content: Option<String>,
}

fn thinking_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<think>(.*?)</think>").expect("valid thinking regex"))
}

fn extra_blank_line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid blank-line regex"))
}

fn merge_reasoning_content(
    explicit_reasoning: Option<String>,
    inline_reasoning: Option<String>,
) -> Option<String> {
    match (explicit_reasoning, inline_reasoning) {
        (Some(explicit), Some(inline)) if explicit == inline => Some(explicit),
        (Some(explicit), Some(inline)) => Some(format!("{explicit}\n\n{inline}")),
        (Some(explicit), None) => Some(explicit),
        (None, Some(inline)) => Some(inline),
        (None, None) => None,
    }
}

pub fn split_assistant_content(
    content: &str,
    explicit_reasoning: Option<&str>,
) -> AssistantContentParts {
    let mut inline_blocks = Vec::new();
    let stripped_content =
        thinking_tag_regex().replace_all(content, |captures: &regex::Captures| {
            let normalized = captures
                .get(1)
                .map(|value| value.as_str().trim())
                .unwrap_or_default();
            if !normalized.is_empty() {
                inline_blocks.push(normalized.to_string());
            }
            ""
        });

    let inline_reasoning = (!inline_blocks.is_empty()).then(|| inline_blocks.join("\n\n"));
    let explicit_reasoning = explicit_reasoning
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let visible_content = if inline_blocks.is_empty() {
        content.to_string()
    } else {
        extra_blank_line_regex()
            .replace_all(stripped_content.trim(), "\n\n")
            .into_owned()
    };

    AssistantContentParts {
        visible_content,
        reasoning_content: merge_reasoning_content(explicit_reasoning, inline_reasoning),
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

pub fn rebuild_reasoning_cache_from_events(events: &[StorageEvent]) -> HashMap<usize, String> {
    let mut cache = HashMap::new();
    let mut assistant_index = 0usize;

    for event in events {
        if let StorageEvent::AssistantFinal {
            content,
            reasoning_content,
        } = event
        {
            let parts = split_assistant_content(content, reasoning_content.as_deref());
            if let Some(reasoning_content) =
                parts.reasoning_content.filter(|value| !value.is_empty())
            {
                cache.insert(assistant_index, reasoning_content);
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
    use crate::events::StorageEvent;
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

    #[test]
    fn split_assistant_content_collapses_extra_blank_lines() {
        // Blank line collapsing only happens when think tags are removed
        let parts = split_assistant_content("before\n<think>step</think>\n\n\n\nafter", None);
        assert_eq!(parts.visible_content, "before\n\nafter");
    }

    #[test]
    fn split_assistant_content_returns_original_text_when_no_think_tags() {
        let parts = split_assistant_content("plain text", None);
        assert_eq!(parts.visible_content, "plain text");
        assert_eq!(parts.reasoning_content, None);
    }

    #[test]
    fn split_assistant_content_handles_case_insensitive_tags() {
        let parts = split_assistant_content("<THINK>thinking</THINK>", None);
        assert_eq!(parts.visible_content, "");
        assert_eq!(parts.reasoning_content.as_deref(), Some("thinking"));
    }

    #[test]
    fn split_assistant_content_deduplicates_identical_reasoning() {
        let parts = split_assistant_content("<think>thinking</think>", Some("thinking"));
        assert_eq!(parts.reasoning_content.as_deref(), Some("thinking"));
    }

    #[test]
    fn split_assistant_content_handles_empty_think_blocks() {
        // Empty/whitespace-only think blocks do NOT trigger tag removal
        let parts = split_assistant_content("<think>   </think>\n\nvisible", None);
        assert_eq!(parts.visible_content, "<think>   </think>\n\nvisible");
        assert_eq!(parts.reasoning_content, None);
    }

    #[test]
    fn split_assistant_content_handles_empty_string() {
        let parts = split_assistant_content("", None);
        assert_eq!(parts.visible_content, "");
        assert_eq!(parts.reasoning_content, None);
    }

    #[test]
    fn split_assistant_content_extracts_inline_thinking_blocks() {
        let parts = split_assistant_content(
            "Answer before\n<think> first step </think>\n<think>second step</think>\nAnswer after",
            None,
        );

        assert_eq!(parts.visible_content, "Answer before\n\nAnswer after");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("first step\n\nsecond step")
        );
    }

    #[test]
    fn split_assistant_content_prefers_explicit_reasoning_and_strips_legacy_tags() {
        let parts = split_assistant_content(
            "<think>legacy</think>\nvisible",
            Some("persisted reasoning"),
        );

        assert_eq!(parts.visible_content, "visible");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("persisted reasoning\n\nlegacy")
        );
    }

    #[test]
    fn rebuild_reasoning_cache_from_events_uses_persisted_or_legacy_reasoning() {
        let cache = rebuild_reasoning_cache_from_events(&[
            StorageEvent::AssistantFinal {
                content: "visible".to_string(),
                reasoning_content: Some("persisted".to_string()),
            },
            StorageEvent::AssistantFinal {
                content: "<think>legacy reasoning</think>done".to_string(),
                reasoning_content: None,
            },
        ]);

        assert_eq!(cache.get(&0).map(String::as_str), Some("persisted"));
        assert_eq!(cache.get(&1).map(String::as_str), Some("legacy reasoning"));
    }
}
