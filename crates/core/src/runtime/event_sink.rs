use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use crate::event_log::EventLog;
use crate::events::StorageEvent;

pub(crate) struct RuntimeEventSink<'a, F> {
    log: &'a mut EventLog,
    events_cache: &'a mut Vec<StorageEvent>,
    emit: &'a mut F,
    cancel_on_persist_failure: CancellationToken,
    append_error: Option<anyhow::Error>,
}

impl<'a, F> RuntimeEventSink<'a, F>
where
    F: FnMut(&StorageEvent),
{
    pub(crate) fn new(
        log: &'a mut EventLog,
        events_cache: &'a mut Vec<StorageEvent>,
        emit: &'a mut F,
        cancel_on_persist_failure: CancellationToken,
    ) -> Self {
        Self {
            log,
            events_cache,
            emit,
            cancel_on_persist_failure,
            append_error: None,
        }
    }

    pub(crate) fn record_user_event(&mut self, event: StorageEvent) -> Result<()> {
        self.log.append(&event)?;
        self.events_cache.push(event.clone());
        (self.emit)(&event);
        Ok(())
    }

    pub(crate) fn cached_events(&self) -> &[StorageEvent] {
        self.events_cache.as_slice()
    }

    pub(crate) fn handle_runtime_event(&mut self, event: StorageEvent) {
        if self.append_error.is_some() {
            return;
        }

        if !matches!(event, StorageEvent::AssistantDelta { .. }) {
            if let Err(err) = self.log.append(&event) {
                self.append_error = Some(anyhow!("failed to append runtime event: {err}"));
                self.cancel_on_persist_failure.cancel();
                (self.emit)(&StorageEvent::Error {
                    message: format!("persistence error: {err}"),
                });
                return;
            }
            self.events_cache.push(event.clone());
        }
        (self.emit)(&event);
    }

    pub(crate) fn finish(self) -> Result<()> {
        if let Some(err) = self.append_error {
            Err(err)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn runtime_event_sink_does_not_persist_assistant_deltas() {
        let tmp = tempdir().expect("tempdir should work");
        let path = tmp.path().join("session-test.jsonl");
        let mut log = EventLog::create_at_path("session-test", path).expect("log should build");
        let mut cache = Vec::new();
        let mut emitted = Vec::new();
        let cancel = CancellationToken::new();

        let mut capture = |event: &StorageEvent| emitted.push(event.clone());
        let mut sink = RuntimeEventSink::new(&mut log, &mut cache, &mut capture, cancel);
        sink.handle_runtime_event(StorageEvent::AssistantDelta {
            token: "hello".to_string(),
        });

        assert!(sink.cached_events().is_empty());
        assert_eq!(emitted.len(), 1);
        assert!(matches!(
            &emitted[0],
            StorageEvent::AssistantDelta { token } if token == "hello"
        ));
    }

    #[test]
    fn runtime_event_sink_persists_non_delta_events_before_finish() {
        let tmp = tempdir().expect("tempdir should work");
        let path = tmp.path().join("session-test.jsonl");
        let mut log =
            EventLog::create_at_path("session-test", path.clone()).expect("log should build");
        let mut cache = Vec::new();
        let mut emitted = Vec::new();
        let cancel = CancellationToken::new();

        let mut capture = |event: &StorageEvent| emitted.push(event.clone());
        let mut sink = RuntimeEventSink::new(&mut log, &mut cache, &mut capture, cancel);
        sink.record_user_event(StorageEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: chrono::Utc::now(),
        })
        .expect("user event should persist");
        sink.finish().expect("sink should finish cleanly");

        let persisted = EventLog::load_from_path(&path).expect("events should load");
        assert_eq!(persisted.len(), 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(emitted.len(), 1);
        assert!(matches!(
            &persisted[0],
            StorageEvent::UserMessage { content, .. } if content == "hello"
        ));
    }
}
