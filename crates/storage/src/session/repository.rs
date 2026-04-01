use astrcode_core::store::{EventLogWriter, SessionManager, StoreResult};
use astrcode_core::{DeleteProjectResult, SessionMeta, StoredEvent};

use super::event_log::EventLog;
use super::iterator::EventLogIterator;
use super::paths::resolve_existing_session_path;

/// 基于本地文件系统的会话仓储实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct FileSystemSessionRepository;

impl SessionManager for FileSystemSessionRepository {
    fn create_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>> {
        Ok(Box::new(EventLog::create(session_id)?))
    }

    fn open_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>> {
        Ok(Box::new(EventLog::open(session_id)?))
    }

    fn replay_events(
        &self,
        session_id: &str,
    ) -> StoreResult<Box<dyn Iterator<Item = StoreResult<StoredEvent>> + Send>> {
        let path = resolve_existing_session_path(session_id)?;
        Ok(Box::new(EventLogIterator::from_path(&path)?))
    }

    fn last_storage_seq(&self, session_id: &str) -> StoreResult<u64> {
        let path = resolve_existing_session_path(session_id)?;
        EventLog::last_storage_seq_from_path(&path)
    }

    fn list_sessions(&self) -> StoreResult<Vec<String>> {
        EventLog::list_sessions()
    }

    fn list_sessions_with_meta(&self) -> StoreResult<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta()
    }

    fn delete_session(&self, session_id: &str) -> StoreResult<()> {
        EventLog::delete_session(session_id)
    }

    fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> StoreResult<DeleteProjectResult> {
        EventLog::delete_sessions_by_working_dir(working_dir)
    }
}
