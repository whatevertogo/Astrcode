//! # 事件转换器
//!
//! 将存储事件（`StorageEvent`）转换为领域事件（`AgentEvent`）。
//!
//! ## 核心职责
//!
//! 1. **Phase 跟踪**: 维护当前会话阶段，在阶段变化时发出 `PhaseChanged` 事件
//! 2. **Turn ID 管理**: 为旧事件（没有 turn_id）生成 legacy turn ID
//! 3. **工具名称缓存**: 存储 `tool_call_id -> tool_name` 映射，用于 ToolResult
//! 4. **事件 ID 生成**: 为每个领域事件生成 `{storage_seq}.{subindex}` 格式的 ID
//!
//! ## 为什么需要这个组件？
//!
//! - `StorageEvent` 是持久化格式，面向存储
//! - `AgentEvent` 是 SSE 推送格式，面向展示
//! - 一个 `StorageEvent` 可能产生多个 `AgentEvent`（如 PhaseChanged + 实际事件）

use std::collections::HashMap;

use super::phase::PhaseTracker;
use crate::{
    AgentEvent, AgentEventContext, Phase, StorageEvent, StoredEvent, ToolExecutionResult,
    UserMessageOrigin, session::SessionEventRecord, split_assistant_content,
};

/// 回放存储事件为会话事件记录
///
/// ## 断点续传
///
/// `last_event_id` 用于 SSE 断点续传，格式为 `{storage_seq}.{subindex}`。
/// 只返回 ID 严格大于 `last_event_id` 的事件。
pub fn replay_records(
    events: &[StoredEvent],
    last_event_id: Option<&str>,
) -> Vec<SessionEventRecord> {
    let mut translator = EventTranslator::new(Phase::Idle);
    let after_id = last_event_id.and_then(parse_event_id);
    let mut history = Vec::new();

    for stored in events {
        for record in translator.translate(stored) {
            if let Some(after_id) = after_id {
                let Some(current_id) = parse_event_id(&record.event_id) else {
                    continue;
                };
                if current_id <= after_id {
                    continue;
                }
            }
            history.push(record);
        }
    }

    history
}

/// 解析事件 ID 为 (storage_seq, subindex) 元组
fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    let storage_seq = storage_seq.parse().ok()?;
    let subindex = subindex.parse().ok()?;
    Some((storage_seq, subindex))
}

fn warn_missing_turn_id(storage_seq: u64, event_name: &str) {
    log::warn!(
        "dropping translated '{}' event at storage_seq {} because turn_id is missing",
        event_name,
        storage_seq
    );
}

/// 事件转换器
///
/// 将存储事件转换为领域事件，同时维护会话状态。
pub struct EventTranslator {
    phase_tracker: PhaseTracker,
    current_turn_id: Option<String>,
    legacy_turn_index: u64,
    tool_call_names: HashMap<String, String>,
}

