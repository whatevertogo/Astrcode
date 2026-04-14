use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{
    CancelToken, EventTranslator, Phase, Result, SessionTurnLease, StorageEvent, StoredEvent,
    ToolEventSink, support,
};

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

fn append_and_broadcast_blocking(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    let stored = session.writer.append_blocking(event)?;
    let records = session.translate_store_and_cache(&stored, translator)?;
    for record in records {
        let _ = session.broadcaster.send(record);
    }
    Ok(stored)
}

/// 从 turn callback 上下文（可能不在 tokio reactor 上）安全地 append 事件。
pub fn append_and_broadcast_from_turn_callback(
    session: &SessionState,
    event: &StorageEvent,
    translator: &mut EventTranslator,
) -> Result<StoredEvent> {
    match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => {
            append_and_broadcast_blocking(session, event, translator)
        },
        _ => tokio::task::block_in_place(|| {
            append_and_broadcast_blocking(session, event, translator)
        }),
    }
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
    translator: StdMutex<EventTranslator>,
}

impl SessionStateEventSink {
    pub fn new(session: Arc<SessionState>) -> Result<Self> {
        let phase = session.current_phase()?;
        Ok(Self {
            session,
            translator: StdMutex::new(EventTranslator::new(phase)),
        })
    }
}

impl ToolEventSink for SessionStateEventSink {
    fn emit(&self, event: StorageEvent) -> astrcode_core::Result<()> {
        let mut translator = self
            .translator
            .lock()
            .expect("session translator lock should not be poisoned");
        append_and_broadcast_from_turn_callback(&self.session, &event, &mut translator)
            .map(|_| ())
            .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
    }
}
