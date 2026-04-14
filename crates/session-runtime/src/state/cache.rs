use std::collections::VecDeque;

use astrcode_core::{SessionEventRecord, StoredEvent};

pub(crate) const SESSION_RECENT_RECORD_LIMIT: usize = 16_384;
pub(crate) const SESSION_RECENT_STORED_LIMIT: usize = 16_384;

#[derive(Default)]
pub(crate) struct RecentSessionEvents {
    records: VecDeque<SessionEventRecord>,
    truncated: bool,
}

#[derive(Default)]
pub(crate) struct RecentStoredEvents {
    events: VecDeque<StoredEvent>,
}

impl RecentStoredEvents {
    pub(crate) fn replace(&mut self, events: Vec<StoredEvent>) {
        self.events = VecDeque::from(events);
        while self.events.len() > SESSION_RECENT_STORED_LIMIT {
            self.events.pop_front();
        }
    }

    pub(crate) fn push(&mut self, stored: StoredEvent) {
        self.events.push_back(stored);
        while self.events.len() > SESSION_RECENT_STORED_LIMIT {
            self.events.pop_front();
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<StoredEvent> {
        self.events.iter().cloned().collect()
    }
}

impl RecentSessionEvents {
    pub(crate) fn replace(&mut self, records: Vec<SessionEventRecord>) {
        self.records = VecDeque::from(records);
        self.truncated = self.records.len() > SESSION_RECENT_RECORD_LIMIT;
        while self.records.len() > SESSION_RECENT_RECORD_LIMIT {
            self.records.pop_front();
        }
    }

    pub(crate) fn push_batch(&mut self, records: &[SessionEventRecord]) {
        for record in records {
            self.records.push_back(record.clone());
            while self.records.len() > SESSION_RECENT_RECORD_LIMIT {
                self.records.pop_front();
                self.truncated = true;
            }
        }
    }

    pub(crate) fn records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Option<Vec<SessionEventRecord>> {
        let Some(last_event_id) = last_event_id else {
            return (!self.truncated).then_some(self.records.iter().cloned().collect());
        };

        let last_seen = parse_event_id(last_event_id)?;
        let first_cached = self
            .records
            .front()
            .and_then(|record| parse_event_id(&record.event_id));
        if self.truncated && first_cached.is_some_and(|first_cached| last_seen < first_cached) {
            return None;
        }

        Some(
            self.records
                .iter()
                .filter_map(|record| {
                    parse_event_id(&record.event_id)
                        .filter(|event_id| *event_id > last_seen)
                        .map(|_| record.clone())
                })
                .collect(),
        )
    }
}

fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}
