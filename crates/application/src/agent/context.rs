//! Agent 协作事实记录与上下文构建。
//!
//! 从 agent/mod.rs 提取出的两个关注点：
//! - `CollaborationFactRecord`：记录一次协作动作（spawn/send/observe/close）的结构化事实
//! - `AgentOrchestrationService` 的上下文构建方法（root/child 的 event context）

use std::path::Path;

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
    AgentCollaborationPolicyContext, AgentEventContext, ChildExecutionIdentity, InvocationKind,
    ModeId, ResolvedExecutionLimitsSnapshot, Result, SubRunHandle, ToolContext,
};

use super::{
    AgentOrchestrationError, AgentOrchestrationService, IMPLICIT_ROOT_PROFILE_ID,
    root_execution_event_context, subrun_event_context,
};
use crate::governance_surface::{GOVERNANCE_POLICY_REVISION, collaboration_policy_context};

pub(crate) struct CollaborationFactRecord<'a> {
    pub(crate) action: AgentCollaborationActionKind,
    pub(crate) outcome: AgentCollaborationOutcomeKind,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) parent_agent_id: Option<String>,
    pub(crate) child: Option<&'a SubRunHandle>,
    pub(crate) delivery_id: Option<String>,
    pub(crate) reason_code: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) latency_ms: Option<u64>,
    pub(crate) source_tool_call_id: Option<String>,
    pub(crate) policy: Option<AgentCollaborationPolicyContext>,
    pub(crate) governance_revision: Option<String>,
    pub(crate) mode_id: Option<ModeId>,
}

impl<'a> CollaborationFactRecord<'a> {
    pub(crate) fn new(
        action: AgentCollaborationActionKind,
        outcome: AgentCollaborationOutcomeKind,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> Self {
        Self {
            action,
            outcome,
            session_id,
            turn_id,
            parent_agent_id: None,
            child: None,
            delivery_id: None,
            reason_code: None,
            summary: None,
            latency_ms: None,
            source_tool_call_id: None,
            policy: None,
            governance_revision: None,
            mode_id: None,
        }
    }

    pub(crate) fn parent_agent_id(mut self, parent_agent_id: Option<String>) -> Self {
        self.parent_agent_id = parent_agent_id;
        self
    }

    pub(crate) fn child(mut self, child: &'a SubRunHandle) -> Self {
        self.child = Some(child);
        self
    }

    pub(crate) fn delivery_id(mut self, delivery_id: impl Into<String>) -> Self {
        self.delivery_id = Some(delivery_id.into());
        self
    }

    pub(crate) fn reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    pub(crate) fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub(crate) fn latency_ms(mut self, latency_ms: u64) -> Self {
        self.latency_ms = Some(latency_ms);
        self
    }

    pub(crate) fn source_tool_call_id(mut self, source_tool_call_id: Option<String>) -> Self {
        self.source_tool_call_id = source_tool_call_id;
        self
    }

    pub(crate) fn policy(mut self, policy: AgentCollaborationPolicyContext) -> Self {
        self.policy = Some(policy);
        self
    }

    pub(crate) fn governance_revision(mut self, governance_revision: impl Into<String>) -> Self {
        self.governance_revision = Some(governance_revision.into());
        self
    }

    pub(crate) fn mode_id(mut self, mode_id: Option<ModeId>) -> Self {
        self.mode_id = mode_id;
        self
    }
}

pub(crate) struct ToolCollaborationContext {
    runtime: astrcode_core::ResolvedRuntimeConfig,
    session_id: String,
    turn_id: String,
    parent_agent_id: Option<String>,
    source_tool_call_id: Option<String>,
    policy: AgentCollaborationPolicyContext,
    governance_revision: String,
    mode_id: ModeId,
}

pub(crate) struct ToolCollaborationContextInput {
    pub(crate) runtime: astrcode_core::ResolvedRuntimeConfig,
    pub(crate) session_id: String,
    pub(crate) turn_id: String,
    pub(crate) parent_agent_id: Option<String>,
    pub(crate) source_tool_call_id: Option<String>,
    pub(crate) policy: AgentCollaborationPolicyContext,
    pub(crate) governance_revision: String,
    pub(crate) mode_id: ModeId,
}

impl ToolCollaborationContext {
    pub(crate) fn new(input: ToolCollaborationContextInput) -> Self {
        Self {
            runtime: input.runtime,
            session_id: input.session_id,
            turn_id: input.turn_id,
            parent_agent_id: input.parent_agent_id,
            source_tool_call_id: input.source_tool_call_id,
            policy: input.policy,
            governance_revision: input.governance_revision,
            mode_id: input.mode_id,
        }
    }

