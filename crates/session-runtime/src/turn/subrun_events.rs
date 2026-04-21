use astrcode_core::{
    AgentEventContext, CompletedParentDeliveryPayload, ParentDelivery, ParentDeliveryOrigin,
    ParentDeliveryPayload, ParentDeliveryTerminalSemantics, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, StorageEvent, StorageEventPayload,
};
use chrono::Utc;

use crate::turn::projector::last_non_empty_assistant_message;

pub(crate) fn subrun_started_event(
    turn_id: &str,
    agent: &AgentEventContext,
    resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunStarted {
            tool_call_id: source_tool_call_id,
            resolved_overrides: resolved_overrides.unwrap_or_default(),
            resolved_limits: resolved_limits.unwrap_or_default(),
            timestamp: Some(Utc::now()),
        },
    })
}

pub(crate) fn subrun_finished_event(
    turn_id: &str,
    agent: &AgentEventContext,
    turn_result: &crate::TurnRunResult,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    let summary =
        last_non_empty_assistant_message(&turn_result.messages).unwrap_or_else(
            || match &turn_result.outcome {
                crate::TurnOutcome::Completed => {
                    "sub-agent completed without readable summary".to_string()
                },
                crate::TurnOutcome::Cancelled => "sub-agent cancelled".to_string(),
                crate::TurnOutcome::Error { message } => message.trim().to_string(),
            },
        );

    let result = match &turn_result.outcome {
        crate::TurnOutcome::Completed => astrcode_core::SubRunResult::Completed {
            outcome: astrcode_core::CompletedSubRunOutcome::Completed,
            handoff: astrcode_core::SubRunHandoff {
                findings: Vec::new(),
                artifacts: Vec::new(),
                delivery: Some(ParentDelivery {
                    idempotency_key: format!(
                        "subrun-finished:{}:{}",
                        agent.sub_run_id.as_deref().unwrap_or("unknown-subrun"),
                        turn_id
                    ),
                    origin: ParentDeliveryOrigin::Fallback,
                    terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                    source_turn_id: Some(turn_id.to_string()),
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: summary,
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    }),
                }),
            },
        },
        crate::TurnOutcome::Cancelled => astrcode_core::SubRunResult::Failed {
            outcome: astrcode_core::FailedSubRunOutcome::Cancelled,
            failure: astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Interrupted,
                display_message: summary,
                technical_message: "interrupted".to_string(),
                retryable: false,
            },
        },
        crate::TurnOutcome::Error { message } => astrcode_core::SubRunResult::Failed {
            outcome: astrcode_core::FailedSubRunOutcome::Failed,
            failure: astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Internal,
                display_message: summary,
                technical_message: message.clone(),
                retryable: true,
            },
        },
    };

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunFinished {
            tool_call_id: source_tool_call_id,
            result,
            step_count: turn_result.summary.step_count as u32,
            estimated_tokens: turn_result.summary.total_tokens_used,
            timestamp: Some(Utc::now()),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use astrcode_core::{
        AgentEventContext, CompletedParentDeliveryPayload, ParentDeliveryPayload,
        StorageEventPayload, SubRunStorageMode,
    };

    use super::subrun_finished_event;

    fn subrun_agent() -> AgentEventContext {
        AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "reviewer",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some("session-child".into()),
        )
    }

    fn summary() -> crate::TurnSummary {
        crate::TurnSummary {
            finish_reason: crate::TurnFinishReason::NaturalEnd,
            stop_cause: crate::turn::loop_control::TurnStopCause::Completed,
            last_transition: None,
            wall_duration: Duration::from_secs(1),
            step_count: 0,
            continuation_count: 0,
            total_tokens_used: 0,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            auto_compaction_count: 0,
            reactive_compact_count: 0,
            max_output_continuation_count: 0,
            tool_result_replacement_count: 0,
            tool_result_reapply_count: 0,
            tool_result_bytes_saved: 0,
            tool_result_over_budget_message_count: 0,
            streaming_tool_launch_count: 0,
            streaming_tool_match_count: 0,
            streaming_tool_fallback_count: 0,
            streaming_tool_discard_count: 0,
            streaming_tool_overlap_ms: 0,
            collaboration: crate::TurnCollaborationSummary::default(),
        }
    }

    #[test]
    fn completed_subrun_fallback_summary_is_language_neutral_in_durable_event() {
        let event = subrun_finished_event(
            "turn-1",
            &subrun_agent(),
            &crate::TurnRunResult {
                messages: Vec::new(),
                events: Vec::new(),
                outcome: crate::TurnOutcome::Completed,
                summary: summary(),
            },
            None,
        )
        .expect("subrun completion should emit a durable event");

        let StorageEventPayload::SubRunFinished { result, .. } = event.payload else {
            panic!("expected SubRunFinished payload");
        };
        let message = match result {
            astrcode_core::SubRunResult::Completed { handoff, .. } => match handoff
                .delivery
                .expect("fallback delivery should exist")
                .payload
            {
                ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                    message,
                    ..
                }) => message,
                other => panic!("expected completed delivery payload, got {other:?}"),
            },
            other => panic!("expected completed subrun result, got {other:?}"),
        };
        assert_eq!(message, "sub-agent completed without readable summary");
    }

    #[test]
    fn cancelled_subrun_fallback_summary_is_language_neutral_in_durable_event() {
        let event = subrun_finished_event(
            "turn-1",
            &subrun_agent(),
            &crate::TurnRunResult {
                messages: Vec::new(),
                events: Vec::new(),
                outcome: crate::TurnOutcome::Cancelled,
                summary: summary(),
            },
            None,
        )
        .expect("subrun cancellation should emit a durable event");

        let StorageEventPayload::SubRunFinished { result, .. } = event.payload else {
            panic!("expected SubRunFinished payload");
        };
        let display_message = match result {
            astrcode_core::SubRunResult::Failed { failure, .. } => failure.display_message,
            other => panic!("expected failed subrun result, got {other:?}"),
        };
        assert_eq!(display_message, "sub-agent cancelled");
    }
}
