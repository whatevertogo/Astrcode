use std::path::Path;

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, ChildSessionNotification, EventTranslator,
    MailboxBatchAckedPayload, MailboxBatchStartedPayload, MailboxDiscardedPayload,
    MailboxQueuedPayload, Result, StorageEvent, StorageEventPayload, StoredEvent,
};
use chrono::Utc;

use crate::{MailboxEventAppend, SessionRuntime, append_and_broadcast, append_mailbox_event};

pub struct SessionCommands<'a> {
    runtime: &'a SessionRuntime,
}

impl<'a> SessionCommands<'a> {
    pub fn new(runtime: &'a SessionRuntime) -> Self {
        Self { runtime }
    }

    pub async fn append_agent_mailbox_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxQueuedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::Queued(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxDiscardedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::Discarded(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxBatchStartedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::BatchStarted(payload),
        )
        .await
    }

    pub async fn append_agent_mailbox_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: MailboxBatchAckedPayload,
    ) -> Result<StoredEvent> {
        self.append_agent_mailbox_event(
            session_id,
            turn_id,
            agent,
            MailboxEventAppend::BatchAcked(payload),
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
    ) -> Result<bool> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let actor = self.runtime.ensure_loaded_session(&session_id).await?;
        if actor
            .state()
            .running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            actor.state().request_manual_compact(runtime.clone())?;
            return Ok(true);
        }
        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        if let Some(events) = crate::turn::manual_compact::build_manual_compact_events(
            crate::turn::manual_compact::ManualCompactRequest {
                gateway: self.runtime.kernel.gateway(),
                prompt_facts_provider: self.runtime.prompt_facts_provider.as_ref(),
                session_state: actor.state(),
                session_id: session_id.as_str(),
                working_dir: Path::new(actor.working_dir()),
                runtime,
            },
        )
        .await?
        {
            for event in &events {
                append_and_broadcast(actor.state(), event, &mut translator).await?;
            }
        }
        Ok(false)
    }

    async fn append_agent_mailbox_event(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        event: MailboxEventAppend,
    ) -> Result<StoredEvent> {
        let session_id = astrcode_core::SessionId::from(crate::normalize_session_id(session_id));
        let session_state = self.runtime.query().session_state(&session_id).await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        append_mailbox_event(&session_state, turn_id, agent, event, &mut translator).await
    }
}
