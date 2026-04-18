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
#[cfg(test)]
mod test_support;
mod writer;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, atomic::AtomicBool},
};

use astrcode_core::{
    AgentEvent, AgentState, AgentStateProjector, CancelToken, ChildSessionNode, EventTranslator,
    InputQueueProjection, Phase, ResolvedRuntimeConfig, Result, SessionEventRecord,
    SessionTurnLease, StoredEvent,
    support::{self},
};
use cache::{RecentSessionEvents, RecentStoredEvents};
use child_sessions::{child_node_from_stored_event, rebuild_child_nodes};
pub(crate) use execution::SessionStateEventSink;
pub use execution::{append_and_broadcast, complete_session_execution, prepare_session_execution};
pub(crate) use input_queue::{InputQueueEventAppend, append_input_queue_event};
pub use paths::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};
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
pub struct SessionState {
    pub phase: StdMutex<Phase>,
    pub running: AtomicBool,
    pub compacting: AtomicBool,
    pub cancel: StdMutex<CancelToken>,
    pub active_turn_id: StdMutex<Option<String>>,
    pub turn_lease: StdMutex<Option<Box<dyn SessionTurnLease>>>,
    pub pending_manual_compact: StdMutex<bool>,
    pub pending_manual_compact_request: StdMutex<Option<PendingManualCompactRequest>>,
    pub compact_failure_count: StdMutex<u32>,
    pub broadcaster: broadcast::Sender<SessionEventRecord>,
    live_broadcaster: broadcast::Sender<AgentEvent>,
    pub writer: Arc<SessionWriter>,
    projector: StdMutex<AgentStateProjector>,
    recent_records: StdMutex<RecentSessionEvents>,
    recent_stored: StdMutex<RecentStoredEvents>,
    child_nodes: StdMutex<HashMap<String, ChildSessionNode>>,
    input_queue_projection_index: StdMutex<HashMap<String, InputQueueProjection>>,
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("running", &self.running)
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
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);
        let mut cached_records = RecentSessionEvents::default();
        cached_records.replace(recent_records);
        let mut cached_stored = RecentStoredEvents::default();
        cached_stored.replace(recent_stored.clone());
        let child_nodes = rebuild_child_nodes(&recent_stored);
        let input_queue_projection_index = InputQueueProjection::replay_index(&recent_stored);
        Self {
            phase: StdMutex::new(phase),
            running: AtomicBool::new(false),
            compacting: AtomicBool::new(false),
            cancel: StdMutex::new(CancelToken::new()),
            active_turn_id: StdMutex::new(None),
            turn_lease: StdMutex::new(None),
            pending_manual_compact: StdMutex::new(false),
            pending_manual_compact_request: StdMutex::new(None),
            compact_failure_count: StdMutex::new(0),
            broadcaster,
            live_broadcaster,
            writer,
            projector: StdMutex::new(projector),
            recent_records: StdMutex::new(cached_records),
            recent_stored: StdMutex::new(cached_stored),
            child_nodes: StdMutex::new(child_nodes),
            input_queue_projection_index: StdMutex::new(input_queue_projection_index),
        }
    }

    pub fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(support::lock_anyhow(&self.projector, "session projector")?.snapshot())
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
        Ok(*support::lock_anyhow(&self.phase, "session phase")?)
    }

    pub fn active_turn_id_snapshot(&self) -> Result<Option<String>> {
        Ok(support::lock_anyhow(&self.active_turn_id, "session active turn")?.clone())
    }

    pub fn manual_compact_pending(&self) -> Result<bool> {
        Ok(*support::lock_anyhow(
            &self.pending_manual_compact,
            "session pending manual compact",
        )?)
    }

    pub fn complete_execution_state(&self, phase: Phase) {
        // Why: 先清除 running 标志再设置 phase，避免外部观察者看到 phase=Idle
        // 但 running 仍为 true 的竞态窗口（如 compact 在 turn 完成后立即被调用）。
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        support::with_lock_recovery(&self.phase, "session phase", |phase_guard| {
            *phase_guard = phase;
        });
        support::with_lock_recovery(
            &self.active_turn_id,
            "session active turn",
            |active_turn_guard| {
                *active_turn_guard = None;
            },
        );
        support::with_lock_recovery(&self.turn_lease, "session turn lease", |lease_guard| {
            *lease_guard = None;
        });
        support::with_lock_recovery(&self.cancel, "session cancel", |cancel_guard| {
            *cancel_guard = CancelToken::new();
        });
    }

    pub fn compacting(&self) -> bool {
        self.compacting.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_compacting(&self, compacting: bool) {
        self.compacting
            .store(compacting, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn request_manual_compact(&self, request: PendingManualCompactRequest) -> Result<bool> {
        let mut guard = support::lock_anyhow(
            &self.pending_manual_compact,
            "session pending manual compact",
        )?;
        let mut request_guard = support::lock_anyhow(
            &self.pending_manual_compact_request,
            "session pending manual compact request",
        )?;
        let already_pending = *guard;
        *guard = true;
        *request_guard = Some(request);
        Ok(!already_pending)
    }

    pub fn take_pending_manual_compact(&self) -> Result<Option<PendingManualCompactRequest>> {
        let mut guard = support::lock_anyhow(
            &self.pending_manual_compact,
            "session pending manual compact",
        )?;
        let mut request_guard = support::lock_anyhow(
            &self.pending_manual_compact_request,
            "session pending manual compact request",
        )?;
        let pending = if *guard { request_guard.take() } else { None };
        *guard = false;
        Ok(pending)
    }

    pub fn translate_store_and_cache(
        &self,
        stored: &StoredEvent,
        translator: &mut EventTranslator,
    ) -> Result<Vec<SessionEventRecord>> {
        stored.event.validate()?;
        {
            let mut projector = support::lock_anyhow(&self.projector, "session projector")?;
            projector.apply(&stored.event);
        }
        let records = translator.translate(stored);
        support::lock_anyhow(&self.recent_records, "session recent records")?.push_batch(&records);
        support::lock_anyhow(&self.recent_stored, "session recent stored events")?
            .push(stored.clone());
        if let Some(node) = child_node_from_stored_event(stored) {
            self.upsert_child_session_node(node)?;
        }
        self.apply_input_queue_event(stored);
        Ok(records)
    }

    pub fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(
            support::lock_anyhow(&self.recent_records, "session recent records")?
                .records_after(last_event_id),
        )
    }

    pub fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(support::lock_anyhow(&self.recent_stored, "session recent stored events")?.snapshot())
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, InvocationKind, Phase, StorageEventPayload, SubRunStorageMode,
        UserMessageOrigin,
    };

    use super::test_support::{
        event, independent_session_sub_run_agent, root_agent, stored, test_session_state,
    };

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
}
