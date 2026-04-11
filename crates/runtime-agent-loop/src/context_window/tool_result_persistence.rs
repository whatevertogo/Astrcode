//! # 管线聚合预算持久化
//!
//! 当消息流中所有未持久化工具结果的总大小超过预算时，将最大的结果
//! 强制落盘并替换为 `<persisted-output>` 引用，减少当前 turn 的上下文压力。
//!
//! # 为什么让管线做 IO
//!
//! `build_bundle()` 之后马上进入 prompt 组装，没有"下一个 step 再落盘"的时机。
//! 只有立即替换消息内容才能减少当前 turn 的上下文压力。
//! IO 操作是同步、幂等的（同一 tool_call_id 写一次，重复调用跳过），
//! 失败时降级（磁盘写入失败 → 不替换，让 PrunePass 截断兜底）。

use std::{cmp::Reverse, collections::HashSet, path::Path};

use astrcode_core::{LlmMessage, is_persisted_output, persist_tool_result};
use astrcode_protocol::capability::CapabilityDescriptor;

/// 聚合预算配置。
#[derive(Debug, Clone)]
pub(crate) struct PersistenceBudgetConfig {
    /// 消息流中未持久化工具结果的总字节预算。
    /// 超过此值时，将最大的结果强制落盘。
    pub aggregate_result_bytes_budget: usize,
}

/// 持久化操作统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PersistenceStats {
    /// 本次被持久化的工具结果数
    pub persisted_count: usize,
    /// 被持久化结果节省的字节数（原始大小 - 引用大小）
    pub bytes_saved: usize,
    /// 已是 persisted 状态而跳过的数量
    pub skipped_already_persisted: usize,
}

/// 从消息内容中可重建的替换状态。
///
/// 不使用内存 Mutex，每次 build_bundle 时从当前消息流扫描重建。
/// 这保证了跨 loop 重建、进程重启后的决策一致性。
///
/// 冻结语义：只冻结"已替换"决策（通过检测 `<persisted-output>` 标签）。
/// 不冻结"已见但未替换"决策——旧消息被 PrunePass/compaction/微压缩清掉后，
/// 未替换的历史消失了，下次重建时自然会重新评估。
#[derive(Debug, Clone, Default)]
struct ReplacementState {
    /// 已被替换为 persisted 引用的 tool_call_id
    replaced_ids: HashSet<String>,
}

/// 从消息流中重建 ReplacementState（纯函数，无 IO）。
///
/// 扫描所有 Tool 消息，检测内容中是否包含 `<persisted-output>` 标签。
fn rebuild_state_from_messages(messages: &[LlmMessage]) -> ReplacementState {
    let mut replaced_ids = HashSet::new();
    for msg in messages {
        if let LlmMessage::Tool {
            tool_call_id,
            content,
        } = msg
        {
            if is_persisted_output(content) {
                replaced_ids.insert(tool_call_id.clone());
            }
        }
    }
    ReplacementState { replaced_ids }
}

/// 对消息流执行聚合预算检查，超预算时做受控 IO 并替换消息内容。
///
/// 算法：
/// 1. 重建已替换状态（扫描 `<persisted-output>` 标签）
/// 2. 收集所有未持久化的 Tool 消息及其大小
/// 3. 若总大小超过预算，按大小降序排列，依次持久化最大的结果直到总额 ≤ 预算
/// 4. 已持久化的结果（含第一层工具执行侧已处理的）不计入 fresh 总额
///
/// # 降级策略
///
/// - `session_dir` 为 None → 直接返回（no-op）
/// - 单个文件写入失败 → 跳过该结果（`persist_tool_result` 内部已降级为截断预览）
pub(crate) fn enforce_aggregate_budget(
    messages: &mut [LlmMessage],
    _descriptors: &[CapabilityDescriptor],
    session_dir: Option<&Path>,
    config: &PersistenceBudgetConfig,
) -> PersistenceStats {
    let Some(session_dir) = session_dir else {
        return PersistenceStats::default();
    };

    let state = rebuild_state_from_messages(messages);
    let mut stats = PersistenceStats {
        skipped_already_persisted: state.replaced_ids.len(),
        ..PersistenceStats::default()
    };

    // 收集未持久化工具结果的索引和大小
    let mut fresh_entries: Vec<(usize, usize)> = Vec::new(); // (message_index, content_len)
    let mut total_fresh_bytes: usize = 0;

    for (idx, msg) in messages.iter().enumerate() {
        let LlmMessage::Tool {
            tool_call_id,
            content,
        } = msg
        else {
            continue;
        };

        if state.replaced_ids.contains(tool_call_id) {
            continue;
        }

        let len = content.len();
        if len == 0 {
            continue;
        }

        fresh_entries.push((idx, len));
        total_fresh_bytes += len;
    }

    if total_fresh_bytes <= config.aggregate_result_bytes_budget {
        return stats;
    }

    // 按大小降序排列，优先持久化最大的结果
    fresh_entries.sort_by_key(|b| Reverse(b.1));

    for (msg_idx, _original_len) in &fresh_entries {
        if total_fresh_bytes <= config.aggregate_result_bytes_budget {
            break;
        }

        let LlmMessage::Tool {
            tool_call_id,
            content,
        } = &mut messages[*msg_idx]
        else {
            continue;
        };

        // 幂等性检查：如果内容在迭代期间已被其他逻辑替换
        if is_persisted_output(content) {
            continue;
        }

        let call_id = tool_call_id.clone();
        let original_content = std::mem::take(content);
        let persisted = persist_tool_result(session_dir, &call_id, &original_content);

        let bytes_reduced = original_content.len().saturating_sub(persisted.len());
        total_fresh_bytes = total_fresh_bytes.saturating_sub(bytes_reduced);
        *content = persisted;
        stats.persisted_count += 1;
        stats.bytes_saved += bytes_reduced;
    }

    stats
}

