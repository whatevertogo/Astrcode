use std::path::{Path, PathBuf};

use astrcode_core::{
    DeleteProjectResult, SessionMeta, StoredEvent,
    store::{EventLogWriter, SessionManager, SessionTurnAcquireResult, StoreResult},
};

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
}

impl SessionManager for FileSystemSessionRepository {
    fn create_event_log(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> StoreResult<Box<dyn EventLogWriter>> {
        let log = match &self.projects_root {
            Some(projects_root) => {
                EventLog::create_in_projects_root(projects_root, session_id, working_dir)?
            },
            None => EventLog::create(session_id, working_dir)?,
        };
        Ok(Box::new(log))
    }

    fn open_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>> {
        let log = match &self.projects_root {
            Some(projects_root) => EventLog::open_in_projects_root(projects_root, session_id)?,
            None => EventLog::open(session_id)?,
        };
        Ok(Box::new(log))
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
        match &self.projects_root {
            Some(projects_root) => {
                try_acquire_session_turn_in_projects_root(projects_root, session_id, turn_id)
            },
            None => try_acquire_session_turn(session_id, turn_id),
        }
    }

    fn last_storage_seq(&self, session_id: &str) -> StoreResult<u64> {
        let path = match &self.projects_root {
            Some(projects_root) => super::paths::resolve_existing_session_path_from_projects_root(
                projects_root,
                session_id,
            )?,
            None => resolve_existing_session_path(session_id)?,
        };
        EventLog::last_storage_seq_from_path(&path)
    }

    fn list_sessions(&self) -> StoreResult<Vec<String>> {
        match &self.projects_root {
            Some(projects_root) => EventLog::list_sessions_from_path(projects_root),
            None => EventLog::list_sessions(),
        }
    }

    fn list_sessions_with_meta(&self) -> StoreResult<Vec<SessionMeta>> {
        match &self.projects_root {
            Some(projects_root) => EventLog::list_sessions_with_meta_from_path(projects_root),
            None => EventLog::list_sessions_with_meta(),
        }
    }

    fn delete_session(&self, session_id: &str) -> StoreResult<()> {
        match &self.projects_root {
            Some(projects_root) => EventLog::delete_session_from_path(projects_root, session_id),
            None => EventLog::delete_session(session_id),
        }
    }

    fn delete_sessions_by_working_dir(
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
}
