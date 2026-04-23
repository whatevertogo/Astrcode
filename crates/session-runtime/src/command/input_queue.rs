use astrcode_core::{
    EventTranslator, InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload,
    InputQueuedPayload, Result, StorageEvent, StorageEventPayload, StoredEvent,
};

use crate::{SessionState, state::append_and_broadcast};

/// input queue durable 事件追加命令。
///
/// 为什么放在 `command`：这是写路径上的命令语义，负责把上层输入变成 durable 事件，
/// 不应继续混在 `state` 的纯投影逻辑里。
#[derive(Debug, Clone)]
pub enum InputQueueEventAppend {
    Queued(InputQueuedPayload),
    BatchStarted(InputBatchStartedPayload),
    BatchAcked(InputBatchAckedPayload),
    Discarded(InputDiscardedPayload),
}

impl InputQueueEventAppend {
    pub(crate) fn into_storage_payload(self) -> StorageEventPayload {
        match self {
            Self::Queued(payload) => StorageEventPayload::AgentInputQueued { payload },
            Self::BatchStarted(payload) => StorageEventPayload::AgentInputBatchStarted { payload },
            Self::BatchAcked(payload) => StorageEventPayload::AgentInputBatchAcked { payload },
            Self::Discarded(payload) => StorageEventPayload::AgentInputDiscarded { payload },
        }
    }
}

pub async fn append_input_queue_event(
    session: &SessionState,
    turn_id: &str,
    agent: astrcode_core::AgentEventContext,
    event: InputQueueEventAppend,
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

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, InputBatchAckedPayload, InputBatchStartedPayload,
        InputDiscardedPayload, InputQueuedPayload, QueuedInputEnvelope, StorageEventPayload,
    };

    use super::InputQueueEventAppend;

    #[test]
    fn input_queue_event_append_maps_to_expected_storage_payload() {
        let envelope = QueuedInputEnvelope {
            delivery_id: "delivery-1".to_string().into(),
            from_agent_id: "agent-parent".to_string(),
            to_agent_id: "agent-child".to_string(),
            message: "hello".to_string(),
            queued_at: chrono::Utc::now(),
            sender_lifecycle_status: AgentLifecycleStatus::Idle,
            sender_last_turn_outcome: None,
            sender_open_session_id: "session-parent".to_string(),
        };

        assert!(matches!(
            InputQueueEventAppend::Queued(InputQueuedPayload {
                envelope: envelope.clone(),
            })
            .into_storage_payload(),
            StorageEventPayload::AgentInputQueued { payload }
                if payload.envelope.delivery_id == "delivery-1".into()
        ));
        assert!(matches!(
            InputQueueEventAppend::BatchStarted(InputBatchStartedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string().into()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentInputBatchStarted { payload }
                if payload.batch_id == "batch-1"
        ));
        assert!(matches!(
            InputQueueEventAppend::BatchAcked(InputBatchAckedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string().into()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentInputBatchAcked { payload }
                if payload.delivery_ids == vec!["delivery-1".to_string().into()]
        ));
        assert!(matches!(
            InputQueueEventAppend::Discarded(InputDiscardedPayload {
                target_agent_id: "agent-child".to_string(),
                delivery_ids: vec!["delivery-1".to_string().into()],
            })
            .into_storage_payload(),
            StorageEventPayload::AgentInputDiscarded { payload }
                if payload.target_agent_id == "agent-child"
        ));
    }
}
