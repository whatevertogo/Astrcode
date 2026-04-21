//! 会话真相状态：事件投影、child-session 节点跟踪、input queue 投影、writer 与广播基础设施。
//!
//! `SessionState` 只拥有 durable truth 与 projection/cache/broadcast 基础设施，
//! 不再承担 turn runtime control；运行时锁、CancelToken 与 compact 控制统一归 `turn/runtime.rs`。

mod cache;
mod child_sessions;
#[cfg(test)]
mod compaction;
mod execution;
mod input_queue;
mod paths;
mod projection_registry;
mod tasks;
#[cfg(test)]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::sample_spawn_child_ref;
mod writer;

use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{
    AgentEvent, AgentState, AgentStateProjector, EventTranslator, LlmMessage, ModeId, Phase,
    Result, SessionEventRecord, SessionRecoveryCheckpoint, StoredEvent, TurnProjectionSnapshot,
    normalize_recovered_phase,
    support::{self},
};
use chrono::Utc;
#[cfg(test)]
pub(crate) use execution::SessionStateEventSink;
pub(crate) use execution::append_and_broadcast;
pub use execution::checkpoint_if_compacted;
pub(crate) use input_queue::replay_input_queue_projection_index;
pub(crate) use paths::compact_history_event_log_path;
pub use paths::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};
use projection_registry::ProjectionRegistry;
use tokio::sync::broadcast;
pub(crate) use writer::SessionWriter;

const SESSION_BROADCAST_CAPACITY: usize = 2048;
const SESSION_LIVE_BROADCAST_CAPACITY: usize = 2048;

pub struct SessionState {
    projection_registry: StdMutex<ProjectionRegistry>,
    pub broadcaster: broadcast::Sender<SessionEventRecord>,
    live_broadcaster: broadcast::Sender<AgentEvent>,
    pub writer: Arc<SessionWriter>,
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState").finish_non_exhaustive()
    }
}

/// 轻量会话快照，用于 observe 返回值（仅包含可序列化的聚合字段）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: astrcode_core::SessionId,
    pub working_dir: String,
    pub latest_turn_id: Option<astrcode_core::TurnId>,
    pub turn_count: usize,
}

impl SessionState {
    pub fn new(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        Self::from_parts(phase, writer, projector, recent_records, recent_stored)
    }

    pub fn from_recovery(
        writer: Arc<SessionWriter>,
        checkpoint: &SessionRecoveryCheckpoint,
        tail_events: Vec<StoredEvent>,
    ) -> Result<Self> {
        let phase = normalize_recovered_phase(checkpoint.agent_state.phase);
        let mut projection_registry = ProjectionRegistry::from_recovery(
            phase,
            &checkpoint.agent_state,
            checkpoint.projection_registry_snapshot(),
            Vec::new(),
            Vec::new(),
        );
        for stored in &tail_events {
            stored.event.validate().map_err(|error| {
                astrcode_core::AstrError::Validation(format!(
                    "session '{}' contains invalid stored event at storage_seq {}: {}",
                    checkpoint.agent_state.session_id, stored.storage_seq, error
                ))
            })?;
            projection_registry.apply(stored)?;
        }
        projection_registry.cache_records(&astrcode_core::replay_records(&tail_events, None));
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);

