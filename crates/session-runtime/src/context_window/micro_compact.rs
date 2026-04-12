//! # Micro Compact
//!
//! 在完整 compaction 之前，先用本地规则清理“已经冷掉”的旧工具结果，
//! 避免每次都因为几段历史工具输出而触发昂贵的摘要压缩。

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use astrcode_core::LlmMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroCompactConfig {
    pub gap_threshold: Duration,
    pub keep_recent_results: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicroCompactStats {
    pub cleared_tool_results: usize,
}

#[derive(Debug, Clone)]
pub struct MicroCompactOutcome {
    pub messages: Vec<LlmMessage>,
    pub stats: MicroCompactStats,
}

#[derive(Debug, Clone)]
struct TrackedToolResult {
    tool_call_id: String,
    recorded_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct MicroCompactState {
    tracked_results: VecDeque<TrackedToolResult>,
    last_tool_activity: Option<Instant>,
}

impl MicroCompactState {
    pub fn seed_from_messages(
        messages: &[LlmMessage],
        config: MicroCompactConfig,
        now: Instant,
    ) -> Self {
        let mut state = Self::default();
        let stale_at = now.checked_sub(config.gap_threshold).unwrap_or(now);

        for message in messages {
            let LlmMessage::Tool { tool_call_id, .. } = message else {
                continue;
            };
            state.tracked_results.push_back(TrackedToolResult {
                tool_call_id: tool_call_id.clone(),
                recorded_at: stale_at,
            });
        }

        if !state.tracked_results.is_empty() {
            state.last_tool_activity = Some(stale_at);
        }

        state
    }

    pub fn record_tool_result(&mut self, tool_call_id: impl Into<String>, now: Instant) {
        let tool_call_id = tool_call_id.into();
        self.tracked_results
            .retain(|entry| entry.tool_call_id != tool_call_id);
        self.tracked_results.push_back(TrackedToolResult {
            tool_call_id,
            recorded_at: now,
        });
        self.last_tool_activity = Some(now);
    }

    pub fn apply_if_idle(
        &mut self,
        messages: &[LlmMessage],
        clearable_tools: &HashSet<String>,
        config: MicroCompactConfig,
        now: Instant,
    ) -> MicroCompactOutcome {
        self.retain_live_tool_results(messages);

        let Some(last_activity) = self.last_tool_activity else {
            return MicroCompactOutcome {
                messages: messages.to_vec(),
                stats: MicroCompactStats::default(),
            };
        };

        if now.duration_since(last_activity) < config.gap_threshold {
            return MicroCompactOutcome {
                messages: messages.to_vec(),
                stats: MicroCompactStats::default(),
            };
        }

        let keep_recent_results = config.keep_recent_results.max(1);
        if self.tracked_results.len() <= keep_recent_results {
            return MicroCompactOutcome {
                messages: messages.to_vec(),
                stats: MicroCompactStats::default(),
            };
        }

        let tool_call_names = tool_call_name_map(messages);
        let protected_ids = self
            .tracked_results
            .iter()
            .rev()
            .take(keep_recent_results)
            .map(|entry| entry.tool_call_id.as_str())
            .collect::<HashSet<_>>();

        let stale_ids = self
            .tracked_results
            .iter()
            .filter(|entry| !protected_ids.contains(entry.tool_call_id.as_str()))
            .filter(|entry| now.duration_since(entry.recorded_at) >= config.gap_threshold)
            .filter_map(|entry| {
                tool_call_names
                    .get(&entry.tool_call_id)
                    .filter(|tool_name| clearable_tools.contains(*tool_name))
                    .map(|_| entry.tool_call_id.clone())
            })
            .collect::<HashSet<_>>();

        if stale_ids.is_empty() {
            return MicroCompactOutcome {
                messages: messages.to_vec(),
                stats: MicroCompactStats::default(),
            };
        }

        let mut compacted = messages.to_vec();
        let mut cleared = 0usize;
        for message in &mut compacted {
            let LlmMessage::Tool {
                tool_call_id,
                content,
            } = message
            else {
                continue;
            };

            if !stale_ids.contains(tool_call_id) || is_micro_compacted(content) {
                continue;
            }

            let tool_name = tool_call_names
                .get(tool_call_id)
                .map(String::as_str)
                .unwrap_or("tool");
            *content = format!(
                "[micro-compacted stale tool result from '{tool_name}' after idle gap; rerun the \
                 tool if exact output is needed]"
            );
            cleared += 1;
        }

        MicroCompactOutcome {
            messages: compacted,
            stats: MicroCompactStats {
                cleared_tool_results: cleared,
            },
        }
    }

    fn retain_live_tool_results(&mut self, messages: &[LlmMessage]) {
        let live_tool_ids = messages
            .iter()
            .filter_map(|message| match message {
                LlmMessage::Tool { tool_call_id, .. } => Some(tool_call_id.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        self.tracked_results
            .retain(|entry| live_tool_ids.contains(entry.tool_call_id.as_str()));
        if self.tracked_results.is_empty() {
            self.last_tool_activity = None;
        }
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

fn is_micro_compacted(content: &str) -> bool {
    content.contains("[micro-compacted stale tool result")
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmMessage, ToolCallRequest, UserMessageOrigin};
    use serde_json::json;

    use super::*;

    #[test]
    fn micro_compact_clears_stale_tool_results_but_preserves_recent_entries() {
        let now = Instant::now();
        let config = MicroCompactConfig {
            gap_threshold: Duration::from_secs(30),
            keep_recent_results: 1,
        };
        let messages = vec![
            LlmMessage::User {
                content: "inspect".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "readFile".to_string(),
                        args: json!({"path":"src/lib.rs"}),
                    },
                    ToolCallRequest {
                        id: "call-2".to_string(),
                        name: "readFile".to_string(),
                        args: json!({"path":"src/main.rs"}),
                    },
                ],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "older result".to_string(),
            },
            LlmMessage::Tool {
                tool_call_id: "call-2".to_string(),
                content: "recent result".to_string(),
            },
        ];

        let mut state = MicroCompactState::seed_from_messages(&messages, config, now);
        state.record_tool_result("call-2", now);

        let mut clearable_tools = HashSet::new();
        clearable_tools.insert("readFile".to_string());
        let outcome = state.apply_if_idle(
            &messages,
            &clearable_tools,
            config,
            now + Duration::from_secs(31),
        );

        assert_eq!(outcome.stats.cleared_tool_results, 1);
        assert!(matches!(
            &outcome.messages[2],
            LlmMessage::Tool { content, .. } if content.contains("micro-compacted")
        ));
        assert!(matches!(
            &outcome.messages[3],
            LlmMessage::Tool { content, .. } if content == "recent result"
        ));
    }

    #[test]
    fn micro_compact_skips_when_idle_gap_not_reached() {
        let now = Instant::now();
        let config = MicroCompactConfig {
            gap_threshold: Duration::from_secs(30),
            keep_recent_results: 1,
        };
        let messages = vec![
            LlmMessage::Assistant {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path":"src/lib.rs"}),
                }],
                reasoning: None,
            },
            LlmMessage::Tool {
                tool_call_id: "call-1".to_string(),
                content: "fresh".to_string(),
            },
        ];

        let mut state = MicroCompactState::default();
        state.record_tool_result("call-1", now);

        let mut clearable_tools = HashSet::new();
        clearable_tools.insert("readFile".to_string());
        let outcome = state.apply_if_idle(
            &messages,
            &clearable_tools,
            config,
            now + Duration::from_secs(5),
        );

        assert_eq!(outcome.stats.cleared_tool_results, 0);
        assert_eq!(outcome.messages, messages);
    }
}
