use astrcode_core::{Result, StorageEvent, StorageEventPayload, StoredEvent};

use crate::{EventTranslator, SessionCatalog, state};

#[derive(Debug, Clone)]
pub struct CompactPersistResult {
    pub persisted_events: Vec<StoredEvent>,
    pub compact_applied: bool,
}

impl SessionCatalog {
    /// Persist compact events through the host-session writer/projection path.
    ///
    /// Summary generation remains outside this owner; host-session owns the
    /// durable append, projection update, broadcast, and checkpoint trigger.
    pub async fn persist_compact_events(
        &self,
        session_id: &astrcode_core::SessionId,
        events: Vec<StorageEvent>,
    ) -> Result<CompactPersistResult> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let phase = loaded.state.current_phase()?;
        let mut translator = EventTranslator::new(phase);
        let mut persisted_events = Vec::with_capacity(events.len());

        for event in events {
            persisted_events
                .push(state::append_and_broadcast(&loaded.state, &event, &mut translator).await?);
        }

        state::checkpoint_if_compacted(
            &self.event_store,
            session_id,
            &loaded.state,
            &persisted_events,
        )
        .await;

        let compact_applied = persisted_events.iter().any(|stored| {
            matches!(
                stored.event.payload,
                StorageEventPayload::CompactApplied { .. }
            )
        });
        Ok(CompactPersistResult {
            persisted_events,
            compact_applied,
        })
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, CompactAppliedMeta, CompactMode, CompactTrigger, StorageEvent,
        StorageEventPayload,
    };

    #[test]
    fn compact_applied_event_shape_remains_durable() {
        let event = StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::CompactApplied {
                trigger: CompactTrigger::Manual,
                summary: "summary".to_string(),
                meta: CompactAppliedMeta {
                    mode: CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 10,
                    output_summary_chars: 7,
                },
                preserved_recent_turns: 1,
                pre_tokens: 100,
                post_tokens_estimate: 20,
                messages_removed: 2,
                tokens_freed: 80,
                timestamp: chrono::Utc::now(),
            },
        };

        assert!(matches!(
            event.payload,
            StorageEventPayload::CompactApplied { summary, .. } if summary == "summary"
        ));
    }
}
