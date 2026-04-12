//! 子执行状态回放与解析模块。
//!
//! 负责从持久化的事件流中解析子执行的状态和结果，包括：
//! - 从 SubRunStarted/SubRunFinished 事件中提取完整状态
//! - 支持活动中的子执行快照（无结果）
//! - 支持已完成的子执行结果解析
//!
//! 设计目的：让 runtime façade 不需要了解事件拼装细节。

use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, ChildSessionLineageKind, ChildSessionNode,
    ChildSessionNotification, ChildSessionNotificationKind, ChildSessionStatusSource,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StorageEvent,
    StorageEventPayload, StoredEvent, SubRunHandle, SubRunResult, SubRunStorageMode,
};

use crate::ExecutionLineageIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedSubRunStatusSource {
    Live,
    Durable,
}

#[derive(Debug, Clone)]
pub struct ParsedSubRunStatus {
    pub handle: SubRunHandle,
    pub tool_call_id: Option<String>,
    pub source: ParsedSubRunStatusSource,
    pub result: Option<SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSubRunResolution {
    CancelLive,
    AlreadyFinalized,
    Missing,
}

pub fn snapshot_from_active_handle(handle: SubRunHandle) -> ParsedSubRunStatus {
    ParsedSubRunStatus {
        tool_call_id: None,
        source: ParsedSubRunStatusSource::Live,
        handle,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: None,
    }
}

pub fn build_child_session_node(
    child: &SubRunHandle,
    parent_session_id: &str,
    parent_turn_id: &str,
    created_by_tool_call_id: Option<String>,
) -> ChildSessionNode {
    let child_session_id = child
        .child_session_id
        .clone()
        .unwrap_or_else(|| child.session_id.clone());

    ChildSessionNode {
        agent_id: child.agent_id.clone(),
        session_id: parent_session_id.to_string(),
        child_session_id,
        sub_run_id: child.sub_run_id.clone(),
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: child.parent_agent_id.clone(),
        parent_sub_run_id: child.parent_sub_run_id.clone(),
        parent_turn_id: parent_turn_id.to_string(),
        lineage_kind: ChildSessionLineageKind::Spawn,
        status: child.lifecycle,
        status_source: ChildSessionStatusSource::Durable,
        created_by_tool_call_id,
        lineage_snapshot: None,
    }
}

pub fn build_child_session_notification(
    node: &ChildSessionNode,
    notification_id: impl Into<String>,
    kind: ChildSessionNotificationKind,
    summary: impl Into<String>,
    lifecycle: AgentLifecycleStatus,
    final_reply_excerpt: Option<String>,
) -> ChildSessionNotification {
    ChildSessionNotification {
        notification_id: notification_id.into(),
        child_ref: node.child_ref(),
        kind,
        summary: summary.into(),
        status: lifecycle,
        source_tool_call_id: node.created_by_tool_call_id.clone(),
        final_reply_excerpt,
    }
}

pub fn build_subrun_started_event(
    parent_turn_id: &str,
    agent: AgentEventContext,
    _child: &SubRunHandle,
    tool_call_id: Option<String>,
    resolved_overrides: ResolvedSubagentContextOverrides,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(parent_turn_id.to_string()),
        agent,
        payload: StorageEventPayload::SubRunStarted {
            tool_call_id,
            resolved_overrides,
            resolved_limits,
            timestamp: Some(chrono::Utc::now()),
        },
    }
}

pub fn build_subrun_finished_event(
    parent_turn_id: &str,
    agent: AgentEventContext,
    _child: &SubRunHandle,
    tool_call_id: Option<String>,
    result: SubRunResult,
    step_count: u32,
    estimated_tokens: u64,
) -> StorageEvent {
    StorageEvent {
        turn_id: Some(parent_turn_id.to_string()),
        agent,
        payload: StorageEventPayload::SubRunFinished {
            tool_call_id,
            result,
            step_count,
            estimated_tokens,
            timestamp: Some(chrono::Utc::now()),
        },
    }
}

pub fn build_execution_lineage_index(events: &[StoredEvent]) -> ExecutionLineageIndex {
    ExecutionLineageIndex::from_stored_events(events)
}

