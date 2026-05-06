use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, ChildSessionNotification, CloseAgentParams,
    CollaborationResult, InputBatchAckedPayload, InputBatchStartedPayload, InputDiscardedPayload,
    InputQueuedPayload, ObserveParams, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, Result, SendAgentParams, SpawnAgentParams, StorageEvent,
    StorageEventPayload, StoredEvent, SubRunResult, ToolContext,
};
use async_trait::async_trait;
use chrono::Utc;

use crate::{EventTranslator, SessionCatalog, state};

/// `host-session` 对外暴露的 sub-run owner bridge。
///
/// 新调用方必须从 `host-session` 导入它，避免把协作执行/read-model 合同挂在 core 顶层。
pub type SubRunHandle = astrcode_core::agent::lineage::SubRunHandle;

/// 子 Agent 启动执行端口。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// 启动子 Agent，返回结构化执行结果。
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}

/// 子 Agent 协作执行端口（send / close / observe）。
#[async_trait]
pub trait CollaborationExecutor: Send + Sync {
    /// 发送追加消息给既有子 Agent。
    async fn send(&self, params: SendAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 关闭目标子 Agent（级联关闭其子树）。
    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 观测目标子 Agent 快照。
    async fn observe(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;
}

pub fn agent_event_context_from_subrun(handle: &SubRunHandle) -> AgentEventContext {
    AgentEventContext::from(handle)
}

impl SessionCatalog {
    pub async fn append_subrun_started(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
        resolved_overrides: Option<ResolvedSubagentContextOverrides>,
        source_tool_call_id: Option<String>,
    ) -> Result<Option<StoredEvent>> {
        let Some(event) = subrun_started_event(
            turn_id,
            &agent,
            resolved_limits,
            resolved_overrides,
            source_tool_call_id,
        ) else {
            return Ok(None);
        };
        self.append_collaboration_event(session_id, event)
            .await
            .map(Some)
    }

    pub async fn append_subrun_finished(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        result: SubRunResult,
        stats: SubRunFinishStats,
        source_tool_call_id: Option<String>,
    ) -> Result<Option<StoredEvent>> {
        let Some(event) =
            subrun_finished_event(turn_id, &agent, result, stats, source_tool_call_id)
        else {
            return Ok(None);
        };
        self.append_collaboration_event(session_id, event)
            .await
            .map(Some)
    }

    pub async fn append_child_session_notification(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        notification: ChildSessionNotification,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::ChildSessionNotification {
                notification,
                timestamp: Some(Utc::now()),
            },
        )
        .await
    }

    pub async fn append_agent_collaboration_fact(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        fact: AgentCollaborationFact,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::AgentCollaborationFact {
                fact,
                timestamp: Some(Utc::now()),
            },
        )
        .await
    }

    pub async fn append_agent_input_queued(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputQueuedPayload,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::AgentInputQueued { payload },
        )
        .await
    }

    pub async fn append_agent_input_batch_started(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchStartedPayload,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::AgentInputBatchStarted { payload },
        )
        .await
    }

    pub async fn append_agent_input_batch_acked(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchAckedPayload,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::AgentInputBatchAcked { payload },
        )
        .await
    }

    pub async fn append_agent_input_discarded(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputDiscardedPayload,
    ) -> Result<StoredEvent> {
        self.append_collaboration_payload(
            session_id,
            Some(turn_id.to_string()),
            agent,
            StorageEventPayload::AgentInputDiscarded { payload },
        )
        .await
    }

    async fn append_collaboration_payload(
        &self,
        session_id: &astrcode_core::SessionId,
        turn_id: Option<String>,
        agent: AgentEventContext,
        payload: StorageEventPayload,
    ) -> Result<StoredEvent> {
        self.append_collaboration_event(
            session_id,
            StorageEvent {
                turn_id,
                agent,
                payload,
            },
        )
        .await
    }

    async fn append_collaboration_event(
        &self,
        session_id: &astrcode_core::SessionId,
        event: StorageEvent,
    ) -> Result<StoredEvent> {
        let loaded = self.ensure_loaded_session(session_id).await?;
        let phase = loaded.state.current_phase()?;
        let mut translator = EventTranslator::new(phase);
        state::append_and_broadcast(&loaded.state, &event, &mut translator).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubRunFinishStats {
    pub step_count: u32,
    pub estimated_tokens: u64,
}

pub fn subrun_started_event(
    turn_id: &str,
    agent: &AgentEventContext,
    resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunStarted {
            tool_call_id: source_tool_call_id,
            resolved_overrides: resolved_overrides.unwrap_or_default(),
            resolved_limits: resolved_limits.unwrap_or_default(),
            timestamp: Some(Utc::now()),
        },
    })
}

pub fn subrun_finished_event(
    turn_id: &str,
    agent: &AgentEventContext,
    result: SubRunResult,
    stats: SubRunFinishStats,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunFinished {
            tool_call_id: source_tool_call_id,
            result,
            step_count: stats.step_count,
            estimated_tokens: stats.estimated_tokens,
            timestamp: Some(Utc::now()),
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubRunStatus {
    #[default]
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeliveryState {
    #[default]
    Pending,
    Delivered,
    Acknowledged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultDeliveryState {
    #[default]
    Pending,
    Delivered,
    Dropped,
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEventContext, AgentLifecycleStatus, CompletedSubRunOutcome,
        ResolvedExecutionLimitsSnapshot, StorageEventPayload, SubRunHandoff, SubRunResult,
        SubRunStorageMode,
    };

    use super::{SubRunFinishStats, SubRunHandle, subrun_finished_event, subrun_started_event};

    #[test]
    fn owner_bridge_exposes_subrun_shape() {
        let handle = SubRunHandle {
            sub_run_id: "subrun-1".into(),
            agent_id: "agent-child".into(),
            session_id: "session-parent".into(),
            child_session_id: Some("session-child".into()),
            depth: 1,
            parent_turn_id: "turn-parent".into(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
            agent_profile: "default".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Pending,
            last_turn_outcome: None,
            resolved_limits: ResolvedExecutionLimitsSnapshot,
            delegation: None,
        };

        assert_eq!(handle.sub_run_id.as_str(), "subrun-1");
        assert_eq!(handle.open_session_id().as_str(), "session-child");
    }

    fn subrun_agent() -> AgentEventContext {
        AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "reviewer",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some("session-child".into()),
        )
    }

    #[test]
    fn subrun_lifecycle_events_require_subrun_context() {
        assert!(
            subrun_started_event("turn-1", &AgentEventContext::default(), None, None, None)
                .is_none()
        );

        let started =
            subrun_started_event("turn-1", &subrun_agent(), None, None, Some("call-1".into()))
                .expect("subrun context should emit started event");
        assert!(matches!(
            started.payload,
            StorageEventPayload::SubRunStarted { tool_call_id, .. }
                if tool_call_id.as_deref() == Some("call-1")
        ));
    }

    #[test]
    fn subrun_finished_event_preserves_result_contract() {
        let event = subrun_finished_event(
            "turn-1",
            &subrun_agent(),
            SubRunResult::Completed {
                outcome: CompletedSubRunOutcome::Completed,
                handoff: SubRunHandoff {
                    findings: Vec::new(),
                    artifacts: Vec::new(),
                    delivery: None,
                },
            },
            SubRunFinishStats {
                step_count: 3,
                estimated_tokens: 99,
            },
            None,
        )
        .expect("subrun context should emit finish event");

        assert!(matches!(
            event.payload,
            StorageEventPayload::SubRunFinished {
                step_count: 3,
                estimated_tokens: 99,
                ..
            }
        ));
    }
}
