use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use astrcode_core::{
    CancelToken, ResolvedRuntimeConfig, Result, SessionTurnLease,
    support::{self},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingManualCompactRequest {
    pub(crate) runtime: ResolvedRuntimeConfig,
    pub(crate) instructions: Option<String>,
}

pub(crate) struct ActiveTurnState {
    pub(crate) turn_id: String,
    pub(crate) generation: u64,
    pub(crate) cancel: CancelToken,
    #[allow(dead_code)]
    pub(crate) turn_lease: Box<dyn SessionTurnLease>,
}

pub(crate) struct TurnRuntimeState {
    generation: AtomicU64,
    running: AtomicBool,
    active_turn: StdMutex<Option<ActiveTurnState>>,
    compact: CompactRuntimeState,
}

pub(crate) struct CompactingGuard<'a> {
    runtime: &'a TurnRuntimeState,
}

pub(crate) struct CompactRuntimeState {
    in_progress: AtomicBool,
    pending_request: StdMutex<Option<PendingManualCompactRequest>>,
    failure_count: StdMutex<u32>,
}

impl CompactRuntimeState {
    fn new() -> Self {
        Self {
            in_progress: AtomicBool::new(false),
            pending_request: StdMutex::new(None),
            failure_count: StdMutex::new(0),
        }
    }

    fn is_in_progress(&self) -> bool {
        self.in_progress.load(Ordering::SeqCst)
    }

    fn set_in_progress(&self, in_progress: bool) {
        self.in_progress.store(in_progress, Ordering::SeqCst);
    }

    fn has_pending_request(&self) -> Result<bool> {
        Ok(support::lock_anyhow(
            &self.pending_request,
            "session pending manual compact request",
        )?
        .is_some())
    }

    fn request_manual_compact(&self, request: PendingManualCompactRequest) -> Result<bool> {
        let mut pending_request = support::lock_anyhow(
            &self.pending_request,
            "session pending manual compact request",
        )?;
        let already_pending = pending_request.is_some();
        *pending_request = Some(request);
        Ok(!already_pending)
    }

    fn take_pending_request(&self) -> Result<Option<PendingManualCompactRequest>> {
        Ok(support::lock_anyhow(
            &self.pending_request,
            "session pending manual compact request",
        )?
        .take())
    }

