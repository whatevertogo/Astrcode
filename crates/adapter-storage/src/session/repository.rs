use std::path::{Path, PathBuf};

use astrcode_core::{
    DeleteProjectResult, Result, SessionId, SessionMeta, SessionTurnAcquireResult, StorageEvent,
    StoredEvent,
    ports::EventStore,
    store::{EventLogWriter, SessionManager, StoreResult},
};
use async_trait::async_trait;

use super::{
    event_log::EventLog,
    iterator::EventLogIterator,
    paths::resolve_existing_session_path,
    turn_lock::{try_acquire_session_turn, try_acquire_session_turn_in_projects_root},
};

/// 基于本地文件系统的会话仓储实现。
#[derive(Debug, Default, Clone)]
pub struct FileSystemSessionRepository {
    projects_root: Option<PathBuf>,
}

impl FileSystemSessionRepository {
    pub fn new() -> Self {
        Self {
            projects_root: None,
        }
    }

    /// 基于显式项目根目录构建仓储。
    ///
    /// server 测试需要每个 runtime 使用独立 sandbox，不能共享进程级
    /// `~/.astrcode/projects`。显式传入根目录后，整个 session 存储链路都会跟随隔离。
    pub fn new_with_projects_root(projects_root: PathBuf) -> Self {
        Self {
            projects_root: Some(projects_root),
        }
    }

    pub fn ensure_session_sync(&self, session_id: &str, working_dir: &Path) -> StoreResult<()> {
        match self.open_event_log_sync(session_id) {
            Ok(_) => Ok(()),
            Err(astrcode_core::StoreError::SessionNotFound(_)) => self
                .create_event_log_sync(session_id, working_dir)
                .map(|_| ()),
            Err(error) => Err(error),
        }
    }

    pub fn create_event_log_sync(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> StoreResult<EventLog> {
        match &self.projects_root {
            Some(projects_root) => {
                EventLog::create_in_projects_root(projects_root, session_id, working_dir)
            },
            None => EventLog::create(session_id, working_dir),
        }
    }

    pub fn open_event_log_sync(&self, session_id: &str) -> StoreResult<EventLog> {
        match &self.projects_root {
            Some(projects_root) => EventLog::open_in_projects_root(projects_root, session_id),
            None => EventLog::open(session_id),
        }
    }

    pub fn append_sync(&self, session_id: &str, event: &StorageEvent) -> StoreResult<StoredEvent> {
        let mut log = self.open_event_log_sync(session_id)?;
        log.append_stored(event)
    }

    pub fn replay_events_sync(&self, session_id: &str) -> StoreResult<Vec<StoredEvent>> {
        let path = match &self.projects_root {
            Some(projects_root) => super::paths::resolve_existing_session_path_from_projects_root(
                projects_root,
                session_id,
            )?,
            None => resolve_existing_session_path(session_id)?,
        };
        EventLogIterator::from_path(&path)?.collect()
    }

    pub fn try_acquire_turn_sync(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> StoreResult<SessionTurnAcquireResult> {
        match &self.projects_root {
            Some(projects_root) => {
                try_acquire_session_turn_in_projects_root(projects_root, session_id, turn_id)
            },
            None => try_acquire_session_turn(session_id, turn_id),
        }
    }

    pub fn list_sessions_sync(&self) -> StoreResult<Vec<String>> {
        match &self.projects_root {
            Some(projects_root) => EventLog::list_sessions_from_path(projects_root),
            None => EventLog::list_sessions(),
        }
    }

    pub fn list_session_metas_sync(&self) -> StoreResult<Vec<SessionMeta>> {
        match &self.projects_root {
            Some(projects_root) => EventLog::list_sessions_with_meta_from_path(projects_root),
            None => EventLog::list_sessions_with_meta(),
        }
    }

    pub fn delete_session_sync(&self, session_id: &str) -> StoreResult<()> {
        match &self.projects_root {
            Some(projects_root) => EventLog::delete_session_from_path(projects_root, session_id),
            None => EventLog::delete_session(session_id),
        }
    }

    pub fn delete_sessions_by_working_dir_sync(
        &self,
        working_dir: &str,
    ) -> StoreResult<DeleteProjectResult> {
        match &self.projects_root {
            Some(projects_root) => {
                EventLog::delete_sessions_by_working_dir_from_path(projects_root, working_dir)
            },
            None => EventLog::delete_sessions_by_working_dir(working_dir),
        }
    }

    pub fn last_storage_seq_sync(&self, session_id: &str) -> StoreResult<u64> {
        let path = match &self.projects_root {
            Some(projects_root) => super::paths::resolve_existing_session_path_from_projects_root(
                projects_root,
                session_id,
            )?,
            None => resolve_existing_session_path(session_id)?,
        };
        EventLog::last_storage_seq_from_path(&path)
    }
}

#[async_trait]
impl EventStore for FileSystemSessionRepository {
    async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()> {
        let repo = self.clone();
        let session_id = session_id.to_string();
        let working_dir = working_dir.to_path_buf();
        run_blocking("ensure storage session", move || {
            repo.ensure_session_sync(&session_id, &working_dir)
        })
        .await
    }

    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent> {
        let repo = self.clone();
        let session_id = session_id.to_string();
        let event = event.clone();
        run_blocking("append storage event", move || {
            repo.append_sync(&session_id, &event)
        })
        .await
    }

    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        let repo = self.clone();
        let session_id = session_id.to_string();
        run_blocking("replay storage events", move || {
            repo.replay_events_sync(&session_id)
        })
        .await
    }

    async fn try_acquire_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<SessionTurnAcquireResult> {
        let repo = self.clone();
        let session_id = session_id.to_string();
        let turn_id = turn_id.to_string();
        run_blocking("acquire session turn", move || {
            repo.try_acquire_turn_sync(&session_id, &turn_id)
        })
        .await
    }

    async fn list_sessions(&self) -> Result<Vec<SessionId>> {
        let repo = self.clone();
        run_blocking("list storage sessions", move || repo.list_sessions_sync())
            .await
            .map(|sessions| sessions.into_iter().map(SessionId::from).collect())
    }

    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>> {
        let repo = self.clone();
        run_blocking("list storage session metas", move || {
            repo.list_session_metas_sync()
        })
        .await
    }

    async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
        let repo = self.clone();
        let session_id = session_id.to_string();
        run_blocking("delete storage session", move || {
            repo.delete_session_sync(&session_id)
        })
        .await
    }

