//! # Agent 编排子域
//!
//! 承接四工具模型（spawn / send / observe / close）的业务编排，
//! 以及父级 delivery 唤醒调度。
//!
//! `AgentOrchestrationService` 是本子域的唯一服务入口，实现
//! `SubAgentExecutor` 和 `CollaborationExecutor` 两个 trait，
//! 通过 `Kernel` + `SessionRuntime` 两个显式依赖完成所有操作。
//!
//! 架构约束：
//! - 不持有 session shadow state
//! - 不直接依赖 adapter-*
//! - 不缓存 session 引用

mod observe;
mod routing;
mod terminal;
#[cfg(test)]
mod test_support;
mod wake;

use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
    AgentCollaborationPolicyContext, AgentEventContext, AgentLifecycleStatus, AgentMailboxEnvelope,
    AgentTurnOutcome, ArtifactRef, CloseAgentParams, CollaborationResult, InvocationKind,
    ObserveParams, Result, RuntimeMetricsRecorder, SendAgentParams, SpawnAgentParams, SubRunHandle,
    SubRunHandoff, SubRunResult, ToolContext,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use async_trait::async_trait;
use thiserror::Error;

use crate::{
    config::ConfigService,
    execution::{ProfileResolutionService, SubagentExecutionRequest, launch_subagent},
    lifecycle::TaskRegistry,
};

/// Agent 编排错误类型。
#[derive(Debug, Error)]
pub enum AgentOrchestrationError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<astrcode_core::AstrError> for AgentOrchestrationError {
    fn from(e: astrcode_core::AstrError) -> Self {
        AgentOrchestrationError::Internal(e.to_string())
    }
}

pub(crate) fn root_execution_event_context(
    agent_id: impl Into<String>,
    profile_id: impl Into<String>,
) -> AgentEventContext {
    AgentEventContext::root_execution(agent_id, profile_id)
}

pub(crate) fn subrun_event_context(handle: &SubRunHandle) -> AgentEventContext {
    AgentEventContext::from(handle)
}

pub(crate) fn subrun_event_context_for_parent_turn(
    handle: &SubRunHandle,
    parent_turn_id: &str,
) -> AgentEventContext {
    AgentEventContext::sub_run(
        handle.agent_id.clone(),
        parent_turn_id.to_string(),
        handle.agent_profile.clone(),
        handle.sub_run_id.clone(),
        handle.parent_sub_run_id.clone(),
        handle.storage_mode,
        handle.child_session_id.clone(),
    )
}

pub(crate) const IMPLICIT_ROOT_PROFILE_ID: &str = "default";
pub(crate) const AGENT_COLLABORATION_POLICY_REVISION: &str = "agent-collaboration-v1";

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
}

pub(crate) struct ToolCollaborationContext {
    runtime: astrcode_core::ResolvedRuntimeConfig,
    session_id: String,
    turn_id: String,
    parent_agent_id: Option<String>,
    source_tool_call_id: Option<String>,
}

impl ToolCollaborationContext {
    pub(crate) fn new(
        runtime: astrcode_core::ResolvedRuntimeConfig,
        session_id: String,
        turn_id: String,
        parent_agent_id: Option<String>,
        source_tool_call_id: Option<String>,
    ) -> Self {
        Self {
            runtime,
            session_id,
            turn_id,
            parent_agent_id,
            source_tool_call_id,
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

    pub(crate) fn fact<'a>(
        &'a self,
        action: AgentCollaborationActionKind,
        outcome: AgentCollaborationOutcomeKind,
    ) -> CollaborationFactRecord<'a> {
        CollaborationFactRecord::new(action, outcome, &self.session_id, &self.turn_id)
            .parent_agent_id(self.parent_agent_id())
            .source_tool_call_id(self.source_tool_call_id())
    }
}

