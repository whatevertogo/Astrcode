use astrcode_core::{
    EventTranslator, MailboxBatchAckedPayload, MailboxBatchStartedPayload, MailboxDiscardedPayload,
    MailboxProjection, MailboxQueuedPayload, Result, StorageEvent, StorageEventPayload,
    StoredEvent, support,
};

use super::{SessionState, append_and_broadcast};

/// mailbox durable 事件追加命令。
///
/// 为什么放在 `session-runtime`：mailbox 事件最终都是单 session event log 的追加动作，
/// 由真相层统一决定如何落成 `StorageEventPayload`，可以避免写侧在多处散落拼装。
#[derive(Debug, Clone)]
pub enum MailboxEventAppend {
    Queued(MailboxQueuedPayload),
    BatchStarted(MailboxBatchStartedPayload),
    BatchAcked(MailboxBatchAckedPayload),
    Discarded(MailboxDiscardedPayload),
}

impl MailboxEventAppend {
    pub(crate) fn into_storage_payload(self) -> StorageEventPayload {
        match self {
            Self::Queued(payload) => StorageEventPayload::AgentMailboxQueued { payload },
            Self::BatchStarted(payload) => {
                StorageEventPayload::AgentMailboxBatchStarted { payload }
            },
            Self::BatchAcked(payload) => StorageEventPayload::AgentMailboxBatchAcked { payload },
            Self::Discarded(payload) => StorageEventPayload::AgentMailboxDiscarded { payload },
        }
    }
}

pub(crate) fn mailbox_projection_target_agent_id(payload: &StorageEventPayload) -> Option<&str> {
    match payload {
        StorageEventPayload::AgentMailboxQueued { payload } => Some(&payload.envelope.to_agent_id),
        StorageEventPayload::AgentMailboxBatchStarted { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentMailboxBatchAcked { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentMailboxDiscarded { payload } => Some(&payload.target_agent_id),
        _ => None,
    }
}

impl SessionState {
    /// 读取指定 agent 的 mailbox durable 投影。
    pub fn mailbox_projection_for_agent(&self, agent_id: &str) -> Result<MailboxProjection> {
        Ok(
            support::lock_anyhow(&self.mailbox_projection_index, "mailbox projection index")?
                .get(agent_id)
                .cloned()
                .unwrap_or_default(),
        )
    }

    /// 增量应用一条 mailbox durable 事件到投影索引。
    pub(crate) fn apply_mailbox_event(&self, stored: &StoredEvent) {
        let Some(target_agent_id) = mailbox_projection_target_agent_id(&stored.event.payload)
        else {
            return;
        };
        let mut index = match support::lock_anyhow(
            &self.mailbox_projection_index,
            "mailbox projection index",
        ) {
            Ok(index) => index,
            Err(_) => return,
        };
        let projection = index
            .entry(target_agent_id.to_string())
            .or_insert_with(MailboxProjection::default);
        MailboxProjection::apply_event_for_agent(projection, stored, target_agent_id);
    }
}

/// 追加一条 mailbox durable 事件。
pub async fn append_mailbox_event(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    event: MailboxEventAppend,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    append_and_broadcast(
        session,
        &StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent,
            payload: event.into_storage_payload(),
        },
        translator,
    )
    .await
}

/// 追加一条 `AgentMailboxQueued` 事件到 session event log。
pub async fn append_mailbox_queued(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    payload: MailboxQueuedPayload,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    append_mailbox_event(
        session,
        turn_id,
        agent,
        MailboxEventAppend::Queued(payload),
        translator,
    )
    .await
}

/// 追加一条 `AgentMailboxBatchStarted` 事件。
pub async fn append_batch_started(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    payload: MailboxBatchStartedPayload,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    append_mailbox_event(
        session,
        turn_id,
        agent,
        MailboxEventAppend::BatchStarted(payload),
        translator,
    )
    .await
}

/// 追加一条 `AgentMailboxBatchAcked` 事件。
pub async fn append_batch_acked(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    payload: MailboxBatchAckedPayload,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    append_mailbox_event(
        session,
        turn_id,
        agent,
        MailboxEventAppend::BatchAcked(payload),
        translator,
    )
    .await
}

/// 追加一条 `AgentMailboxDiscarded` 事件。
pub async fn append_mailbox_discarded(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    payload: MailboxDiscardedPayload,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    append_mailbox_event(
        session,
        turn_id,
        agent,
        MailboxEventAppend::Discarded(payload),
        translator,
    )
    .await
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, AgentMailboxEnvelope, MailboxBatchAckedPayload,
        MailboxBatchStartedPayload, MailboxDiscardedPayload, MailboxQueuedPayload,
        StorageEventPayload,
    };

    use super::*;

    #[test]
    fn mailbox_event_append_maps_to_expected_storage_payload() {
        let envelope = AgentMailboxEnvelope {
            delivery_id: "delivery-1".to_string(),
            from_agent_id: "agent-parent".to_string(),
            to_agent_id: "agent-child".to_string(),
            message: "hello".to_string(),
            queued_at: chrono::Utc::now(),
            sender_lifecycle_status: AgentLifecycleStatus::Idle,
            sender_last_turn_outcome: None,
            sender_open_session_id: "session-parent".to_string(),
        };

        assert!(matches!(
            MailboxEventAppend::Queued(MailboxQueuedPayload {
                envelope: envelope.clone(),
            })
            .into_storage_payload(),
            StorageEventPayload::AgentMailboxQueued { payload }
                if payload.envelope.delivery_id == "delivery-1"
        ));
        assert!(matches!(
            MailboxEventAppend::BatchStarted(MailboxBatchStartedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentMailboxBatchStarted { payload }
                if payload.batch_id == "batch-1"
        ));
        assert!(matches!(
            MailboxEventAppend::BatchAcked(MailboxBatchAckedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentMailboxBatchAcked { payload }
                if payload.delivery_ids == vec!["delivery-1".to_string()]
        ));
        assert!(matches!(
            MailboxEventAppend::Discarded(MailboxDiscardedPayload {
                target_agent_id: "agent-child".to_string(),
                delivery_ids: vec!["delivery-1".to_string()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentMailboxDiscarded { payload }
                if payload.target_agent_id == "agent-child"
        ));
    }
}
