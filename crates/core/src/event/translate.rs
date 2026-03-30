use std::collections::HashMap;

use crate::{
    session::SessionEventRecord, split_assistant_content, AgentEvent, Phase, StorageEvent,
    StoredEvent, ToolExecutionResult,
};

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

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    let storage_seq = storage_seq.parse().ok()?;
    let subindex = subindex.parse().ok()?;
    Some((storage_seq, subindex))
}

/// 根据 storage event 推断当前 phase。
/// 注意：Error 事件在 message == "interrupted" 时应映射为 Phase::Interrupted，
/// 但该函数仅用于 tail 扫描等轻量级场景，完整的 phase 转换由 EventTranslator 处理。
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

pub struct EventTranslator {
    pub phase: Phase,
    current_turn_id: Option<String>,
    legacy_turn_index: u64,
    tool_call_names: HashMap<String, String>,
}

impl EventTranslator {
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            current_turn_id: None,
            legacy_turn_index: 0,
            tool_call_names: HashMap::new(),
        }
    }

    pub fn translate(&mut self, stored: &StoredEvent) -> Vec<SessionEventRecord> {
        let mut subindex = 0u32;
        let mut records = Vec::new();
        let turn_id = self.turn_id_for(&stored.event);

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
                }
                self.phase = Phase::CallingTool;
            }
            StorageEvent::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
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
                                error: None,
                                metadata: None,
                                duration_ms: *duration_ms,
                                truncated: false,
                            },
                        },
                        &mut records,
                    );
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
            }
            StorageEvent::SessionStart { .. } => None,
            _ => self.current_turn_id.clone(),
        }
    }
}
