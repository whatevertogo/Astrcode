//! Turn 终态投影。
//!
//! Why: 这里专门表达“某个 turn 最终发生了什么”，
//! 不让这类终态推断逻辑回流到 `application`。

use astrcode_core::{
    AgentTurnOutcome, Phase, StoredEvent, TurnProjectionSnapshot, TurnTerminalKind,
};

use crate::turn::projector::{
    last_non_empty_assistant_event, last_non_empty_error_event, project_turn_projection,
};

#[derive(Debug, Clone)]
pub struct TurnTerminalSnapshot {
    pub phase: Phase,
    pub projection: Option<TurnProjectionSnapshot>,
    pub events: Vec<StoredEvent>,
}

#[derive(Debug, Clone)]
pub struct ProjectedTurnOutcome {
    pub outcome: AgentTurnOutcome,
    pub summary: String,
    pub technical_message: String,
}

pub(crate) fn project_turn_outcome(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    events: &[StoredEvent],
) -> ProjectedTurnOutcome {
    let replayed_projection = project_turn_projection(events);
    let projection = projection.or(replayed_projection.as_ref());
    let last_assistant = last_non_empty_assistant_event(events);
    let last_error = last_non_empty_error_event(events);
    let terminal_kind = resolve_terminal_kind(phase, projection, last_error.as_deref());
    let outcome = project_agent_turn_outcome(terminal_kind.as_ref());

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
    let technical_message = match terminal_kind {
        Some(TurnTerminalKind::Error { message }) => last_error.unwrap_or(message),
        _ => last_error.unwrap_or(summary.clone()),
    };

    ProjectedTurnOutcome {
        outcome,
        summary: summary.clone(),
        technical_message,
    }
}

fn resolve_terminal_kind(
    phase: Phase,
    projection: Option<&TurnProjectionSnapshot>,
    last_error: Option<&str>,
) -> Option<TurnTerminalKind> {
    if let Some(turn_done_kind) = projection.and_then(|projection| projection.terminal_kind.clone())
    {
        return Some(turn_done_kind);
    }

    if matches!(phase, Phase::Interrupted) {
        return match projection
            .and_then(|projection| projection.last_error.as_deref())
            .or(last_error)
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            Some("interrupted") | None => Some(TurnTerminalKind::Cancelled),
            Some(message) => Some(TurnTerminalKind::Error {
                message: message.to_string(),
            }),
        };
    }

    projection
        .and_then(|projection| projection.last_error.as_deref())
        .or(last_error)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| TurnTerminalKind::Error {
            message: message.to_string(),
        })
}

fn project_agent_turn_outcome(terminal_kind: Option<&TurnTerminalKind>) -> AgentTurnOutcome {
    match terminal_kind {
        Some(
            TurnTerminalKind::Completed
            | TurnTerminalKind::BudgetStoppedContinuation
            | TurnTerminalKind::ContinuationLimitReached,
        )
        | None => AgentTurnOutcome::Completed,
        Some(TurnTerminalKind::MaxOutputContinuationLimitReached) => {
            AgentTurnOutcome::TokenExceeded
        },
        Some(TurnTerminalKind::Cancelled) => AgentTurnOutcome::Cancelled,
        Some(TurnTerminalKind::Error { .. } | TurnTerminalKind::StepLimitExceeded) => {
            AgentTurnOutcome::Failed
        },
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentTurnOutcome, Phase, StorageEvent, StorageEventPayload, StoredEvent,
        TurnProjectionSnapshot,
    };

    use super::project_turn_outcome;
    use crate::turn::projector::{has_terminal_projection, project_turn_projection};

    #[test]
    fn has_terminal_projection_detects_typed_terminal_kind() {
        assert!(has_terminal_projection(Some(&TurnProjectionSnapshot {
            terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
            last_error: None,
        })));
    }

    #[test]
    fn project_turn_projection_projects_legacy_turn_done_reason() {
        let projection = project_turn_projection(&[StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::TurnDone {
                    timestamp: chrono::Utc::now(),
                    terminal_kind: None,
                    reason: Some("completed".to_string()),
                },
            },
        }])
        .expect("projection should replay");

        assert_eq!(
            projection.terminal_kind,
            Some(astrcode_core::TurnTerminalKind::Completed)
        );
    }

    #[test]
    fn project_turn_outcome_prefers_assistant_summary_on_success() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            None,
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
            Some(&TurnProjectionSnapshot {
                terminal_kind: Some(
                    astrcode_core::TurnTerminalKind::MaxOutputContinuationLimitReached,
                ),
                last_error: None,
            }),
            &[StoredEvent {
                storage_seq: 1,
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
            }],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::TokenExceeded);
        assert_eq!(outcome.summary, "仍然视为完成");
    }

    #[test]
    fn project_turn_outcome_prefers_typed_terminal_kind_over_legacy_reason() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            Some(&TurnProjectionSnapshot {
                terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                last_error: None,
            }),
            &[],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::Completed);
    }

    #[test]
    fn project_turn_outcome_treats_unknown_turn_done_reason_as_completed() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            Some(&TurnProjectionSnapshot {
                terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                last_error: None,
            }),
            &[StoredEvent {
                storage_seq: 1,
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
            }],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::Completed);
        assert_eq!(outcome.summary, "普通完成");
    }

    #[test]
    fn project_turn_outcome_uses_legacy_projection_error_for_interrupted_turns() {
        let outcome = project_turn_outcome(
            Phase::Interrupted,
            Some(&TurnProjectionSnapshot {
                terminal_kind: None,
                last_error: Some("interrupted".to_string()),
            }),
            &[],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::Cancelled);
    }

    #[test]
    fn project_turn_outcome_uses_turn_done_event_when_projection_is_missing() {
        let outcome = project_turn_outcome(
            Phase::Idle,
            None,
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: None,
                        reason: Some("token_exceeded".to_string()),
                    },
                },
            }],
        );

        assert_eq!(outcome.outcome, AgentTurnOutcome::TokenExceeded);
    }
}
