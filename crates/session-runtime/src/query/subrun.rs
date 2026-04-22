use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, InvocationKind, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, StorageEventPayload, StoredEvent, SubRunHandle,
    SubRunStorageMode,
};

use crate::{SubRunStatusSnapshot, SubRunStatusSource};

#[derive(Debug, Clone)]
struct DurableSubRunStatusProjection {
    handle: SubRunHandle,
    tool_call_id: Option<String>,
    result: Option<astrcode_core::SubRunResult>,
    step_count: Option<u32>,
    estimated_tokens: Option<u64>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
}

pub(crate) fn project_durable_subrun_status_snapshot(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    stored_events: &[StoredEvent],
) -> Option<SubRunStatusSnapshot> {
    let mut projection: Option<DurableSubRunStatusProjection> = None;

    for stored in stored_events {
        let agent = &stored.event.agent;
        if !matches_requested_subrun(agent, requested_subrun_id) {
            continue;
        }

        match &stored.event.payload {
            StorageEventPayload::SubRunStarted {
                tool_call_id,
                resolved_overrides,
                resolved_limits,
                ..
            } => {
                projection = Some(DurableSubRunStatusProjection {
                    handle: build_subrun_handle(
                        parent_session_id,
                        child_session_id,
                        requested_subrun_id,
                        agent,
                        AgentLifecycleStatus::Running,
                        None,
                        resolved_limits.clone(),
                    ),
                    tool_call_id: tool_call_id.clone(),
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: Some(resolved_overrides.clone()),
                });
            },
            StorageEventPayload::SubRunFinished {
                tool_call_id,
                result,
                step_count,
                estimated_tokens,
                ..
            } => {
                let entry = projection.get_or_insert_with(|| DurableSubRunStatusProjection {
                    handle: build_subrun_handle(
                        parent_session_id,
                        child_session_id,
                        requested_subrun_id,
                        agent,
                        result.status().lifecycle(),
                        result.status().last_turn_outcome(),
                        ResolvedExecutionLimitsSnapshot,
                    ),
                    tool_call_id: None,
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: None,
                });
                entry.tool_call_id = tool_call_id.clone().or_else(|| entry.tool_call_id.clone());
                entry.handle.lifecycle = result.status().lifecycle();
                entry.handle.last_turn_outcome = result.status().last_turn_outcome();
                entry.result = Some(result.clone());
                entry.step_count = Some(*step_count);
                entry.estimated_tokens = Some(*estimated_tokens);
            },
            _ => {},
        }
    }

    projection.map(|projection| SubRunStatusSnapshot {
        handle: projection.handle,
        tool_call_id: projection.tool_call_id,
        source: SubRunStatusSource::Durable,
        result: projection.result,
        step_count: projection.step_count,
        estimated_tokens: projection.estimated_tokens,
        resolved_overrides: projection.resolved_overrides,
    })
}

fn build_subrun_handle(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    agent: &AgentEventContext,
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
) -> SubRunHandle {
    SubRunHandle {
        sub_run_id: agent
            .sub_run_id
            .clone()
            .unwrap_or_else(|| requested_subrun_id.to_string().into()),
        agent_id: agent
            .agent_id
            .clone()
            .unwrap_or_else(|| requested_subrun_id.to_string().into()),
        session_id: parent_session_id.to_string().into(),
        child_session_id: Some(
            agent
                .child_session_id
                .clone()
                .unwrap_or_else(|| child_session_id.to_string().into()),
        ),
        depth: 1,
        parent_turn_id: agent.parent_turn_id.clone().unwrap_or_default(),
        parent_agent_id: None,
        parent_sub_run_id: agent.parent_sub_run_id.clone(),
        lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
        agent_profile: agent
            .agent_profile
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        storage_mode: agent
            .storage_mode
            .unwrap_or(SubRunStorageMode::IndependentSession),
        lifecycle,
        last_turn_outcome,
        resolved_limits,
        delegation: None,
    }
}

