use crate::{event::EventLog, session::DeleteProjectResult, session::SessionMeta, Result};

pub trait SessionManager: Send + Sync {
    fn list_sessions(&self) -> Result<Vec<String>>;
    fn list_sessions_with_meta(&self) -> Result<Vec<SessionMeta>>;
    fn delete_session(&self, session_id: &str) -> Result<()>;
    fn delete_sessions_by_working_dir(&self, working_dir: &str) -> Result<DeleteProjectResult>;
    fn create_event_log(&self, session_id: &str) -> Result<EventLog>;
    fn open_event_log(&self, session_id: &str) -> Result<EventLog>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileSystemSessionRepository;

impl SessionManager for FileSystemSessionRepository {
    fn list_sessions(&self) -> Result<Vec<String>> {
        EventLog::list_sessions()
    }

    fn list_sessions_with_meta(&self) -> Result<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta()
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        EventLog::delete_session(session_id)
    }

    fn delete_sessions_by_working_dir(&self, working_dir: &str) -> Result<DeleteProjectResult> {
        EventLog::delete_sessions_by_working_dir(working_dir)
    }

    fn create_event_log(&self, session_id: &str) -> Result<EventLog> {
        EventLog::create(session_id)
    }

    fn open_event_log(&self, session_id: &str) -> Result<EventLog> {
        EventLog::open(session_id)
    }
}
