#![cfg(test)]

use std::sync::Arc;

use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AgentStateProjector, ChildAgentRef,
    ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNotification,
    ChildSessionNotificationKind, EventLogWriter, ExecutionTaskItem, ExecutionTaskSnapshotMetadata,
    InvocationKind, ParentExecutionRef, Phase, StorageEvent, StorageEventPayload, StoreResult,
    StoredEvent, SubRunStorageMode, TaskSnapshot,
};

use super::{SessionState, SessionWriter};

pub(crate) struct NoopEventLogWriter;

impl EventLogWriter for NoopEventLogWriter {
    fn append(&mut self, _event: &StorageEvent) -> StoreResult<StoredEvent> {
        unreachable!("session_state tests do not persist through the writer")
    }
}

pub(crate) fn test_session_state() -> SessionState {
    SessionState::new(
        Phase::Idle,
        Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
        AgentStateProjector::default(),
        Vec::new(),
        Vec::new(),
    )
}

pub(crate) fn root_agent() -> AgentEventContext {
    AgentEventContext::default()
}

pub(crate) fn independent_session_sub_run_agent() -> AgentEventContext {
    AgentEventContext {
        agent_id: Some("agent-child".to_string().into()),
        parent_turn_id: Some("turn-root".to_string().into()),
        parent_sub_run_id: None,
        agent_profile: Some("explore".to_string()),
        sub_run_id: Some("subrun-1".to_string().into()),
        invocation_kind: Some(InvocationKind::SubRun),
        storage_mode: Some(SubRunStorageMode::IndependentSession),
        child_session_id: Some("session-child".to_string().into()),
    }
}

pub(crate) fn event(
    turn_id: Option<&str>,
    agent: AgentEventContext,
    payload: StorageEventPayload,
) -> StorageEvent {
    StorageEvent {
        turn_id: turn_id.map(str::to_string),
        agent,
        payload,
    }
}

pub(crate) fn stored(storage_seq: u64, event: StorageEvent) -> StoredEvent {
    StoredEvent { storage_seq, event }
}

pub(crate) fn child_notification_event(
    kind: ChildSessionNotificationKind,
    status: AgentLifecycleStatus,
) -> StorageEvent {
    event(
        Some("turn-root"),
        independent_session_sub_run_agent(),
        StorageEventPayload::ChildSessionNotification {
            notification: ChildSessionNotification {
                notification_id: format!("child:{kind:?}").into(),
                child_ref: sample_spawn_child_ref(status),
                kind,
                source_tool_call_id: Some("call-1".into()),
                delivery: Some(astrcode_core::ParentDelivery {
                    idempotency_key: format!("child:{kind:?}"),
                    origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                    terminal_semantics: match kind {
                        ChildSessionNotificationKind::Started
                        | ChildSessionNotificationKind::ProgressSummary
                        | ChildSessionNotificationKind::Waiting
                        | ChildSessionNotificationKind::Resumed => {
                            astrcode_core::ParentDeliveryTerminalSemantics::NonTerminal
                        },
                        ChildSessionNotificationKind::Delivered
                        | ChildSessionNotificationKind::Closed
                        | ChildSessionNotificationKind::Failed => {
                            astrcode_core::ParentDeliveryTerminalSemantics::Terminal
                        },
                    },
                    source_turn_id: Some("turn-root".to_string()),
                    payload: astrcode_core::ParentDeliveryPayload::Progress(
                        astrcode_core::ProgressParentDeliveryPayload {
                            message: "child summary".to_string(),
                        },
                    ),
                }),
            },
            timestamp: Some(chrono::Utc::now()),
        },
    )
}

pub(crate) fn sample_spawn_child_ref(status: AgentLifecycleStatus) -> ChildAgentRef {
    ChildAgentRef {
        identity: ChildExecutionIdentity {
            agent_id: "agent-child".into(),
            session_id: "session-parent".into(),
            sub_run_id: "subrun-1".into(),
        },
        parent: ParentExecutionRef {
            parent_agent_id: Some("agent-parent".into()),
            parent_sub_run_id: Some("subrun-parent".into()),
        },
        lineage_kind: ChildSessionLineageKind::Spawn,
        status,
        open_session_id: "session-child".into(),
    }
}

pub(crate) fn root_task_tool_result_event(
    turn_id: &str,
    owner: &str,
    items: Vec<ExecutionTaskItem>,
) -> StorageEvent {
    let snapshot = TaskSnapshot {
        owner: owner.to_string(),
        items,
    };
    StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: AgentEventContext::default(),
        payload: StorageEventPayload::ToolResult {
            tool_call_id: format!("call-{turn_id}"),
            tool_name: "taskWrite".to_string(),
            output: "updated execution tasks".to_string(),
            success: true,
            error: None,
            metadata: Some(
                serde_json::to_value(ExecutionTaskSnapshotMetadata::from_snapshot(&snapshot))
                    .expect("task metadata should serialize"),
            ),
            continuation: None,
            duration_ms: 1,
        },
    }
}

pub(crate) fn root_task_write_stored(
    storage_seq: u64,
    owner: &str,
    items: Vec<ExecutionTaskItem>,
) -> StoredEvent {
    stored(
        storage_seq,
        root_task_tool_result_event(&format!("turn-{storage_seq}"), owner, items),
    )
}
