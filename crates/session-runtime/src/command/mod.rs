use std::path::Path;

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, ChildSessionNotification, EventTranslator,
    InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload, InputQueuedPayload,
    ModeId, Result, StorageEvent, StorageEventPayload, StoredEvent,
};
use chrono::Utc;

use crate::{
    InputQueueEventAppend, SessionRuntime, append_and_broadcast, append_input_queue_event,
    state::checkpoint_if_compacted,
};

pub(crate) struct SessionCommands<'a> {
    runtime: &'a SessionRuntime,
}

impl<'a> SessionCommands<'a> {
    pub(crate) fn new(runtime: &'a SessionRuntime) -> Self {
        Self { runtime }
    }

    pub async fn append_agent_input_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputQueuedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_input_event(
            session_id,
            turn_id,
            agent,
            InputQueueEventAppend::Queued(payload),
        )
        .await
    }

    pub async fn append_agent_input_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputDiscardedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_input_event(
            session_id,
            turn_id,
            agent,
            InputQueueEventAppend::Discarded(payload),
        )
        .await
    }

    pub async fn append_agent_input_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchStartedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_input_event(
            session_id,
            turn_id,
            agent,
            InputQueueEventAppend::BatchStarted(payload),
        )
        .await
    }

    pub async fn append_agent_input_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchAckedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_input_event(
            session_id,
            turn_id,
            agent,
            InputQueueEventAppend::BatchAcked(payload),
        )
        .await
    }

    pub async fn append_child_session_notification(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        notification: ChildSessionNotification,
    ) -> Result<StoredEvent> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.runtime.query().session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent,
                payload: StorageEventPayload::ChildSessionNotification {
                    notification,
                    timestamp: Some(Utc::now()),
                },
            },
            &mut translator,
        )
        .await
    }

    pub async fn append_agent_collaboration_fact(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        fact: AgentCollaborationFact,
    ) -> Result<StoredEvent> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.runtime.query().session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some(turn_id.to_string()),
                agent,
                payload: StorageEventPayload::AgentCollaborationFact {
                    fact,
                    timestamp: Some(Utc::now()),
                },
            },
            &mut translator,
        )
        .await
    }

    pub async fn compact_session(
        &self,
        session_id: &str,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        instructions: Option<&str>,
    ) -> Result<bool> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        if actor
            .state()
            .running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            actor
                .state()
                .request_manual_compact(crate::state::PendingManualCompactRequest {
                    runtime: runtime.clone(),
                    instructions: instructions.map(str::to_string),
                })?;
            return Ok(true);
        }
        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        actor.state().set_compacting(true);
        let built = crate::turn::manual_compact::build_manual_compact_events(
            crate::turn::manual_compact::ManualCompactRequest {
                gateway: self.runtime.kernel.gateway(),
                prompt_facts_provider: self.runtime.prompt_facts_provider.as_ref(),
                session_state: actor.state(),
                session_id: session_id.as_str(),
                working_dir: Path::new(actor.working_dir()),
                runtime,
                trigger: astrcode_core::CompactTrigger::Manual,
                instructions,
            },
        )
        .await;
        actor.state().set_compacting(false);
        if let Some(events) = built? {
            let mut persisted = Vec::with_capacity(events.len());
            for event in &events {
                persisted.push(append_and_broadcast(actor.state(), event, &mut translator).await?);
            }
            checkpoint_if_compacted(
                &self.runtime.event_store,
                &session_id,
                actor.state(),
                &persisted,
            )
            .await;
        }
        Ok(false)
    }

    pub async fn switch_mode(
        &self,
        session_id: &str,
        from: ModeId,
        to: ModeId,
    ) -> Result<StoredEvent> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.runtime.query().session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: None,
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::ModeChanged {
                    from,
                    to,
                    timestamp: Utc::now(),
                },
            },
            &mut translator,
        )
        .await
    }

    async fn append_agent_input_event(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        event: InputQueueEventAppend,
    ) -> Result<StoredEvent> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.runtime.query().session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_input_queue_event(&session_state, turn_id, agent, event, &mut translator).await
    }
}
