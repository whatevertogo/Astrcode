//! # 时间触发微压缩
//!
//! 当会话空闲时间超过阈值时，清除标记为 `compact_clearable` 的旧工具结果，
//! 释放上下文空间。
//!
//! # 与 PrunePass 的区别
//!
//! | 特性 | MicroCompact | PrunePass |
//! |------|-------------|-----------|
//! | 触发条件 | 时间（空闲超阈值） | 每个 step |
//! | 保留策略 | 保留最近 N 条结果 | 保留最近 N 轮 |
//! | 清除方式 | 占位文本替换 | 截断/清除 |

use std::collections::{HashMap, HashSet};

use astrcode_core::{LlmMessage, is_persisted_output};
use astrcode_protocol::capability::CapabilityDescriptor;
use chrono::{DateTime, Utc};

/// 微压缩配置。
#[derive(Debug, Clone)]
pub(crate) struct MicroCompactConfig {
    /// 空闲阈值（秒），从最后一次 assistant 输出算起。
    pub gap_threshold_secs: u64,
    /// 保留最近工具结果数。
    pub keep_recent_results: usize,
}

/// 微压缩统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MicroCompactStats {
    pub cleared_count: usize,
}

/// 判断是否应触发微压缩。
pub(crate) fn should_trigger(
    last_assistant_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    gap_threshold_secs: u64,
) -> bool {
    let Some(last) = last_assistant_at else {
        return false;
    };
    if last > now {
        return false;
    }
    let gap = now.signed_duration_since(last);
    gap.num_seconds() as u64 >= gap_threshold_secs
}

/// 对消息流执行微压缩。
///
/// 清除标记为 `compact_clearable` 的旧工具结果，但保留最近 N 条。
/// 已持久化的结果（`<persisted-output>`）不受微压缩影响。
pub(crate) fn apply_micro_compact(
    messages: &mut [LlmMessage],
    descriptors: &[CapabilityDescriptor],
    keep_recent_results: usize,
) -> MicroCompactStats {
    let clearable_tools: HashSet<String> = descriptors
        .iter()
        .filter(|d| d.compact_clearable)
        .map(|d| d.name.clone())
        .collect();

    if clearable_tools.is_empty() {
        return MicroCompactStats::default();
    }

    let tool_call_names = build_tool_call_name_map(messages);

    // 从尾部收集最近 N 个可清除的 Tool 消息索引，这些不会被清除
    let mut recent_clearable_indices: Vec<usize> = Vec::new();
    for (idx, msg) in messages.iter().enumerate().rev() {
        if recent_clearable_indices.len() >= keep_recent_results {
            break;
        }
        if let LlmMessage::Tool {
            tool_call_id,
            content,
        } = msg
        {
            // 已持久化的结果不受微压缩影响
            if is_persisted_output(content) {
                continue;
            }
            if let Some(name) = tool_call_names.get(tool_call_id) {
                if clearable_tools.contains(name) {
                    recent_clearable_indices.push(idx);
                }
            }
        }
    }

    let mut stats = MicroCompactStats::default();
    let protected: HashSet<usize> = recent_clearable_indices.into_iter().collect();

    for (idx, msg) in messages.iter_mut().enumerate() {
        if protected.contains(&idx) {
            continue;
        }

        let LlmMessage::Tool {
            tool_call_id,
            content,
        } = msg
        else {
            continue;
        };

        // 已持久化的结果不受微压缩影响
        if is_persisted_output(content) {
            continue;
        }

        if let Some(name) = tool_call_names.get(tool_call_id) {
            if clearable_tools.contains(name) {
                *content = "[工具结果因会话空闲已清除；如需请重新执行工具]".to_string();
                stats.cleared_count += 1;
            }
        }
    }

    stats
}

/// 构建 tool_call_id → tool_name 的映射。
fn build_tool_call_name_map(messages: &[LlmMessage]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for msg in messages {
        if let LlmMessage::Assistant { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.clone(), call.name.clone());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use astrcode_core::ToolCallRequest;
    use astrcode_protocol::capability::{CapabilityDescriptor, CapabilityKind};
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
    fn should_trigger_returns_true_when_gap_exceeds_threshold() {
        let now = Utc::now();
        let past = now - chrono::Duration::seconds(3600);
        assert!(should_trigger(Some(past), now, 1800));
    }

    #[test]
    fn should_trigger_returns_false_when_gap_below_threshold() {
        let now = Utc::now();
        let recent = now - chrono::Duration::seconds(100);
        assert!(!should_trigger(Some(recent), now, 1800));
    }

    #[test]
    fn should_trigger_returns_false_when_no_timestamp() {
        assert!(!should_trigger(None, Utc::now(), 1800));
    }

    #[test]
    fn should_trigger_returns_false_when_timestamp_is_in_future() {
        let now = Utc::now();
        let future = now + chrono::Duration::seconds(3600);
        assert!(!should_trigger(Some(future), now, 1800));
    }

    #[test]
    fn clears_old_clearable_results_but_keeps_recent() {
        let descriptors = vec![descriptor("readFile", true)];
        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "readFile".to_string(),
                        args: json!({"path": "old.rs"}),
                    },
                    ToolCallRequest {
                        id: "call-2".to_string(),
                        name: "readFile".to_string(),
                        args: json!({"path": "recent.rs"}),
                    },
                ],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "old content".to_string(),
            },
            LlmMessage::Tool {
                tool_call_id: "call-2".to_string(),
                content: "recent content".to_string(),
            },
        ];

        let stats = apply_micro_compact(&mut messages, &descriptors, 1);

        assert_eq!(stats.cleared_count, 1);
        // call-1 应被清除
        match &messages[1] {
            LlmMessage::Tool { content, .. } => assert!(content.contains("已清除")),
            _ => panic!("expected tool message"),
        }
        // call-2 应被保留
        match &messages[2] {
            LlmMessage::Tool { content, .. } => assert_eq!(content, "recent content"),
            _ => panic!("expected tool message"),
        }
    }

    #[test]
    fn does_not_clear_persisted_results() {
        let descriptors = vec![descriptor("readFile", true)];
        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "<persisted-output>\nsaved\n</persisted-output>".to_string(),
            },
        ];

        let stats = apply_micro_compact(&mut messages, &descriptors, 0);
        assert_eq!(stats.cleared_count, 0);
    }

    #[test]
    fn no_op_when_no_clearable_tools() {
        let descriptors = vec![descriptor("shell", false)];
        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "output".to_string(),
            },
        ];

        let stats = apply_micro_compact(&mut messages, &descriptors, 0);
        assert_eq!(stats.cleared_count, 0);
    }
}