/// 将 live snapshot 叠加到 durable snapshot 上。
///
/// durable 继续提供结果、限制快照和计数等历史真相；
/// live 只覆盖运行态状态以及更接近当前的 lineage/tool_call 字段。
pub fn overlay_live_snapshot_on_durable(
    live_snapshot: ParsedSubRunStatus,
    durable_snapshot: ParsedSubRunStatus,
) -> ParsedSubRunStatus {
    let mut merged_handle = durable_snapshot.handle;
    merged_handle.lifecycle = live_snapshot.handle.lifecycle;
    merged_handle.last_turn_outcome = live_snapshot.handle.last_turn_outcome;

    ParsedSubRunStatus {
        handle: merged_handle,
        tool_call_id: live_snapshot.tool_call_id.or(durable_snapshot.tool_call_id),
        source: ParsedSubRunStatusSource::Live,
        result: durable_snapshot.result,
        step_count: durable_snapshot.step_count,
        estimated_tokens: durable_snapshot.estimated_tokens,
        resolved_overrides: durable_snapshot.resolved_overrides,
        resolved_limits: durable_snapshot.resolved_limits,
    }
}

pub fn live_handle_owned_by_session<F>(
    session_id: &str,
    live_handle: &SubRunHandle,
    durable_snapshot: Option<&ParsedSubRunStatus>,
    normalize_session_id: F,
) -> bool
where
    F: Fn(&str) -> String,
{
    let live_session_id = normalize_session_id(&live_handle.session_id);
    if live_session_id == session_id {
        return true;
    }

    let live_child_session_id = live_handle
        .child_session_id
        .as_deref()
        .map(&normalize_session_id);
    if live_child_session_id.as_deref() == Some(session_id) {
        return true;
    }

    let Some(durable_snapshot) = durable_snapshot else {
        return false;
    };
    let durable_session_id = normalize_session_id(&durable_snapshot.handle.session_id);
    if durable_session_id != session_id {
        return false;
    }

    let durable_child_session_id = durable_snapshot
        .handle
        .child_session_id
        .as_deref()
        .map(normalize_session_id);
    durable_snapshot.handle.agent_id == live_handle.agent_id
        && durable_child_session_id.as_deref() == Some(live_session_id.as_str())
}

pub fn resolve_subrun_status_snapshot<F>(
    session_id: &str,
    live_handle: Option<SubRunHandle>,
    durable_snapshot: Option<ParsedSubRunStatus>,
    normalize_session_id: F,
) -> Option<ParsedSubRunStatus>
where
    F: Fn(&str) -> String + Copy,
{
    if let Some(handle) = live_handle {
        if live_handle_owned_by_session(
            session_id,
            &handle,
            durable_snapshot.as_ref(),
            normalize_session_id,
        ) {
            let live_snapshot = snapshot_from_active_handle(handle);
            return Some(durable_snapshot.map_or(live_snapshot.clone(), |durable| {
                overlay_live_snapshot_on_durable(live_snapshot, durable)
            }));
        }
    }
    durable_snapshot
}

pub fn resolve_cancel_subrun_resolution<F>(
    session_id: &str,
    live_handle: Option<&SubRunHandle>,
    durable_snapshot: Option<&ParsedSubRunStatus>,
    normalize_session_id: F,
) -> CancelSubRunResolution
where
    F: Fn(&str) -> String + Copy,
{
    if let Some(handle) = live_handle {
        if live_handle_owned_by_session(session_id, handle, durable_snapshot, normalize_session_id)
        {
            return CancelSubRunResolution::CancelLive;
        }
    }

    if durable_snapshot.is_some() {
        CancelSubRunResolution::AlreadyFinalized
    } else {
        CancelSubRunResolution::Missing
    }
}

