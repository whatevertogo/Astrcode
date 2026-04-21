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
                    "子 Agent 已完成，但没有返回可读总结。".to_string()
                },
                crate::TurnOutcome::Cancelled => "子 Agent 已关闭。".to_string(),
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
