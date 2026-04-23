use astrcode_core::StorageEvent;

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnJournal {
    events: Vec<StorageEvent>,
}

impl TurnJournal {
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub(crate) fn events_mut(&mut self) -> &mut Vec<StorageEvent> {
        &mut self.events
    }

    pub(crate) fn push(&mut self, event: StorageEvent) {
        self.events.push(event);
    }

    pub(crate) fn extend<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = StorageEvent>,
    {
        self.events.extend(events);
    }

    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }

    pub(crate) fn take_events(&mut self) -> Vec<StorageEvent> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &StorageEvent> {
        self.events.iter()
    }
}
