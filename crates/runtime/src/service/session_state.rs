//! # 会话状态管理 (Session State Management)
//!
//! 管理单个会话的运行时状态，包括：
//! - 事件广播（broadcast channel）供 SSE 客户端订阅
//! - 事件持久化（SessionWriter 包装 EventLogWriter）
//! - 内存缓存（RecentSessionEvents）支持快速 SSE 重连回放
//! - 投影状态（AgentStateProjector）提供会话快照
//! - 取消令牌和 Phase 状态管理
//!
//! ## 设计
//!
//! ### 事件广播与缓存策略
//!
//! - `SESSION_BROADCAST_CAPACITY` (2048): broadcast channel 容量，慢速 SSE 客户端若未 在 2048
//!   条事件内消费，旧事件会被丢弃。这是权衡值：足够覆盖一次完整 turn（通常 < 100 条事件），
//!   同时限制内存占用。
//! - `SESSION_RECENT_RECORD_LIMIT` (4096): 内存中保留的最近事件记录数。超过此限制时从头部
//!   淘汰旧记录（truncated = true）。约覆盖 40-50 次典型 turn，足以满足大多数 SSE resume 场景
//!   无需回磁盘。
//!
//! ### 恢复策略
//!
//! SSE 客户端丢失事件后的恢复策略是回退到磁盘回放（见 `replay.rs`）。
//! `RecentSessionEvents::records_after` 返回 `None` 时表示缓存不足以满足请求，
//! 调用方应回退到磁盘回放。
//!
//! ### 锁选择
//!
//! `SessionWriter` 使用 `std::sync::Mutex` 而非 `tokio::sync::Mutex`，因为：
//! writer 被 `spawn_blocking` 上下文和直接异步上下文交替调用，且临界区内只做纯文件 I/O，
//! 没有任何 await 点。std::sync::Mutex 在此场景下更轻量，避免 tokio Mutex 的额外开销。

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex as StdMutex, atomic::AtomicBool},
};

use anyhow::Result;
use astrcode_core::{
    AgentState, AgentStateProjector, CancelToken, EventLogWriter, EventTranslator, Phase,
    SessionEventRecord, SessionTurnLease, StorageEvent, StoredEvent,
};
use tokio::sync::broadcast;

use super::support::{lock_anyhow, spawn_blocking_anyhow};

/// broadcast channel 容量。慢速 SSE 客户端若未在 2048 条事件内消费，旧事件会被丢弃。
/// 2048 是权衡值：足够覆盖一次完整 turn（通常 < 100 条事件），同时限制内存占用。
/// SSE 客户端丢失事件后的恢复策略是回退到磁盘回放（见 replay.rs）。
const SESSION_BROADCAST_CAPACITY: usize = 2048;
/// 内存中保留的最近事件记录数。超过此限制时从头部淘汰旧记录（truncated = true）。
/// 4096 约覆盖 40-50 次典型 turn，足以满足大多数 SSE resume 场景无需回磁盘。
const SESSION_RECENT_RECORD_LIMIT: usize = 4096;
/// 最近存储事件缓存上限。
///
/// Compaction rebuild 只需要真实的“尾部事件”来恢复保留的最近 turn，因此这里沿用和
/// SSE 缓存相同的上限即可，避免为了 rebuild 再回磁盘 replay 全量历史。
const SESSION_RECENT_STORED_LIMIT: usize = 4096;

/// 最近会话事件缓存
///
/// 使用 `VecDeque` 维护固定大小的事件环形缓冲区，支持 SSE 客户端快速重连回放。
/// 当缓存被截断（truncated = true）时，表示头部事件已被淘汰，请求早期 cursor 的
/// 客户端必须回退到磁盘回放。
#[derive(Default)]
struct RecentSessionEvents {
    records: VecDeque<SessionEventRecord>,
    truncated: bool,
}

/// 最近存储事件缓存。
///
/// 这份缓存和前端用的 `RecentSessionEvents` 分开保存，因为 compaction rebuild 需要的是
/// `StoredEvent` 真相，而不是已经翻译成 `SessionEventRecord` 的展示记录。
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
    /// 替换全部记录，并检查是否超过限制需要截断。
    fn replace(&mut self, records: Vec<SessionEventRecord>) {
        self.records = VecDeque::from(records);
        self.truncated = self.records.len() > SESSION_RECENT_RECORD_LIMIT;
        while self.records.len() > SESSION_RECENT_RECORD_LIMIT {
            self.records.pop_front();
        }
    }

    /// 批量追加记录，超过限制时从头部淘汰。
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
        // 无 cursor 时：若缓存未被截断，返回全部记录；否则需要回退磁盘
        let Some(last_event_id) = last_event_id else {
            return (!self.truncated).then_some(self.records.iter().cloned().collect());
        };

        let last_seen = parse_event_id(last_event_id)?;
        let first_cached = self
            .records
            .front()
            .and_then(|record| parse_event_id(&record.event_id));
        // 安全不变量：若缓存曾被截断（头部事件被淘汰），则请求的 cursor 若早于
        // 缓存中最老的事件，就说明被请求的事件已不可恢复，必须回退到磁盘回放。
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

/// 会话事件写入器，包装 `EventLogWriter` trait object。
///
/// 使用 `std::sync::Mutex` 而非 `tokio::sync::Mutex` 的原因：
/// writer 被 `spawn_blocking` 上下文（`append_and_broadcast_blocking`）和
/// 直接异步上下文（`SessionWriter::append`）交替调用，且临界区内只做纯文件 I/O，
/// 没有任何 await 点。std::sync::Mutex 在此场景下更轻量，避免 tokio Mutex 的
/// 额外开销和对 `Send` 闭包的要求。
pub(super) struct SessionWriter {
    inner: StdMutex<Box<dyn EventLogWriter>>,
}