fn matches_requested_subrun(agent: &AgentEventContext, requested_subrun_id: &str) -> bool {
    if agent.invocation_kind != Some(InvocationKind::SubRun) {
        return false;
    }

    agent.sub_run_id.as_deref() == Some(requested_subrun_id)
        || agent.agent_id.as_deref() == Some(requested_subrun_id)
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        ArtifactRef, CompletedParentDeliveryPayload, CompletedSubRunOutcome, ForkMode,
        ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
        ParentDeliveryTerminalSemantics, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, StorageEvent, StorageEventPayload, SubRunHandoff,
        SubRunResult, SubRunStorageMode,
    };

    use super::project_durable_subrun_status_snapshot;
    use crate::{AgentEventContext, StoredEvent};

    #[test]
    fn durable_subrun_projection_preserves_typed_handoff_delivery() {
        let child_agent = AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "reviewer",
            "subrun-child",
            Some("subrun-parent".into()),
            SubRunStorageMode::IndependentSession,
            Some("session-child".into()),
        );
        let explicit_delivery = ParentDelivery {
            idempotency_key: "delivery-explicit".to_string(),
            origin: ParentDeliveryOrigin::Explicit,
            terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
            source_turn_id: Some("turn-child".to_string()),
            payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                message: "显式交付".to_string(),
                findings: vec!["finding-1".to_string()],
                artifacts: vec![ArtifactRef {
                    kind: "session".to_string(),
                    id: "session-child".to_string(),
                    label: "Child Session".to_string(),
                    session_id: Some("session-child".to_string()),
                    storage_seq: None,
                    uri: None,
                }],
            }),
        };
        let stored_events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-child".to_string()),
                agent: child_agent.clone(),
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: Some("call-1".to_string()),
                    result: SubRunResult::Completed {
                        outcome: CompletedSubRunOutcome::Completed,
                        handoff: SubRunHandoff {
                            findings: vec!["finding-1".to_string()],
                            artifacts: vec![ArtifactRef {
                                kind: "session".to_string(),
                                id: "session-child".to_string(),
                                label: "Child Session".to_string(),
                                session_id: Some("session-child".to_string()),
                                storage_seq: None,
                                uri: None,
                            }],
                            delivery: Some(explicit_delivery.clone()),
                        },
                    },
                    timestamp: Some(chrono::Utc::now()),
                    step_count: 3,
                    estimated_tokens: 120,
                },
            },
        }];

        let projection = project_durable_subrun_status_snapshot(
            "session-parent",
            "session-child",
            "subrun-child",
            &stored_events,
        )
        .expect("projection should exist");

        let result = projection.result.expect("durable result should exist");
        let handoff = match result {
            SubRunResult::Running { handoff } | SubRunResult::Completed { handoff, .. } => handoff,
            SubRunResult::Failed { .. } => panic!("expected successful durable handoff"),
        };
        let delivery = handoff
            .delivery
            .expect("typed delivery should survive durable projection");
        assert_eq!(delivery.idempotency_key, "delivery-explicit");
        assert_eq!(delivery.origin, ParentDeliveryOrigin::Explicit);
        assert_eq!(
            delivery.terminal_semantics,
            ParentDeliveryTerminalSemantics::Terminal
        );
        match delivery.payload {
            ParentDeliveryPayload::Completed(payload) => {
                assert_eq!(payload.message, "显式交付");
                assert_eq!(payload.findings, vec!["finding-1".to_string()]);
            },
            payload => panic!("unexpected delivery payload: {payload:?}"),
        }
    }

    #[test]
    fn resolved_overrides_projection_preserves_fork_mode() {
        let projection = project_durable_subrun_status_snapshot(
            "session-parent",
            "session-child",
            "subrun-child",
            &[StoredEvent {
                storage_seq: 1,
                event: StorageEvent {
                    turn_id: Some("turn-child".to_string()),
                    agent: AgentEventContext::sub_run(
                        "agent-child",
                        "turn-parent",
                        "reviewer",
                        "subrun-child",
                        Some("subrun-parent".into()),
                        SubRunStorageMode::IndependentSession,
                        Some("session-child".into()),
                    ),
                    payload: StorageEventPayload::SubRunStarted {
                        tool_call_id: Some("call-1".to_string()),
                        resolved_overrides: ResolvedSubagentContextOverrides {
                            fork_mode: Some(ForkMode::LastNTurns(7)),
                            ..ResolvedSubagentContextOverrides::default()
                        },
                        resolved_limits: ResolvedExecutionLimitsSnapshot,
                        timestamp: Some(chrono::Utc::now()),
                    },
                },
            }],
        )
        .expect("projection should exist");

        assert_eq!(
            projection
                .resolved_overrides
                .expect("resolved overrides should exist")
                .fork_mode,
            Some(ForkMode::LastNTurns(7))
        );
    }
}
