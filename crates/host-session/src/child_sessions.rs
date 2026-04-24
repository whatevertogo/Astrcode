use std::collections::HashMap;

use astrcode_core::{ChildSessionNode, StorageEventPayload, StoredEvent};

pub(crate) fn rebuild_child_nodes(events: &[StoredEvent]) -> HashMap<String, ChildSessionNode> {
    let mut nodes = HashMap::new();
    for stored in events {
        if let Some(node) = child_node_from_stored_event(stored) {
            nodes.insert(node.sub_run_id().to_string(), node);
        }
    }
    nodes
}

pub(crate) fn child_node_from_stored_event(stored: &StoredEvent) -> Option<ChildSessionNode> {
    match &stored.event.payload {
        StorageEventPayload::ChildSessionNotification { notification, .. } => {
            Some(notification.child_ref.to_child_session_node(
                stored.event.turn_id.clone().unwrap_or_default().into(),
                astrcode_core::ChildSessionStatusSource::Durable,
                notification.source_tool_call_id.clone(),
                None,
            ))
        },
        _ => None,
    }
}
