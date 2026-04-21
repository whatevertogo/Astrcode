//! 会话真相状态：事件投影、child-session 节点跟踪、input queue 投影、turn 生命周期。
//!
//! 从 `runtime-session/session_state.rs` 迁入，去掉了 `anyhow` 依赖，
//! 所有 `Result` 统一使用 `astrcode_core::Result`。

mod cache;
mod child_sessions;
#[cfg(test)]
mod compaction;
mod execution;
mod input_queue;
mod paths;
mod projection_registry;
mod tasks;
#[cfg(test)]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::sample_spawn_child_ref;
mod writer;

use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use astrcode_core::{
    AgentEvent, AgentState, AgentStateProjector, CancelToken, EventTranslator, LlmMessage, ModeId,
    Phase, ResolvedRuntimeConfig, Result, SessionEventRecord, SessionRecoveryCheckpoint,
    SessionTurnLease, StoredEvent, TurnProjectionSnapshot, normalize_recovered_phase,
    support::{self},
};
use chrono::Utc;
pub use execution::checkpoint_if_compacted;
pub(crate) use execution::{SessionStateEventSink, append_and_broadcast};
pub(crate) use paths::compact_history_event_log_path;
pub use paths::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};
use projection_registry::ProjectionRegistry;
use tokio::sync::broadcast;
pub(crate) use writer::SessionWriter;

const SESSION_BROADCAST_CAPACITY: usize = 2048;
const SESSION_LIVE_BROADCAST_CAPACITY: usize = 2048;

// ── SessionState ──────────────────────────────────────────

// ── SessionState ──────────────────────────────────────────

/// 会话 live 真相：事件投影、child-session 节点跟踪、input queue 投影、turn 生命周期。
///
/// 使用 per-field `StdMutex` 而非外层 `RwLock`，
/// 允许不同字段的并发读写互不阻塞（如 broadcaster 广播不阻塞 projector 读取）。
pub struct ActiveTurnState {
    pub turn_id: String,
    pub generation: u64,
    pub cancel: CancelToken,
    #[allow(dead_code)]
    pub turn_lease: Box<dyn SessionTurnLease>,
}

pub struct TurnRuntimeState {
    generation: AtomicU64,
    running: AtomicBool,
    active_turn: StdMutex<Option<ActiveTurnState>>,
    compact: CompactRuntimeState,
}

pub struct CompactRuntimeState {
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
        self.in_progress.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_in_progress(&self, in_progress: bool) {
        self.in_progress
            .store(in_progress, std::sync::atomic::Ordering::SeqCst);
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

impl TurnRuntimeState {
    fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            running: AtomicBool::new(false),
            active_turn: StdMutex::new(None),
            compact: CompactRuntimeState::new(),
        }
    }

    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn active_turn_id_snapshot(&self) -> Result<Option<String>> {
        Ok(
            support::lock_anyhow(&self.active_turn, "session active turn")?
                .as_ref()
                .map(|active| active.turn_id.clone()),
        )
    }

