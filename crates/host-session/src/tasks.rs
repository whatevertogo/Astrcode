use std::collections::HashMap;

use astrcode_core::{
    EXECUTION_TASK_SNAPSHOT_SCHEMA, ExecutionTaskSnapshotMetadata, StorageEventPayload,
    StoredEvent, TaskSnapshot,
};

pub(crate) fn rebuild_active_tasks(events: &[StoredEvent]) -> HashMap<String, TaskSnapshot> {
    let mut tasks = HashMap::new();
    for stored in events {
        if let Some(snapshot) = task_snapshot_from_stored_event(stored) {
            apply_snapshot_to_map(&mut tasks, snapshot);
        }
    }
    tasks
}

pub(crate) fn task_snapshot_from_stored_event(stored: &StoredEvent) -> Option<TaskSnapshot> {
    let StorageEventPayload::ToolResult {
        tool_name,
        metadata: Some(metadata),
        ..
    } = &stored.event.payload
    else {
        return None;
    };

    if tool_name != "taskWrite" {
        return None;
    }

    let parsed = serde_json::from_value::<ExecutionTaskSnapshotMetadata>(metadata.clone()).ok()?;
    if parsed.schema != EXECUTION_TASK_SNAPSHOT_SCHEMA {
        return None;
    }

    Some(parsed.into_snapshot())
}

pub(crate) fn apply_snapshot_to_map(
    tasks: &mut HashMap<String, TaskSnapshot>,
    snapshot: TaskSnapshot,
) {
    if snapshot.should_clear() {
        tasks.remove(snapshot.owner.as_str());
    } else {
        tasks.insert(snapshot.owner.clone(), snapshot);
    }
}