    async fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult> {
        let repo = self.clone();
        let working_dir = working_dir.to_string();
        run_blocking("delete storage project sessions", move || {
            repo.delete_sessions_by_working_dir_sync(&working_dir)
        })
        .await
    }
}

async fn run_blocking<T, F>(label: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> StoreResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| {
            astrcode_core::AstrError::Internal(format!(
                "storage blocking task '{label}' failed: {error}"
            ))
        })?
        .map_err(crate::map_store_error)
}

impl SessionManager for FileSystemSessionRepository {
    fn create_event_log(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> StoreResult<Box<dyn EventLogWriter>> {
        self.create_event_log_sync(session_id, working_dir)
            .map(|log| Box::new(log) as Box<dyn EventLogWriter>)
    }

    fn open_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>> {
        self.open_event_log_sync(session_id)
            .map(|log| Box::new(log) as Box<dyn EventLogWriter>)
    }

    fn replay_events(
        &self,
        session_id: &str,
    ) -> StoreResult<Box<dyn Iterator<Item = StoreResult<StoredEvent>> + Send>> {
        let path = match &self.projects_root {
            Some(projects_root) => super::paths::resolve_existing_session_path_from_projects_root(
                projects_root,
                session_id,
            )?,
            None => resolve_existing_session_path(session_id)?,
        };
        Ok(Box::new(EventLogIterator::from_path(&path)?))
    }

    fn try_acquire_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> StoreResult<SessionTurnAcquireResult> {
        self.try_acquire_turn_sync(session_id, turn_id)
    }

    fn last_storage_seq(&self, session_id: &str) -> StoreResult<u64> {
        self.last_storage_seq_sync(session_id)
    }

    fn list_sessions(&self) -> StoreResult<Vec<String>> {
        self.list_sessions_sync()
    }

    fn list_sessions_with_meta(&self) -> StoreResult<Vec<SessionMeta>> {
        self.list_session_metas_sync()
    }

    fn delete_session(&self, session_id: &str) -> StoreResult<()> {
        self.delete_session_sync(session_id)
    }

    fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> StoreResult<DeleteProjectResult> {
        self.delete_sessions_by_working_dir_sync(working_dir)
    }
}
