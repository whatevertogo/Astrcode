use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use astrcode_core::{
    AgentState, AgentStateProjector, CancelToken, EventLogWriter, EventTranslator, Phase,
    SessionEventRecord,
};
use tokio::sync::broadcast;

use astrcode_core::{StorageEvent, StoredEvent};

use super::support::{lock_anyhow, spawn_blocking_anyhow};

const SESSION_BROADCAST_CAPACITY: usize = 2048;
const SESSION_RECENT_RECORD_LIMIT: usize = 4096;

#[derive(Default)]
struct RecentSessionEvents {
    records: VecDeque<SessionEventRecord>,
    truncated: bool,
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

    /// 从内存缓存中返回 `last_event_id` 之后的事件。
    ///
    /// 返回 `None` 表示缓存不足以满足请求，调用方应回退到磁盘回放。
    /// 具体来说：
    /// - `last_event_id == None` 且缓存曾被截断 → 完整历史不在缓存中，必须走磁盘
    /// - `last_event_id` 对应的事件早于缓存中最老的事件 → 被截断的部分无法提供
    fn records_after(&self, last_event_id: Option<&str>) -> Option<Vec<SessionEventRecord>> {
        let snapshot = self.records.iter().cloned().collect::<Vec<_>>();
        let Some(last_event_id) = last_event_id else {
            return (!self.truncated).then_some(snapshot);
        };

        let last_seen = parse_event_id(last_event_id)?;
        let first_cached = snapshot
            .first()
            .and_then(|record| parse_event_id(&record.event_id));
        if self.truncated && first_cached.is_some_and(|first_cached| last_seen < first_cached) {
            return None;
        }

        Some(
            snapshot
                .into_iter()
                .filter(|record| {
                    parse_event_id(&record.event_id)
                        .map(|event_id| event_id > last_seen)
                        .unwrap_or(false)
                })
                .collect(),
        )
    }
}

pub(super) struct SessionWriter {
    inner: StdMutex<Box<dyn EventLogWriter>>,
}

impl SessionWriter {
    pub(super) fn new(writer: Box<dyn EventLogWriter>) -> Self {
        Self {
            inner: StdMutex::new(writer),
        }
    }

    pub(super) fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = lock_anyhow(&self.inner, "session writer")?;
        Ok(guard.append(event)?)
    }

    pub(super) async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        spawn_blocking_anyhow("append session event", move || self.append_blocking(&event)).await
    }
}

pub(super) struct SessionState {
    // 保留以备将来 session 级工作目录切换时使用；当前所有工具通过 ToolContext.working_dir 获取路径
    #[allow(dead_code)]
    pub(super) working_dir: PathBuf,
    pub(super) phase: StdMutex<Phase>,
    pub(super) running: AtomicBool,
    pub(super) cancel: StdMutex<CancelToken>,
    pub(super) broadcaster: broadcast::Sender<SessionEventRecord>,
    pub(super) writer: Arc<SessionWriter>,
    projector: StdMutex<AgentStateProjector>,
    recent_records: StdMutex<RecentSessionEvents>,
}

impl SessionState {
    pub(super) fn new(
        working_dir: PathBuf,
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
    ) -> Self {
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let mut cached_records = RecentSessionEvents::default();
        cached_records.replace(recent_records);
        Self {
            working_dir,
            phase: StdMutex::new(phase),
            running: AtomicBool::new(false),
            cancel: StdMutex::new(CancelToken::new()),
            broadcaster,
            writer,
            projector: StdMutex::new(projector),
            recent_records: StdMutex::new(cached_records),
        }
    }

    pub(super) fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(lock_anyhow(&self.projector, "session projector")?.snapshot())
    }

    pub(super) fn translate_store_and_cache(
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
        Ok(records)
    }

    pub(super) fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(lock_anyhow(&self.recent_records, "session recent records")?
            .records_after(last_event_id))
    }
}

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEvent, SessionEventRecord};

    use super::{RecentSessionEvents, SESSION_RECENT_RECORD_LIMIT};

    fn record(event_id: &str) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event: AgentEvent::SessionStarted {
                session_id: "session-1".to_string(),
            },
        }
    }

    #[test]
    fn recent_records_after_returns_incremental_tail_when_cursor_is_cached() {
        let mut recent = RecentSessionEvents::default();
        recent.replace(vec![record("1.0"), record("2.0"), record("3.0")]);

        let tail = recent
            .records_after(Some("1.0"))
            .expect("cached cursor should be replayable");

        assert_eq!(
            tail.into_iter()
                .map(|record| record.event_id)
                .collect::<Vec<_>>(),
            vec!["2.0".to_string(), "3.0".to_string()]
        );
    }

    #[test]
    fn recent_records_after_requires_disk_fallback_when_cursor_fell_out_of_cache() {
        let mut recent = RecentSessionEvents::default();
        recent.replace(
            (1..=(SESSION_RECENT_RECORD_LIMIT + 1))
                .map(|seq| record(&format!("{seq}.0")))
                .collect(),
        );

        assert!(
            recent.records_after(Some("1.0")).is_none(),
            "truncated in-memory history must force durable replay for stale cursors"
        );
    }
}
