use std::sync::Arc;

use astrcode_core::{
    EventStore, EventTranslator, Result, SessionId, StorageEvent, StorageEventPayload, StoredEvent,
    ToolEventSink,
};
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::SessionState;

/// 广播并缓存一条事件到 session 的 durable event log。
pub async fn append_and_broadcast(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    session.append_and_broadcast(event, translator).await
}

pub async fn checkpoint_if_compacted(
    event_store: &Arc<dyn EventStore>,
    session_id: &SessionId,
    session_state: &Arc<SessionState>,
    persisted_events: &[StoredEvent],
) {
    let Some(checkpoint_storage_seq) = persisted_events.last().map(|stored| stored.storage_seq)
    else {
        return;
    };
    if !persisted_events.iter().any(|stored| {
        matches!(
            stored.event.payload,
            StorageEventPayload::CompactApplied { .. }
        )
    }) {
        return;
    }
    let checkpoint = match session_state.recovery_checkpoint(checkpoint_storage_seq) {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            log::warn!(
                "failed to build recovery checkpoint for session '{}': {}",
                session_id,
                error
            );
            return;
        },
    };
    if let Err(error) = event_store
        .checkpoint_session(session_id, &checkpoint)
        .await
    {
        log::warn!(
            "failed to persist recovery checkpoint for session '{}': {}",
            session_id,
            error
        );
    }
}

pub struct SessionStateEventSink {
    session: Arc<SessionState>,
    translator: Mutex<EventTranslator>,
}

impl SessionStateEventSink {
    pub fn new(session: Arc<SessionState>) -> Result<Self> {
        let phase = session.current_phase()?;
        Ok(Self {
            session,
            translator: Mutex::new(EventTranslator::new(phase)),
        })
    }
}

#[async_trait]
impl ToolEventSink for SessionStateEventSink {
    async fn emit(&self, event: StorageEvent) -> astrcode_core::Result<()> {
        let mut translator = self.translator.lock().await;
        append_and_broadcast(&self.session, &event, &mut translator)
            .await
            .map(|_| ())
    }
}