pub(crate) fn implicit_session_root_agent_id(session_id: &str) -> String {
    // 为什么按 session 生成 synthetic root id：
    // `AgentControl` 以 agent_id 作为全局索引键，普通会话若都共用 `root-agent`
    // 会把不同 session 的父子树混在一起。
    format!(
        "root-agent:{}",
        astrcode_session_runtime::normalize_session_id(session_id)
    )
}

pub(crate) fn child_delivery_mailbox_envelope(
    notification: &astrcode_core::ChildSessionNotification,
    target_agent_id: String,
) -> AgentMailboxEnvelope {
    AgentMailboxEnvelope {
        delivery_id: notification.notification_id.clone(),
        from_agent_id: notification.child_ref.agent_id.clone(),
        to_agent_id: target_agent_id,
        message: terminal_notification_message(notification),
        queued_at: chrono::Utc::now(),
        sender_lifecycle_status: AgentLifecycleStatus::Idle,
        sender_last_turn_outcome: terminal_notification_turn_outcome(notification),
        sender_open_session_id: notification.child_ref.open_session_id.clone(),
    }
}

pub(crate) fn terminal_notification_message(
    notification: &astrcode_core::ChildSessionNotification,
) -> String {
    notification
        .final_reply_excerpt
        .as_deref()
        .filter(|excerpt| !excerpt.trim().is_empty())
        .unwrap_or(notification.summary.as_str())
        .to_string()
}

pub(crate) fn terminal_notification_turn_outcome(
    notification: &astrcode_core::ChildSessionNotification,
) -> Option<AgentTurnOutcome> {
    if !matches!(notification.status, AgentLifecycleStatus::Idle) {
        return None;
    }
    match notification.kind {
        astrcode_core::ChildSessionNotificationKind::Delivered => Some(AgentTurnOutcome::Completed),
        astrcode_core::ChildSessionNotificationKind::Failed => Some(AgentTurnOutcome::Failed),
        astrcode_core::ChildSessionNotificationKind::Closed => Some(AgentTurnOutcome::Cancelled),
        _ => None,
    }
}

pub(crate) fn child_open_session_id(child: &SubRunHandle) -> String {
    child
        .child_session_id
        .clone()
        .unwrap_or_else(|| child.session_id.clone())
}

pub(crate) fn artifact_ref(
    kind: &str,
    id: impl Into<String>,
    label: &str,
    session_id: Option<String>,
) -> ArtifactRef {
    ArtifactRef {
        kind: kind.to_string(),
        id: id.into(),
        label: label.to_string(),
        session_id,
        storage_seq: None,
        uri: None,
    }
}

pub(crate) fn child_collaboration_artifacts(
    child: &SubRunHandle,
    parent_session_id: &str,
    include_parent_sub_run: bool,
) -> Vec<ArtifactRef> {
    let mut artifacts = vec![artifact_ref(
        "subRun",
        child.sub_run_id.clone(),
        "Sub Run",
        Some(parent_session_id.to_string()),
    )];
    artifacts.extend(child_reference_artifacts(
        child,
        parent_session_id,
        include_parent_sub_run,
    ));
    artifacts
}

pub(crate) fn child_reference_artifacts(
    child: &SubRunHandle,
    parent_session_id: &str,
    include_parent_sub_run: bool,
) -> Vec<ArtifactRef> {
    let child_session_id = child_open_session_id(child);
    let mut artifacts = vec![
        artifact_ref(
            "agent",
            child.agent_id.clone(),
            "Agent",
            Some(child_session_id.clone()),
        ),
        artifact_ref(
            "parentSession",
            parent_session_id.to_string(),
            "Parent Session",
            Some(parent_session_id.to_string()),
        ),
        artifact_ref(
            "session",
            child_session_id.clone(),
            "Child Session",
            Some(child_session_id),
        ),
    ];
    if let Some(parent_agent_id) = &child.parent_agent_id {
        artifacts.push(artifact_ref(
            "parentAgent",
            parent_agent_id.clone(),
            "Parent Agent",
            Some(parent_session_id.to_string()),
        ));
    }
    if include_parent_sub_run {
        if let Some(parent_sub_run_id) = &child.parent_sub_run_id {
            artifacts.push(artifact_ref(
                "parentSubRun",
                parent_sub_run_id.clone(),
                "Parent Sub Run",
                Some(parent_session_id.to_string()),
            ));
        }
    }
    artifacts
}