    pub(crate) fn with_parent_agent_id(mut self, parent_agent_id: Option<String>) -> Self {
        self.parent_agent_id = parent_agent_id;
        self
    }

    pub(crate) fn runtime(&self) -> &astrcode_core::ResolvedRuntimeConfig {
        &self.runtime
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub(crate) fn parent_agent_id(&self) -> Option<String> {
        self.parent_agent_id.clone()
    }

    pub(crate) fn source_tool_call_id(&self) -> Option<String> {
        self.source_tool_call_id.clone()
    }

    pub(crate) fn policy(&self) -> AgentCollaborationPolicyContext {
        self.policy.clone()
    }

    pub(crate) fn governance_revision(&self) -> &str {
        &self.governance_revision
    }

    pub(crate) fn mode_id(&self) -> &ModeId {
        &self.mode_id
    }

    pub(crate) fn fact<'a>(
        &'a self,
        action: AgentCollaborationActionKind,
        outcome: AgentCollaborationOutcomeKind,
    ) -> CollaborationFactRecord<'a> {
        CollaborationFactRecord::new(action, outcome, &self.session_id, &self.turn_id)
            .parent_agent_id(self.parent_agent_id())
            .source_tool_call_id(self.source_tool_call_id())
            .policy(self.policy())
            .governance_revision(self.governance_revision())
            .mode_id(Some(self.mode_id().clone()))
    }
}

pub(crate) fn implicit_session_root_agent_id(session_id: &str) -> String {
    format!(
        "root-agent:{}",
        astrcode_session_runtime::normalize_session_id(session_id)
    )
}

fn default_resolved_limits_for_gateway(
    gateway: &astrcode_kernel::KernelGateway,
    max_steps: Option<u32>,
) -> ResolvedExecutionLimitsSnapshot {
    ResolvedExecutionLimitsSnapshot {
        allowed_tools: gateway.capabilities().tool_names(),
        max_steps,
    }
}

async fn ensure_handle_has_resolved_limits(
    kernel: &dyn crate::AgentKernelPort,
    gateway: &astrcode_kernel::KernelGateway,
    handle: SubRunHandle,
    max_steps: Option<u32>,
) -> std::result::Result<SubRunHandle, String> {
    if !handle.resolved_limits.allowed_tools.is_empty() {
        return Ok(handle);
    }

    super::persist_resolved_limits_for_handle(
        kernel,
        handle,
        default_resolved_limits_for_gateway(gateway, max_steps),
    )
    .await
}

impl AgentOrchestrationService {
    pub(super) fn resolve_runtime_config_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> std::result::Result<astrcode_core::ResolvedRuntimeConfig, AgentOrchestrationError> {
        self.config_service
            .load_resolved_runtime_config(Some(working_dir))
            .map_err(|error| AgentOrchestrationError::Internal(error.to_string()))
    }

    pub(super) async fn resolve_runtime_config_for_session(
        &self,
        session_id: &str,
    ) -> std::result::Result<astrcode_core::ResolvedRuntimeConfig, AgentOrchestrationError> {
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await
            .map_err(AgentOrchestrationError::from)?;
        self.resolve_runtime_config_for_working_dir(Path::new(&working_dir))
    }

    pub(super) fn resolve_subagent_profile(
        &self,
        working_dir: &Path,
        profile_id: &str,
    ) -> std::result::Result<astrcode_core::AgentProfile, AgentOrchestrationError> {
        self.profiles
            .find_profile(working_dir, profile_id)
            .map_err(|error| match error {
                crate::ApplicationError::NotFound(message) => {
                    AgentOrchestrationError::NotFound(message)
                },
                crate::ApplicationError::InvalidArgument(message) => {
                    AgentOrchestrationError::InvalidInput(message)
                },
                other => AgentOrchestrationError::Internal(other.to_string()),
            })
    }

