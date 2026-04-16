use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AgentTurnOutcome, InvocationKind,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, StorageEventPayload,
    StoredEvent, SubRunResult, SubRunStorageMode,
};
use astrcode_kernel::SubRunStatusView;

/// ! 这是 App 的用例实现，不是 ports
use crate::{
    AgentExecuteSummary, App, ApplicationError, RootExecutionRequest, SubRunStatusSourceSummary,
    SubRunStatusSummary, summarize_session_meta,
};

impl App {
    // ── Agent 控制用例（通过 kernel 稳定控制合同） ──────────

    /// 查询子运行状态。
    pub async fn get_subrun_status(
        &self,
        agent_id: &str,
    ) -> Result<Option<SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("agentId", agent_id)?;
        Ok(self.kernel.query_subrun_status(agent_id).await)
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn get_root_agent_status(
        &self,
        session_id: &str,
    ) -> Result<Option<SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        Ok(self.kernel.query_root_status(session_id).await)
    }

    /// 列出所有 agent 状态。
    pub async fn list_agent_statuses(&self) -> Vec<SubRunStatusView> {
        self.kernel.list_statuses().await
    }

    /// 执行 root agent 并返回共享摘要输入。
    pub async fn execute_root_agent_summary(
        &self,
        request: RootExecutionRequest,
    ) -> Result<AgentExecuteSummary, ApplicationError> {
        let accepted = self.execute_root_agent(request).await?;
        let session_id = accepted.session_id.to_string();
        Ok(AgentExecuteSummary {
            accepted: true,
            message: format!(
                "agent '{}' execution accepted; subscribe to \
                 /api/v1/conversation/sessions/{}/stream for progress",
                accepted.agent_id.as_deref().unwrap_or("unknown-agent"),
                session_id
            ),
            session_id: Some(session_id),
            turn_id: Some(accepted.turn_id.to_string()),
            agent_id: accepted.agent_id.map(|value| value.to_string()),
        })
    }

    /// 查询指定 session/sub-run 的共享状态摘要。
    pub async fn get_subrun_status_summary(
        &self,
        session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<SubRunStatusSummary, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        self.validate_non_empty("subRunId", requested_subrun_id)?;

        if let Some(view) = self.get_subrun_status(requested_subrun_id).await? {
            return Ok(summarize_live_subrun_status(view, session_id.to_string()));
        }

        if let Some(view) = self.get_root_agent_status(session_id).await? {
            if view.sub_run_id == requested_subrun_id {
                return Ok(summarize_live_subrun_status(view, session_id.to_string()));
            }
            return Err(ApplicationError::NotFound(format!(
                "subrun '{}' not found in session '{}'",
                requested_subrun_id, session_id
            )));
        }

        if let Some(summary) = self
            .durable_subrun_status_summary(session_id, requested_subrun_id)
            .await?
        {
            return Ok(summary);
        }

        Ok(default_subrun_status_summary(
            session_id.to_string(),
            requested_subrun_id.to_string(),
        ))
    }

    async fn durable_subrun_status_summary(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> Result<Option<SubRunStatusSummary>, ApplicationError> {
        let child_sessions = self
            .list_sessions()
            .await?
            .into_iter()
            .map(summarize_session_meta)
            .filter(|summary| summary.parent_session_id.as_deref() == Some(parent_session_id))
            .collect::<Vec<_>>();

        for child_session in child_sessions {
            let stored_events = self
                .session_stored_events(&child_session.session_id)
                .await?;
            if let Some(summary) = project_durable_subrun_status_summary(
                parent_session_id,
                &child_session.session_id,
                requested_subrun_id,
                &stored_events,
            ) {
                return Ok(Some(summary));
            }
        }

        Ok(None)
    }

    /// 关闭 agent 及其子树。
    pub async fn close_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<astrcode_kernel::CloseSubtreeResult, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        self.validate_non_empty("agentId", agent_id)?;
        let Some(handle) = self.kernel.get_handle(agent_id).await else {
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found",
                agent_id
            )));
        };
        if handle.session_id.as_str() != session_id {
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found in session '{}'",
                agent_id, session_id
            )));
        }
        self.kernel
            .close_subtree(agent_id)
            .await
            .map_err(|error| ApplicationError::Internal(error.to_string()))
    }
}

fn summarize_live_subrun_status(view: SubRunStatusView, session_id: String) -> SubRunStatusSummary {
    SubRunStatusSummary {
        sub_run_id: view.sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceSummary::Live,
        agent_id: view.agent_id,
        agent_profile: view.agent_profile,
        session_id,
        child_session_id: view.child_session_id,
        depth: view.depth,
        parent_agent_id: view.parent_agent_id,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageMode::IndependentSession,
        lifecycle: view.lifecycle,
        last_turn_outcome: view.last_turn_outcome,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(view.resolved_limits),
    }
}

fn default_subrun_status_summary(session_id: String, sub_run_id: String) -> SubRunStatusSummary {
    SubRunStatusSummary {
        sub_run_id,
        tool_call_id: None,
        source: SubRunStatusSourceSummary::Live,
        agent_id: "root-agent".to_string(),
        agent_profile: "default".to_string(),
        session_id,
        child_session_id: None,
        depth: 0,
        parent_agent_id: None,
        parent_sub_run_id: None,
        storage_mode: SubRunStorageMode::IndependentSession,
        lifecycle: AgentLifecycleStatus::Idle,
        last_turn_outcome: None,
        result: None,
        step_count: None,
        estimated_tokens: None,
        resolved_overrides: None,
        resolved_limits: Some(ResolvedExecutionLimitsSnapshot {
            allowed_tools: Vec::new(),
            max_steps: None,
        }),
    }
}

