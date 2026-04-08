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
