use astrcode_core::{
    EventTranslator, InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload,
    InputQueueProjection, InputQueuedPayload, Result, StorageEvent, StorageEventPayload,
    StoredEvent, support,
};

use super::{SessionState, append_and_broadcast};

/// input queue durable 事件追加命令。
///
/// 为什么放在 `session-runtime`：input queue 事件最终都是单 session event log 的追加动作，
/// 由真相层统一决定如何落成 `StorageEventPayload`，可以避免写侧在多处散落拼装。
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

pub(crate) fn input_queue_projection_target_agent_id(
    payload: &StorageEventPayload,
) -> Option<&str> {
    match payload {
        StorageEventPayload::AgentInputQueued { payload } => Some(&payload.envelope.to_agent_id),
        StorageEventPayload::AgentInputBatchStarted { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputBatchAcked { payload } => Some(&payload.target_agent_id),
        StorageEventPayload::AgentInputDiscarded { payload } => Some(&payload.target_agent_id),
        _ => None,
    }
}

impl SessionState {
    /// 读取指定 agent 的 input queue durable 投影。
    pub fn input_queue_projection_for_agent(&self, agent_id: &str) -> Result<InputQueueProjection> {
        Ok(support::lock_anyhow(
            &self.input_queue_projection_index,
            "input queue projection index",
        )?
        .get(agent_id)
        .cloned()
        .unwrap_or_default())
    }

    /// 增量应用一条 input queue durable 事件到投影索引。
    pub(crate) fn apply_input_queue_event(&self, stored: &StoredEvent) {
        let mut index = match support::lock_anyhow(
            &self.input_queue_projection_index,
            "input queue projection index",
        ) {
            Ok(index) => index,
            Err(_) => return,
        };
        apply_input_queue_event_to_index(&mut index, stored);
    }
}

pub(crate) fn apply_input_queue_event_to_index(
    index: &mut std::collections::HashMap<String, InputQueueProjection>,
    stored: &StoredEvent,
) {
    let Some(target_agent_id) = input_queue_projection_target_agent_id(&stored.event.payload)
    else {
        return;
    };
    let projection = index
        .entry(target_agent_id.to_string())
        .or_insert_with(InputQueueProjection::default);
    InputQueueProjection::apply_event_for_agent(projection, stored, target_agent_id);
}

/// 追加一条 input queue durable 事件。
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

    use super::*;

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
