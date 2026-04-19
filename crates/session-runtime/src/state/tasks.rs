use std::collections::HashMap;

use astrcode_core::{
    EXECUTION_TASK_SNAPSHOT_SCHEMA, ExecutionTaskSnapshotMetadata, Result, StorageEventPayload,
    StoredEvent, TaskSnapshot, support,
};

use super::SessionState;

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

impl SessionState {
    pub(crate) fn apply_task_snapshot_event(&self, stored: &StoredEvent) -> Result<()> {
        let Some(snapshot) = task_snapshot_from_stored_event(stored) else {
            return Ok(());
        };
        self.replace_active_task_snapshot(snapshot)
    }

    pub(crate) fn replace_active_task_snapshot(&self, snapshot: TaskSnapshot) -> Result<()> {
        let mut tasks = support::lock_anyhow(&self.active_tasks, "session active tasks")?;
        apply_snapshot_to_map(&mut tasks, snapshot);
        Ok(())
    }

    pub fn active_tasks_for(&self, owner: &str) -> Result<Option<TaskSnapshot>> {
        Ok(
            support::lock_anyhow(&self.active_tasks, "session active tasks")?
                .get(owner)
                .cloned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{EventTranslator, ExecutionTaskItem, ExecutionTaskStatus, Phase};

    use super::*;
    use crate::state::test_support::{root_task_write_stored, test_session_state};

    #[test]
    fn session_state_rehydrates_active_tasks_from_replay() {
        let session = SessionState::new(
            Phase::Idle,
            test_session_state().writer.clone(),
            astrcode_core::AgentStateProjector::default(),
            Vec::new(),
            vec![root_task_write_stored(
                1,
                "owner-a",
                vec![ExecutionTaskItem {
                    content: "补充 task 投影".to_string(),
                    status: ExecutionTaskStatus::InProgress,
                    active_form: Some("正在补充 task 投影".to_string()),
                }],
            )],
        );

        let snapshot = session
            .active_tasks_for("owner-a")
            .expect("task lookup should succeed")
            .expect("task snapshot should exist");
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].content, "补充 task 投影");
    }

    #[test]
    fn translate_store_and_cache_clears_task_snapshot_when_latest_snapshot_is_completed_only() {
        let session = test_session_state();
        let mut translator = EventTranslator::new(Phase::Idle);

        for stored in [
            root_task_write_stored(
                1,
                "owner-a",
                vec![ExecutionTaskItem {
                    content: "实现 runtime 投影".to_string(),
                    status: ExecutionTaskStatus::InProgress,
                    active_form: Some("正在实现 runtime 投影".to_string()),
                }],
            ),
            root_task_write_stored(
                2,
                "owner-a",
                vec![ExecutionTaskItem {
                    content: "实现 runtime 投影".to_string(),
                    status: ExecutionTaskStatus::Completed,
                    active_form: Some("已完成 runtime 投影".to_string()),
                }],
            ),
        ] {
            session
                .translate_store_and_cache(&stored, &mut translator)
                .expect("task event should translate");
        }

        assert!(
            session
                .active_tasks_for("owner-a")
                .expect("task lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn translate_store_and_cache_isolates_task_snapshots_by_owner() {
        let session = test_session_state();
        let mut translator = EventTranslator::new(Phase::Idle);

        let stored_events = [
            root_task_write_stored(
                1,
                "owner-a",
                vec![ExecutionTaskItem {
                    content: "任务 A".to_string(),
                    status: ExecutionTaskStatus::Pending,
                    active_form: None,
                }],
            ),
            root_task_write_stored(
                2,
                "owner-b",
                vec![ExecutionTaskItem {
                    content: "任务 B".to_string(),
                    status: ExecutionTaskStatus::InProgress,
                    active_form: Some("正在处理任务 B".to_string()),
                }],
            ),
            root_task_write_stored(3, "owner-a", Vec::new()),
        ];

        for event in stored_events {
            session
                .translate_store_and_cache(&event, &mut translator)
                .expect("task event should translate");
        }

        assert!(
            session
                .active_tasks_for("owner-a")
                .expect("task lookup should succeed")
                .is_none()
        );
        let owner_b = session
            .active_tasks_for("owner-b")
            .expect("task lookup should succeed")
            .expect("owner-b snapshot should exist");
        assert_eq!(owner_b.items[0].content, "任务 B");
    }
}
