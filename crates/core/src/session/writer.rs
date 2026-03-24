use std::sync::Mutex;

use crate::{EventLog, Result, StorageEvent, StoredEvent};

pub struct SessionWriter {
    inner: Mutex<EventLog>,
}

impl SessionWriter {
    pub fn new(log: EventLog) -> Self {
        Self {
            inner: Mutex::new(log),
        }
    }

    pub fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| crate::AstrError::LockPoisoned("session writer".to_string()))?;
        guard.append(event)
    }
}
