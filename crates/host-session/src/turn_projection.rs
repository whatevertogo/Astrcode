use astrcode_core::{StorageEventPayload, StoredEvent};

use crate::TurnProjectionSnapshot;

pub(crate) fn apply_turn_projection_event(
    projection: &mut TurnProjectionSnapshot,
    stored: &StoredEvent,
) {
    match &stored.event.payload {
        StorageEventPayload::TurnDone { terminal_kind, .. } => {
            projection.terminal_kind = terminal_kind.clone()
        },
        StorageEventPayload::Error { message, .. } => {
            let message = message.trim();
            if !message.is_empty() {
                projection.last_error = Some(message.to_string());
            }
        },
        _ => {},
    }
}

pub(crate) fn project_turn_projection(events: &[StoredEvent]) -> Option<TurnProjectionSnapshot> {
    if events.is_empty() {
        return None;
    }

    let mut projection = TurnProjectionSnapshot {
        terminal_kind: None,
        last_error: None,
    };
    for stored in events {
        apply_turn_projection_event(&mut projection, stored);
    }
    Some(projection)
}