    fn prepare(
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

    fn cancel_active_turn(&self) -> Result<Option<String>> {
        let active_turn = support::lock_anyhow(&self.active_turn, "session active turn")?;
        if let Some(active_turn) = active_turn.as_ref() {
            active_turn.cancel.cancel();
            return Ok(Some(active_turn.turn_id.clone()));
        }
        Ok(None)
    }

    fn complete(&self, generation: u64) -> Result<(bool, Option<PendingManualCompactRequest>)> {
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

    fn force_complete(&self) -> Result<ForcedTurnCompletion> {
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

    fn interrupt_if_running(&self) -> Result<Option<ForcedTurnCompletion>> {
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

    fn compacting(&self) -> bool {
        self.compact.is_in_progress()
    }

    fn set_compacting(&self, compacting: bool) {
        self.compact.set_in_progress(compacting);
    }

    fn has_pending_manual_compact(&self) -> Result<bool> {
        self.compact.has_pending_request()
    }

    fn request_manual_compact(&self, request: PendingManualCompactRequest) -> Result<bool> {
        self.compact.request_manual_compact(request)
    }
}

pub struct SessionState {
    turn_runtime: TurnRuntimeState,
    projection_registry: StdMutex<ProjectionRegistry>,
    pub broadcaster: broadcast::Sender<SessionEventRecord>,
    live_broadcaster: broadcast::Sender<AgentEvent>,
    pub writer: Arc<SessionWriter>,
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("running", &self.turn_runtime.is_running())
            .finish_non_exhaustive()
    }
}

/// 轻量会话快照，用于 observe 返回值（仅包含可序列化的聚合字段）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: astrcode_core::SessionId,
    pub working_dir: String,
    pub latest_turn_id: Option<astrcode_core::TurnId>,
    pub turn_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingManualCompactRequest {
    pub runtime: ResolvedRuntimeConfig,
    pub instructions: Option<String>,
}

impl SessionState {
    pub fn new(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        Self::from_parts(phase, writer, projector, recent_records, recent_stored)
    }

    pub fn from_recovery(
        writer: Arc<SessionWriter>,
        checkpoint: &SessionRecoveryCheckpoint,
        tail_events: Vec<StoredEvent>,
    ) -> Result<Self> {
        let phase = normalize_recovered_phase(checkpoint.agent_state.phase);
        let mut projection_registry = ProjectionRegistry::from_recovery(
            phase,
            &checkpoint.agent_state,
            checkpoint.projection_registry_snapshot(),
            Vec::new(),
            Vec::new(),
        );
        for stored in &tail_events {
            stored.event.validate().map_err(|error| {
                astrcode_core::AstrError::Validation(format!(
                    "session '{}' contains invalid stored event at storage_seq {}: {}",
                    checkpoint.agent_state.session_id, stored.storage_seq, error
                ))
            })?;
            projection_registry.apply(stored)?;
        }
        projection_registry.cache_records(&astrcode_core::replay_records(&tail_events, None));
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);

        Ok(Self {
            turn_runtime: TurnRuntimeState::new(),
            projection_registry: StdMutex::new(projection_registry),
            broadcaster,
            live_broadcaster,
            writer,
        })
    }

    fn from_parts(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);
        Self {
            turn_runtime: TurnRuntimeState::new(),
            projection_registry: StdMutex::new(ProjectionRegistry::new(
                phase,
                projector,
                recent_records,
                recent_stored,
            )),
            broadcaster,
            live_broadcaster,
            writer,
        }
    }

    pub fn recovery_checkpoint(
        &self,
        checkpoint_storage_seq: u64,
    ) -> Result<SessionRecoveryCheckpoint> {
        let projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        Ok(SessionRecoveryCheckpoint::new(
            projection_registry.snapshot_projected_state(),
            projection_registry.projection_snapshot(),
            checkpoint_storage_seq,
        ))
    }

    pub fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_projected_state(),
        )
    }

    pub fn current_turn_messages(&self) -> Result<Vec<LlmMessage>> {
        Ok(self.snapshot_projected_state()?.messages)
    }

    /// 订阅 live-only 事件流（token 级 delta 等瞬时事件，不参与 durable replay）。
    pub fn subscribe_live(&self) -> broadcast::Receiver<AgentEvent> {
        self.live_broadcaster.subscribe()
    }

    /// 广播一条 live-only 事件（无订阅者时不视为错误）。
    pub fn broadcast_live_event(&self, event: AgentEvent) {
        let _ = self.live_broadcaster.send(event);
    }

    pub fn current_phase(&self) -> Result<Phase> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_phase(),
        )
    }

    pub fn active_turn_id_snapshot(&self) -> Result<Option<String>> {
        self.turn_runtime.active_turn_id_snapshot()
    }

    pub fn manual_compact_pending(&self) -> Result<bool> {
        self.turn_runtime.has_pending_manual_compact()
    }

    pub fn is_running(&self) -> bool {
        self.turn_runtime.is_running()
    }

    pub fn prepare_execution(
        &self,
        session_id: &str,
        turn_id: &str,
        cancel: CancelToken,
        turn_lease: Box<dyn SessionTurnLease>,
    ) -> Result<u64> {
        self.turn_runtime
            .prepare(session_id, turn_id, cancel, turn_lease)
    }

    pub fn cancel_active_turn(&self) -> Result<Option<String>> {
        self.turn_runtime.cancel_active_turn()
    }

    pub fn current_mode_id(&self) -> Result<ModeId> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_mode_id(),
        )
    }

    pub fn last_mode_changed_at(&self) -> Result<Option<chrono::DateTime<Utc>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .last_mode_changed_at(),
        )
    }

    pub fn complete_execution_state(
        &self,
        generation: u64,
    ) -> Result<Option<PendingManualCompactRequest>> {
        let (completed, pending_request) = self.turn_runtime.complete(generation)?;
        if !completed {
            return Ok(None);
        }
        Ok(pending_request)
    }

    pub(crate) fn force_complete_execution_state(&self) -> Result<ForcedTurnCompletion> {
        self.turn_runtime.force_complete()
    }

    pub(crate) fn interrupt_execution_if_running(&self) -> Result<Option<ForcedTurnCompletion>> {
        self.turn_runtime.interrupt_if_running()
    }

    pub fn compacting(&self) -> bool {
        self.turn_runtime.compacting()
    }

    pub fn set_compacting(&self, compacting: bool) {
        self.turn_runtime.set_compacting(compacting);
    }

    pub fn request_manual_compact(&self, request: PendingManualCompactRequest) -> Result<bool> {
        self.turn_runtime.request_manual_compact(request)
    }

    pub fn translate_store_and_cache(
        &self,
        stored: &StoredEvent,
        translator: &mut EventTranslator,
    ) -> Result<Vec<SessionEventRecord>> {
        stored.event.validate()?;
        let mut projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        projection_registry.apply(stored)?;
        let records = translator.translate(stored);
        projection_registry.cache_records(&records);
        Ok(records)
    }

    pub fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .recent_records_after(last_event_id),
        )
    }

    pub fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_recent_stored_events(),
        )
    }

    pub fn turn_projection(&self, turn_id: &str) -> Result<Option<TurnProjectionSnapshot>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .turn_projection(turn_id),
        )
    }

    pub async fn append_and_broadcast(
        &self,
        event: &astrcode_core::StorageEvent,
        translator: &mut EventTranslator,
    ) -> Result<StoredEvent> {
        let stored = self.writer.clone().append(event.clone()).await?;
        let records = self.translate_store_and_cache(&stored, translator)?;
        for record in records {
            let _ = self.broadcaster.send(record);
        }
        Ok(stored)
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentEventContext, CancelToken, ExecutionTaskItem, ExecutionTaskStatus, InvocationKind,
        ModeId, Phase, SessionRecoveryCheckpoint, SessionTurnLease, StorageEventPayload,
        SubRunStorageMode, UserMessageOrigin,
    };
    use chrono::Utc;

    use super::{
        SessionState, SessionWriter,
        test_support::{
            NoopEventLogWriter, event, independent_session_sub_run_agent, root_agent, stored,
            test_session_state,
        },
    };

    struct StubTurnLease;

    impl SessionTurnLease for StubTurnLease {}

    #[test]
    fn translate_store_and_cache_keeps_sub_run_events_out_of_parent_snapshot() {
        let session = test_session_state();
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);

        let events = vec![
            stored(
                1,
                event(
                    None,
                    root_agent(),
                    StorageEventPayload::SessionStart {
                        session_id: "session-1".into(),
                        timestamp: chrono::Utc::now(),
                        working_dir: "/tmp".into(),
                        parent_session_id: None,
                        parent_storage_seq: None,
                    },
                ),
            ),
            stored(
                2,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::UserMessage {
                        content: "root task".into(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                3,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::AssistantFinal {
                        content: "root answer".into(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: None,
                    },
                ),
            ),
            stored(
                4,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".into()),
                    },
                ),
            ),
            stored(
                5,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::UserMessage {
                        content: "child task".into(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                6,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::AssistantFinal {
                        content: "child answer".into(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        timestamp: None,
                    },
                ),
            ),
            stored(
                7,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".into()),
                    },
                ),
            ),
        ];

        for stored in &events {
            session
                .translate_store_and_cache(stored, &mut translator)
                .expect("event should translate into session cache");
        }

        let projected = session
            .snapshot_projected_state()
            .expect("snapshot should be available");

        assert_eq!(projected.turn_count, 1);
        assert_eq!(projected.messages.len(), 2);
        assert!(matches!(
            &projected.messages[0],
            astrcode_core::LlmMessage::User { content, .. } if content == "root task"
        ));
        assert!(matches!(
            &projected.messages[1],
            astrcode_core::LlmMessage::Assistant { content, .. } if content == "root answer"
        ));
    }

    #[test]
    fn translate_store_and_cache_rejects_invalid_stored_event() {
        let session = test_session_state();
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);
        let malformed = stored(
            1,
            event(
                Some("turn-child"),
                AgentEventContext {
                    agent_id: Some("agent-child".to_string().into()),
                    parent_turn_id: Some("turn-root".to_string().into()),
                    agent_profile: Some("explore".to_string()),
                    sub_run_id: Some("subrun-1".to_string().into()),
                    parent_sub_run_id: None,
                    invocation_kind: Some(InvocationKind::SubRun),
                    storage_mode: Some(SubRunStorageMode::IndependentSession),
                    child_session_id: None,
                },
                StorageEventPayload::UserMessage {
                    content: "child task".into(),
                    origin: UserMessageOrigin::User,
                    timestamp: chrono::Utc::now(),
                },
            ),
        );

        let error = session
            .translate_store_and_cache(&malformed, &mut translator)
            .expect_err("invalid stored event should be rejected");

        assert!(error.to_string().contains("child_session_id"));
    }

    #[test]
    fn turn_runtime_state_keeps_running_cache_and_active_turn_in_sync() {
        let session = test_session_state();
        let cancel = CancelToken::new();

        let generation = session
            .prepare_execution(
                "session-1",
                "turn-1",
                cancel.clone(),
                Box::new(StubTurnLease),
            )
            .expect("turn runtime should enter running state");

        assert!(session.is_running());
        assert_eq!(
            session
                .active_turn_id_snapshot()
                .expect("active turn should be readable")
                .as_deref(),
            Some("turn-1")
        );

        let cancelled_turn_id = session.cancel_active_turn().expect("cancel should succeed");
        assert_eq!(cancelled_turn_id.as_deref(), Some("turn-1"));
        assert!(cancel.is_cancelled(), "cancel token should be triggered");

        let pending_request = session
            .complete_execution_state(generation)
            .expect("turn runtime should complete successfully");
        assert_eq!(pending_request, None);

        assert!(!session.is_running());
        assert_eq!(
            session
                .active_turn_id_snapshot()
                .expect("active turn should be readable"),
            None
        );
        assert_eq!(
            session.current_phase().expect("phase should be readable"),
            Phase::Idle
        );
    }

    #[test]
    fn recovery_resets_turn_runtime_to_idle_without_active_turn() {
        let session = test_session_state();
        session
            .prepare_execution(
                "session-1",
                "turn-1",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("turn runtime should enter running state");
        session
            .request_manual_compact(super::PendingManualCompactRequest {
                runtime: astrcode_core::ResolvedRuntimeConfig::default(),
                instructions: Some("compact".to_string()),
            })
            .expect("manual compact should be queued");
        session.set_compacting(true);

        let checkpoint = session
            .recovery_checkpoint(7)
            .expect("checkpoint should build");
        let recovered = SessionState::from_recovery(
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
            &checkpoint,
            Vec::new(),
        )
        .expect("session should recover from checkpoint");

        assert!(!recovered.is_running());
        assert_eq!(
            recovered
                .active_turn_id_snapshot()
                .expect("active turn should be readable"),
            None
        );
        assert!(
            !recovered
                .manual_compact_pending()
                .expect("manual compact state should be readable")
        );
        assert!(!recovered.compacting());
    }

    #[test]
    fn stale_complete_generation_does_not_clear_resubmitted_turn() {
        let session = test_session_state();
        let generation_a = session
            .prepare_execution(
                "session-1",
                "turn-a",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("first turn should prepare");
        let interrupted = session
            .force_complete_execution_state()
            .expect("interrupt should clear active turn");
        assert_eq!(interrupted.turn_id.as_deref(), Some("turn-a"));

        let generation_b = session
            .prepare_execution(
                "session-1",
                "turn-b",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("second turn should prepare");

        assert_eq!(
            session
                .complete_execution_state(generation_a)
                .expect("stale finalize should not error"),
            None
        );
        assert!(
            session.is_running(),
            "stale finalize must not clear running cache"
        );
        assert_eq!(
            session
                .active_turn_id_snapshot()
                .expect("active turn should stay readable")
                .as_deref(),
            Some("turn-b")
        );
        assert_eq!(
            session.current_phase().expect("phase should stay thinking"),
            Phase::Idle
        );

        session
            .complete_execution_state(generation_b)
            .expect("current generation should complete");
        assert!(!session.is_running());
        assert_eq!(
            session
                .active_turn_id_snapshot()
                .expect("active turn should be cleared"),
            None
        );
    }

    #[test]
    fn interrupt_execution_if_running_is_noop_after_turn_already_completed() {
        let session = test_session_state();
        let generation = session
            .prepare_execution(
                "session-1",
                "turn-1",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("turn should prepare");

        session
            .complete_execution_state(generation)
            .expect("turn should complete");

        let interrupted = session
            .interrupt_execution_if_running()
            .expect("interrupt should not fail");

        assert_eq!(interrupted, None);
        assert!(!session.is_running());
        assert_eq!(
            session
                .current_phase()
                .expect("phase should remain readable"),
            Phase::Idle
        );
    }

    #[test]
    fn legacy_checkpoint_fields_migrate_into_projection_registry_snapshot() {
        let checkpoint_json = serde_json::json!({
            "agentState": {
                "session_id": "session-legacy",
                "working_dir": "/tmp",
                "messages": [],
                "phase": "idle",
                "mode_id": ModeId::default(),
                "turn_count": 0,
                "last_assistant_at": serde_json::Value::Null,
            },
            "phase": "idle",
            "lastModeChangedAt": "2026-04-21T00:00:00Z",
            "childNodes": {},
            "activeTasks": {
                "owner-a": {
                    "owner": "owner-a",
                    "items": [{
                        "content": "迁移旧 checkpoint",
                        "status": "in_progress",
                        "activeForm": "正在迁移旧 checkpoint"
                    }]
                }
            },
            "inputQueueProjectionIndex": {},
            "checkpointStorageSeq": 9
        });
        let checkpoint: SessionRecoveryCheckpoint =
            serde_json::from_value(checkpoint_json).expect("legacy checkpoint should deserialize");

        let projection_snapshot = checkpoint.projection_registry_snapshot();
        assert_eq!(
            projection_snapshot.last_mode_changed_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-21T00:00:00Z")
                    .expect("timestamp should parse")
                    .with_timezone(&Utc)
            )
        );
        assert!(projection_snapshot.active_tasks.contains_key("owner-a"));

        let recovered = SessionState::from_recovery(
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
            &checkpoint,
            Vec::new(),
        )
        .expect("legacy checkpoint should recover");

        let recovered_task = recovered
            .active_tasks_for("owner-a")
            .expect("task lookup should succeed")
            .expect("legacy task should survive migration");
        assert_eq!(
            recovered_task.items,
            vec![ExecutionTaskItem {
                content: "迁移旧 checkpoint".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在迁移旧 checkpoint".to_string()),
            }]
        );
        assert_eq!(
            recovered
                .last_mode_changed_at()
                .expect("mode timestamp should exist"),
            projection_snapshot.last_mode_changed_at
        );
    }
}
