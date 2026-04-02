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

use crate::{
    session::SessionEventRecord, split_assistant_content, AgentEvent, Phase, StorageEvent,
    StoredEvent, ToolExecutionResult,
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
                // 只返回 ID 大于断点的事件（>= 的部分已经发送过）
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

/// 根据 storage event 推断当前 phase。
///
/// ## 用途
///
/// 此函数用于 tail 扫描等轻量级场景（如会话列表展示状态），
/// 不跟踪完整的 phase 转换历史。
///
/// ## 注意
///
/// - Error 事件在 message == "interrupted" 时应映射为 Phase::Interrupted
/// - 完整的 phase 转换由 `EventTranslator` 处理
pub fn phase_of_storage_event(event: &StorageEvent) -> Phase {
    match event {
        StorageEvent::SessionStart { .. } => Phase::Idle,
        StorageEvent::UserMessage { .. } => Phase::Thinking,
        StorageEvent::AssistantDelta { .. }
        | StorageEvent::ThinkingDelta { .. }
        | StorageEvent::AssistantFinal { .. } => Phase::Streaming,
        StorageEvent::ToolCall { .. } | StorageEvent::ToolResult { .. } => Phase::CallingTool,
        StorageEvent::TurnDone { .. } => Phase::Idle,
        // "interrupted" 错误应映射为 Interrupted 而非 Idle，
        // 否则会话列表中中断的会话会错误地显示为 Idle
        StorageEvent::Error { message, .. } if message == "interrupted" => Phase::Interrupted,
        StorageEvent::Error { .. } => Phase::Idle,
    }
}

/// 事件转换器
///
/// 将存储事件转换为领域事件，同时维护会话状态。
pub struct EventTranslator {
    /// 当前会话阶段
    pub phase: Phase,
    /// 当前 Turn ID
    current_turn_id: Option<String>,
    /// 旧事件没有 turn_id，需要生成 legacy turn ID
    legacy_turn_index: u64,
    /// 工具调用 ID -> 工具名称的映射
    ///
    /// 为什么需要：ToolResult 事件的 tool_name 字段可能为空，
    /// 需要从之前的 ToolCall 事件中查找。
    tool_call_names: HashMap<String, String>,
}

fn warn_missing_turn_id(storage_seq: u64, event_name: &str) {
    log::warn!(
        "dropping translated '{}' event at storage_seq {} because turn_id is missing",
        event_name,
        storage_seq
    );
}

impl EventTranslator {
    /// 创建新的转换器
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            current_turn_id: None,
            legacy_turn_index: 0,
            tool_call_names: HashMap::new(),
        }
    }

    /// 转换单个存储事件为多个领域事件
    ///
    /// ## 返回多个事件的原因
    ///
    /// 一个存储事件可能触发：
    /// 1. PhaseChanged 事件（如果阶段发生变化）
    /// 2. 实际的事件内容
    ///
    /// ## 子序号
    ///
    /// 每个事件携带 `{storage_seq}.{subindex}` 格式的 ID，
    /// subindex 从 0 开始递增，用于 SSE 断点续传。
    pub fn translate(&mut self, stored: &StoredEvent) -> Vec<SessionEventRecord> {
        let mut subindex = 0u32;
        let mut records = Vec::new();
        let turn_id = self.turn_id_for(&stored.event);

        // 闭包：添加一个事件记录，自动分配子序号
        let mut push = |event: AgentEvent, records: &mut Vec<SessionEventRecord>| {
            records.push(SessionEventRecord {
                event_id: format!("{}.{}", stored.storage_seq, subindex),
                event,
            });
            subindex = subindex.saturating_add(1);
        };

        match &stored.event {
            StorageEvent::SessionStart { session_id, .. } => {
                push(
                    AgentEvent::SessionStarted {
                        session_id: session_id.clone(),
                    },
                    &mut records,
                );
                self.phase = Phase::Idle;
            }
            StorageEvent::UserMessage { .. } => {
                // 收到用户消息意味着新 turn 开始，状态切换到 Thinking。
                // 不检查当前 phase 是否已经是 Thinking，因为
                // UserMessage 总是新 turn 的起点。
                if self.phase != Phase::Thinking {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Thinking,
                        },
                        &mut records,
                    );
                }
                self.phase = Phase::Thinking;
            }
            StorageEvent::AssistantDelta { token, .. } => {
                // LLM 文本增量输出。首次收到时需从 Thinking 切换到 Streaming，
                // 后续增量不再触发 PhaseChanged（避免 SSE 抖动）。
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                if let Some(turn_id) = turn_id.clone() {
                    push(
                        AgentEvent::ModelDelta {
                            turn_id,
                            delta: token.clone(),
                        },
                        &mut records,
                    );
                } else if !token.is_empty() {
                    warn_missing_turn_id(stored.storage_seq, "modelDelta");
                }
                self.phase = Phase::Streaming;
            }
            StorageEvent::ThinkingDelta { token, .. } => {
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                if let Some(turn_id) = turn_id.clone() {
                    push(
                        AgentEvent::ThinkingDelta {
                            turn_id,
                            delta: token.clone(),
                        },
                        &mut records,
                    );
                } else if !token.is_empty() {
                    warn_missing_turn_id(stored.storage_seq, "thinkingDelta");
                }
                self.phase = Phase::Streaming;
            }
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                ..
            } => {
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                if let Some(turn_id) = turn_id.clone() {
                    if !parts.visible_content.is_empty() || parts.reasoning_content.is_some() {
                        push(
                            AgentEvent::AssistantMessage {
                                turn_id,
                                content: parts.visible_content,
                                reasoning_content: parts.reasoning_content,
                            },
                            &mut records,
                        );
                    }
                } else if !parts.visible_content.is_empty() || parts.reasoning_content.is_some() {
                    warn_missing_turn_id(stored.storage_seq, "assistantMessage");
                }
                self.phase = Phase::Streaming;
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                if self.phase != Phase::CallingTool {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::CallingTool,
                        },
                        &mut records,
                    );
                }
                if let Some(turn_id) = turn_id.clone() {
                    self.tool_call_names
                        .insert(tool_call_id.clone(), tool_name.clone());
                    push(
                        AgentEvent::ToolCallStart {
                            turn_id,
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_name.clone(),
                            input: args.clone(),
                        },
                        &mut records,
                    );
                } else {
                    warn_missing_turn_id(stored.storage_seq, "toolCallStart");
                }
                self.phase = Phase::CallingTool;
            }
            // 工具执行结果。不触发 PhaseChanged —— 在同一个 turn 内，
            // 可能有多个工具调用和结果交替出现，phase 保持 CallingTool。
            // 只有 TurnDone 才将 phase 切回 Idle。
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
                if let Some(turn_id) = turn_id.clone() {
                    let name = if !tool_name.is_empty() {
                        tool_name.clone()
                    } else {
                        self.tool_call_names
                            .remove(tool_call_id)
                            .unwrap_or_default()
                    };
                    push(
                        AgentEvent::ToolCallResult {
                            turn_id,
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
                        },
                        &mut records,
                    );
                } else {
                    warn_missing_turn_id(stored.storage_seq, "toolCallResult");
                }
                self.phase = Phase::CallingTool;
            }
            StorageEvent::TurnDone { .. } => {
                push(
                    AgentEvent::PhaseChanged {
                        turn_id: turn_id.clone(),
                        phase: Phase::Idle,
                    },
                    &mut records,
                );
                if let Some(turn_id) = turn_id.clone() {
                    push(AgentEvent::TurnDone { turn_id }, &mut records);
                } else {
                    warn_missing_turn_id(stored.storage_seq, "turnDone");
                }
                self.phase = Phase::Idle;
                self.current_turn_id = None;
            }
            StorageEvent::Error { message, .. } => {
                if message == "interrupted" {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Interrupted,
                        },
                        &mut records,
                    );
                    self.phase = Phase::Interrupted;
                }
                push(
                    AgentEvent::Error {
                        turn_id,
                        code: if message == "interrupted" {
                            "interrupted".to_string()
                        } else {
                            "agent_error".to_string()
                        },
                        message: message.clone(),
                    },
                    &mut records,
                );
            }
        }

        records
    }

    fn turn_id_for(&mut self, event: &StorageEvent) -> Option<String> {
        // 如果事件自带 turn_id，直接使用并更新当前 turn
        if let Some(turn_id) = event.turn_id() {
            let turn_id = turn_id.to_string();
            self.current_turn_id = Some(turn_id.clone());
            return Some(turn_id);
        }

        // 旧事件没有 turn_id，需要生成或复用
        match event {
            // UserMessage 开始新的 turn，生成新的 legacy turn ID
            StorageEvent::UserMessage { .. } => {
                self.legacy_turn_index = self.legacy_turn_index.saturating_add(1);
                let turn_id = format!("legacy-turn-{}", self.legacy_turn_index);
                self.current_turn_id = Some(turn_id.clone());
                Some(turn_id)
            }
            // SessionStart 不属于任何 turn
            StorageEvent::SessionStart { .. } => None,
            // 其他事件复用当前 turn_id
            _ => self.current_turn_id.clone(),
        }
    }
}