impl SessionWriter {
    pub(super) fn new(writer: Box<dyn EventLogWriter>) -> Self {
        Self {
            inner: StdMutex::new(writer),
        }
    }

    /// 在阻塞上下文中追加事件到 JSONL 文件。
    pub(super) fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = lock_anyhow(&self.inner, "session writer")?;
        Ok(guard.append(event)?)
    }

    /// 异步追加事件，内部桥接到阻塞线程池执行。
    pub(super) async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        spawn_blocking_anyhow("append session event", move || self.append_blocking(&event)).await
    }
}

/// 会话 Token 预算状态
///
/// 用于自动 continue 机制的会话级 token 消耗跟踪。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SessionTokenBudgetState {
    pub total_budget: u64,
    pub used_tokens: u64,
    pub continuation_count: u8,
}

/// 会话运行时状态
///
/// 包含单个会话的完整运行时上下文：
/// - `phase`: 当前会话阶段（Idle, Running, Compacting 等）
/// - `running`: 是否有 turn 正在执行
/// - `cancel`: 取消令牌，用于中断正在运行的 turn
/// - `turn_lease`: 当前 turn 的租约（防止并发 turn）
/// - `token_budget`: 会话级 token 预算跟踪
/// - `compact_failure_count`: 压缩失败计数器（运行时本地，重启后重置）
/// - `broadcaster`: SSE 事件广播发送端
/// - `writer`: 事件持久化写入器
/// - `projector`: 状态投影器，将 StorageEvent 转换为 AgentState
/// - `recent_records`: 内存中的最近事件缓存
pub(super) struct SessionState {
    pub(super) phase: StdMutex<Phase>,
    pub(super) running: AtomicBool,
    pub(super) cancel: StdMutex<CancelToken>,
    pub(super) turn_lease: StdMutex<Option<Box<dyn SessionTurnLease>>>,
    /// Session-scoped token budget bookkeeping for auto-continue.
    pub(super) token_budget: StdMutex<Option<SessionTokenBudgetState>>,
    /// Per-session compact circuit breaker. We keep it in memory because the failure mode is
    /// runtime-local: once the process restarts, a fresh provider/config combination may recover.
    pub(super) compact_failure_count: StdMutex<u32>,
    pub(super) broadcaster: broadcast::Sender<SessionEventRecord>,
    pub(super) writer: Arc<SessionWriter>,
    projector: StdMutex<AgentStateProjector>,
    recent_records: StdMutex<RecentSessionEvents>,
    recent_stored: StdMutex<RecentStoredEvents>,
}

impl SessionState {
    /// 创建新的会话状态实例。
    ///
    /// 初始化 broadcast channel 和事件缓存，其他状态设为默认值。
    pub(super) fn new(
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
            turn_lease: StdMutex::new(None),
            token_budget: StdMutex::new(None),
            compact_failure_count: StdMutex::new(0),
            broadcaster,
            writer,
            projector: StdMutex::new(projector),
            recent_records: StdMutex::new(cached_records),
            recent_stored: StdMutex::new(cached_stored),
        }
    }

    /// 获取当前投影的会话状态快照。
    pub(super) fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(lock_anyhow(&self.projector, "session projector")?.snapshot())
    }

    /// 读取当前运行时 phase。
    ///
    /// 历史事件回放只能重建“最后一个已持久化 phase 事件”，但活跃会话在内存中的
    /// phase 才是前端初始化时应该信任的当前真相，因此这里暴露一个轻量读取接口。
    pub(super) fn current_phase(&self) -> Result<Phase> {
        Ok(*lock_anyhow(&self.phase, "session phase")?)
    }

    /// 三步原子操作：应用投影 → 翻译事件 → 推入内存缓存。
    ///
    /// 执行顺序（projector → translate → cache）是有意设计的：
    /// projector 必须先于 cache 更新，确保后续的 `snapshot_projected_state()`
    /// 调用看到的投影状态至少和缓存中的记录一致。两个锁分别获取而非一次性
    /// 持有，因为 projector 更新和 cache 更新之间不存在竞争——它们只被
    /// 同一个 turn 的顺序事件流调用。
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
        lock_anyhow(&self.recent_stored, "session recent stored events")?.push(stored.clone());
        Ok(records)
    }

    /// 从内存缓存中获取指定 cursor 之后的事件。
    ///
    /// 返回 `None` 表示缓存不足，调用方应回退到磁盘回放。
    pub(super) fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(lock_anyhow(&self.recent_records, "session recent records")?
            .records_after(last_event_id))
    }

    /// 返回最近的真实存储事件尾部快照。
    ///
    /// Compaction rebuild 只需要保留的尾部事件，不需要为了这一点重放整份 JSONL。
    pub(super) fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(lock_anyhow(&self.recent_stored, "session recent stored events")?.snapshot())
    }
}

/// 解析 SSE 事件 ID（格式：`{storage_seq}.{subindex}`）。
///
/// 宽容解析：格式非法时返回 `None` 而非错误。调用方（`records_after`）
/// 收到 `None` 后会触发全量磁盘回放作为兜底，确保 SSE 客户端不会因
/// 发送了畸形的 `Last-Event-ID` 头而丢失数据。
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

    /// 验证缓存命中时返回增量尾部事件。
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

    /// 验证 cursor 超出缓存范围时强制磁盘回放。
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
