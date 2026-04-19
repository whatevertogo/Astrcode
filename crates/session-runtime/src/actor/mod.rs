//! Session actor 与 live truth。
//!
//! 边界约束：
//! - 这里只负责推进所需的 live state 与 durable writer 桥接
//! - 不负责 observe 视图拼装
//! - 不负责外部订阅协议映射

use std::sync::Arc;

use astrcode_core::{
    AgentId, AgentStateProjector, EventStore, EventTranslator, Phase, RecoveredSessionState,
    SessionId, StorageEvent, StoredEvent, TurnId, normalize_recovered_phase, replay_records,
};
#[cfg(test)]
use astrcode_core::{EventLogWriter, StoreResult};

use crate::state::{SessionSnapshot, SessionState, SessionWriter};

/// 空操作 EventLogWriter，仅用于测试态 actor。
#[cfg(test)]
struct NopEventLogWriter;

#[cfg(test)]
impl EventLogWriter for NopEventLogWriter {
    fn append(&mut self, _event: &astrcode_core::StorageEvent) -> StoreResult<StoredEvent> {
        // 空操作 writer 不持久化，但返回一个虚拟序号以满足调用方契约
        Ok(StoredEvent {
            storage_seq: 0,
            event: _event.clone(),
        })
    }
}

/// 会话 actor 持有完整的会话真相，不直接持有 tool/llm/prompt/resource provider。
#[derive(Debug)]
pub struct SessionActor {
    state: Arc<SessionState>,
    session_id: SessionId,
    working_dir: String,
}

impl SessionActor {
    /// 创建一个带 durable writer 的 actor。
    #[cfg(test)]
    pub async fn new_persistent(
        session_id: SessionId,
        working_dir: impl Into<String>,
        root_agent_id: AgentId,
        event_store: Arc<dyn EventStore>,
    ) -> astrcode_core::Result<Self> {
        Self::new_persistent_with_lineage(
            session_id,
            working_dir,
            root_agent_id,
            event_store,
            None,
            None,
        )
        .await
    }

    /// 创建一个带 durable writer 的 actor，并写入 lineage 元数据。
    pub async fn new_persistent_with_lineage(
        session_id: SessionId,
        working_dir: impl Into<String>,
        _root_agent_id: AgentId,
        event_store: Arc<dyn EventStore>,
        parent_session_id: Option<String>,
        parent_storage_seq: Option<u64>,
    ) -> astrcode_core::Result<Self> {
        let working_dir = working_dir.into();
        let writer = Arc::new(SessionWriter::from_event_store(
            Arc::clone(&event_store),
            session_id.clone(),
        ));

        let session_start = StorageEvent {
            turn_id: None,
            agent: astrcode_core::AgentEventContext::default(),
            payload: astrcode_core::StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: chrono::Utc::now(),
                working_dir: working_dir.clone(),
                parent_session_id,
                parent_storage_seq,
            },
        };
        let stored = event_store.append(&session_id, &session_start).await?;
        let mut translator = EventTranslator::new(Phase::Idle);
        let recent_records = translator.translate(&stored);
        let mut projector = astrcode_core::AgentStateProjector::default();
        projector.apply(&stored.event);
        let state = SessionState::new(Phase::Idle, writer, projector, recent_records, vec![stored]);