fn map_orchestration_error(error: AgentOrchestrationError) -> astrcode_core::AstrError {
    match error {
        AgentOrchestrationError::InvalidInput(message)
        | AgentOrchestrationError::NotFound(message) => {
            astrcode_core::AstrError::Validation(message)
        },
        AgentOrchestrationError::Internal(message) => astrcode_core::AstrError::Internal(message),
    }
}

/// Agent 编排服务。
///
/// 持有 `Kernel` + `SessionRuntime` 两个显式依赖，
/// 不持有 session shadow state，不缓存 session 引用。
#[derive(Clone)]
pub struct AgentOrchestrationService {
    kernel: Arc<Kernel>,
    session_runtime: Arc<SessionRuntime>,
    config_service: Arc<ConfigService>,
    profiles: Arc<ProfileResolutionService>,
    task_registry: Arc<TaskRegistry>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
}

impl AgentOrchestrationService {
    pub fn new(
        kernel: Arc<Kernel>,
        session_runtime: Arc<SessionRuntime>,
        config_service: Arc<ConfigService>,
        profiles: Arc<ProfileResolutionService>,
        task_registry: Arc<TaskRegistry>,
        metrics: Arc<dyn RuntimeMetricsRecorder>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            config_service,
            profiles,
            task_registry,
            metrics,
        }
    }

    /// 解析指定工作目录的有效 RuntimeConfig。
    fn resolve_runtime_config_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> std::result::Result<astrcode_core::ResolvedRuntimeConfig, AgentOrchestrationError> {
        self.config_service
            .load_resolved_runtime_config(Some(working_dir))
            .map_err(|error| AgentOrchestrationError::Internal(error.to_string()))
    }

    /// 解析指定 session 对应工作目录的有效 RuntimeConfig。
    async fn resolve_runtime_config_for_session(
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

    fn resolve_subagent_profile(
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

    fn collaboration_policy_context(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
    ) -> AgentCollaborationPolicyContext {
        AgentCollaborationPolicyContext {
            policy_revision: AGENT_COLLABORATION_POLICY_REVISION.to_string(),
            max_subrun_depth: runtime.agent.max_subrun_depth,
            max_spawn_per_turn: runtime.agent.max_spawn_per_turn,
        }
    }

    async fn append_collaboration_fact(
        &self,
        fact: AgentCollaborationFact,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let turn_id = fact.turn_id.clone();
        let parent_session_id = fact.parent_session_id.clone();
        let event_agent = if let Some(parent_agent_id) = fact.parent_agent_id.as_deref() {
            self.kernel
                .get_agent_handle(parent_agent_id)
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

    async fn record_collaboration_fact(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let fact = AgentCollaborationFact {
            fact_id: format!("acf-{}", uuid::Uuid::new_v4()),
            action: record.action,
            outcome: record.outcome,
            parent_session_id: record.session_id.to_string(),
            turn_id: record.turn_id.to_string(),
            parent_agent_id: record.parent_agent_id,
            child_agent_id: record.child.map(|handle| handle.agent_id.clone()),
            child_session_id: record
                .child
                .and_then(|handle| handle.child_session_id.clone()),
            child_sub_run_id: record.child.map(|handle| handle.sub_run_id.clone()),
            delivery_id: record.delivery_id,
            reason_code: record.reason_code,
            summary: record.summary,
            latency_ms: record.latency_ms,
            source_tool_call_id: record.source_tool_call_id,
            policy: self.collaboration_policy_context(runtime),
        };
        self.append_collaboration_fact(fact).await
    }

    fn tool_collaboration_context(
        &self,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolCollaborationContext, AgentOrchestrationError> {
        Ok(ToolCollaborationContext::new(
            self.resolve_runtime_config_for_working_dir(ctx.working_dir())?,
            ctx.session_id().to_string(),
            ctx.turn_id().unwrap_or("unknown-turn").to_string(),
            ctx.agent_context().agent_id.clone(),
            ctx.tool_call_id().map(ToString::to_string),
        ))
    }

    async fn record_fact_best_effort(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
    ) {
        let _ = self.record_collaboration_fact(runtime, record).await;
    }

    async fn reject_with_fact<T>(
        &self,
        runtime: &astrcode_core::ResolvedRuntimeConfig,
        record: CollaborationFactRecord<'_>,
        error: AgentOrchestrationError,
    ) -> std::result::Result<T, AgentOrchestrationError> {
        self.record_fact_best_effort(runtime, record).await;
        Err(error)
    }

    async fn ensure_parent_agent_handle(
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
            if let Some(handle) = self.kernel.get_agent_handle(&agent_id).await {
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
                return self
                    .kernel
                    .agent_control()
                    .register_root_agent(agent_id, session_id, profile_id)
                    .await
                    .map_err(|error| {
                        AgentOrchestrationError::Internal(format!(
                            "failed to register root agent for parent context: {error}"
                        ))
                    });
            }

            return Err(AgentOrchestrationError::NotFound(format!(
                "agent '{}' not found",
                agent_id
            )));
        }

        if let Some(handle) = self
            .kernel
            .agent_control()
            .find_root_agent_for_session(&session_id)
            .await
        {
            return Ok(handle);
        }

        self.kernel
            .agent_control()
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
            })
    }

    async fn enforce_spawn_budget_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
        max_spawn_per_turn: usize,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let spawned_for_turn = self
            .kernel
            .agent_control()
            .list()
            .await
            .into_iter()
            .filter(|handle| {
                handle.parent_turn_id == parent_turn_id
                    && handle.parent_agent_id.as_deref() == Some(parent_agent_id)
            })
            .count();

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

// ── 实现 SubAgentExecutor（供 spawn 工具使用）──────────────────────

#[async_trait]
impl astrcode_core::SubAgentExecutor for AgentOrchestrationService {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let parent_handle = self
            .ensure_parent_agent_handle(ctx)
            .await
            .map_err(map_orchestration_error)?;
        let collaboration = self
            .tool_collaboration_context(ctx)
            .map_err(map_orchestration_error)?
            .with_parent_agent_id(Some(parent_handle.agent_id.clone()));
        let parent_agent_id = parent_handle.agent_id.clone();
        let parent_turn_id = collaboration.turn_id().to_string();
        let parent_session_id = collaboration.session_id().to_string();
        let profile_id = params
            .r#type
            .clone()
            .unwrap_or_else(|| "explore".to_string());
        let runtime_config = collaboration.runtime().clone();
        let profile = match self.resolve_subagent_profile(ctx.working_dir(), &profile_id) {
            Ok(profile) => profile,
            Err(error) => {
                self.record_fact_best_effort(
                    &runtime_config,
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Spawn,
                            AgentCollaborationOutcomeKind::Rejected,
                        )
                        .reason_code("profile_resolution_failed")
                        .summary(error.to_string()),
                )
                .await;
                return Err(map_orchestration_error(error));
            },
        };

        let request = SubagentExecutionRequest {
            parent_session_id: parent_session_id.clone(),
            parent_agent_id: parent_agent_id.clone(),
            parent_turn_id: parent_turn_id.clone(),
            working_dir: ctx.working_dir().display().to_string(),
            profile,
            task: params.prompt,
            context: params.context,
        };
        if let Err(error) = self
            .enforce_spawn_budget_for_turn(
                &parent_agent_id,
                &request.parent_turn_id,
                runtime_config.agent.max_spawn_per_turn,
            )
            .await
        {
            self.record_fact_best_effort(
                &runtime_config,
                collaboration
                    .fact(
                        AgentCollaborationActionKind::Spawn,
                        AgentCollaborationOutcomeKind::Rejected,
                    )
                    .reason_code("spawn_budget_exhausted")
                    .summary(error.to_string()),
            )
            .await;
            return Err(map_orchestration_error(error));
        }

        let accepted = match launch_subagent(
            &self.kernel,
            &self.session_runtime,
            request,
            runtime_config.clone(),
            &self.metrics,
        )
        .await
        {
            Ok(accepted) => accepted,
            Err(error) => {
                self.record_fact_best_effort(
                    &runtime_config,
                    collaboration
                        .fact(
                            AgentCollaborationActionKind::Spawn,
                            AgentCollaborationOutcomeKind::Failed,
                        )
                        .reason_code("launch_subagent_failed")
                        .summary(error.to_string()),
                )
                .await;
                return Err(astrcode_core::AstrError::Internal(error.to_string()));
            },
        };
        let mut child_handle_for_handoff = None;
        if let (Some(child_agent_id), Some(parent_turn_id)) =
            (accepted.agent_id.clone(), ctx.turn_id())
        {
            if let Some(child_handle) = self.kernel.get_agent_handle(&child_agent_id).await {
                let fact = {
                    let mut fact = collaboration
                        .fact(
                            AgentCollaborationActionKind::Spawn,
                            AgentCollaborationOutcomeKind::Accepted,
                        )
                        .child(&child_handle);
                    if let Some(summary) =
                        Some(params.description.trim()).filter(|value| !value.is_empty())
                    {
                        fact = fact.summary(summary.to_string());
                    }
                    fact
                };
                self.record_fact_best_effort(&runtime_config, fact).await;
                self.spawn_child_turn_terminal_watcher(
                    child_handle.clone(),
                    accepted.session_id.to_string(),
                    accepted.turn_id.to_string(),
                    parent_session_id.clone(),
                    parent_turn_id.to_string(),
                    collaboration.source_tool_call_id(),
                );
                child_handle_for_handoff = Some(child_handle);
            }
        }

        let handoff_artifacts = if let Some(child_handle) = child_handle_for_handoff.as_ref() {
            let mut artifacts = vec![artifact_ref(
                "subRun",
                accepted.turn_id.to_string(),
                "Sub Run",
                Some(parent_session_id.clone()),
            )];
            artifacts.extend(child_reference_artifacts(
                child_handle,
                &parent_session_id,
                false,
            ));
            artifacts
        } else {
            vec![
                artifact_ref(
                    "subRun",
                    accepted.turn_id.to_string(),
                    "Sub Run",
                    Some(parent_session_id.clone()),
                ),
                artifact_ref(
                    "agent",
                    accepted.agent_id.clone().unwrap_or_default().to_string(),
                    "Agent",
                    Some(accepted.session_id.to_string()),
                ),
                artifact_ref(
                    "parentSession",
                    ctx.session_id().to_string(),
                    "Parent Session",
                    Some(ctx.session_id().to_string()),
                ),
                artifact_ref(
                    "session",
                    accepted.session_id.to_string(),
                    "Child Session",
                    Some(accepted.session_id.to_string()),
                ),
                artifact_ref(
                    "parentAgent",
                    parent_agent_id.clone(),
                    "Parent Agent",
                    Some(ctx.session_id().to_string()),
                ),
            ]
        };

        Ok(SubRunResult {
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
            handoff: Some(SubRunHandoff {
                summary: if params.description.trim().is_empty() {
                    "子 Agent 已启动。".to_string()
                } else {
                    format!("子 Agent 已启动：{}", params.description.trim())
                },
                findings: Vec::new(),
                artifacts: handoff_artifacts,
            }),
            failure: None,
        })
    }
}