        Ok(Self {
            projection_registry: StdMutex::new(projection_registry),
            broadcaster,
            live_broadcaster,
            writer,
        })
    }

    fn from_parts(
        phase: Phase,
        writer: Arc<SessionWriter>,
        projector: AgentStateProjector,
        recent_records: Vec<SessionEventRecord>,
        recent_stored: Vec<StoredEvent>,
    ) -> Self {
        let (broadcaster, _) = broadcast::channel(SESSION_BROADCAST_CAPACITY);
        let (live_broadcaster, _) = broadcast::channel(SESSION_LIVE_BROADCAST_CAPACITY);
        Self {
            projection_registry: StdMutex::new(ProjectionRegistry::new(
                phase,
                projector,
                recent_records,
                recent_stored,
            )),
            broadcaster,
            live_broadcaster,
            writer,
        }
    }

    pub fn recovery_checkpoint(
        &self,
        checkpoint_storage_seq: u64,
    ) -> Result<SessionRecoveryCheckpoint> {
        let projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        Ok(SessionRecoveryCheckpoint::new(
            projection_registry.snapshot_projected_state(),
            projection_registry.projection_snapshot(),
            checkpoint_storage_seq,
        ))
    }

    pub fn snapshot_projected_state(&self) -> Result<AgentState> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_projected_state(),
        )
    }

    pub fn current_turn_messages(&self) -> Result<Vec<LlmMessage>> {
        Ok(self.snapshot_projected_state()?.messages)
    }

    /// 订阅 live-only 事件流（token 级 delta 等瞬时事件，不参与 durable replay）。
    pub fn subscribe_live(&self) -> broadcast::Receiver<AgentEvent> {
        self.live_broadcaster.subscribe()
    }

    /// 广播一条 live-only 事件（无订阅者时不视为错误）。
    pub fn broadcast_live_event(&self, event: AgentEvent) {
        let _ = self.live_broadcaster.send(event);
    }

    pub fn current_phase(&self) -> Result<Phase> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_phase(),
        )
    }

    pub fn current_mode_id(&self) -> Result<ModeId> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .current_mode_id(),
        )
    }

    pub fn last_mode_changed_at(&self) -> Result<Option<chrono::DateTime<Utc>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .last_mode_changed_at(),
        )
    }

    pub fn translate_store_and_cache(
        &self,
        stored: &StoredEvent,
        translator: &mut EventTranslator,
    ) -> Result<Vec<SessionEventRecord>> {
        stored.event.validate()?;
        let mut projection_registry =
            support::lock_anyhow(&self.projection_registry, "session projection registry")?;
        projection_registry.apply(stored)?;
        let records = translator.translate(stored);
        projection_registry.cache_records(&records);
        Ok(records)
    }

    pub fn recent_records_after(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<Option<Vec<SessionEventRecord>>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .recent_records_after(last_event_id),
        )
    }

    pub fn snapshot_recent_stored_events(&self) -> Result<Vec<StoredEvent>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .snapshot_recent_stored_events(),
        )
    }

    pub fn turn_projection(&self, turn_id: &str) -> Result<Option<TurnProjectionSnapshot>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .turn_projection(turn_id),
        )
    }

    pub async fn append_and_broadcast(
        &self,
        event: &astrcode_core::StorageEvent,
        translator: &mut EventTranslator,
    ) -> Result<StoredEvent> {
        let stored = self.writer.clone().append(event.clone()).await?;
        let records = self.translate_store_and_cache(&stored, translator)?;
        for record in records {
            let _ = self.broadcaster.send(record);
        }
        Ok(stored)
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentEventContext, ExecutionTaskItem, ExecutionTaskStatus, InvocationKind, ModeId, Phase,
        SessionRecoveryCheckpoint, StorageEventPayload, SubRunStorageMode, UserMessageOrigin,
    };
    use chrono::Utc;

    use super::{
        SessionState, SessionWriter,
        test_support::{
            NoopEventLogWriter, event, independent_session_sub_run_agent, root_agent, stored,
            test_session_state,
        },
    };

    #[test]
    fn translate_store_and_cache_keeps_sub_run_events_out_of_parent_snapshot() {
        let session = test_session_state();
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);

        let events = vec![
            stored(
                1,
                event(
                    None,
                    root_agent(),
                    StorageEventPayload::SessionStart {
                        session_id: "session-1".into(),
                        timestamp: chrono::Utc::now(),
                        working_dir: "/tmp".into(),
                        parent_session_id: None,
                        parent_storage_seq: None,
                    },
                ),
            ),
            stored(
                2,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::UserMessage {
                        content: "root task".into(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                3,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::AssistantFinal {
                        content: "root answer".into(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        step_index: None,
                        timestamp: None,
                    },
                ),
            ),
            stored(
                4,
                event(
                    Some("turn-root"),
                    root_agent(),
                    StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".into()),
                    },
                ),
            ),
            stored(
                5,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::UserMessage {
                        content: "child task".into(),
                        origin: UserMessageOrigin::User,
                        timestamp: chrono::Utc::now(),
                    },
                ),
            ),
            stored(
                6,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::AssistantFinal {
                        content: "child answer".into(),
                        reasoning_content: None,
                        reasoning_signature: None,
                        step_index: None,
                        timestamp: None,
                    },
                ),
            ),
            stored(
                7,
                event(
                    Some("turn-child"),
                    independent_session_sub_run_agent(),
                    StorageEventPayload::TurnDone {
                        timestamp: chrono::Utc::now(),
                        terminal_kind: Some(astrcode_core::TurnTerminalKind::Completed),
                        reason: Some("completed".into()),
                    },
                ),
            ),
        ];

        for stored in &events {
            session
                .translate_store_and_cache(stored, &mut translator)
                .expect("event should translate into session cache");
        }

        let projected = session
            .snapshot_projected_state()
            .expect("snapshot should be available");

        assert_eq!(projected.turn_count, 1);
        assert_eq!(projected.messages.len(), 2);
        assert!(matches!(
            &projected.messages[0],
            astrcode_core::LlmMessage::User { content, .. } if content == "root task"
        ));
        assert!(matches!(
            &projected.messages[1],
            astrcode_core::LlmMessage::Assistant { content, .. } if content == "root answer"
        ));
    }

    #[test]
    fn translate_store_and_cache_rejects_invalid_stored_event() {
        let session = test_session_state();
        let mut translator = astrcode_core::EventTranslator::new(Phase::Idle);
        let malformed = stored(
            1,
            event(
                Some("turn-child"),
                AgentEventContext {
                    agent_id: Some("agent-child".to_string().into()),
                    parent_turn_id: Some("turn-root".to_string().into()),
                    agent_profile: Some("explore".to_string()),
                    sub_run_id: Some("subrun-1".to_string().into()),
                    parent_sub_run_id: None,
                    invocation_kind: Some(InvocationKind::SubRun),
                    storage_mode: Some(SubRunStorageMode::IndependentSession),
                    child_session_id: None,
                },
                StorageEventPayload::UserMessage {
                    content: "child task".into(),
                    origin: UserMessageOrigin::User,
                    timestamp: chrono::Utc::now(),
                },
            ),
        );

        let error = session
            .translate_store_and_cache(&malformed, &mut translator)
            .expect_err("invalid stored event should be rejected");

        assert!(error.to_string().contains("child_session_id"));
    }

    #[test]
    fn legacy_checkpoint_fields_migrate_into_projection_registry_snapshot() {
        let checkpoint_json = serde_json::json!({
            "agentState": {
                "session_id": "session-legacy",
                "working_dir": "/tmp",
                "messages": [],
                "phase": "idle",
                "mode_id": ModeId::default(),
                "turn_count": 0,
                "last_assistant_at": serde_json::Value::Null,
            },
            "phase": "idle",
            "lastModeChangedAt": "2026-04-21T00:00:00Z",
            "childNodes": {},
            "activeTasks": {
                "owner-a": {
                    "owner": "owner-a",
                    "items": [{
                        "content": "迁移旧 checkpoint",
                        "status": "in_progress",
                        "activeForm": "正在迁移旧 checkpoint"
                    }]
                }
            },
            "inputQueueProjectionIndex": {},
            "checkpointStorageSeq": 9
        });
        let checkpoint: SessionRecoveryCheckpoint =
            serde_json::from_value(checkpoint_json).expect("legacy checkpoint should deserialize");

        let projection_snapshot = checkpoint.projection_registry_snapshot();
        assert_eq!(
            projection_snapshot.last_mode_changed_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-21T00:00:00Z")
                    .expect("timestamp should parse")
                    .with_timezone(&Utc)
            )
        );
        assert!(projection_snapshot.active_tasks.contains_key("owner-a"));

        let recovered = SessionState::from_recovery(
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
            &checkpoint,
            Vec::new(),
        )
        .expect("legacy checkpoint should recover");

        let recovered_task = recovered
            .active_tasks_for("owner-a")
            .expect("task lookup should succeed")
            .expect("legacy task should survive migration");
        assert_eq!(
            recovered_task.items,
            vec![ExecutionTaskItem {
                content: "迁移旧 checkpoint".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在迁移旧 checkpoint".to_string()),
            }]
        );
        assert_eq!(
            recovered
                .last_mode_changed_at()
                .expect("mode timestamp should exist"),
            projection_snapshot.last_mode_changed_at
        );
    }
}