    pub(super) async fn append_collaboration_fact(
        &self,
        fact: AgentCollaborationFact,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let turn_id = fact.turn_id.clone();
        let parent_session_id = fact.parent_session_id.clone();
        let event_agent = if let Some(parent_agent_id) = fact.parent_agent_id.as_deref() {
            self.kernel
                .get_handle(parent_agent_id)
                .await
                .map(|handle| {
                    if handle.depth == 0 {
                        root_execution_event_context(handle.agent_id, handle.agent_profile)
                    } else {
                        subrun_event_context(&handle)
                    }
                })
                .unwrap_or_default()
        } else {
            AgentEventContext::default()
        };
        self.session_runtime
            .append_agent_collaboration_fact(
                &parent_session_id,
                &turn_id,
                event_agent,
                fact.clone(),
            )
            .await
            .map_err(AgentOrchestrationError::from)?;
        self.metrics.record_agent_collaboration_fact(&fact);
        Ok(())
    }

    pub(super) async fn record_collaboration_fact(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let fact = AgentCollaborationFact {
            fact_id: format!("acf-{}", uuid::Uuid::new_v4()).into(),
            action: record.action,
            outcome: record.outcome,
            parent_session_id: record.session_id.to_string().into(),
            turn_id: record.turn_id.to_string().into(),
            parent_agent_id: record.parent_agent_id.map(Into::into),
            child_identity: record.child.and_then(|handle| {
                handle
                    .child_session_id
                    .clone()
                    .map(|child_session_id| ChildExecutionIdentity {
                        agent_id: handle.agent_id.clone(),
                        session_id: child_session_id,
                        sub_run_id: handle.sub_run_id.clone(),
                    })
            }),
            delivery_id: record.delivery_id.map(Into::into),
            reason_code: record.reason_code,
            summary: record.summary,
            latency_ms: record.latency_ms,
            source_tool_call_id: record.source_tool_call_id.map(Into::into),
            governance_revision: record.governance_revision,
            mode_id: record.mode_id,
            policy: record
                .policy
                .unwrap_or_else(|| collaboration_policy_context(runtime)),
        };
        self.append_collaboration_fact(fact).await
    }

