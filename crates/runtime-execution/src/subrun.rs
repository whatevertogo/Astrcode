use astrcode_core::{
    AgentEventContext, AgentStatus, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, StorageEvent, StoredEvent, SubRunHandle, SubRunOutcome,
    SubRunResult, SubRunStorageMode,
};

#[derive(Debug, Clone)]
pub struct ParsedSubRunStatus {
    pub handle: SubRunHandle,
    pub result: Option<SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
}

pub fn snapshot_from_active_handle(handle: SubRunHandle) -> ParsedSubRunStatus {
    ParsedSubRunStatus {
        handle,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: None,
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
    let mut resolved_overrides = None;
    let mut resolved_limits = None;
    let mut finished: Option<(SubRunResult, u32, u64)> = None;

    for stored in events {
        match &stored.event {
            StorageEvent::SubRunStarted {
                agent,
                resolved_overrides: started_overrides,
                resolved_limits: started_limits,
                ..
            } if agent.sub_run_id.as_deref() == Some(sub_run_id) => {
                started_agent = Some(agent.clone());
                resolved_overrides = Some(started_overrides.clone());
                resolved_limits = Some(started_limits.clone());
            },
            StorageEvent::SubRunFinished {
                agent,
                result,
                step_count,
                estimated_tokens,
                ..
            } if agent.sub_run_id.as_deref() == Some(sub_run_id) => {
                if started_agent.is_none() {
                    started_agent = Some(agent.clone());
                }
                finished = Some((result.clone(), *step_count, *estimated_tokens));
            },
            _ => {},
        }
    }

    started_agent.map(|agent| ParsedSubRunStatus {
        handle: build_replayed_handle(session_id, sub_run_id, agent, finished.as_ref()),
        result: finished.as_ref().map(|(result, _, _)| result.clone()),
        step_count: finished.as_ref().map(|(_, step_count, _)| *step_count),
        estimated_tokens: finished
            .as_ref()
            .map(|(_, _, estimated_tokens)| *estimated_tokens),
        resolved_overrides,
        resolved_limits,
    })
}

fn build_replayed_handle(
    session_id: &str,
    sub_run_id: &str,
    agent: AgentEventContext,
    finished: Option<&(SubRunResult, u32, u64)>,
) -> SubRunHandle {
    SubRunHandle {
        sub_run_id: sub_run_id.to_string(),
        agent_id: agent
            .agent_id
            .unwrap_or_else(|| "unknown-agent".to_string()),
        session_id: session_id.to_string(),
        child_session_id: agent.child_session_id.clone(),
        depth: 1,
        parent_turn_id: agent.parent_turn_id.clone(),
        parent_agent_id: None,
        agent_profile: agent
            .agent_profile
            .unwrap_or_else(|| "unknown-profile".to_string()),
        storage_mode: agent
            .storage_mode
            .unwrap_or(SubRunStorageMode::SharedSession),
        status: finished
            .map(|(result, _, _)| status_from_result(result))
            .unwrap_or(AgentStatus::Pending),
    }
}

fn status_from_result(result: &SubRunResult) -> AgentStatus {
    match result.status {
        SubRunOutcome::Completed | SubRunOutcome::TokenExceeded => AgentStatus::Completed,
        SubRunOutcome::Aborted => AgentStatus::Cancelled,
        SubRunOutcome::Failed { .. } => AgentStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
        StorageEvent, StoredEvent, SubRunHandle, SubRunOutcome, SubRunResult, SubRunStorageMode,
    };

    use super::{find_subrun_status_in_events, snapshot_from_active_handle};

    #[test]
    fn snapshot_from_active_handle_keeps_fast_path_shape() {
        let handle = SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: Some("child-1".to_string()),
            depth: 2,
            parent_turn_id: Some("turn-1".to_string()),
            parent_agent_id: Some("parent-agent".to_string()),
            agent_profile: "review".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            status: astrcode_core::AgentStatus::Running,
        };

        let snapshot = snapshot_from_active_handle(handle.clone());

        assert_eq!(snapshot.handle, handle);
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
            SubRunStorageMode::IndependentSession,
            Some("child-1".to_string()),
        );
        let overrides = ResolvedSubagentContextOverrides {
            storage_mode: SubRunStorageMode::IndependentSession,
            ..Default::default()
        };
        let limits = ResolvedExecutionLimitsSnapshot {
            max_steps: Some(3),
            token_budget: Some(4000),
            allowed_tools: vec!["readFile".to_string()],
        };
        let result = SubRunResult {
            status: SubRunOutcome::Completed,
            summary: "done".to_string(),
            artifacts: Vec::new(),
            findings: vec!["ok".to_string()],
        };
        let events = vec![
            StoredEvent {
                storage_seq: 1,
                event: StorageEvent::SubRunStarted {
                    turn_id: Some("turn-1".to_string()),
                    agent: agent.clone(),
                    resolved_overrides: overrides.clone(),
                    resolved_limits: limits.clone(),
                    timestamp: None,
                },
            },
            StoredEvent {
                storage_seq: 2,
                event: StorageEvent::SubRunFinished {
                    turn_id: Some("turn-1".to_string()),
                    agent,
                    result: result.clone(),
                    step_count: 2,
                    estimated_tokens: 123,
                    timestamp: None,
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
        assert_eq!(
            snapshot.handle.status,
            astrcode_core::AgentStatus::Completed
        );
        assert_eq!(
            snapshot.result.as_ref().map(|item| item.summary.as_str()),
            Some("done")
        );
        assert_eq!(snapshot.step_count, Some(2));
        assert_eq!(snapshot.estimated_tokens, Some(123));
        assert_eq!(snapshot.resolved_overrides, Some(overrides));
        assert_eq!(snapshot.resolved_limits, Some(limits));
    }

    #[test]
    fn find_subrun_status_in_events_returns_none_when_missing() {
        let unrelated = StoredEvent {
            storage_seq: 1,
            event: StorageEvent::SubRunStarted {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-2".to_string(),
                    "turn-1".to_string(),
                    "review".to_string(),
                    "subrun-2".to_string(),
                    SubRunStorageMode::SharedSession,
                    None,
                ),
                resolved_overrides: ResolvedSubagentContextOverrides::default(),
                resolved_limits: ResolvedExecutionLimitsSnapshot {
                    max_steps: Some(1),
                    token_budget: None,
                    allowed_tools: vec!["readFile".to_string()],
                },
                timestamp: None,
            },
        };

        assert!(find_subrun_status_in_events(&[unrelated], "session-1", "subrun-1").is_none());
    }
}
