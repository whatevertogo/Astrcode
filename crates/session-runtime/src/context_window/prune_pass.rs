//! # Prune Pass
//!
//! 轻量级上下文优化，不需要调用 LLM，直接在本地执行：
//! - 截断过长的工具结果（超过 `max_tool_result_bytes`）
//! - 清除标记为 `compact_clearable` 的旧工具结果
//!
//! ## 与完整压缩的区别
//!
//! | 特性 | Prune Pass | 完整压缩 |
//! |------|-----------|----------|
//! | 是否需要 LLM | 否 | 是 |
//! | 触发条件 | 每个 step | 配置阈值 |
//! | 操作 | 截断/清除 | 摘要替换 |
//! | 速度 | 即时 | 需要 LLM 调用 |

use std::collections::HashSet;

use astrcode_core::{LlmMessage, UserMessageOrigin};

use super::tool_results::tool_call_name_map;

/// Prune pass 执行统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneStats {
    pub truncated_tool_results: usize,
    pub cleared_tool_results: usize,
}

/// Prune pass 执行结果。
#[derive(Debug, Clone)]
pub struct PruneOutcome {
    pub messages: Vec<LlmMessage>,
    pub stats: PruneStats,
}

/// 执行轻量级上下文优化。
///
/// - 截断超过 `max_tool_result_bytes` 的工具结果
/// - 清除 `clearable_tools` 中指定的旧工具结果（保留最近 `keep_recent_turns` 轮）
pub fn apply_prune_pass(
    messages: &[LlmMessage],
    clearable_tools: &HashSet<String>,
    max_tool_result_bytes: usize,
    keep_recent_turns: usize,
) -> PruneOutcome {
    // tool_call_id → tool_name 映射
    let tool_call_names = tool_call_name_map(messages);
    // 保留最近的 keep_recent_turns 个用户 turn 不受 prune 影响
    let keep_start = recent_turn_start_index(messages, keep_recent_turns.max(1));
    let mut truncated_tool_results = 0usize;
    let mut cleared_tool_results = 0usize;
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
                "[cleared older tool result from '{tool_name}' to reduce prompt size; reload it \
                 if needed]"
            );
            cleared_tool_results += 1;
        }
    }

    PruneOutcome {
        messages: compacted,
        stats: PruneStats {
            truncated_tool_results,
            cleared_tool_results,
        },
    }
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

/// 找到保留区域的起始索引（最近 `requested_recent_turns` 个用户 turn 之前的第一个位置）。
fn recent_turn_start_index(messages: &[LlmMessage], requested_recent_turns: usize) -> usize {
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

    let keep_turns = requested_recent_turns.min(user_turn_indices.len()).max(1);
    user_turn_indices[user_turn_indices.len() - keep_turns]
}

#[cfg(test)]
mod tests {
    use astrcode_core::ToolCallRequest;
    use serde_json::json;

    use super::*;

    #[test]
    fn prune_pass_truncates_large_tool_results_and_clears_old_safe_tools() {
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

        let mut clearable = HashSet::new();
        clearable.insert("readFile".to_string());
        let result = apply_prune_pass(&messages, &clearable, 128, 1);

        assert_eq!(result.stats.truncated_tool_results, 1);
        assert_eq!(result.stats.cleared_tool_results, 1);
        match &result.messages[2] {
            LlmMessage::Tool { content, .. } => {
                assert!(content.contains("[cleared older tool result"));
            },
            other => panic!("expected tool message, got {other:?}"),
        }
    }

    #[test]
    fn prune_pass_does_not_reduce_requested_recent_turns_when_suffix_is_large() {
        let protected_tool = "protected result ".repeat(200);
        let messages = vec![
            LlmMessage::User {
                content: "turn-1".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path":"old.rs"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: protected_tool.clone(),
            },
            LlmMessage::User {
                content: "turn-2".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-2".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path":"recent.rs"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-2".to_string(),
                content: "latest result ".repeat(200),
            },
        ];

        let mut clearable = HashSet::new();
        clearable.insert("readFile".to_string());
        let result = apply_prune_pass(&messages, &clearable, usize::MAX, 2);

        match &result.messages[2] {
            LlmMessage::Tool { content, .. } => {
                assert_eq!(
                    content, &protected_tool,
                    "when the caller requests the recent two turns, prune pass must not degrade \
                     that guarantee to one turn just because the suffix is large"
                );
            },
            other => panic!("expected tool message, got {other:?}"),
        }
        assert_eq!(result.stats.cleared_tool_results, 0);
    }
}