    pub(super) async fn tool_collaboration_context(
        &self,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolCollaborationContext, AgentOrchestrationError> {
        let runtime = self.resolve_runtime_config_for_working_dir(ctx.working_dir())?;
        let mode_id = self
            .session_runtime
            .session_mode_state(ctx.session_id())
            .await
            .map_err(AgentOrchestrationError::from)?
            .current_mode_id;
        Ok(ToolCollaborationContext::new(
            ToolCollaborationContextInput {
                runtime: runtime.clone(),
                session_id: ctx.session_id().to_string(),
                turn_id: ctx.turn_id().unwrap_or("unknown-turn").to_string(),
                parent_agent_id: ctx.agent_context().agent_id.clone().map(Into::into),
                source_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
                policy: collaboration_policy_context(&runtime),
                governance_revision: GOVERNANCE_POLICY_REVISION.to_string(),
                mode_id,
            },
        ))
    }

    pub(super) async fn record_fact_best_effort(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
    ) {
        let _ = self.record_collaboration_fact(runtime, record).await;
    }

    pub(super) async fn reject_with_fact<T>(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
        error: AgentOrchestrationError,
    ) -> std::result::Result<T, AgentOrchestrationError> {
        self.record_fact_best_effort(runtime, record).await;
        Err(error)
    }

    pub(super) async fn reject_spawn<T>(
        &self,
        collaboration: &ToolCollaborationContext,
        reason_code: &str,
        error: AgentOrchestrationError,
    ) -> Result<T> {
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Spawn,
                    AgentCollaborationOutcomeKind::Rejected,
                )
                .reason_code(reason_code)
                .summary(error.to_string()),
        )
        .await;
        Err(super::map_orchestration_error(error))
    }

    pub(super) async fn fail_spawn_internal<T>(
        &self,
        collaboration: &ToolCollaborationContext,
        reason_code: &str,
        message: String,
    ) -> Result<T> {
        self.record_fact_best_effort(
            collaboration.runtime(),
            collaboration
                .fact(
                    AgentCollaborationActionKind::Spawn,
                    AgentCollaborationOutcomeKind::Failed,
                )
                .reason_code(reason_code)
                .summary(message.clone()),
        )
        .await;
        Err(astrcode_core::AstrError::Internal(message))
    }

    pub(super) async fn ensure_parent_agent_handle(
        &self,
        ctx: &ToolContext,
    ) -> std::result::Result<SubRunHandle, AgentOrchestrationError> {
        let session_id = ctx.session_id().to_string();
        let explicit_agent_id = ctx
            .agent_context()
            .agent_id
            .clone()
            .filter(|agent_id| !agent_id.trim().is_empty());

        if let Some(agent_id) = explicit_agent_id {
            if let Some(handle) = self.kernel.get_handle(&agent_id).await {
                if handle.depth == 0 && handle.resolved_limits.allowed_tools.is_empty() {
                    return ensure_handle_has_resolved_limits(
                        self.kernel.as_ref(),
                        &self.kernel.gateway(),
                        handle,
                        None,
                    )
                    .await
                    .map_err(AgentOrchestrationError::Internal);
                }
                return Ok(handle);
            }

            let is_root_execution = matches!(
                ctx.agent_context().invocation_kind,
                Some(InvocationKind::RootExecution)
            );
            if is_root_execution {
                let profile_id = ctx
                    .agent_context()
                    .agent_profile
                    .clone()
                    .filter(|profile_id| !profile_id.trim().is_empty())
                    .unwrap_or_else(|| IMPLICIT_ROOT_PROFILE_ID.to_string());
                let handle = self
                    .kernel
                    .register_root_agent(agent_id.to_string(), session_id, profile_id)
                    .await
                    .map_err(|error| {
                        AgentOrchestrationError::Internal(format!(
                            "failed to register root agent for parent context: {error}"
                        ))
                    })?;
                return ensure_handle_has_resolved_limits(
                    self.kernel.as_ref(),
                    &self.kernel.gateway(),
                    handle,
                    None,
                )
                .await
                .map_err(AgentOrchestrationError::Internal);
            }

            return Err(AgentOrchestrationError::NotFound(format!(
                "agent '{}' not found",
                agent_id
            )));
        }

        if let Some(handle) = self.kernel.find_root_handle_for_session(&session_id).await {
            return ensure_handle_has_resolved_limits(
                self.kernel.as_ref(),
                &self.kernel.gateway(),
                handle,
                None,
            )
            .await
            .map_err(AgentOrchestrationError::Internal);
        }

        let handle = self
            .kernel
            .register_root_agent(
                implicit_session_root_agent_id(&session_id),
                session_id,
                IMPLICIT_ROOT_PROFILE_ID.to_string(),
            )
            .await
            .map_err(|error| {
                AgentOrchestrationError::Internal(format!(
                    "failed to register implicit root agent for session parent context: {error}"
                ))
            })?;
        ensure_handle_has_resolved_limits(
            self.kernel.as_ref(),
            &self.kernel.gateway(),
            handle,
            None,
        )
        .await
        .map_err(AgentOrchestrationError::Internal)
    }

    pub(super) async fn enforce_spawn_budget_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
        max_spawn_per_turn: usize,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let spawned_for_turn = self
            .kernel
            .count_children_spawned_for_turn(parent_agent_id, parent_turn_id)
            .await;

        if spawned_for_turn >= max_spawn_per_turn {
            return Err(AgentOrchestrationError::InvalidInput(format!(
                "spawn budget exhausted for this turn ({spawned_for_turn}/{max_spawn_per_turn}); \
                 reuse an existing child with send/observe/close, or continue the work in the \
                 current agent"
            )));
        }

        Ok(())
    }
}
