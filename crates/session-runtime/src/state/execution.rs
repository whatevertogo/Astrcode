use std::sync::Arc;

use astrcode_core::{
    CancelToken, EventTranslator, Phase, Result, SessionTurnLease, StorageEvent, StoredEvent,
    ToolEventSink, support,
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
    let stored = session.writer.clone().append(event.clone()).await?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(stored)
}

/// 准备 session 进入执行状态。
pub fn prepare_session_execution(
    session: &SessionState,
    session_id: &str,
    turn_id: &str,
    cancel: CancelToken,
    turn_lease: Box<dyn SessionTurnLease>,
) -> Result<()> {
    let mut cancel_guard = support::lock_anyhow(&session.cancel, "session cancel")?;
    let mut active_turn_guard =
        support::lock_anyhow(&session.active_turn_id, "session active turn")?;
    let mut lease_guard = support::lock_anyhow(&session.turn_lease, "session turn lease")?;
    if session
        .running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Err(astrcode_core::AstrError::Validation(format!(
            "session '{}' entered an inconsistent running state",
            session_id
        )));
    }
    *cancel_guard = cancel;
    *active_turn_guard = Some(turn_id.to_string());
    *lease_guard = Some(turn_lease);
    Ok(())
}

/// 完成 session 执行状态。
pub fn complete_session_execution(session: &SessionState, phase: Phase) {
    session.complete_execution_state(phase);
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
