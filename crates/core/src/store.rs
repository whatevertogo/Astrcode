use std::path::Path;

use chrono::{DateTime, Utc};

use crate::{DeleteProjectResult, SessionMeta, StorageEvent, StoredEvent};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("IO error: {context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse error: {context}")]
    Parse {
        context: String,
        #[source]
        source: serde_json::Error,
    },
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

impl From<std::io::Error> for StoreError {
    fn from(source: std::io::Error) -> Self {
        Self::Io {
            context: String::new(),
            source,
        }
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(source: serde_json::Error) -> Self {
        Self::Parse {
            context: String::new(),
            source,
        }
    }
}

/// Streaming writer handle for append-only session event logs.
///
/// `Send + Sync` 约束是必须的：实现通过 `Arc<dyn EventLogWriter>` 在异步任务间
/// 共享，并传入 `spawn_blocking` 闭包中执行文件 I/O。缺少这些约束会导致
/// 编译错误，因为跨线程传递需要 `Send`，Arc 共享引用需要 `Sync`。
pub trait EventLogWriter: Send + Sync {
    /// Appends one storage event and returns the fully assigned stored record.
    fn append(&mut self, event: &StorageEvent) -> StoreResult<StoredEvent>;
}

/// 跨进程 session turn 执行租约。
///
/// 该 trait 故意不暴露行为方法，只依赖 RAII 语义：当租约对象被 drop 时，
/// 底层实现必须释放对应的跨进程 session 锁。这样 runtime 无需了解
/// 文件锁、命名锁等具体机制，只需要持有租约直到 turn 结束。
pub trait SessionTurnLease: Send + Sync {}

/// 另一个执行者已经持有该 session 的 turn 执行权。
///
/// `turn_id` 是 branch 逻辑的关键输入：后发请求需要从「最后一个稳定完成的 turn」
/// 分叉，所以必须知道当前正在进行的是哪个 turn，才能在复制历史时排除其事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTurnBusy {
    pub turn_id: String,
    pub owner_pid: u32,
    pub acquired_at: DateTime<Utc>,
}

pub enum SessionTurnAcquireResult {
    Acquired(Box<dyn SessionTurnLease>),
    Busy(SessionTurnBusy),
}

/// Session persistence contract used by runtime services.
///
/// 与 `EventLogWriter` 相同的 `Send + Sync` 约束理由：`Arc<dyn SessionManager>`
/// 在 `RuntimeService` 和 `spawn_blocking_service` 之间共享。
pub trait SessionManager: Send + Sync {
    /// `working_dir` 必须显式传入，因为存储层需要在首次落盘前就决定项目分桶目录。
    /// 如果只传 `session_id`，实现只能先写到错误位置再搬文件，这会引入竞态和部分失败。
    fn create_event_log(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> StoreResult<Box<dyn EventLogWriter>>;
    fn open_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>>;
    fn replay_events(
        &self,
        session_id: &str,
    ) -> StoreResult<Box<dyn Iterator<Item = StoreResult<StoredEvent>> + Send>>;
    /// 尝试获取某个 session 的 turn 执行权。
    ///
    /// 获取失败不算错误，而是返回 `Busy`，让调用方可以选择自动分叉新 session。
    fn try_acquire_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> StoreResult<SessionTurnAcquireResult>;
    fn last_storage_seq(&self, session_id: &str) -> StoreResult<u64>;
    fn list_sessions(&self) -> StoreResult<Vec<String>>;
    fn list_sessions_with_meta(&self) -> StoreResult<Vec<SessionMeta>>;
    fn delete_session(&self, session_id: &str) -> StoreResult<()>;
    fn delete_sessions_by_working_dir(&self, working_dir: &str)
        -> StoreResult<DeleteProjectResult>;
}
