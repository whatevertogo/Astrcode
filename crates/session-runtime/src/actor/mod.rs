use std::sync::Arc;

use astrcode_core::{AgentId, SessionId, StoreResult, StoredEvent, TurnId};

use crate::state::{SessionSnapshot, SessionState, SessionWriter};

/// 空操作 EventLogWriter，用于无持久化需求的 actor（如测试或空闲 session）。
struct NopEventLogWriter;

impl astrcode_core::EventLogWriter for NopEventLogWriter {
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
    root_agent_id: AgentId,
}

impl SessionActor {
    /// 创建 actor，包装一个已有的 live session state。
    pub fn new(
        session_id: SessionId,
        working_dir: impl Into<String>,
        root_agent_id: AgentId,
        state: Arc<SessionState>,
    ) -> Self {
        Self {
            state,
            session_id,
            working_dir: working_dir.into(),
            root_agent_id,
        }
    }

    /// 创建一个空闲状态的 actor（无事件历史、无持久化）。
    ///
    /// 实际生产中应使用带持久化 writer 的 `new()` 构造路径。
    pub fn new_idle(
        session_id: SessionId,
        working_dir: impl Into<String>,
        root_agent_id: AgentId,
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
            root_agent_id,
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

    /// 标记 turn 完成。
    pub fn mark_turn_completed(&self, _turn_id: TurnId) {
        // Turn 完成由 SessionState.complete_execution_state 驱动，
        // 此处仅作为外部标记入口保留。
    }

    pub fn root_agent_id(&self) -> &AgentId {
        &self.root_agent_id
    }

    pub fn state(&self) -> &Arc<SessionState> {
        &self.state
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }
}
