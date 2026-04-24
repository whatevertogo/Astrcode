use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

#[cfg(test)]
use astrcode_core::{EventLogWriter, support};
use astrcode_core::{Result, SessionId, StorageEvent, StoredEvent};

use crate::EventStore;

/// Async-safe session event writer owned by host-session.
///
/// Production appends go through the async `EventStore`. The sync
/// `EventLogWriter` bridge is retained only for tests and event replay
/// fixtures while the old runtime boundary is being deleted.
pub struct SessionWriter {
    inner: SessionWriterInner,
}

enum SessionWriterInner {
    EventStore {
        event_store: Arc<dyn EventStore>,
        session_id: SessionId,
    },
    #[cfg(test)]
    SyncWriter(StdMutex<Box<dyn EventLogWriter>>),
}

impl SessionWriter {
    #[cfg(test)]
    pub fn new(writer: Box<dyn EventLogWriter>) -> Self {
        Self {
            inner: SessionWriterInner::SyncWriter(StdMutex::new(writer)),
        }
    }

    pub fn from_event_store(event_store: Arc<dyn EventStore>, session_id: SessionId) -> Self {
        Self {
            inner: SessionWriterInner::EventStore {
                event_store,
                session_id,
            },
        }
    }

    #[cfg(test)]
    pub fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        event.validate()?;
        match &self.inner {
            SessionWriterInner::EventStore {
                event_store,
                session_id,
            } => block_on_event_store_append(
                Arc::clone(event_store),
                session_id.clone(),
                event.clone(),
            ),
            #[cfg(test)]
            SessionWriterInner::SyncWriter(inner) => {
                let mut guard = support::lock_anyhow(inner, "session writer")?;
                guard.append(event).map_err(|error| {
                    astrcode_core::AstrError::Internal(format!("session write failed: {error}"))
                })
            },
        }
    }

    pub async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        event.validate()?;
        match &self.inner {
            SessionWriterInner::EventStore {
                event_store,
                session_id,
            } => event_store.append(session_id, &event).await,
            #[cfg(test)]
            SessionWriterInner::SyncWriter(_) => {
                spawn_blocking_result("append session event", move || self.append_blocking(&event))
                    .await
            },
        }
    }
}

#[cfg(test)]
fn block_on_event_store_append(
    event_store: Arc<dyn EventStore>,
    session_id: SessionId,
    event: StorageEvent,
) -> Result<StoredEvent> {
    let run_append = move || -> Result<StoredEvent> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                astrcode_core::AstrError::Internal(format!(
                    "build temporary tokio runtime for session append failed: {error}"
                ))
            })?;
        runtime.block_on(event_store.append(&session_id, &event))
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(run_append).join().map_err(|_| {
            astrcode_core::AstrError::Internal("session append bridge thread panicked".to_string())
        })?
    } else {
        run_append()
    }
}

#[cfg(test)]
async fn spawn_blocking_result<T, F>(label: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        log::error!("blocking task '{label}' failed: {error}");
        astrcode_core::AstrError::Internal(format!("blocking task '{label}' failed: {error}"))
    })?
}