    #[allow(dead_code)]
    fn failure_count(&self) -> Result<u32> {
        Ok(*support::lock_anyhow(
            &self.failure_count,
            "session compact failure count",
        )?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForcedTurnCompletion {
    pub(crate) turn_id: Option<String>,
    pub(crate) pending_request: Option<PendingManualCompactRequest>,
}

impl std::fmt::Debug for TurnRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnRuntimeState")
            .field("running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl Drop for CompactingGuard<'_> {
    fn drop(&mut self) {
        self.runtime.set_compacting(false);
    }
}

impl TurnRuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            running: AtomicBool::new(false),
            active_turn: StdMutex::new(None),
            compact: CompactRuntimeState::new(),
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub(crate) fn active_turn_id_snapshot(&self) -> Result<Option<String>> {
        Ok(
            support::lock_anyhow(&self.active_turn, "session active turn")?
                .as_ref()
                .map(|active| active.turn_id.clone()),
        )
    }

    pub(crate) fn prepare(
        &self,
        session_id: &str,
        turn_id: &str,
        cancel: CancelToken,
        turn_lease: Box<dyn SessionTurnLease>,
    ) -> Result<u64> {
        let mut active_turn = support::lock_anyhow(&self.active_turn, "session active turn")?;
        if active_turn.is_some() || self.is_running() {
            return Err(astrcode_core::AstrError::Validation(format!(
                "session '{}' entered an inconsistent running state",
                session_id
            )));
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *active_turn = Some(ActiveTurnState {
            turn_id: turn_id.to_string(),
            generation,
            cancel,
            turn_lease,
        });
        self.running.store(true, Ordering::SeqCst);
        Ok(generation)
    }

    pub(crate) fn complete(
        &self,
        generation: u64,
    ) -> Result<(bool, Option<PendingManualCompactRequest>)> {
        if self.generation.load(Ordering::SeqCst) != generation {
            return Ok((false, None));
        }
        let mut active_turn = support::lock_anyhow(&self.active_turn, "session active turn")?;
        if active_turn.as_ref().map(|active| active.generation) != Some(generation) {
            return Ok((false, None));
        }
        *active_turn = None;
        self.running.store(false, Ordering::SeqCst);
        Ok((true, self.compact.take_pending_request()?))
    }

    pub(crate) fn force_complete(&self) -> Result<ForcedTurnCompletion> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut active_turn = support::lock_anyhow(&self.active_turn, "session active turn")?;
        let turn_id = active_turn.take().map(|active| {
            active.cancel.cancel();
            active.turn_id
        });
        self.running.store(false, Ordering::SeqCst);
        Ok(ForcedTurnCompletion {
            turn_id,
            pending_request: self.compact.take_pending_request()?,
        })
    }

    pub(crate) fn interrupt_if_running(&self) -> Result<Option<ForcedTurnCompletion>> {
        let mut active_turn = support::lock_anyhow(&self.active_turn, "session active turn")?;
        let Some(active_turn_state) = active_turn.take() else {
            self.running.store(false, Ordering::SeqCst);
            return Ok(None);
        };
        self.generation.fetch_add(1, Ordering::SeqCst);
        active_turn_state.cancel.cancel();
        self.running.store(false, Ordering::SeqCst);
        Ok(Some(ForcedTurnCompletion {
            turn_id: Some(active_turn_state.turn_id),
            pending_request: self.compact.take_pending_request()?,
        }))
    }

    pub(crate) fn compacting(&self) -> bool {
        self.compact.is_in_progress()
    }

    pub(crate) fn set_compacting(&self, compacting: bool) {
        self.compact.set_in_progress(compacting);
    }

    pub(crate) fn enter_compacting(&self) -> CompactingGuard<'_> {
        self.set_compacting(true);
        CompactingGuard { runtime: self }
    }

    pub(crate) fn has_pending_manual_compact(&self) -> Result<bool> {
        self.compact.has_pending_request()
    }

    pub(crate) fn request_manual_compact(
        &self,
        request: PendingManualCompactRequest,
    ) -> Result<bool> {
        self.compact.request_manual_compact(request)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentId, CancelToken, EventStore, Phase, RecoveredSessionState, SessionTurnAcquireResult,
        SessionTurnLease,
    };
    use async_trait::async_trait;

    use super::TurnRuntimeState;
    use crate::{ROOT_AGENT_ID, actor::SessionActor, state::SessionWriter};

    struct StubTurnLease;

    impl SessionTurnLease for StubTurnLease {}

    #[test]
    fn turn_runtime_state_keeps_running_cache_and_active_turn_in_sync() {
        let runtime = TurnRuntimeState::new();
        let cancel = CancelToken::new();
        runtime
            .prepare(
                "session-1",
                "turn-1",
                cancel.clone(),
                Box::new(StubTurnLease),
            )
            .expect("turn runtime should enter running state");

        assert!(runtime.is_running());
        assert_eq!(
            runtime
                .active_turn_id_snapshot()
                .expect("active turn should be readable")
                .as_deref(),
            Some("turn-1")
        );

        let interrupted = runtime
            .interrupt_if_running()
            .expect("interrupt should succeed");
        assert_eq!(
            interrupted
                .as_ref()
                .and_then(|completion| completion.turn_id.as_deref()),
            Some("turn-1")
        );
        assert!(cancel.is_cancelled(), "cancel token should be triggered");
        assert!(!runtime.is_running());
        assert_eq!(
            runtime
                .active_turn_id_snapshot()
                .expect("active turn should be readable"),
            None
        );
    }

    #[test]
    fn stale_complete_generation_does_not_clear_resubmitted_turn() {
        let runtime = TurnRuntimeState::new();
        let generation_a = runtime
            .prepare(
                "session-1",
                "turn-a",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("first turn should prepare");
        let interrupted = runtime
            .force_complete()
            .expect("interrupt should clear active turn");
        assert_eq!(interrupted.turn_id.as_deref(), Some("turn-a"));

        let generation_b = runtime
            .prepare(
                "session-1",
                "turn-b",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("second turn should prepare");

        assert_eq!(
            runtime
                .complete(generation_a)
                .expect("stale finalize should not error"),
            (false, None)
        );
        assert!(
            runtime.is_running(),
            "stale finalize must not clear running cache"
        );
        assert_eq!(
            runtime
                .active_turn_id_snapshot()
                .expect("active turn should stay readable")
                .as_deref(),
            Some("turn-b")
        );

        assert_eq!(
            runtime
                .complete(generation_b)
                .expect("current generation should complete"),
            (true, None)
        );
        assert!(!runtime.is_running());
        assert_eq!(
            runtime
                .active_turn_id_snapshot()
                .expect("active turn should be cleared"),
            None
        );
    }

    #[test]
    fn interrupt_execution_if_running_is_noop_after_turn_already_completed() {
        let runtime = TurnRuntimeState::new();
        let generation = runtime
            .prepare(
                "session-1",
                "turn-1",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("turn should prepare");

        assert_eq!(
            runtime.complete(generation).expect("turn should complete"),
            (true, None)
        );

        let interrupted = runtime
            .interrupt_if_running()
            .expect("interrupt should not fail");

        assert_eq!(interrupted, None);
        assert!(!runtime.is_running());
    }

    #[test]
    fn recovery_resets_turn_runtime_to_idle_without_active_turn() {
        let writer = Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter)));
        let state = crate::state::SessionState::new(
            Phase::Idle,
            writer,
            astrcode_core::AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        );
        let checkpoint = state
            .recovery_checkpoint(7)
            .expect("checkpoint should build");

        let actor = SessionActor::from_recovery(
            astrcode_core::SessionId::from("session-1".to_string()),
            ".",
            AgentId::from(ROOT_AGENT_ID.to_string()),
            Arc::new(NoopEventStore),
            RecoveredSessionState {
                checkpoint: Some(checkpoint),
                tail_events: Vec::new(),
            },
        )
        .expect("session should recover");

        assert!(!actor.turn_runtime().is_running());
        assert_eq!(
            actor
                .turn_runtime()
                .active_turn_id_snapshot()
                .expect("active turn should be readable"),
            None
        );
        assert!(
            !actor
                .turn_runtime()
                .has_pending_manual_compact()
                .expect("manual compact state should be readable")
        );
        assert!(!actor.turn_runtime().compacting());
    }

    #[test]
    fn compacting_guard_resets_flag_on_drop() {
        let runtime = TurnRuntimeState::new();
        assert!(!runtime.compacting());
        {
            let _guard = runtime.enter_compacting();
            assert!(runtime.compacting());
        }
        assert!(!runtime.compacting());
    }

    #[test]
    fn compacting_guard_resets_flag_when_unwinding() {
        let runtime = TurnRuntimeState::new();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = runtime.enter_compacting();
            assert!(runtime.compacting());
            panic!("boom");
        }));

        assert!(result.is_err(), "guard panic should propagate");
        assert!(
            !runtime.compacting(),
            "compacting flag must be cleared even if the guarded future panics"
        );
    }

    #[derive(Debug)]
    struct NoopEventStore;

    #[async_trait]
    impl EventStore for NoopEventStore {
        async fn ensure_session(
            &self,
            _session_id: &astrcode_core::SessionId,
            _working_dir: &std::path::Path,
        ) -> astrcode_core::Result<()> {
            Ok(())
        }

        async fn append(
            &self,
            _session_id: &astrcode_core::SessionId,
            event: &astrcode_core::StorageEvent,
        ) -> astrcode_core::Result<astrcode_core::StoredEvent> {
            Ok(astrcode_core::StoredEvent {
                storage_seq: 1,
                event: event.clone(),
            })
        }

        async fn replay(
            &self,
            _session_id: &astrcode_core::SessionId,
        ) -> astrcode_core::Result<Vec<astrcode_core::StoredEvent>> {
            Ok(Vec::new())
        }

        async fn try_acquire_turn(
            &self,
            _session_id: &astrcode_core::SessionId,
            _turn_id: &str,
        ) -> astrcode_core::Result<SessionTurnAcquireResult> {
            Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
        }

        async fn list_sessions(&self) -> astrcode_core::Result<Vec<astrcode_core::SessionId>> {
            Ok(Vec::new())
        }

        async fn list_session_metas(
            &self,
        ) -> astrcode_core::Result<Vec<astrcode_core::SessionMeta>> {
            Ok(Vec::new())
        }

        async fn delete_session(
            &self,
            _session_id: &astrcode_core::SessionId,
        ) -> astrcode_core::Result<()> {
            Ok(())
        }

        async fn delete_sessions_by_working_dir(
            &self,
            _working_dir: &str,
        ) -> astrcode_core::Result<astrcode_core::DeleteProjectResult> {
            Ok(astrcode_core::DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            })
        }
    }

    #[derive(Default)]
    struct NoopEventLogWriter;

    impl astrcode_core::EventLogWriter for NoopEventLogWriter {
        fn append(
            &mut self,
            event: &astrcode_core::StorageEvent,
        ) -> astrcode_core::StoreResult<astrcode_core::StoredEvent> {
            Ok(astrcode_core::StoredEvent {
                storage_seq: 0,
                event: event.clone(),
            })
        }
    }
}
