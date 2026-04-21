use astrcode_core::{InputQueueProjection, Result, StorageEventPayload, StoredEvent, support};

use super::SessionState;

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
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .input_queue_projection_for_agent(agent_id),
        )
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
    let projection = index.entry(target_agent_id.to_string()).or_default();
    InputQueueProjection::apply_event_for_agent(projection, stored, target_agent_id);
}

#[cfg(test)]
mod tests {
    use astrcode_core::StorageEventPayload;

    use super::*;

    #[test]
    fn input_queue_projection_target_agent_id_reads_supported_payloads() {
        let payload = StorageEventPayload::AgentInputBatchStarted {
            payload: astrcode_core::InputBatchStartedPayload {
                target_agent_id: "agent-child".to_string(),
                turn_id: "turn-1".to_string(),
                batch_id: "batch-1".to_string(),
                delivery_ids: vec!["delivery-1".to_string().into()],
            },
        };

        assert_eq!(
            input_queue_projection_target_agent_id(&payload),
            Some("agent-child")
        );
    }
}