        Ok(Self {
            state: Arc::new(state),
            session_id,
            working_dir,
        })
    }

    /// 从 durable 事件日志重建一个会话 actor。
    ///
    /// Why: `session-runtime` 需要在 application 不持有 shadow state 的前提下，
    /// 按需把任意 session 从持久化存储恢复成可执行的 live actor。
    pub fn from_replay(
        session_id: SessionId,
        working_dir: impl Into<String>,
        _root_agent_id: AgentId,
        event_store: Arc<dyn EventStore>,
        stored_events: Vec<StoredEvent>,
    ) -> astrcode_core::Result<Self> {
        let working_dir = working_dir.into();
        let writer = Arc::new(SessionWriter::from_event_store(
            event_store,
            session_id.clone(),
        ));
        let mut projector = AgentStateProjector::default();
        for stored in &stored_events {
            stored.event.validate().map_err(|error| {
                astrcode_core::AstrError::Validation(format!(
                    "session '{}' contains invalid stored event at storage_seq {}: {}",
                    session_id, stored.storage_seq, error
                ))
            })?;
            projector.apply(&stored.event);
        }
        let phase = normalize_recovered_phase(projector.snapshot().phase);
        let recent_records = replay_records(&stored_events, None);
        let state = SessionState::new(phase, writer, projector, recent_records, stored_events);

        Ok(Self {
            state: Arc::new(state),
            session_id,
            working_dir,
        })
    }

    pub fn from_recovery(
        session_id: SessionId,
        working_dir: impl Into<String>,
        root_agent_id: AgentId,
        event_store: Arc<dyn EventStore>,
        recovered: RecoveredSessionState,
    ) -> astrcode_core::Result<Self> {
        let RecoveredSessionState {
            checkpoint,
            tail_events,
        } = recovered;
        let working_dir = working_dir.into();
        let Some(checkpoint) = checkpoint else {
            return Self::from_replay(
                session_id,
                working_dir,
                root_agent_id,
                event_store,
                tail_events,
            );
        };
        let writer = Arc::new(SessionWriter::from_event_store(
            event_store,
            session_id.clone(),
        ));
        let state = SessionState::from_recovery(writer, &checkpoint, tail_events)?;

        Ok(Self {
            state: Arc::new(state),
            session_id,
            working_dir,
        })
    }

    /// 创建一个空闲状态的 actor（无事件历史、无持久化）。
    ///
    /// 实际生产中应使用带持久化 writer 的 `new()` 构造路径。
    #[cfg(test)]
    pub fn new_idle(
        session_id: SessionId,
        working_dir: impl Into<String>,
        _root_agent_id: AgentId,
    ) -> Self {
        let writer = Arc::new(SessionWriter::new(Box::new(NopEventLogWriter)));
        let state = SessionState::new(
            astrcode_core::Phase::Idle,
            writer,
            astrcode_core::AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        );
        Self {
            state: Arc::new(state),
            session_id,
            working_dir: working_dir.into(),
        }
    }

    /// 返回轻量快照用于 observe。
    pub fn snapshot(&self) -> SessionSnapshot {
        let turn_count = self
            .state
            .snapshot_projected_state()
            .map(|s| s.turn_count)
            .unwrap_or(0);
        let active_turn = self
            .state
            .active_turn_id
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .map(TurnId::from);
        SessionSnapshot {
            session_id: self.session_id.clone(),
            working_dir: self.working_dir.clone(),
            latest_turn_id: active_turn,
            turn_count,
        }
    }

    pub fn state(&self) -> &Arc<SessionState> {
        &self.state
    }

    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, EventStore, InvocationKind, Result, SessionMeta,
        SessionTurnAcquireResult, StorageEvent, StorageEventPayload, StoredEvent,
        SubRunStorageMode, UserMessageOrigin,
    };
    use async_trait::async_trait;

    use super::*;
    use crate::append_and_broadcast;

    #[derive(Debug, Default)]
    struct StubEventStore;

    struct StubTurnLease;

    impl astrcode_core::SessionTurnLease for StubTurnLease {}

    #[async_trait]
    impl EventStore for StubEventStore {
        async fn ensure_session(
            &self,
            _session_id: &SessionId,
            _working_dir: &std::path::Path,
        ) -> Result<()> {
            Ok(())
        }

        async fn append(
            &self,
            _session_id: &SessionId,
            event: &StorageEvent,
        ) -> Result<StoredEvent> {
            Ok(StoredEvent {
                storage_seq: 1,
                event: event.clone(),
            })
        }

        async fn replay(&self, _session_id: &SessionId) -> Result<Vec<StoredEvent>> {
            Ok(Vec::new())
        }

        async fn try_acquire_turn(
            &self,
            _session_id: &SessionId,
            _turn_id: &str,
        ) -> Result<SessionTurnAcquireResult> {
            Ok(SessionTurnAcquireResult::Acquired(Box::new(StubTurnLease)))
        }

        async fn list_sessions(&self) -> Result<Vec<SessionId>> {
            Ok(Vec::new())
        }

        async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
            Ok(Vec::new())
        }

        async fn delete_session(&self, _session_id: &SessionId) -> Result<()> {
            Ok(())
        }

        async fn delete_sessions_by_working_dir(
            &self,
            _working_dir: &str,
        ) -> Result<astrcode_core::DeleteProjectResult> {
            Ok(astrcode_core::DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn new_persistent_primes_projector_with_session_start_for_child_sessions() {
        let actor = SessionActor::new_persistent(
            SessionId::from("session-child".to_string()),
            "/tmp/project",
            AgentId::from("root-agent".to_string()),
            Arc::new(StubEventStore),
        )
        .await
        .expect("actor should be created");

        let child_agent = AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "explore",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some("session-child".to_string().into()),
        );
        let event = StorageEvent {
            turn_id: Some("turn-child".to_string()),
            agent: child_agent,
            payload: StorageEventPayload::UserMessage {
                content: "child task".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: chrono::Utc::now(),
            },
        };

        let mut translator = EventTranslator::new(Phase::Idle);
        append_and_broadcast(actor.state(), &event, &mut translator)
            .await
            .expect("child event should append");

        let projected = actor
            .state()
            .snapshot_projected_state()
            .expect("snapshot should work");
        assert!(matches!(
            projected.messages.as_slice(),
            [astrcode_core::LlmMessage::User { content, .. }] if content == "child task"
        ));
    }

    #[test]
    fn from_replay_rejects_invalid_stored_events() {
        let malformed = StoredEvent {
            storage_seq: 7,
            event: StorageEvent {
                turn_id: Some("turn-child".to_string()),
                agent: AgentEventContext {
                    agent_id: Some("agent-child".to_string().into()),
                    parent_turn_id: Some("turn-root".to_string().into()),
                    agent_profile: Some("explore".to_string()),
                    sub_run_id: Some("subrun-1".to_string().into()),
                    parent_sub_run_id: None,
                    invocation_kind: Some(InvocationKind::SubRun),
                    storage_mode: Some(SubRunStorageMode::IndependentSession),
                    child_session_id: None,
                },
                payload: StorageEventPayload::TurnDone {
                    timestamp: chrono::Utc::now(),
                    reason: Some("completed".to_string()),
                },
            },
        };

        let error = SessionActor::from_replay(
            SessionId::from("session-parent".to_string()),
            "/tmp/project",
            AgentId::from("root-agent".to_string()),
            Arc::new(StubEventStore),
            vec![malformed],
        )
        .expect_err("replay should reject malformed stored events");

        assert!(error.to_string().contains("storage_seq 7"));
        assert!(error.to_string().contains("child_session_id"));
    }
}