#[cfg(test)]
mod tests {
    use astrcode_core::ToolCallRequest;
    use serde_json::json;

    use super::*;

    fn make_tool_messages(items: &[(&str, &str)]) -> Vec<LlmMessage> {
        let mut messages = Vec::new();
        messages.push(LlmMessage::Assistant {
            content: String::new(),
            tool_calls: items
                .iter()
                .map(|(id, _)| ToolCallRequest {
                    id: id.to_string(),
                    name: "test_tool".to_string(),
                    args: json!({}),
                })
                .collect(),
            reasoning: None,
        });
        for (id, content) in items {
            messages.push(LlmMessage::Tool {
                tool_call_id: id.to_string(),
                content: content.to_string(),
            });
        }
        messages
    }

    #[test]
    fn rebuild_state_detects_persisted_results() {
        let messages = vec![
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "normal output".to_string(),
            },
            LlmMessage::Tool {
                tool_call_id: "call-2".to_string(),
                content: "<persisted-output>\nsome ref\n</persisted-output>".to_string(),
            },
        ];

        let state = rebuild_state_from_messages(&messages);
        assert!(!state.replaced_ids.contains("call-1"));
        assert!(state.replaced_ids.contains("call-2"));
    }

    #[test]
    fn rebuild_state_empty_messages_is_empty() {
        let state = rebuild_state_from_messages(&[]);
        assert!(state.replaced_ids.is_empty());
    }

    #[test]
    fn no_persistence_when_below_budget() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut messages =
            make_tool_messages(&[("call-1", "short result"), ("call-2", "another result")]);

        let config = PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 1024,
        };
        let stats = enforce_aggregate_budget(&mut messages, &[], Some(dir.path()), &config);

        assert_eq!(stats.persisted_count, 0);
        assert_eq!(stats.skipped_already_persisted, 0);
    }

    #[test]
    fn persists_largest_when_over_budget() {
        let dir = tempfile::tempdir().expect("tempdir");

        let big = "x".repeat(40_000);
        let medium = "y".repeat(20_000);
        let small = "z".repeat(10_000);

        let mut messages = make_tool_messages(&[
            ("call-big", &big),
            ("call-medium", &medium),
            ("call-small", &small),
        ]);

        let config = PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 50_000, // 总量 70000 > 50000
        };
        let stats = enforce_aggregate_budget(&mut messages, &[], Some(dir.path()), &config);

        // 应至少持久化最大的结果
        assert!(stats.persisted_count >= 1);
        assert!(stats.bytes_saved > 0);

        // call-big 应该被持久化（40KB → ~2KB 引用）
        let big_msg = messages.iter().find_map(|m| match m {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } if tool_call_id == "call-big" => Some(content.clone()),
            _ => None,
        });
        assert!(big_msg.unwrap().contains("<persisted-output>"));
    }

    #[test]
    fn skips_already_persisted_results() {
        let dir = tempfile::tempdir().expect("tempdir");

        let persisted_content = "<persisted-output>\nsaved to file\n</persisted-output>";
        let big = "y".repeat(40_000);

        let mut messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "test".to_string(),
                        args: json!({}),
                    },
                    ToolCallRequest {
                        id: "call-2".to_string(),
                        name: "test".to_string(),
                        args: json!({}),
                    },
                ],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: persisted_content.to_string(), // 已持久化
            },
            LlmMessage::Tool {
                tool_call_id: "call-2".to_string(),
                content: big.clone(), // 未持久化
            },
        ];

        let config = PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 20_000, // 40KB > 20KB，call-2 应被持久化
        };
        let stats = enforce_aggregate_budget(&mut messages, &[], Some(dir.path()), &config);

        assert_eq!(stats.skipped_already_persisted, 1);
        assert_eq!(stats.persisted_count, 1);
    }

    #[test]
    fn no_op_when_no_session_dir() {
        let big = "x".repeat(100_000);
        let mut messages = make_tool_messages(&[("call-1", &big)]);

        let config = PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 100,
        };
        let stats = enforce_aggregate_budget(&mut messages, &[], None, &config);

        assert_eq!(stats.persisted_count, 0);
        // 原始内容不变
        assert_eq!(
            messages[1],
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: big,
            }
        );
    }

    #[test]
    fn is_idempotent_on_repeated_calls() {
        let dir = tempfile::tempdir().expect("tempdir");

        let big = "x".repeat(40_000);
        let mut messages = make_tool_messages(&[("call-1", &big)]);

        let config = PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 10_000,
        };

        // 第一次调用
        let stats1 = enforce_aggregate_budget(&mut messages, &[], Some(dir.path()), &config);
        assert_eq!(stats1.persisted_count, 1);

        // 第二次调用——已持久化的应被跳过
        let stats2 = enforce_aggregate_budget(&mut messages, &[], Some(dir.path()), &config);
        assert_eq!(stats2.persisted_count, 0);
        assert_eq!(stats2.skipped_already_persisted, 1);
    }
}
