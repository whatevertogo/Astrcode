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
pub trait EventLogWriter: Send + Sync {
    /// Appends one storage event and returns the fully assigned stored record.
    fn append(&mut self, event: &StorageEvent) -> StoreResult<StoredEvent>;
}

/// Session persistence contract used by runtime services.
pub trait SessionManager: Send + Sync {
    fn create_event_log(&self, session_id: &str) -> StoreResult<Box<dyn EventLogWriter>>;
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