// 这里把 finalized sub-run 的回放解释收进 execution crate，避免 runtime façade
// 继续直接了解 SubRunStarted/SubRunFinished 的事件拼装细节。
pub fn find_subrun_status_in_events(
    events: &[StoredEvent],
    session_id: &str,
    sub_run_id: &str,
) -> Option<ParsedSubRunStatus> {
    let mut started_agent: Option<AgentEventContext> = None;
    let mut started_tool_call_id = None;
    let mut resolved_overrides = None;
    let mut resolved_limits = None;
    let mut finished_tool_call_id = None;
    let mut finished: Option<(SubRunResult, u32, u64)> = None;

    for stored in events {
        match &stored.event.payload {
            StorageEventPayload::SubRunStarted {
                tool_call_id,
                resolved_overrides: started_overrides,
                resolved_limits: started_limits,
                ..
            } if stored.event.agent.sub_run_id.as_deref() == Some(sub_run_id) => {
                let agent = &stored.event.agent;
                started_agent = Some(agent.clone());
                started_tool_call_id = tool_call_id.clone();
                resolved_overrides = Some(started_overrides.clone());
                resolved_limits = Some(started_limits.clone());
            },
            StorageEventPayload::SubRunFinished {
                tool_call_id,
                result,
                step_count,
                estimated_tokens,
                ..
            } if stored.event.agent.sub_run_id.as_deref() == Some(sub_run_id) => {
                let agent = &stored.event.agent;
                if started_agent.is_none() {
                    started_agent = Some(agent.clone());
                }
                finished_tool_call_id = tool_call_id.clone();
                finished = Some((result.clone(), *step_count, *estimated_tokens));
            },
            _ => {},
        }
    }

    started_agent.map(|agent| {
        let tool_call_id = finished_tool_call_id.or(started_tool_call_id);

        ParsedSubRunStatus {
            handle: build_replayed_handle(session_id, sub_run_id, &agent, finished.as_ref()),
            tool_call_id,
            source: ParsedSubRunStatusSource::Durable,
            result: finished.as_ref().map(|(result, _, _)| result.clone()),
            step_count: finished.as_ref().map(|(_, step_count, _)| *step_count),
            estimated_tokens: finished
                .as_ref()
                .map(|(_, _, estimated_tokens)| *estimated_tokens),
            resolved_overrides,
            resolved_limits,
        }
    })
}

