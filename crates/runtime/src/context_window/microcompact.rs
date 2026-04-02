use std::collections::{HashMap, HashSet};

use astrcode_core::{CapabilityDescriptor, LlmMessage, UserMessageOrigin};

use crate::context_window::estimate_message_tokens;

#[derive(Debug, Clone)]
pub(crate) struct MicrocompactResult {
    pub messages: Vec<LlmMessage>,
    pub truncated_tool_results: usize,
}

pub(crate) fn apply_microcompact(
    messages: &[LlmMessage],
    descriptors: &[CapabilityDescriptor],
    max_tool_result_bytes: usize,
    keep_recent_turns: usize,
    effective_window: usize,
) -> MicrocompactResult {
    // TODO(claude-auto-compact): when Astrcode gains prompt-cache prefix editing, extend this
    // module with Claude-style cache-edit microcompact instead of only replacing old tool results.
    // TODO(claude-auto-compact): Claude also uses gap-based/time-based microcompact triggers tied
    // to cache TTL; Astrcode currently lacks prompt-cache semantics, so v1 keeps the trigger purely
    // token-pressure based.
    let clearable_tools = descriptors
        .iter()
        .filter(|descriptor| descriptor.compact_clearable)
        .map(|descriptor| descriptor.name.clone())
        .collect::<HashSet<_>>();
    let tool_call_names = tool_call_name_map(messages);
    let keep_start =
        recent_turn_start_with_cap(messages, keep_recent_turns.max(1), effective_window / 2);
    let mut truncated_tool_results = 0usize;
    let mut compacted = messages.to_vec();

    for (index, message) in compacted.iter_mut().enumerate() {
        let LlmMessage::Tool {
            tool_call_id,
            content,
        } = message
        else {
            continue;
        };

        if content.len() > max_tool_result_bytes {
            *content = truncate_tool_content(content, max_tool_result_bytes);
            truncated_tool_results += 1;
        }

        if index >= keep_start {
            continue;
        }

        let Some(tool_name) = tool_call_names.get(tool_call_id) else {
            continue;
        };
        if clearable_tools.contains(tool_name) {
            *content = format!(
                "[cleared older tool result from '{tool_name}' to reduce prompt size; reload it if needed]"
            );
        }
    }

    MicrocompactResult {
        messages: compacted,
        truncated_tool_results,
    }
}

fn tool_call_name_map(messages: &[LlmMessage]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        let LlmMessage::Assistant { tool_calls, .. } = message else {
            continue;
        };
        for call in tool_calls {
            names.insert(call.id.clone(), call.name.clone());
        }
    }
    names
}

fn truncate_tool_content(content: &str, max_bytes: usize) -> String {
    let total_bytes = content.len();
    let mut visible_bytes = max_bytes.saturating_sub(96).max(64).min(total_bytes);
    while !content.is_char_boundary(visible_bytes) {
        visible_bytes = visible_bytes.saturating_sub(1);
    }
    let visible = &content[..visible_bytes];
    format!(
        "[truncated: original {total_bytes} bytes, showing first {visible_bytes} bytes]\n{visible}"
    )
}

fn recent_turn_start_with_cap(
    messages: &[LlmMessage],
    requested_recent_turns: usize,
    suffix_token_cap: usize,
) -> usize {
    let user_turn_indices = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            } => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    if user_turn_indices.is_empty() {
        return messages.len();
    }

    let mut keep_turns = requested_recent_turns.min(user_turn_indices.len()).max(1);
    loop {
        let start = user_turn_indices[user_turn_indices.len() - keep_turns];
        let suffix_tokens = messages[start..]
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>();
        if suffix_tokens <= suffix_token_cap || keep_turns == 1 {
            return start;
        }
        keep_turns -= 1;
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, ToolCallRequest};
    use serde_json::json;

    use super::*;

    fn descriptor(name: &str, compact_clearable: bool) -> CapabilityDescriptor {
        CapabilityDescriptor::builder(name, CapabilityKind::tool())
            .description("test")
            .schema(json!({"type": "object"}), json!({"type": "string"}))
            .compact_clearable(compact_clearable)
            .build()
            .expect("descriptor should build")
    }

    #[test]
    fn microcompact_truncates_large_tool_results_and_clears_old_safe_tools() {
        let messages = vec![
            LlmMessage::User {
                content: "inspect".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path":"Cargo.toml"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "x".repeat(512),
            },
            LlmMessage::User {
                content: "follow up".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let result = apply_microcompact(&messages, &[descriptor("readFile", true)], 128, 1, 10_000);

        assert_eq!(result.truncated_tool_results, 1);
        match &result.messages[2] {
            LlmMessage::Tool { content, .. } => {
                assert!(content.contains("[cleared older tool result"));
            }
            other => panic!("expected tool message, got {other:?}"),
        }
    }
}
