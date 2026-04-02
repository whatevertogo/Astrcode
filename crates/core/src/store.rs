use std::path::Path;

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
    fn last_storage_seq(&self, session_id: &str) -> StoreResult<u64>;
    fn list_sessions(&self) -> StoreResult<Vec<String>>;
    fn list_sessions_with_meta(&self) -> StoreResult<Vec<SessionMeta>>;
    fn delete_session(&self, session_id: &str) -> StoreResult<()>;
    fn delete_sessions_by_working_dir(&self, working_dir: &str)
        -> StoreResult<DeleteProjectResult>;
}