fn build_replayed_handle(
    session_id: &str,
    sub_run_id: &str,
    agent: &AgentEventContext,
    finished: Option<&(SubRunResult, u32, u64)>,
) -> SubRunHandle {
    // parent_turn_id 为必填字段，缺失说明事件来自旧版本记录，应明确记录但不阻断回放。
    // 不再静默降级为空串——生产写入路径已保证必填，空串仅在旧 JSONL 回放时出现。
    let parent_turn_id = agent.parent_turn_id.clone().unwrap_or_else(|| {
        tracing::warn!(
            sub_run_id,
            session_id,
            "parent_turn_id missing from event context, treating as legacy descriptorless input"
        );
        String::new()
    });

    SubRunHandle {
        sub_run_id: sub_run_id.to_string(),
        agent_id: agent
            .agent_id
            .clone()
            .unwrap_or_else(|| "unknown-agent".to_string()),
        session_id: session_id.to_string(),
        child_session_id: agent.child_session_id.clone(),
        depth: 0,
        parent_turn_id,
        parent_agent_id: None,
        parent_sub_run_id: agent.parent_sub_run_id.clone(),
        agent_profile: agent
            .agent_profile
            .clone()
            .unwrap_or_else(|| "unknown-profile".to_string()),
        storage_mode: agent
            .storage_mode
            .unwrap_or(SubRunStorageMode::IndependentSession),
        lifecycle: finished
            .as_ref()
            .map(|(result, _, _)| result.lifecycle)
            .unwrap_or(AgentLifecycleStatus::Pending),
        last_turn_outcome: finished
            .as_ref()
            .and_then(|(result, _, _)| result.last_turn_outcome),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotificationKind,
        ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StorageEvent,
        StorageEventPayload, StoredEvent, SubRunHandle, SubRunHandoff, SubRunResult,
        SubRunStorageMode,
    };

    use super::{
        CancelSubRunResolution, ParsedSubRunStatus, ParsedSubRunStatusSource,
        build_child_session_node, build_child_session_notification, build_subrun_finished_event,
        build_subrun_started_event, find_subrun_status_in_events, overlay_live_snapshot_on_durable,
        resolve_cancel_subrun_resolution, resolve_subrun_status_snapshot,
        snapshot_from_active_handle,
    };

    #[test]
    fn snapshot_from_active_handle_keeps_fast_path_shape() {
        let handle = SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: Some("child-1".to_string()),
            depth: 2,
            parent_turn_id: "turn-1".to_string(),
            parent_agent_id: Some("parent-agent".to_string()),
            parent_sub_run_id: Some("subrun-parent".to_string()),
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };

        let snapshot = snapshot_from_active_handle(handle.clone());

        assert_eq!(snapshot.handle, handle);
        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Live);
        assert!(snapshot.result.is_none());
        assert!(snapshot.step_count.is_none());
        assert!(snapshot.estimated_tokens.is_none());
        assert!(snapshot.resolved_overrides.is_none());
        assert!(snapshot.resolved_limits.is_none());
    }

    #[test]
    fn find_subrun_status_in_events_rebuilds_finished_snapshot() {
        let agent = AgentEventContext::sub_run(
            "agent-1".to_string(),
            "turn-1".to_string(),
            "review".to_string(),
            "subrun-1".to_string(),
            None,
            SubRunStorageMode::IndependentSession,
            Some("child-1".to_string()),
        );
        let overrides = ResolvedSubagentContextOverrides {
            storage_mode: SubRunStorageMode::IndependentSession,
            ..Default::default()
        };
        let limits = ResolvedExecutionLimitsSnapshot {
            allowed_tools: vec!["readFile".to_string()],
        };
        let result = SubRunResult {
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(AgentTurnOutcome::Completed),
            handoff: Some(SubRunHandoff {
                summary: "done".to_string(),
                findings: vec!["ok".to_string()],
                artifacts: Vec::new(),
            }),
            failure: None,
        };
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::SubRunStarted {
                        tool_call_id: None,
                        resolved_overrides: overrides.clone(),
                        resolved_limits: limits.clone(),
                        timestamp: None,
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent,
                    payload: StorageEventPayload::SubRunFinished {
                        tool_call_id: None,
                        result: result.clone(),
                        step_count: 2,
                        estimated_tokens: 123,
                        timestamp: None,
                    },
                },
            },
        ];

        let snapshot =
            find_subrun_status_in_events(&events, "session-1", "subrun-1").expect("snapshot");

        assert_eq!(snapshot.handle.session_id, "session-1");
        assert_eq!(snapshot.handle.sub_run_id, "subrun-1");
        assert_eq!(snapshot.handle.child_session_id.as_deref(), Some("child-1"));
        assert_eq!(snapshot.handle.agent_profile, "review");
        assert_eq!(
            snapshot.handle.storage_mode,
            SubRunStorageMode::IndependentSession
        );
        assert_eq!(snapshot.handle.lifecycle, AgentLifecycleStatus::Idle);
        assert_eq!(
            snapshot.handle.last_turn_outcome,
            Some(AgentTurnOutcome::Completed)
        );
        assert_eq!(
            snapshot
                .result
                .as_ref()
                .and_then(|item| item.handoff.as_ref())
                .map(|item| item.summary.as_str()),
            Some("done")
        );
        assert_eq!(snapshot.step_count, Some(2));
        assert_eq!(snapshot.estimated_tokens, Some(123));
        assert_eq!(snapshot.resolved_overrides, Some(overrides));
        assert_eq!(snapshot.resolved_limits, Some(limits));
        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
        assert_eq!(snapshot.handle.depth, 0);
        assert!(snapshot.handle.parent_agent_id.is_none());
        assert!(snapshot.tool_call_id.is_none());
    }

    #[test]
    fn find_subrun_status_in_events_rebuilds_started_only_snapshot() {
        let agent = AgentEventContext::sub_run(
            "agent-1".to_string(),
            "turn-1".to_string(),
            "review".to_string(),
            "subrun-1".to_string(),
            None,
            SubRunStorageMode::IndependentSession,
            None,
        );
        let overrides = ResolvedSubagentContextOverrides::default();
        let limits = ResolvedExecutionLimitsSnapshot {
            allowed_tools: vec!["readFile".to_string(), "grep".to_string()],
        };
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent,
                payload: StorageEventPayload::SubRunStarted {
                    tool_call_id: None,
                    resolved_overrides: overrides.clone(),
                    resolved_limits: limits.clone(),
                    timestamp: None,
                },
            },
        }];

        let snapshot =
            find_subrun_status_in_events(&events, "session-1", "subrun-1").expect("snapshot");

        assert_eq!(snapshot.handle.lifecycle, AgentLifecycleStatus::Pending);
        assert_eq!(snapshot.handle.last_turn_outcome, None);
        assert!(snapshot.result.is_none());
        assert!(snapshot.step_count.is_none());
        assert!(snapshot.estimated_tokens.is_none());
        assert_eq!(snapshot.resolved_overrides, Some(overrides));
        assert_eq!(snapshot.resolved_limits, Some(limits));
        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
        assert_eq!(snapshot.handle.depth, 0);
        assert!(snapshot.handle.parent_agent_id.is_none());
    }

    #[test]
    fn find_subrun_status_in_events_uses_finished_event_without_start() {
        let result = SubRunResult {
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(AgentTurnOutcome::TokenExceeded),
            handoff: None,
            failure: None,
        };
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-2".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-2".to_string(),
                    "turn-2".to_string(),
                    "plan".to_string(),
                    "subrun-2".to_string(),
                    None,
                    SubRunStorageMode::IndependentSession,
                    Some("child-2".to_string()),
                ),
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: None,
                    result: result.clone(),
                    step_count: 7,
                    estimated_tokens: 321,
                    timestamp: None,
                },
            },
        }];

        let snapshot =
            find_subrun_status_in_events(&events, "session-2", "subrun-2").expect("snapshot");

        assert_eq!(snapshot.handle.agent_profile, "plan");
        assert_eq!(snapshot.handle.child_session_id.as_deref(), Some("child-2"));
        assert_eq!(
            snapshot.handle.storage_mode,
            SubRunStorageMode::IndependentSession
        );
        assert_eq!(snapshot.handle.lifecycle, AgentLifecycleStatus::Idle);
        assert_eq!(
            snapshot.handle.last_turn_outcome,
            Some(AgentTurnOutcome::TokenExceeded)
        );
        assert_eq!(snapshot.result, Some(result));
        assert_eq!(snapshot.step_count, Some(7));
        assert_eq!(snapshot.estimated_tokens, Some(321));
        assert!(snapshot.resolved_overrides.is_none());
        assert!(snapshot.resolved_limits.is_none());
        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
        assert_eq!(snapshot.handle.depth, 0);
        assert!(snapshot.handle.parent_agent_id.is_none());
    }

    #[test]
    fn find_subrun_status_in_events_maps_parent_abort_to_cancelled_status() {
        let agent = AgentEventContext::sub_run(
            "agent-abort".to_string(),
            "turn-parent".to_string(),
            "review".to_string(),
            "subrun-abort".to_string(),
            None,
            SubRunStorageMode::IndependentSession,
            Some("child-abort".to_string()),
        );
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::SubRunStarted {
                        tool_call_id: None,
                        resolved_overrides: ResolvedSubagentContextOverrides::default(),
                        resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                        timestamp: None,
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-parent".to_string()),
                    agent,
                    payload: StorageEventPayload::SubRunFinished {
                        tool_call_id: None,
                        result: SubRunResult {
                            lifecycle: AgentLifecycleStatus::Terminated,
                            last_turn_outcome: Some(AgentTurnOutcome::Cancelled),
                            handoff: None,
                            failure: None,
                        },
                        step_count: 1,
                        estimated_tokens: 20,
                        timestamp: None,
                    },
                },
            },
        ];

        let snapshot = find_subrun_status_in_events(&events, "session-abort", "subrun-abort")
            .expect("snapshot should exist");

        assert_eq!(snapshot.handle.lifecycle, AgentLifecycleStatus::Terminated);
        assert_eq!(
            snapshot.handle.last_turn_outcome,
            Some(AgentTurnOutcome::Cancelled)
        );
        assert_eq!(
            snapshot.handle.storage_mode,
            SubRunStorageMode::IndependentSession
        );
        assert_eq!(
            snapshot.result.as_ref().map(|item| item.lifecycle),
            Some(AgentLifecycleStatus::Terminated)
        );
        assert_eq!(
            snapshot
                .result
                .as_ref()
                .and_then(|item| item.last_turn_outcome),
            Some(AgentTurnOutcome::Cancelled)
        );
        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
    }

    #[test]
    fn find_subrun_status_in_events_resolves_tool_call_id_from_events() {
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-legacy".to_string()),
                    agent: AgentEventContext::sub_run(
                        "agent-legacy".to_string(),
                        "turn-legacy".to_string(),
                        "review".to_string(),
                        "subrun-legacy".to_string(),
                        None,
                        SubRunStorageMode::IndependentSession,
                        None,
                    ),
                    payload: StorageEventPayload::SubRunStarted {
                        tool_call_id: Some("call-legacy".to_string()),
                        resolved_overrides: ResolvedSubagentContextOverrides::default(),
                        resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                        timestamp: None,
                    },
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent {
                    turn_id: Some("turn-legacy".to_string()),
                    agent: AgentEventContext::sub_run(
                        "agent-legacy".to_string(),
                        "turn-legacy".to_string(),
                        "review".to_string(),
                        "subrun-legacy".to_string(),
                        None,
                        SubRunStorageMode::IndependentSession,
                        None,
                    ),
                    payload: StorageEventPayload::SubRunFinished {
                        tool_call_id: Some("call-legacy".to_string()),
                        result: SubRunResult {
                            lifecycle: AgentLifecycleStatus::Idle,
                            last_turn_outcome: Some(AgentTurnOutcome::Completed),
                            handoff: None,
                            failure: None,
                        },
                        step_count: 1,
                        estimated_tokens: 8,
                        timestamp: None,
                    },
                },
            },
        ];

        let snapshot = find_subrun_status_in_events(&events, "session-legacy", "subrun-legacy")
            .expect("snapshot should exist");

        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
        assert_eq!(snapshot.tool_call_id.as_deref(), Some("call-legacy"));
    }

    #[test]
    fn find_subrun_status_in_events_uses_agent_context_parent_turn_id() {
        let events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-parent".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-3".to_string(),
                    "turn-parent".to_string(),
                    "review".to_string(),
                    "subrun-3".to_string(),
                    None,
                    SubRunStorageMode::IndependentSession,
                    None,
                ),
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: Some("call-3".to_string()),
                    result: SubRunResult {
                        lifecycle: AgentLifecycleStatus::Idle,
                        last_turn_outcome: Some(AgentTurnOutcome::Completed),
                        handoff: None,
                        failure: None,
                    },
                    step_count: 1,
                    estimated_tokens: 12,
                    timestamp: None,
                },
            },
        }];

        let snapshot =
            find_subrun_status_in_events(&events, "session-3", "subrun-3").expect("snapshot");

        assert_eq!(snapshot.source, ParsedSubRunStatusSource::Durable);
        assert_eq!(snapshot.tool_call_id.as_deref(), Some("call-3"));
        assert_eq!(snapshot.handle.parent_turn_id, "turn-parent".to_string());
    }

    #[test]
    fn overlay_live_snapshot_on_durable_prefers_live_tool_call_and_status() {
        let live = ParsedSubRunStatus {
            handle: SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-live".to_string(),
                session_id: "session-child".to_string(),
                child_session_id: Some("session-child".to_string()),
                depth: 1,
                parent_turn_id: "turn-live".to_string(),
                parent_agent_id: Some("agent-root-live".to_string()),
                parent_sub_run_id: Some("subrun-root-live".to_string()),
                agent_profile: "review".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
            tool_call_id: Some("call-live".to_string()),
            source: ParsedSubRunStatusSource::Live,
            result: None,
            step_count: None,
            estimated_tokens: None,
            resolved_overrides: None,
            resolved_limits: None,
        };
        let durable = ParsedSubRunStatus {
            handle: SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-durable".to_string(),
                session_id: "session-parent".to_string(),
                child_session_id: Some("session-child".to_string()),
                depth: 1,
                parent_turn_id: "turn-durable".to_string(),
                parent_agent_id: Some("agent-root-durable".to_string()),
                parent_sub_run_id: Some("subrun-root-durable".to_string()),
                agent_profile: "review".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
            },
            tool_call_id: Some("call-durable".to_string()),
            source: ParsedSubRunStatusSource::Durable,
            result: Some(SubRunResult {
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                handoff: None,
                failure: None,
            }),
            step_count: Some(5),
            estimated_tokens: Some(256),
            resolved_overrides: Some(ResolvedSubagentContextOverrides::default()),
            resolved_limits: Some(ResolvedExecutionLimitsSnapshot::default()),
        };

        let merged = overlay_live_snapshot_on_durable(live, durable);

        assert_eq!(merged.source, ParsedSubRunStatusSource::Live);
        assert_eq!(merged.handle.session_id, "session-parent");
        assert_eq!(merged.handle.lifecycle, AgentLifecycleStatus::Running);
        assert_eq!(merged.handle.last_turn_outcome, None);
        assert_eq!(merged.tool_call_id.as_deref(), Some("call-live"));
        assert_eq!(merged.step_count, Some(5));
        assert_eq!(merged.estimated_tokens, Some(256));
    }

    #[test]
    fn find_subrun_status_in_events_returns_none_when_missing() {
        let unrelated = StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-2".to_string(),
                    "turn-1".to_string(),
                    "review".to_string(),
                    "subrun-2".to_string(),
                    None,
                    SubRunStorageMode::IndependentSession,
                    None,
                ),
                payload: StorageEventPayload::SubRunStarted {
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot {
                        allowed_tools: vec!["readFile".to_string()],
                    },
                    timestamp: None,
                },
            },
        };

        assert!(find_subrun_status_in_events(&[unrelated], "session-1", "subrun-1").is_none());
    }

    #[test]
    fn subrun_lifecycle_event_builders_produce_correct_event_shape() {
        let handle = SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: Some("child-1".to_string()),
            depth: 2,
            parent_turn_id: "turn-parent".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            parent_sub_run_id: Some("subrun-parent".to_string()),
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };
        let agent = AgentEventContext::sub_run(
            "agent-1".to_string(),
            "turn-parent".to_string(),
            "review".to_string(),
            "subrun-1".to_string(),
            Some("subrun-parent".to_string()),
            SubRunStorageMode::IndependentSession,
            Some("child-1".to_string()),
        );
        let started = build_subrun_started_event(
            "turn-parent",
            agent.clone(),
            &handle,
            Some("call-1".to_string()),
            ResolvedSubagentContextOverrides::default(),
            ResolvedExecutionLimitsSnapshot::default(),
        );
        let finished = build_subrun_finished_event(
            "turn-parent",
            agent,
            &handle,
            Some("call-1".to_string()),
            SubRunResult {
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                handoff: None,
                failure: None,
            },
            3,
            42,
        );

        match started {
            StorageEvent {
                turn_id,
                payload: StorageEventPayload::SubRunStarted { .. },
                ..
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-parent"));
            },
            _ => panic!("expected subrun started event"),
        }
        match finished {
            StorageEvent {
                turn_id,
                payload:
                    StorageEventPayload::SubRunFinished {
                        step_count,
                        estimated_tokens,
                        ..
                    },
                ..
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-parent"));
                assert_eq!(step_count, 3);
                assert_eq!(estimated_tokens, 42);
            },
            _ => panic!("expected subrun finished event"),
        }
    }

    #[test]
    fn resolve_subrun_status_snapshot_prefers_owned_live_handle() {
        let live = snapshot_from_active_handle(SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-live".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: None,
            depth: 1,
            parent_turn_id: "turn-1".to_string(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        });
        let durable = ParsedSubRunStatus {
            handle: SubRunHandle {
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                ..live.handle.clone()
            },
            source: ParsedSubRunStatusSource::Durable,
            result: Some(SubRunResult {
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                handoff: None,
                failure: None,
            }),
            step_count: Some(4),
            estimated_tokens: Some(21),
            resolved_overrides: None,
            resolved_limits: None,
            tool_call_id: Some("call-1".to_string()),
        };

        let resolved = resolve_subrun_status_snapshot(
            "session-1",
            Some(live.handle.clone()),
            Some(durable),
            str::to_string,
        )
        .expect("snapshot should resolve");

        assert_eq!(resolved.source, ParsedSubRunStatusSource::Live);
        assert_eq!(resolved.handle.lifecycle, AgentLifecycleStatus::Running);
        assert_eq!(resolved.handle.last_turn_outcome, None);
        assert_eq!(resolved.step_count, Some(4));
    }

    #[test]
    fn resolve_cancel_subrun_resolution_distinguishes_live_and_finalized_states() {
        let live = SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: None,
            depth: 1,
            parent_turn_id: "turn-1".to_string(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };
        let durable = ParsedSubRunStatus {
            handle: SubRunHandle {
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                ..live.clone()
            },
            source: ParsedSubRunStatusSource::Durable,
            result: None,
            step_count: None,
            estimated_tokens: None,
            resolved_overrides: None,
            resolved_limits: None,
            tool_call_id: None,
        };

        assert_eq!(
            resolve_cancel_subrun_resolution(
                "session-1",
                Some(&live),
                Some(&durable),
                str::to_string,
            ),
            CancelSubRunResolution::CancelLive
        );
        assert_eq!(
            resolve_cancel_subrun_resolution("session-2", None, Some(&durable), str::to_string,),
            CancelSubRunResolution::AlreadyFinalized
        );
        assert_eq!(
            resolve_cancel_subrun_resolution("session-2", None, None, str::to_string,),
            CancelSubRunResolution::Missing
        );
    }

    #[test]
    fn resolve_cancel_subrun_resolution_allows_parent_session_to_cancel_independent_child_live_handle()
     {
        let live = SubRunHandle {
            sub_run_id: "subrun-child-1".to_string(),
            agent_id: "agent-child-1".to_string(),
            session_id: "session-child-1".to_string(),
            child_session_id: Some("session-child-1".to_string()),
            depth: 1,
            parent_turn_id: "turn-parent-1".to_string(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };
        let durable = ParsedSubRunStatus {
            handle: SubRunHandle {
                session_id: "session-parent-1".to_string(),
                ..live.clone()
            },
            source: ParsedSubRunStatusSource::Durable,
            result: None,
            step_count: None,
            estimated_tokens: None,
            resolved_overrides: None,
            resolved_limits: None,
            tool_call_id: None,
        };

        assert_eq!(
            resolve_cancel_subrun_resolution(
                "session-parent-1",
                Some(&live),
                Some(&durable),
                str::to_string,
            ),
            CancelSubRunResolution::CancelLive
        );
    }

    #[test]
    fn build_child_session_node_uses_stable_parent_and_open_session_identity() {
        let child = SubRunHandle {
            sub_run_id: "subrun-11".to_string(),
            agent_id: "agent-11".to_string(),
            session_id: "session-child-11".to_string(),
            child_session_id: Some("session-child-11".to_string()),
            depth: 1,
            parent_turn_id: "turn-parent-11".to_string(),
            parent_agent_id: Some("agent-parent-11".to_string()),
            parent_sub_run_id: Some("subrun-parent-11".to_string()),
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };

        let node = build_child_session_node(
            &child,
            "session-parent-11",
            "turn-parent-11",
            Some("call-11".to_string()),
        );

        assert_eq!(node.session_id, "session-parent-11");
        assert_eq!(node.parent_session_id, "session-parent-11");
        assert_eq!(node.child_session_id, "session-child-11");
        assert_eq!(node.sub_run_id, "subrun-11");
        assert_eq!(node.parent_agent_id.as_deref(), Some("agent-parent-11"));
        assert_eq!(node.parent_sub_run_id.as_deref(), Some("subrun-parent-11"));
        assert_eq!(node.created_by_tool_call_id.as_deref(), Some("call-11"));
    }

    #[test]
    fn build_child_session_notification_reuses_child_ref_and_open_session_id() {
        let child = SubRunHandle {
            sub_run_id: "subrun-12".to_string(),
            agent_id: "agent-12".to_string(),
            session_id: "session-parent-12".to_string(),
            child_session_id: None,
            depth: 1,
            parent_turn_id: "turn-parent-12".to_string(),
            parent_agent_id: Some("agent-parent-12".to_string()),
            parent_sub_run_id: Some("subrun-parent-12".to_string()),
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
        };
        let node = build_child_session_node(
            &child,
            "session-parent-12",
            "turn-parent-12",
            Some("call-12".to_string()),
        );

        let notification = build_child_session_notification(
            &node,
            "child-started:subrun-12",
            ChildSessionNotificationKind::Started,
            "child started",
            AgentLifecycleStatus::Running,
            None,
        );

        assert_eq!(notification.notification_id, "child-started:subrun-12");
        assert_eq!(notification.child_ref.agent_id, "agent-12");
        assert_eq!(notification.child_ref.sub_run_id, "subrun-12");
        assert_eq!(
            notification.child_ref.parent_sub_run_id.as_deref(),
            Some("subrun-parent-12")
        );
        assert_eq!(notification.child_ref.open_session_id, "session-parent-12");
        assert_eq!(notification.child_ref.open_session_id, "session-parent-12");
        assert_eq!(notification.source_tool_call_id.as_deref(), Some("call-12"));
    }
}
