use std::collections::HashMap;

use astrcode_core::{AgentEvent, Phase, ToolExecutionResult};
use async_trait::async_trait;
use chrono::Utc;

use crate::events::{StorageEvent, StoredEvent};

use super::session_ops::{load_events, normalize_session_id};
use super::{
    AgentService, ServiceResult, SessionEventRecord, SessionMessage, SessionReplay,
    SessionReplaySource,
};

#[async_trait]
impl SessionReplaySource for AgentService {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        let session_id = normalize_session_id(session_id);
        let state = self.ensure_session_loaded(&session_id).await?;

        let receiver = state.broadcaster.subscribe();
        let history = load_events(&session_id)
            .await
            .map(|events| replay_records(&events, last_event_id))?;
        Ok(SessionReplay { history, receiver })
    }
}

pub(super) fn convert_events_to_messages(events: &[StoredEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();

    for stored in events {
        match &stored.event {
            StorageEvent::UserMessage {
                content, timestamp, ..
            } => messages.push(SessionMessage::User {
                content: content.clone(),
                timestamp: timestamp.to_rfc3339(),
            }),
            StorageEvent::AssistantFinal {
                content, timestamp, ..
            } if !content.is_empty() => {
                messages.push(SessionMessage::Assistant {
                    content: content.clone(),
                    timestamp: timestamp
                        .as_ref()
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                });
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => pending_tool_calls.push((tool_call_id.clone(), tool_name.clone(), args.clone())),
            StorageEvent::ToolResult {
                tool_call_id,
                output,
                success,
                duration_ms,
                ..
            } => {
                if let Some(index) = pending_tool_calls
                    .iter()
                    .position(|(pending_id, _, _)| pending_id == tool_call_id)
                {
                    let (_, tool_name, args) = pending_tool_calls.remove(index);
                    messages.push(SessionMessage::ToolCall {
                        tool_call_id: tool_call_id.clone(),
                        tool_name,
                        args,
                        output: Some(output.clone()),
                        ok: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            _ => {}
        }
    }

    for (tool_call_id, tool_name, args) in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output: None,
            ok: None,
            duration_ms: None,
        });
    }

    messages
}

pub(super) fn replay_records(
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

pub(super) fn phase_of_storage_event(event: &StorageEvent) -> Phase {
    match event {
        StorageEvent::SessionStart { .. } => Phase::Idle,
        StorageEvent::UserMessage { .. } => Phase::Thinking,
        StorageEvent::AssistantDelta { .. } | StorageEvent::AssistantFinal { .. } => {
            Phase::Streaming
        }
        StorageEvent::ToolCall { .. } | StorageEvent::ToolResult { .. } => Phase::CallingTool,
        StorageEvent::TurnDone { .. } | StorageEvent::Error { .. } => Phase::Idle,
    }
}

pub(super) struct EventTranslator {
    pub(super) phase: Phase,
    current_turn_id: Option<String>,
    legacy_turn_index: u64,
    tool_call_names: HashMap<String, String>,
}

impl EventTranslator {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            phase,
            current_turn_id: None,
            legacy_turn_index: 0,
            tool_call_names: HashMap::new(),
        }
    }

    pub(super) fn translate(&mut self, stored: &StoredEvent) -> Vec<SessionEventRecord> {
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
            StorageEvent::AssistantFinal { content, .. } => {
                if self.phase != Phase::Streaming {
                    push(
                        AgentEvent::PhaseChanged {
                            turn_id: turn_id.clone(),
                            phase: Phase::Streaming,
                        },
                        &mut records,
                    );
                }
                if !content.is_empty() {
                    if let Some(turn_id) = turn_id.clone() {
                        push(
                            AgentEvent::AssistantMessage {
                                turn_id,
                                content: content.clone(),
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
                                duration_ms: *duration_ms as u128,
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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    #[test]
    fn empty_assistant_final_only_updates_phase() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let stored = StoredEvent {
            storage_seq: 7,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-1".to_string()),
                content: String::new(),
                timestamp: None,
            },
        };

        let records = translator.translate(&stored);

        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].event,
            AgentEvent::PhaseChanged {
                turn_id: Some(ref turn_id),
                phase: Phase::Streaming,
            } if turn_id == "turn-1"
        ));
    }

    #[test]
    fn non_empty_assistant_final_emits_message() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let stored = StoredEvent {
            storage_seq: 8,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-2".to_string()),
                content: "hello".to_string(),
                timestamp: None,
            },
        };

        let records = translator.translate(&stored);

        assert_eq!(records.len(), 2);
        assert!(matches!(
            records[1].event,
            AgentEvent::AssistantMessage {
                ref turn_id,
                ref content,
            } if turn_id == "turn-2" && content == "hello"
        ));
    }

    #[test]
    fn replay_skips_empty_assistant_messages() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SessionStart {
                    session_id: "session-1".to_string(),
                    timestamp: Utc::now(),
                    working_dir: "/tmp".to_string(),
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-3".to_string()),
                    content: "run tool".to_string(),
                    timestamp: Utc::now(),
                },
            },
            StoredEvent {
                storage_seq: 3,
                event: StorageEvent::AssistantFinal {
                    turn_id: Some("turn-3".to_string()),
                    content: String::new(),
                    timestamp: None,
                },
            },
        ];

        let records = replay_records(&events, None);

        assert!(!records
            .iter()
            .any(|record| matches!(record.event, AgentEvent::AssistantMessage { .. })));
    }

    #[test]
    fn snapshot_keeps_assistant_timestamp_from_log() {
        let expected = DateTime::parse_from_rfc3339("2026-03-17T01:02:03Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-4".to_string()),
                content: "persisted".to_string(),
                timestamp: Some(expected),
            },
        }];

        let messages = convert_events_to_messages(&events);

        assert!(matches!(
            messages.as_slice(),
            [SessionMessage::Assistant { timestamp, .. }]
            if timestamp == &expected.to_rfc3339()
        ));
    }

    #[test]
    fn tool_call_result_keeps_tool_name() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let _ = translator.translate(&StoredEvent {
            storage_seq: 1,
            event: StorageEvent::ToolCall {
                turn_id: Some("turn-5".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: "grep".to_string(),
                args: serde_json::json!({"pattern":"TODO"}),
            },
        });

        let records = translator.translate(&StoredEvent {
            storage_seq: 2,
            event: StorageEvent::ToolResult {
                turn_id: Some("turn-5".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: "grep".to_string(),
                output: "ok".to_string(),
                success: true,
                duration_ms: 10,
            },
        });

        assert!(matches!(
            &records[0].event,
            AgentEvent::ToolCallResult { result, .. }
            if result.tool_name == "grep"
        ));
    }

    #[test]
    fn tool_result_falls_back_to_hashmap_when_stored_name_is_empty() {
        let mut translator = EventTranslator::new(Phase::Thinking);
        let _ = translator.translate(&StoredEvent {
            storage_seq: 1,
            event: StorageEvent::ToolCall {
                turn_id: Some("turn-1".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: "read_file".to_string(),
                args: serde_json::json!({"path":"/tmp/test.txt"}),
            },
        });

        let records = translator.translate(&StoredEvent {
            storage_seq: 2,
            event: StorageEvent::ToolResult {
                turn_id: Some("turn-1".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: String::new(),
                output: "file contents".to_string(),
                success: true,
                duration_ms: 5,
            },
        });

        assert!(matches!(
            &records[0].event,
            AgentEvent::ToolCallResult { result, .. }
            if result.tool_name == "read_file"
        ));
    }
}