#[derive(Debug, Clone)]
struct DurableSubRunStatusProjection {
    sub_run_id: String,
    tool_call_id: Option<String>,
    agent_id: String,
    agent_profile: String,
    child_session_id: String,
    depth: usize,
    parent_agent_id: Option<String>,
    parent_sub_run_id: Option<String>,
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<AgentTurnOutcome>,
    result: Option<SubRunResult>,
    step_count: Option<u32>,
    estimated_tokens: Option<u64>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
}

fn project_durable_subrun_status_summary(
    parent_session_id: &str,
    child_session_id: &str,
    requested_subrun_id: &str,
    stored_events: &[StoredEvent],
) -> Option<SubRunStatusSummary> {
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
                    sub_run_id: agent
                        .sub_run_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string().into())
                        .to_string(),
                    tool_call_id: tool_call_id.clone(),
                    agent_id: agent
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string().into())
                        .to_string(),
                    agent_profile: agent
                        .agent_profile
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    child_session_id: child_session_id.to_string(),
                    depth: 1,
                    parent_agent_id: None,
                    parent_sub_run_id: agent.parent_sub_run_id.clone().map(|id| id.to_string()),
                    lifecycle: AgentLifecycleStatus::Running,
                    last_turn_outcome: None,
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: Some(resolved_overrides.clone()),
                    resolved_limits: resolved_limits.clone(),
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
                    sub_run_id: agent
                        .sub_run_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string().into())
                        .to_string(),
                    tool_call_id: None,
                    agent_id: agent
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| requested_subrun_id.to_string().into())
                        .to_string(),
                    agent_profile: agent
                        .agent_profile
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    child_session_id: child_session_id.to_string(),
                    depth: 1,
                    parent_agent_id: None,
                    parent_sub_run_id: agent.parent_sub_run_id.clone().map(|id| id.to_string()),
                    lifecycle: result.status().lifecycle(),
                    last_turn_outcome: result.status().last_turn_outcome(),
                    result: None,
                    step_count: None,
                    estimated_tokens: None,
                    resolved_overrides: None,
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                });
                entry.tool_call_id = tool_call_id.clone().or_else(|| entry.tool_call_id.clone());
                entry.lifecycle = result.status().lifecycle();
                entry.last_turn_outcome = result.status().last_turn_outcome();
                entry.result = Some(result.clone());
                entry.step_count = Some(*step_count);
                entry.estimated_tokens = Some(*estimated_tokens);
            },
            _ => {},
        }
    }

    projection.map(|projection| SubRunStatusSummary {
        sub_run_id: projection.sub_run_id,
        tool_call_id: projection.tool_call_id,
        source: SubRunStatusSourceSummary::Durable,
        agent_id: projection.agent_id,
        agent_profile: projection.agent_profile,
        session_id: parent_session_id.to_string(),
        child_session_id: Some(projection.child_session_id),
        depth: projection.depth,
        parent_agent_id: projection.parent_agent_id,
        parent_sub_run_id: projection.parent_sub_run_id,
        storage_mode: SubRunStorageMode::IndependentSession,
        lifecycle: projection.lifecycle,
        last_turn_outcome: projection.last_turn_outcome,
        result: projection.result,
        step_count: projection.step_count,
        estimated_tokens: projection.estimated_tokens,
        resolved_overrides: projection.resolved_overrides,
        resolved_limits: Some(projection.resolved_limits),
    })
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
        AgentTurnOutcome, ArtifactRef, CompletedParentDeliveryPayload, CompletedSubRunOutcome,
        ForkMode, ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
        ParentDeliveryTerminalSemantics, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, StorageEvent, StorageEventPayload, SubRunResult,
        SubRunStorageMode,
    };

    use super::project_durable_subrun_status_summary;
    use crate::{AgentEventContext, StoredEvent, SubRunHandoff};

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

        let projection = project_durable_subrun_status_summary(
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
        let projection = project_durable_subrun_status_summary(
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
                        resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
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

    #[test]
    fn durable_subrun_projection_maps_token_exceeded_to_successful_handoff_result() {
        let child_agent = AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "reviewer",
            "subrun-child",
            Some("subrun-parent".into()),
            SubRunStorageMode::IndependentSession,
            Some("session-child".into()),
        );
        let stored_events = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent {
                turn_id: Some("turn-child".to_string()),
                agent: child_agent,
                payload: StorageEventPayload::SubRunFinished {
                    tool_call_id: Some("call-1".to_string()),
                    result: SubRunResult::Completed {
                        outcome: CompletedSubRunOutcome::TokenExceeded,
                        handoff: SubRunHandoff {
                            findings: vec!["partial-finding".to_string()],
                            artifacts: Vec::new(),
                            delivery: None,
                        },
                    },
                    timestamp: Some(chrono::Utc::now()),
                    step_count: 5,
                    estimated_tokens: 2048,
                },
            },
        }];

        let projection = project_durable_subrun_status_summary(
            "session-parent",
            "session-child",
            "subrun-child",
            &stored_events,
        )
        .expect("projection should exist");

        let result = projection.result.expect("durable result should exist");
        match result {
            SubRunResult::Completed { outcome, handoff } => {
                assert_eq!(outcome, CompletedSubRunOutcome::TokenExceeded);
                assert_eq!(handoff.findings, vec!["partial-finding".to_string()]);
            },
            other => panic!("expected token exceeded handoff result, got {other:?}"),
        }
        assert_eq!(
            projection.last_turn_outcome,
            Some(AgentTurnOutcome::TokenExceeded)
        );
    }
}
