use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex as StdMutex, atomic::AtomicBool},
};

use anyhow::Result;
use astrcode_core::{
    AgentState, AgentStateProjector, CancelToken, ChildSessionNode, EventLogWriter,
    EventTranslator, Phase, SessionEventRecord, SessionTurnLease, StorageEvent, StoredEvent,
    ToolEventSink,
};
use tokio::sync::broadcast;

use crate::{
    append_and_broadcast_from_turn_callback,
    support::{lock_anyhow, spawn_blocking_anyhow, with_lock_recovery},
};

const SESSION_BROADCAST_CAPACITY: usize = 2048;
const SESSION_RECENT_RECORD_LIMIT: usize = 4096;
const SESSION_RECENT_STORED_LIMIT: usize = 4096;

#[derive(Default)]
struct RecentSessionEvents {
    records: VecDeque<SessionEventRecord>,
    truncated: bool,
}

#[derive(Default)]
struct RecentStoredEvents {
    events: VecDeque<StoredEvent>,
}

impl RecentStoredEvents {
    fn replace(&mut self, events: Vec<StoredEvent>) {
        self.events = VecDeque::from(events);
        while self.events.len() > SESSION_RECENT_STORED_LIMIT {
            self.events.pop_front();
        }
    }

    fn push(&mut self, stored: StoredEvent) {
        self.events.push_back(stored);
        while self.events.len() > SESSION_RECENT_STORED_LIMIT {
            self.events.pop_front();
        }
    }

    fn snapshot(&self) -> Vec<StoredEvent> {
        self.events.iter().cloned().collect()
    }
}

impl RecentSessionEvents {
    fn replace(&mut self, records: Vec<SessionEventRecord>) {
        self.records = VecDeque::from(records);
        self.truncated = self.records.len() > SESSION_RECENT_RECORD_LIMIT;
        while self.records.len() > SESSION_RECENT_RECORD_LIMIT {
            self.records.pop_front();
        }
    }

    fn push_batch(&mut self, records: &[SessionEventRecord]) {
        for record in records {
            self.records.push_back(record.clone());
            while self.records.len() > SESSION_RECENT_RECORD_LIMIT {
                self.records.pop_front();
                self.truncated = true;
            }
        }
    }

    fn records_after(&self, last_event_id: Option<&str>) -> Option<Vec<SessionEventRecord>> {
        let Some(last_event_id) = last_event_id else {
            return (!self.truncated).then_some(self.records.iter().cloned().collect());
        };

        let last_seen = parse_event_id(last_event_id)?;
        let first_cached = self
            .records
            .front()
            .and_then(|record| parse_event_id(&record.event_id));
        if self.truncated && first_cached.is_some_and(|first_cached| last_seen < first_cached) {
            return None;
        }

        Some(
            self.records
                .iter()
                .filter_map(|record| {
                    parse_event_id(&record.event_id)
                        .filter(|event_id| *event_id > last_seen)
                        .map(|_| record.clone())
                })
                .collect(),
        )
    }
}

pub struct SessionWriter {
    inner: StdMutex<Box<dyn EventLogWriter>>,
}

impl SessionWriter {
    pub fn new(writer: Box<dyn EventLogWriter>) -> Self {
        Self {
            inner: StdMutex::new(writer),
        }
    }

    pub fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = lock_anyhow(&self.inner, "session writer")?;
        Ok(guard.append(event)?)
    }

    pub async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        spawn_blocking_anyhow("append session event", move || self.append_blocking(&event)).await
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionTokenBudgetState {
    pub total_budget: u64,
    pub used_tokens: u64,
    pub continuation_count: u8,
}