impl EventTranslator {
    pub fn new(phase: Phase) -> Self {
        Self {
            phase_tracker: PhaseTracker::new(phase),
            current_turn_id: None,
            legacy_turn_index: 0,
            tool_call_names: HashMap::new(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase_tracker.current()
    }

    pub fn translate(&mut self, stored: &StoredEvent) -> Vec<SessionEventRecord> {
        let mut subindex = 0u32;
        let mut records = Vec::new();
        let turn_id = self.turn_id_for(&stored.event);
        let agent = stored.event.agent_context().cloned().unwrap_or_default();

        let mut push = |event: AgentEvent| {
            records.push(SessionEventRecord {
                event_id: format!("{}.{}", stored.storage_seq, subindex),
                event,
            });
            subindex = subindex.saturating_add(1);
        };

        if let Some(phase_event) =
            self.phase_tracker
                .on_event(&stored.event, turn_id.clone(), agent.clone())
        {
            push(phase_event);
        }

        self.convert_event(stored, turn_id, agent, &mut push);

        records
    }

    fn convert_event(
        &mut self,
        stored: &StoredEvent,
        turn_id: Option<String>,
        agent: AgentEventContext,
        push: &mut impl FnMut(AgentEvent),
    ) {
        let turn_id_ref = turn_id.as_ref();

        match &stored.event {
            StorageEvent::SessionStart { session_id, .. } => {
                push(AgentEvent::SessionStarted {
                    session_id: session_id.clone(),
                });
                self.phase_tracker
                    .force_to(Phase::Idle, None, AgentEventContext::default());
            },
            StorageEvent::UserMessage {
                content, origin, ..
            } => {
                if matches!(origin, UserMessageOrigin::User) {
                    if let Some(turn_id) = turn_id_ref {
                        push(AgentEvent::UserMessage {
                            turn_id: turn_id.clone(),
                            agent: agent.clone(),
                            content: content.clone(),
                        });
                    } else if !content.is_empty() {
                        warn_missing_turn_id(stored.storage_seq, "userMessage");
                    }
                }
                if self.phase_tracker.current() != Phase::Thinking {
                    push(AgentEvent::PhaseChanged {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        phase: Phase::Thinking,
                    });
                }
                self.phase_tracker
                    .force_to(Phase::Thinking, turn_id.clone(), agent.clone());
            },
            StorageEvent::PromptMetrics {
                step_index,
                estimated_tokens,
                context_window,
                effective_window,
                threshold_tokens,
                truncated_tool_results,
                provider_input_tokens,
                provider_output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                ..
            } => {
                push(AgentEvent::PromptMetrics {
                    turn_id: turn_id.clone(),
                    agent: agent.clone(),
                    step_index: *step_index,
                    estimated_tokens: *estimated_tokens,
                    context_window: *context_window,
                    effective_window: *effective_window,
                    threshold_tokens: *threshold_tokens,
                    truncated_tool_results: *truncated_tool_results,
                    provider_input_tokens: *provider_input_tokens,
                    provider_output_tokens: *provider_output_tokens,
                    cache_creation_input_tokens: *cache_creation_input_tokens,
                    cache_read_input_tokens: *cache_read_input_tokens,
                });
            },
            StorageEvent::CompactApplied {
                trigger,
                summary,
                preserved_recent_turns,
                ..
            } => {
                push(AgentEvent::CompactApplied {
                    turn_id: turn_id.clone(),
                    agent: agent.clone(),
                    trigger: *trigger,
                    summary: summary.clone(),
                    preserved_recent_turns: *preserved_recent_turns,
                });
            },
            StorageEvent::SubRunStarted {
                resolved_overrides,
                resolved_limits,
                ..
            } => {
                push(AgentEvent::SubRunStarted {
                    turn_id: turn_id.clone(),
                    agent: agent.clone(),
                    resolved_overrides: resolved_overrides.clone(),
                    resolved_limits: resolved_limits.clone(),
                });
            },
            StorageEvent::SubRunFinished {
                result,
                step_count,
                estimated_tokens,
                ..
            } => {
                push(AgentEvent::SubRunFinished {
                    turn_id: turn_id.clone(),
                    agent: agent.clone(),
                    result: result.clone(),
                    step_count: *step_count,
                    estimated_tokens: *estimated_tokens,
                });
            },
            StorageEvent::AssistantDelta { token, .. } => {
                if let Some(turn_id) = turn_id_ref {
                    push(AgentEvent::ModelDelta {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        delta: token.clone(),
                    });
                } else if !token.is_empty() {
                    warn_missing_turn_id(stored.storage_seq, "modelDelta");
                }
            },
            StorageEvent::ThinkingDelta { token, .. } => {
                if let Some(turn_id) = turn_id_ref {
                    push(AgentEvent::ThinkingDelta {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        delta: token.clone(),
                    });
                } else if !token.is_empty() {
                    warn_missing_turn_id(stored.storage_seq, "thinkingDelta");
                }
            },
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                ..
            } => {
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                let has_content =
                    !parts.visible_content.is_empty() || parts.reasoning_content.is_some();
                if let Some(turn_id) = turn_id_ref {
                    if has_content {
                        push(AgentEvent::AssistantMessage {
                            turn_id: turn_id.clone(),
                            agent: agent.clone(),
                            content: parts.visible_content,
                            reasoning_content: parts.reasoning_content,
                        });
                    }
                } else if has_content {
                    warn_missing_turn_id(stored.storage_seq, "assistantMessage");
                }
            },
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                if let Some(turn_id) = turn_id_ref {
                    self.tool_call_names
                        .insert(tool_call_id.clone(), tool_name.clone());
                    push(AgentEvent::ToolCallStart {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        tool_call_id: tool_call_id.clone(),
                        tool_name: tool_name.clone(),
                        input: args.clone(),
                    });
                } else {
                    warn_missing_turn_id(stored.storage_seq, "toolCallStart");
                }
            },
            StorageEvent::ToolCallDelta {
                tool_call_id,
                tool_name,
                stream,
                delta,
                ..
            } => {
                if let Some(turn_id) = turn_id_ref {
                    let name = if !tool_name.is_empty() {
                        tool_name.clone()
                    } else {
                        self.tool_call_names
                            .get(tool_call_id)
                            .cloned()
                            .unwrap_or_default()
                    };
                    push(AgentEvent::ToolCallDelta {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        tool_call_id: tool_call_id.clone(),
                        tool_name: name,
                        stream: *stream,
                        delta: delta.clone(),
                    });
                } else if !delta.is_empty() {
                    warn_missing_turn_id(stored.storage_seq, "toolCallDelta");
                }
            },
            StorageEvent::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                error,
                metadata,
                duration_ms,
                ..
            } => {
                if let Some(turn_id) = turn_id_ref {
                    let cached_name = self.tool_call_names.remove(tool_call_id);
                    let name = if !tool_name.is_empty() {
                        tool_name.clone()
                    } else {
                        cached_name.unwrap_or_default()
                    };
                    push(AgentEvent::ToolCallResult {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        result: ToolExecutionResult {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: name,
                            ok: *success,
                            output: output.clone(),
                            error: error.clone(),
                            metadata: metadata.clone(),
                            duration_ms: *duration_ms,
                            truncated: false,
                        },
                    });
                } else {
                    warn_missing_turn_id(stored.storage_seq, "toolCallResult");
                }
            },
            StorageEvent::TurnDone { .. } => {
                if let Some(turn_id) = turn_id_ref {
                    push(AgentEvent::TurnDone {
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                    });
                } else {
                    warn_missing_turn_id(stored.storage_seq, "turnDone");
                }
                self.phase_tracker
                    .force_to(Phase::Idle, None, AgentEventContext::default());
                self.current_turn_id = None;
            },
            StorageEvent::Error { message, .. } => {
                push(AgentEvent::Error {
                    turn_id: turn_id.clone(),
                    agent: agent.clone(),
                    code: if message == "interrupted" {
                        "interrupted".to_string()
                    } else {
                        "agent_error".to_string()
                    },
                    message: message.clone(),
                });
                if message == "interrupted" {
                    self.phase_tracker
                        .force_to(Phase::Interrupted, turn_id, agent);
                }
            },
        }
    }

    fn turn_id_for(&mut self, event: &StorageEvent) -> Option<String> {
        if let Some(turn_id) = event.turn_id() {
            let turn_id = turn_id.to_string();
            self.current_turn_id = Some(turn_id.clone());
            return Some(turn_id);
        }

        match event {
            StorageEvent::UserMessage { .. } => {
                self.legacy_turn_index = self.legacy_turn_index.saturating_add(1);
                let turn_id = format!("legacy-turn-{}", self.legacy_turn_index);
                self.current_turn_id = Some(turn_id.clone());
                Some(turn_id)
            },
            StorageEvent::SessionStart { .. } => None,
            _ => self.current_turn_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        AgentEvent, AgentEventContext, StoredEvent, ToolOutputStream, UserMessageOrigin,
        phase_of_storage_event,
    };

    #[test]
    fn user_message_replays_before_phase_change() {
        let records = replay_records(
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    content: "hello".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: chrono::Utc::now(),
                },
            }],
            None,
        );

        assert_eq!(records.len(), 2);
        assert!(matches!(
            records[0].event,
            AgentEvent::PhaseChanged {
                phase: Phase::Thinking,
                ..
            }
        ));
        assert!(matches!(
            records[1].event,
            AgentEvent::UserMessage {
                ref turn_id,
                ref content,
                ..
            } if turn_id == "turn-1" && content == "hello"
        ));
        assert_eq!(records[0].event_id, "1.0");
        assert_eq!(records[1].event_id, "1.1");
    }

    #[test]
    fn tool_call_delta_replays_with_cached_tool_name() {
        let mut translator = EventTranslator::new(Phase::Idle);
        let tool_call = StoredEvent {
            storage_seq: 1,
            event: StorageEvent::ToolCall {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                tool_call_id: "call-1".to_string(),
                tool_name: "shell".to_string(),
                args: json!({"command": "echo ok"}),
            },
        };
        let tool_delta = StoredEvent {
            storage_seq: 2,
            event: StorageEvent::ToolCallDelta {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                tool_call_id: "call-1".to_string(),
                tool_name: String::new(),
                stream: ToolOutputStream::Stdout,
                delta: "ok\n".to_string(),
            },
        };
        let tool_result = StoredEvent {
            storage_seq: 3,
            event: StorageEvent::ToolResult {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                tool_call_id: "call-1".to_string(),
                tool_name: String::new(),
                output: "ok\n".to_string(),
                success: true,
                error: None,
                metadata: None,
                duration_ms: 12,
            },
        };

        let _ = translator.translate(&tool_call);
        let delta_records = translator.translate(&tool_delta);
        let result_records = translator.translate(&tool_result);

        assert!(matches!(
            delta_records.last().map(|record| &record.event),
            Some(AgentEvent::ToolCallDelta {
                tool_name,
                stream: ToolOutputStream::Stdout,
                delta,
                ..
            }) if tool_name == "shell" && delta == "ok\n"
        ));
        assert!(matches!(
            result_records.last().map(|record| &record.event),
            Some(AgentEvent::ToolCallResult { result, .. })
                if result.tool_name == "shell" && result.output == "ok\n"
        ));
    }

    #[test]
    fn phase_of_tool_call_delta_is_calling_tool() {
        let phase = phase_of_storage_event(&StorageEvent::ToolCallDelta {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            tool_call_id: "call-1".to_string(),
            tool_name: "shell".to_string(),
            stream: ToolOutputStream::Stdout,
            delta: "ok\n".to_string(),
        });

        assert_eq!(phase, Phase::CallingTool);
    }

    #[test]
    fn replay_records_keeps_tool_call_delta_ids_monotonic() {
        let records = replay_records(
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent::ToolCallDelta {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    tool_call_id: "call-1".to_string(),
                    tool_name: "shell".to_string(),
                    stream: ToolOutputStream::Stderr,
                    delta: "warn\n".to_string(),
                },
            }],
            None,
        );

        assert_eq!(records.len(), 2);
        assert!(matches!(
            records[0].event,
            AgentEvent::PhaseChanged {
                phase: Phase::CallingTool,
                ..
            }
        ));
        assert_eq!(records[0].event_id, "1.0");
        assert_eq!(records[1].event_id, "1.1");
    }

    #[test]
    fn compact_applied_replays_as_dedicated_agent_event() {
        let records = replay_records(
            &[StoredEvent {
                storage_seq: 7,
                event: StorageEvent::CompactApplied {
                    turn_id: None,
                    agent: AgentEventContext::default(),
                    trigger: crate::CompactTrigger::Manual,
                    summary: "保留最近上下文".to_string(),
                    preserved_recent_turns: 2,
                    pre_tokens: 200,
                    post_tokens_estimate: 80,
                    messages_removed: 5,
                    tokens_freed: 120,
                    timestamp: chrono::Utc::now(),
                },
            }],
            None,
        );

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_id, "7.0");
        assert!(matches!(
            &records[0].event,
            AgentEvent::CompactApplied {
                turn_id: None,
                trigger: crate::CompactTrigger::Manual,
                summary,
                preserved_recent_turns,
                ..
            } if summary == "保留最近上下文" && *preserved_recent_turns == 2
        ));
    }

    #[test]
    fn prompt_metrics_replays_as_dedicated_agent_event() {
        let records = replay_records(
            &[StoredEvent {
                storage_seq: 9,
                event: StorageEvent::PromptMetrics {
                    turn_id: Some("turn-9".to_string()),
                    agent: AgentEventContext::default(),
                    step_index: 1,
                    estimated_tokens: 1024,
                    context_window: 200_000,
                    effective_window: 180_000,
                    threshold_tokens: 162_000,
                    truncated_tool_results: 1,
                    provider_input_tokens: Some(900),
                    provider_output_tokens: Some(120),
                    cache_creation_input_tokens: Some(700),
                    cache_read_input_tokens: Some(650),
                },
            }],
            None,
        );

        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].event,
            AgentEvent::PromptMetrics {
                turn_id,
                step_index,
                provider_input_tokens,
                cache_read_input_tokens,
                ..
            } if turn_id.as_deref() == Some("turn-9")
                && *step_index == 1
                && *provider_input_tokens == Some(900)
                && *cache_read_input_tokens == Some(650)
        ));
    }
}
