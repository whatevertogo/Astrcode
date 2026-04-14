//! Turn 终态投影。
//!
//! Why: 这里专门表达“某个 turn 最终发生了什么”，
//! 不让这类终态推断逻辑回流到 `application`。

use astrcode_core::{AgentTurnOutcome, Phase, StorageEventPayload, StoredEvent};

#[derive(Debug, Clone)]
pub struct TurnTerminalSnapshot {
    pub phase: Phase,
    pub events: Vec<StoredEvent>,
}

#[derive(Debug, Clone)]
pub struct ProjectedTurnOutcome {
    pub outcome: AgentTurnOutcome,
    pub summary: String,
    pub technical_message: String,
}

pub fn has_terminal_turn_signal(events: &[StoredEvent]) -> bool {
    events.iter().any(|stored| {
        matches!(
            stored.event.payload,
            StorageEventPayload::TurnDone { .. } | StorageEventPayload::Error { .. }
        )
    })
}

pub fn project_turn_outcome(phase: Phase, events: &[StoredEvent]) -> ProjectedTurnOutcome {
    let last_assistant = events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::AssistantFinal { content, .. } if !content.trim().is_empty() => {
                Some(content.trim().to_string())
            },
            _ => None,
        });
    let last_error = events
        .iter()
        .rev()
        .find_map(|stored| match &stored.event.payload {
            StorageEventPayload::Error { message, .. } if !message.trim().is_empty() => {
                Some(message.trim().to_string())
            },
            _ => None,
        });
    let last_turn_done_reason =
        events
            .iter()
            .rev()
            .find_map(|stored| match &stored.event.payload {
                StorageEventPayload::TurnDone { reason, .. } => reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|reason| !reason.is_empty())
                    .map(ToString::to_string),
                _ => None,
            });
    let outcome = if matches!(phase, Phase::Interrupted) {
        match last_error.as_deref() {
            Some("interrupted") | None => AgentTurnOutcome::Cancelled,
            Some(_) => AgentTurnOutcome::Failed,
        }
    } else if last_error.is_some() {
        AgentTurnOutcome::Failed
    } else if matches!(last_turn_done_reason.as_deref(), Some("token_exceeded")) {
        // Why: `TurnDone.reason` 是 durable 终态语义，明确标注 token_exceeded 时，
        // 不应再把这轮 turn 当作普通 completed。
        AgentTurnOutcome::TokenExceeded
    } else {
        AgentTurnOutcome::Completed
    };

    let summary = match outcome {
        AgentTurnOutcome::Completed => last_assistant
            .clone()
            .unwrap_or_else(|| "子 Agent 已完成，但没有返回可读总结。".to_string()),
        AgentTurnOutcome::TokenExceeded => last_assistant
            .clone()
            .unwrap_or_else(|| "子 Agent 因 token 限额结束，但没有返回可读总结。".to_string()),
        AgentTurnOutcome::Failed => last_error
            .clone()
            .or(last_assistant.clone())
            .unwrap_or_else(|| "子 Agent 失败，且没有返回可读错误信息。".to_string()),
        AgentTurnOutcome::Cancelled => last_error
            .clone()
            .unwrap_or_else(|| "子 Agent 已关闭。".to_string()),
    };

    ProjectedTurnOutcome {
        outcome,
        summary: summary.clone(),
        technical_message: last_error.unwrap_or(summary),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentTurnOutcome, Phase, StorageEvent, StorageEventPayload, StoredEvent,
    };

    use super::{has_terminal_turn_signal, project_turn_outcome};

    #[test]
    fn has_terminal_turn_signal_detects_turn_done() {
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::TurnDone {
                    timestamp: chrono::Utc::now(),
                    reason: Some("completed".to_string()),
                },
            },
        }];

        assert!(has_terminal_turn_signal(&events));
    }

    #[test]
    fn project_turn_outcome_prefers_assistant_summary_on_success() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::AssistantFinal {
                        content: "完成总结".to_string(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            }],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::Completed);
        assert_eq!(outcome.summary, "完成总结");
    }

    #[test]
    fn project_turn_outcome_marks_token_exceeded_when_turn_done_reason_matches() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            &[
                StoredEvent {
                    storage_seq: 1,
                    event: StorageEvent {
                        turn_id: Some("turn-1".to_string()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::TurnDone {
                            timestamp: chrono::Utc::now(),
                            reason: Some("token_exceeded".to_string()),
                        },
                    },
                },
                StoredEvent {
                    storage_seq: 2,
                    event: StorageEvent {
                        turn_id: Some("turn-1".to_string()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::AssistantFinal {
                            content: "仍然视为完成".to_string(),
                            reasoning_content: None,
                            reasoning_signature: None,
                            timestamp: Some(chrono::Utc::now()),
                        },
                    },
                },
            ],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::TokenExceeded);
        assert_eq!(outcome.summary, "仍然视为完成");
    }

    #[test]
    fn project_turn_outcome_treats_unknown_turn_done_reason_as_completed() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            &[
                StoredEvent {
                    storage_seq: 1,
                    event: StorageEvent {
                        turn_id: Some("turn-1".to_string()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::TurnDone {
                            timestamp: chrono::Utc::now(),
                            reason: Some("completed".to_string()),
                        },
                    },
                },
                StoredEvent {
                    storage_seq: 2,
                    event: StorageEvent {
                        turn_id: Some("turn-1".to_string()),
                        agent: AgentEventContext::default(),
                        payload: StorageEventPayload::AssistantFinal {
                            content: "普通完成".to_string(),
                            reasoning_content: None,
                            reasoning_signature: None,
                            timestamp: Some(chrono::Utc::now()),
                        },
                    },
                },
            ],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::Completed);
        assert_eq!(outcome.summary, "普通完成");
    }
}