pub struct SessionState {
    pub phase: StdMutex<Phase>,
    pub running: AtomicBool,
    pub cancel: StdMutex<CancelToken>,
    pub active_turn_id: StdMutex<Option<String>>,
    pub turn_lease: StdMutex<Option<Box<dyn SessionTurnLease>>>,
    pub token_budget: StdMutex<Option<SessionTokenBudgetState>>,
    pub compact_failure_count: StdMutex<u32>,
    pub broadcaster: broadcast::Sender<SessionEventRecord>,
    pub writer: Arc<SessionWriter>,
    projector: StdMutex<AgentStateProjector>,
    recent_records: StdMutex<RecentSessionEvents>,
    recent_stored: StdMutex<RecentStoredEvents>,
    child_nodes: StdMutex<HashMap<String, ChildSessionNode>>,
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
        let mut cached_records = RecentSessionEvents::default();
        cached_records.replace(recent_records);
        let mut cached_stored = RecentStoredEvents::default();
        cached_stored.replace(recent_stored);
        Self {
            phase: StdMutex::new(phase),
            running: AtomicBool::new(false),
            cancel: StdMutex::new(CancelToken::new()),
            active_turn_id: StdMutex::new(None),
            turn_lease: StdMutex::new(None),
            token_budget: StdMutex::new(None),
            compact_failure_count: StdMutex::new(0),
            broadcaster,
            writer,
            projector: StdMutex::new(projector),
            recent_records: StdMutex::new(cached_records),
            recent_stored: StdMutex::new(cached_stored),
            child_nodes: StdMutex::new(HashMap::new()),
        }
    }

    pub fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(lock_anyhow(&self.projector, "session projector")?.snapshot())
    }

    pub fn current_phase(&self) -> Result<Phase> {
        Ok(*lock_anyhow(&self.phase, "session phase")?)
    }

    pub fn complete_execution_state(&self, phase: Phase) {
        with_lock_recovery(&self.phase, "session phase", |phase_guard| {
            *phase_guard = phase;
        });
        with_lock_recovery(
            &self.active_turn_id,
            "session active turn",
            |active_turn_guard| {
                *active_turn_guard = None;
            },
        );
        with_lock_recovery(&self.turn_lease, "session turn lease", |lease_guard| {
            *lease_guard = None;
        });
        with_lock_recovery(&self.token_budget, "session token budget", |budget_guard| {
            *budget_guard = None;
        });
        with_lock_recovery(&self.cancel, "session cancel", |cancel_guard| {
            *cancel_guard = CancelToken::new();
        });
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn translate_store_and_cache(
        &self,
        stored: &StoredEvent,
        translator: &mut EventTranslator,
    ) -> Result<Vec<SessionEventRecord>> {
        {
            let mut projector = lock_anyhow(&self.projector, "session projector")?;
            projector.apply(&stored.event);
        }
        let records = translator.translate(stored);
        lock_anyhow(&self.recent_records, "session recent records")?.push_batch(&records);
        lock_anyhow(&self.recent_stored, "session recent stored events")?.push(stored.clone());
        Ok(records)
    }

    pub fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(lock_anyhow(&self.recent_records, "session recent records")?
            .records_after(last_event_id))
    }

    pub fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(lock_anyhow(&self.recent_stored, "session recent stored events")?.snapshot())
    }

    /// 写入或覆盖一个 child-session durable 节点。
    ///
    /// 节点按 `sub_run_id` 去重，便于同一 child 在终态更新时保持稳定身份。
    pub fn upsert_child_session_node(&self, node: ChildSessionNode) -> Result<()> {
        lock_anyhow(&self.child_nodes, "session child nodes")?
            .insert(node.sub_run_id.clone(), node);
        Ok(())
    }

    /// 查询某个 sub-run 对应的 child-session 节点快照。
    pub fn child_session_node(&self, sub_run_id: &str) -> Result<Option<ChildSessionNode>> {
        Ok(lock_anyhow(&self.child_nodes, "session child nodes")?
            .get(sub_run_id)
            .cloned())
    }

    /// 列出当前 session 所有 child-session 节点快照。
    ///
    /// 返回按 sub_run_id 排序的节点列表，用于层级遍历和子树查询。
    pub fn list_child_session_nodes(&self) -> Result<Vec<ChildSessionNode>> {
        let nodes = lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result: Vec<_> = nodes.values().cloned().collect();
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
    }

    /// 查找某个 agent 的直接子节点。
    ///
    /// 遍历所有 child_session_node，返回 parent_agent_id 匹配的节点。
    pub fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        let nodes = lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result: Vec<_> = nodes
            .values()
            .filter(|node| node.parent_agent_id.as_deref() == Some(parent_agent_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
    }

    /// 收集指定 agent 子树的所有节点（含自身）。
    ///
    /// 从 root_agent_id 出发递归查找所有后代（不含自身），
    /// 用于级联关闭时确定影响范围。
    pub fn subtree_nodes(&self, root_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        let nodes = lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root_agent_id.to_string());
        while let Some(agent_id) = queue.pop_front() {
            for node in nodes.values() {
                if node.parent_agent_id.as_deref() == Some(&agent_id) {
                    queue.push_back(node.agent_id.clone());
                    result.push(node.clone());
                }
            }
        }
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
    }
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

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentEventContext, AgentStateProjector, EventLogWriter, InvocationKind, Phase,
        StorageEvent, StoreResult, StoredEvent, UserMessageOrigin,
    };
    use chrono::Utc;

    use super::{SessionState, SessionWriter};

    struct NoopEventLogWriter;

    impl EventLogWriter for NoopEventLogWriter {
        fn append(&mut self, _event: &StorageEvent) -> StoreResult<StoredEvent> {
            unreachable!("session_state tests do not persist through the writer")
        }
    }

    fn root_agent() -> AgentEventContext {
        AgentEventContext::default()
    }

    fn sub_run_agent() -> AgentEventContext {
        AgentEventContext {
            agent_id: Some("agent-child".to_string()),
            parent_turn_id: Some("turn-root".to_string()),
            agent_profile: Some("explore".to_string()),
            sub_run_id: Some("subrun-1".to_string()),
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(astrcode_core::SubRunStorageMode::SharedSession),
            child_session_id: None,
        }
    }

    fn stored(storage_seq: u64, event: StorageEvent) -> StoredEvent {
        StoredEvent { storage_seq, event }
    }

    #[test]
    fn translate_store_and_cache_keeps_sub_run_events_out_of_parent_snapshot() {
        let session = SessionState::new(
            Phase::Idle,
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
            AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        );
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);

        let events = vec![
            stored(
                1,
                StorageEvent::SessionStart {
                    session_id: "session-1".into(),
                    timestamp: Utc::now(),
                    working_dir: "/tmp".into(),
                    parent_session_id: None,
                    parent_storage_seq: None,
                },
            ),
            stored(
                2,
                StorageEvent::UserMessage {
                    turn_id: Some("turn-root".into()),
                    agent: root_agent(),
                    content: "root task".into(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
            stored(
                3,
                StorageEvent::AssistantFinal {
                    turn_id: Some("turn-root".into()),
                    agent: root_agent(),
                    content: "root answer".into(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: None,
                },
            ),
            stored(
                4,
                StorageEvent::TurnDone {
                    turn_id: Some("turn-root".into()),
                    agent: root_agent(),
                    timestamp: Utc::now(),
                    reason: Some("completed".into()),
                },
            ),
            stored(
                5,
                StorageEvent::UserMessage {
                    turn_id: Some("turn-child".into()),
                    agent: sub_run_agent(),
                    content: "child task".into(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            ),
            stored(
                6,
                StorageEvent::AssistantFinal {
                    turn_id: Some("turn-child".into()),
                    agent: sub_run_agent(),
                    content: "child answer".into(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: None,
                },
            ),
            stored(
                7,
                StorageEvent::TurnDone {
                    turn_id: Some("turn-child".into()),
                    agent: sub_run_agent(),
                    timestamp: Utc::now(),
                    reason: Some("completed".into()),
                },
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
}