// ── 实现 CollaborationExecutor（供 send/close/observe 工具使用）─────

#[async_trait]
impl astrcode_core::CollaborationExecutor for AgentOrchestrationService {
    async fn send(
        &self,
        params: SendAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        self.send_to_child(params, ctx)
            .await
            .map_err(map_orchestration_error)
    }

    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        self.close_child(params, ctx)
            .await
            .map_err(map_orchestration_error)
    }

    async fn observe(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        self.observe_child(params, ctx)
            .await
            .map_err(map_orchestration_error)
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentLifecycleStatus,
        CancelToken, ChildAgentRef, ChildSessionLineageKind, ChildSessionNotification,
        ChildSessionNotificationKind, SessionId, SpawnAgentParams, StorageEventPayload,
        ToolContext, agent::executor::SubAgentExecutor,
    };

    use super::{
        IMPLICIT_ROOT_PROFILE_ID, child_delivery_mailbox_envelope, implicit_session_root_agent_id,
        root_execution_event_context, terminal_notification_message,
        terminal_notification_turn_outcome,
    };
    use crate::agent::test_support::{
        TestLlmBehavior, build_agent_test_harness, build_agent_test_harness_with_agent_config,
    };

    #[test]
    fn root_execution_event_context_uses_explicit_agent_id() {
        let context = root_execution_event_context("root-agent", "planner");

        assert_eq!(context.agent_id.as_deref(), Some("root-agent"));
        assert_eq!(context.agent_profile.as_deref(), Some("planner"));
        assert_eq!(
            context.invocation_kind,
            Some(astrcode_core::InvocationKind::RootExecution)
        );
    }

    #[test]
    fn child_delivery_mailbox_envelope_reuses_terminal_projection_fields() {
        let notification = ChildSessionNotification {
            notification_id: "delivery-1".to_string(),
            child_ref: ChildAgentRef {
                agent_id: "agent-child".to_string(),
                session_id: "session-parent".to_string(),
                sub_run_id: "subrun-child".to_string(),
                parent_agent_id: Some("agent-parent".to_string()),
                parent_sub_run_id: Some("subrun-parent".to_string()),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: "session-child".to_string(),
            },
            kind: ChildSessionNotificationKind::Delivered,
            summary: "summary".to_string(),
            status: AgentLifecycleStatus::Idle,
            source_tool_call_id: None,
            final_reply_excerpt: Some("final reply".to_string()),
        };

        let envelope = child_delivery_mailbox_envelope(&notification, "agent-parent".to_string());

        assert_eq!(terminal_notification_message(&notification), "final reply");
        assert_eq!(
            terminal_notification_turn_outcome(&notification),
            Some(astrcode_core::AgentTurnOutcome::Completed)
        );
        assert_eq!(envelope.to_agent_id, "agent-parent");
        assert_eq!(envelope.message, "final reply");
        assert_eq!(
            envelope.sender_last_turn_outcome,
            Some(astrcode_core::AgentTurnOutcome::Completed)
        );
    }

    #[tokio::test]
    async fn launch_without_explicit_agent_context_registers_session_root_parent() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "子代理已完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        let ctx = ToolContext::new(
            parent.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-1");

        let result = harness
            .service
            .launch(
                SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "仓库审查".to_string(),
                    prompt: "请阅读代码".to_string(),
                    context: None,
                },
                &ctx,
            )
            .await
            .expect("subagent should launch with implicit root parent");

        let parent_agent_artifact = result
            .handoff
            .as_ref()
            .expect("handoff should exist")
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "parentAgent")
            .expect("parent agent artifact should exist");
        let expected_parent_agent_id = implicit_session_root_agent_id(&parent.session_id);
        assert_eq!(parent_agent_artifact.id, expected_parent_agent_id);

        let root_status = harness
            .kernel
            .query_root_agent_status(&parent.session_id)
            .await
            .expect("implicit root agent should be registered");
        assert_eq!(root_status.agent_id, expected_parent_agent_id);
        assert_eq!(root_status.agent_profile, IMPLICIT_ROOT_PROFILE_ID);
    }

    #[tokio::test]
    async fn launch_preserves_independent_child_session_lineage_in_handle_and_events() {
        let harness = build_agent_test_harness(TestLlmBehavior::Succeed {
            content: "子代理已完成。".to_string(),
        })
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should be registered");
        let ctx = ToolContext::new(
            parent.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-1")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        let result = harness
            .service
            .launch(
                SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "仓库审查".to_string(),
                    prompt: "请阅读代码".to_string(),
                    context: Some("关注最近修改".to_string()),
                },
                &ctx,
            )
            .await
            .expect("subagent should launch");

        let handoff = result.handoff.expect("handoff should exist");
        let child_agent_id = handoff
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "agent")
            .map(|artifact| artifact.id.clone())
            .expect("child agent artifact should exist");
        let child_session_id = handoff
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "session")
            .map(|artifact| artifact.id.clone())
            .expect("child session artifact should exist");

        let child_handle = harness
            .kernel
            .get_agent_handle(&child_agent_id)
            .await
            .expect("child handle should exist");
        assert_eq!(
            child_handle.session_id, parent.session_id,
            "independent child should remain attached to parent session in control tree"
        );
        assert_eq!(
            child_handle.child_session_id.as_deref(),
            Some(child_session_id.as_str()),
            "independent child should carry its open child session id"
        );

        let child_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(child_session_id.clone()))
            .await
            .expect("child session events should replay");
        let child_meta = harness
            .session_runtime
            .list_session_metas()
            .await
            .expect("child session metas should list")
            .into_iter()
            .find(|meta| meta.session_id == child_session_id)
            .expect("child session meta should exist");
        let child_prompt = child_events
            .iter()
            .find(|stored| {
                matches!(
                    stored.event.payload,
                    StorageEventPayload::UserMessage { .. }
                )
            })
            .expect("child session should persist its first user prompt");
        assert_eq!(
            child_prompt.event.agent.child_session_id.as_deref(),
            Some(child_session_id.as_str()),
            "child prompt event should be stamped with its independent child session id"
        );
        assert_eq!(
            child_meta.parent_session_id.as_deref(),
            Some(parent.session_id.as_str()),
            "independent child session should carry its parent session lineage"
        );
    }

    #[tokio::test]
    async fn launch_rejects_spawns_that_exceed_per_turn_budget() {
        let harness = build_agent_test_harness_with_agent_config(
            TestLlmBehavior::Succeed {
                content: "子代理已完成。".to_string(),
            },
            Some(astrcode_core::AgentConfig {
                max_spawn_per_turn: Some(1),
                ..astrcode_core::AgentConfig::default()
            }),
        )
        .expect("test harness should build");
        let project = tempfile::tempdir().expect("tempdir should be created");
        let parent = harness
            .session_runtime
            .create_session(project.path().display().to_string())
            .await
            .expect("parent session should be created");
        harness
            .kernel
            .agent_control()
            .register_root_agent(
                "root-agent".to_string(),
                parent.session_id.clone(),
                "root-profile".to_string(),
            )
            .await
            .expect("root agent should be registered");
        let ctx = ToolContext::new(
            parent.session_id.clone().into(),
            project.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-1")
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"));

        harness
            .service
            .launch(
                SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "第一次".to_string(),
                    prompt: "请阅读代码".to_string(),
                    context: None,
                },
                &ctx,
            )
            .await
            .expect("first spawn should succeed");

        let error = harness
            .service
            .launch(
                SpawnAgentParams {
                    r#type: Some("reviewer".to_string()),
                    description: "第二次".to_string(),
                    prompt: "请继续阅读代码".to_string(),
                    context: None,
                },
                &ctx,
            )
            .await
            .expect_err("second spawn should hit the per-turn budget");

        assert!(
            error
                .to_string()
                .contains("spawn budget exhausted for this turn"),
            "unexpected error: {error}"
        );

        let parent_events = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay");
        assert!(parent_events.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::AgentCollaborationFact { fact, .. }
                if fact.action == AgentCollaborationActionKind::Spawn
                    && fact.outcome == AgentCollaborationOutcomeKind::Rejected
                    && fact.reason_code.as_deref() == Some("spawn_budget_exhausted")
                    && fact.policy.max_spawn_per_turn == 1
        )));
    }
}
