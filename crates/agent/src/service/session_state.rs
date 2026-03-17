use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use astrcode_core::{CancelToken, Phase};
use tokio::sync::broadcast;

use crate::event_log::EventLog;
use crate::events::{StorageEvent, StoredEvent};

use super::support::{lock_anyhow, spawn_blocking_anyhow};
use super::SessionEventRecord;

pub(super) struct SessionWriter {
    inner: StdMutex<EventLog>,
}

impl SessionWriter {
    pub(super) fn new(log: EventLog) -> Self {
        Self {
            inner: StdMutex::new(log),
        }
    }

    fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = lock_anyhow(&self.inner, "session writer")?;
        Ok(guard.append(event)?)
    }

    pub(super) async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        spawn_blocking_anyhow("append session event", move || self.append_blocking(&event)).await
    }
}

pub(super) struct SessionState {
    #[allow(dead_code)]
    pub(super) working_dir: PathBuf,
    pub(super) phase: StdMutex<Phase>,
    pub(super) running: AtomicBool,
    pub(super) cancel: StdMutex<CancelToken>,
    pub(super) broadcaster: broadcast::Sender<SessionEventRecord>,
    pub(super) writer: Arc<SessionWriter>,
}

impl SessionState {
    pub(super) fn new(working_dir: PathBuf, phase: Phase, writer: Arc<SessionWriter>) -> Self {
        let (broadcaster, _) = broadcast::channel(512);
        Self {
            working_dir,
            phase: StdMutex::new(phase),
            running: AtomicBool::new(false),
            cancel: StdMutex::new(CancelToken::new()),
            broadcaster,
            writer,
        }
    }
}
