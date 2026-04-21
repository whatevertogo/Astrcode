use std::sync::Arc;

use astrcode_core::{
    AgentEventContext, EventStore, EventTranslator, Phase, Result, SessionId, StorageEvent,
    StoredEvent,
};
use chrono::Utc;

use crate::{
    SessionState,
    state::{append_and_broadcast, checkpoint_if_compacted},
    turn::{
        events::error_event,
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
    message: String,
) {
    let failure = error_event(Some(turn_id), &agent, message, Some(Utc::now()));
    if let Err(append_error) = append_and_broadcast(session_state, &failure, translator).await {
        log::error!(
            "failed to persist turn failure for session '{}': {}",
            session_id,
            append_error
        );
    }
}

async fn persist_deferred_manual_compact(
    gateway: &astrcode_kernel::KernelGateway,
    prompt_facts_provider: &dyn astrcode_core::PromptFactsProvider,
    event_store: &Arc<dyn EventStore>,
    working_dir: &str,
    turn_runtime: &crate::turn::TurnRuntimeState,
    session_state: &Arc<SessionState>,
    session_id: &str,
    request: &crate::turn::PendingManualCompactRequest,
) {
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
    gateway: &astrcode_kernel::KernelGateway,
    prompt_facts_provider: &dyn astrcode_core::PromptFactsProvider,
    event_store: &Arc<dyn EventStore>,
    working_dir: &str,
    turn_runtime: &crate::turn::TurnRuntimeState,
    session_state: &Arc<SessionState>,
    session_id: &str,
    pending_runtime: Option<crate::turn::PendingManualCompactRequest>,
) {
    if let Some(request) = pending_runtime {
        persist_deferred_manual_compact(
            gateway,
            prompt_facts_provider,
            event_store,
            working_dir,
            turn_runtime,
            session_state,
            session_id,
            &request,
        )
        .await;
    }
}
