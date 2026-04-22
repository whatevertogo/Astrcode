use std::{sync::Arc, time::Duration};

use astrcode_core::{
    AgentEventContext, EventStore, EventTranslator, Phase, Result, SessionId, StorageEvent,
    StoredEvent, TurnTerminalKind,
};
use chrono::Utc;

use crate::{
    SessionState,
    state::{append_and_broadcast, checkpoint_if_compacted},
    turn::{
        TurnCollaborationSummary, TurnFinishReason, TurnOutcome, TurnRunResult, TurnStopCause,
        TurnSummary,
        events::{error_event, turn_done_event},
        manual_compact::{ManualCompactRequest, build_manual_compact_events},
        subrun_events::subrun_finished_event,
    },
};

pub(crate) async fn persist_storage_events(
    event_store: &Arc<dyn EventStore>,
    session_state: &Arc<SessionState>,
    session_id: &str,
    translator: &mut EventTranslator,
    events: &[StorageEvent],
) -> Result<Vec<StoredEvent>> {
    let mut persisted_events = Vec::<StoredEvent>::new();
    for event in events {
        persisted_events.push(append_and_broadcast(session_state, event, translator).await?);
    }
    checkpoint_if_compacted(
        event_store,
        &SessionId::from(session_id.to_string()),
        session_state,
        &persisted_events,
    )
    .await;
    Ok(persisted_events)
}

pub(crate) async fn persist_subrun_finished_event(
    session_state: &Arc<SessionState>,
    translator: &mut EventTranslator,
    persisted_turn_id: &str,
    persisted_agent: &AgentEventContext,
    turn_result: &crate::TurnRunResult,
    source_tool_call_id: Option<String>,
) -> Result<()> {
    let Some(event) = subrun_finished_event(
        persisted_turn_id,
        persisted_agent,
        turn_result,
        source_tool_call_id,
    ) else {
        return Ok(());
    };
    append_and_broadcast(session_state, &event, translator).await?;
    Ok(())
}

pub(crate) async fn persist_turn_failure(
    session_state: &Arc<SessionState>,
    session_id: &str,
    turn_id: &str,
    agent: AgentEventContext,
    translator: &mut EventTranslator,
    source_tool_call_id: Option<String>,
    message: String,
) {
    let turn_done = turn_done_event(
        turn_id,
        &agent,
        Some(TurnTerminalKind::Error {
            message: message.clone(),
        }),
        None,
        Utc::now(),
    );
    if let Err(append_error) = append_and_broadcast(session_state, &turn_done, translator).await {
        log::error!(
            "failed to persist turn failure for session '{}': {}",
            session_id,
            append_error
        );
        return;
    }

    let failure = error_event(Some(turn_id), &agent, message.clone(), Some(Utc::now()));
    if let Err(append_error) = append_and_broadcast(session_state, &failure, translator).await {
        log::error!(
            "failed to persist turn error details for session '{}': {}",
            session_id,
            append_error
        );
    }

    let Some(subrun_finished) = subrun_finished_event(
        turn_id,
        &agent,
        &failed_turn_result(message),
        source_tool_call_id,
    ) else {
        return;
    };
    if let Err(append_error) =
        append_and_broadcast(session_state, &subrun_finished, translator).await
    {
        log::error!(
            "failed to persist failed subrun result for session '{}': {}",
            session_id,
            append_error
        );
    }
}

fn failed_turn_result(message: String) -> TurnRunResult {
    TurnRunResult {
        outcome: TurnOutcome::Error { message },
        messages: Vec::new(),
        events: Vec::new(),
        summary: TurnSummary {
            finish_reason: TurnFinishReason::Error,
            stop_cause: TurnStopCause::Error,
            last_transition: None,
            wall_duration: Duration::default(),
            step_count: 0,
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
            collaboration: TurnCollaborationSummary::default(),
        },
    }
}

pub(crate) struct DeferredManualCompactContext<'a> {
    pub(crate) gateway: &'a astrcode_kernel::KernelGateway,
    pub(crate) prompt_facts_provider: &'a dyn astrcode_core::PromptFactsProvider,
    pub(crate) event_store: &'a Arc<dyn EventStore>,
    pub(crate) working_dir: &'a str,
    pub(crate) turn_runtime: &'a crate::turn::TurnRuntimeState,
    pub(crate) session_state: &'a Arc<SessionState>,
    pub(crate) session_id: &'a str,
}

async fn persist_deferred_manual_compact(
    context: DeferredManualCompactContext<'_>,
    request: &crate::turn::PendingManualCompactRequest,
) {
    let DeferredManualCompactContext {
        gateway,
        prompt_facts_provider,
        event_store,
        working_dir,
        turn_runtime,
        session_state,
        session_id,
    } = context;
    let compacting_guard = turn_runtime.enter_compacting();
    let built = build_manual_compact_events(ManualCompactRequest {
        gateway,
        prompt_facts_provider,
        session_state,
        session_id,
        working_dir: std::path::Path::new(working_dir),
        runtime: &request.runtime,
        trigger: astrcode_core::CompactTrigger::Deferred,
        instructions: request.instructions.as_deref(),
    })
    .await;
    drop(compacting_guard);
    let events = match built {
        Ok(Some(events)) => events,
        Ok(None) => return,
        Err(error) => {
            log::warn!(
                "failed to build deferred compact for session '{}': {}",
                session_id,
                error
            );
            return;
        },
    };
    let mut compact_translator =
        EventTranslator::new(session_state.current_phase().unwrap_or(Phase::Idle));
    if let Err(error) = persist_storage_events(
        event_store,
        session_state,
        session_id,
        &mut compact_translator,
        &events,
    )
    .await
    {
        log::warn!(
            "failed to persist deferred compact for session '{}': {}",
            session_id,
            error
        );
    }
}

pub(crate) async fn persist_pending_manual_compact_if_any(
    context: DeferredManualCompactContext<'_>,
    pending_runtime: Option<crate::turn::PendingManualCompactRequest>,
) {
    if let Some(request) = pending_runtime {
        persist_deferred_manual_compact(context, &request).await;
    }
}
