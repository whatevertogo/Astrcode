use astrcode_core::StorageEvent;

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnJournal {
    events: Vec<StorageEvent>,
}

impl TurnJournal {
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

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &StorageEvent> {
        self.events.iter()
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Vec<StorageEvent> {
        self.events.clone()
    }

    pub(crate) fn into_events(self) -> Vec<StorageEvent> {
        self.events
    }
}
