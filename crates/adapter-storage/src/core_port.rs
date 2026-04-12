//! 桥接 `adapter-storage` 的 `SessionManager` 与 `core::ports::EventStore`。
//!
//! `core::ports::EventStore` 是 session-runtime 消费的简化端口接口，
//! 本模块将其适配到 `FileSystemSessionRepository` 的完整文件系统实现上。

use astrcode_core::{
    DeleteProjectResult, Result, SessionId, SessionMeta, StorageEvent, StoredEvent,
    ports::EventStore, store::SessionManager,
};
use async_trait::async_trait;

use crate::session::FileSystemSessionRepository;

/// 基于 `FileSystemSessionRepository` 的 `EventStore` 实现。
///
/// 将 `EventStore` 的简化接口（append/replay/list/delete）
/// 适配到 `SessionManager` 的文件系统操作。
pub struct FsEventStore {
    repo: FileSystemSessionRepository,
}

impl FsEventStore {
    pub fn new() -> Self {
        Self {
            repo: FileSystemSessionRepository,
        }
    }
}

impl Default for FsEventStore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FsEventStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsEventStore").finish()
    }
}

#[async_trait]
impl EventStore for FsEventStore {
    async fn ensure_session(
        &self,
        session_id: &SessionId,
        working_dir: &std::path::Path,
    ) -> Result<()> {
        if self.repo.open_event_log(session_id.as_str()).is_ok() {
            return Ok(());
        }

        self.repo
            .create_event_log(session_id.as_str(), working_dir)
            .map(|_| ())
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }

    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent> {
        // 先尝试打开已有日志，如果不存在则创建（需要 working_dir，这里用空路径作为 fallback）
        let mut writer = self
            .repo
            .open_event_log(session_id.as_str())
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;

        writer
            .append(event)
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }

    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        let iter = self
            .repo
            .replay_events(session_id.as_str())
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;

        let mut events = Vec::new();
        for item in iter {
            let event = item.map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;
            events.push(event);
        }
        Ok(events)
    }

    async fn list_sessions(&self) -> Result<Vec<SessionId>> {
        self.repo
            .list_sessions()
            .map(|ids| ids.into_iter().map(SessionId::from).collect())
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }

    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        self.repo
            .list_sessions_with_meta()
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }

    async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
        self.repo
            .delete_session(session_id.as_str())
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }

    async fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult> {
        self.repo
            .delete_sessions_by_working_dir(working_dir)
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))
    }
}
