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
    AgentTurnOutcome, ArtifactRef, CloseAgentParams, CollaborationResult, DelegationMetadata,
    InvocationKind, ObserveParams, PromptDeclaration, PromptDeclarationKind,
    PromptDeclarationRenderTarget, PromptDeclarationSource, ResolvedExecutionLimitsSnapshot,
    Result, RuntimeMetricsRecorder, SendAgentParams, SpawnAgentParams, SubRunHandle, SubRunHandoff,
    SubRunResult, SystemPromptLayer, ToolContext,
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

fn default_resolved_limits_for_gateway(
    gateway: &astrcode_kernel::KernelGateway,
    max_steps: Option<u32>,
) -> ResolvedExecutionLimitsSnapshot {
    ResolvedExecutionLimitsSnapshot {
        allowed_tools: gateway.capabilities().tool_names(),
        max_steps,
    }
}

fn effective_tool_names_for_handle(
    handle: &SubRunHandle,
    gateway: &astrcode_kernel::KernelGateway,
) -> Vec<String> {
    if handle.resolved_limits.allowed_tools.is_empty() {
        gateway.capabilities().tool_names()
    } else {
        handle.resolved_limits.allowed_tools.clone()
    }
}

fn compact_delegation_summary(description: &str, prompt: &str) -> String {
    let candidate = if !description.trim().is_empty() {
        description.trim()
    } else {
        prompt.trim()
    };
    let normalized = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let truncated = chars.by_ref().take(160).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn capability_limit_summary(allowed_tools: &[String]) -> Option<String> {
    if allowed_tools.is_empty() {
        return None;
    }

    Some(format!(
        "本分支当前只允许使用这些工具：{}。",
        allowed_tools.join(", ")
    ))
}

pub(crate) fn build_delegation_metadata(
    description: &str,
    prompt: &str,
    resolved_limits: &ResolvedExecutionLimitsSnapshot,
    restricted: bool,
) -> DelegationMetadata {
    let responsibility_summary = compact_delegation_summary(description, prompt);
    let reuse_scope_summary = if restricted {
        "只有当下一步仍属于同一责任分支，且所需操作仍落在当前收缩后的 capability surface \
         内时，才应继续复用这个 child。"
            .to_string()
    } else {
        "只有当下一步仍属于同一责任分支时，才应继续复用这个 child；若责任边界已经改变，应 close \
         当前分支并重新选择更合适的执行主体。"
            .to_string()
    };

    DelegationMetadata {
        responsibility_summary,
        reuse_scope_summary,
        restricted,
        capability_limit_summary: restricted
            .then(|| capability_limit_summary(&resolved_limits.allowed_tools))
            .flatten(),
    }
}

pub(crate) fn build_fresh_child_contract(metadata: &DelegationMetadata) -> PromptDeclaration {
    let mut content = format!(
        "You are a delegated child responsible for one isolated branch.\n\nResponsibility \
         branch:\n- {}\n\nFresh-child rule:\n- Treat this as a new responsibility branch with its \
         own ownership boundary.\n- Do not expand into unrelated exploration or \
         implementation.\n\nDelivery contract:\n- End the turn with a concise reusable summary \
         for the parent.\n- State what you finished, the key findings, and any remaining \
         follow-up.\n\nReuse boundary:\n- {}",
        metadata.responsibility_summary, metadata.reuse_scope_summary
    );
    if let Some(limit_summary) = &metadata.capability_limit_summary {
        content.push_str(&format!(
            "\n\nCapability limit:\n- {limit_summary}\n- Do not take work that needs tools \
             outside this surface."
        ));
    }

    PromptDeclaration {
        block_id: "child.execution.contract".to_string(),
        title: "Child Execution Contract".to_string(),
        content,
        render_target: PromptDeclarationRenderTarget::System,
        layer: SystemPromptLayer::Inherited,
        kind: PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(585),
        always_include: true,
        source: PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some("child-contract:fresh".to_string()),
    }
}

pub(crate) fn build_resumed_child_contract(
    metadata: &DelegationMetadata,
    message: &str,
    context: Option<&str>,
) -> PromptDeclaration {
    let mut content = format!(
        "You are continuing an existing delegated child branch.\n\nResponsibility continuity:\n- \
         Keep ownership of the same branch: {}\n\nResumed-child rule:\n- Prioritize the latest \
         delta instruction from the parent.\n- Do not restate or reinterpret the whole original \
         brief unless the new delta requires it.\n\nDelta instruction:\n- {}",
        metadata.responsibility_summary,
        message.trim()
    );
    if let Some(context) = context.filter(|value| !value.trim().is_empty()) {
        content.push_str(&format!("\n- Supplementary context: {}", context.trim()));
    }
    content.push_str(&format!(
        "\n\nDelivery contract:\n- Reply with the concrete progress on this delta, key findings, \
         and remaining follow-up.\n\nReuse boundary:\n- {}",
        metadata.reuse_scope_summary
    ));
    if let Some(limit_summary) = &metadata.capability_limit_summary {
        content.push_str(&format!(
            "\n\nCapability limit:\n- {limit_summary}\n- If the delta now needs broader tools, \
             stop stretching this child and let the parent choose a different branch."
        ));
    }

    PromptDeclaration {
        block_id: "child.execution.contract".to_string(),
        title: "Child Execution Contract".to_string(),
        content,
        render_target: PromptDeclarationRenderTarget::System,
        layer: SystemPromptLayer::Inherited,
        kind: PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(585),
        always_include: true,
        source: PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some("child-contract:resumed".to_string()),
    }
}

pub(crate) async fn persist_resolved_limits_for_handle(
    kernel: &Kernel,
    handle: SubRunHandle,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
) -> std::result::Result<SubRunHandle, String> {
    if kernel
        .agent()
        .set_resolved_limits(&handle.agent_id, resolved_limits.clone())
        .await
        .is_none()
    {
        return Err(format!(
            "failed to persist resolved limits for agent '{}' because the control handle \
             disappeared before the limits snapshot was recorded",
            handle.agent_id
        ));
    }

    let mut handle = handle;
    handle.resolved_limits = resolved_limits;
    Ok(handle)
}

pub(crate) async fn persist_delegation_for_handle(
    kernel: &Kernel,
    handle: SubRunHandle,
    delegation: DelegationMetadata,
) -> std::result::Result<SubRunHandle, String> {
    if kernel
        .agent()
        .set_delegation(&handle.agent_id, Some(delegation.clone()))
        .await
        .is_none()
    {
        return Err(format!(
            "failed to persist delegation metadata for agent '{}' because the control handle \
             disappeared before the branch contract was recorded",
            handle.agent_id
        ));
    }

    let mut handle = handle;
    handle.delegation = Some(delegation);
    Ok(handle)
}

async fn ensure_handle_has_resolved_limits(
    kernel: &Kernel,
    gateway: &astrcode_kernel::KernelGateway,
    handle: SubRunHandle,
    max_steps: Option<u32>,
) -> std::result::Result<SubRunHandle, String> {
    if !handle.resolved_limits.allowed_tools.is_empty() {
        return Ok(handle);
    }

    persist_resolved_limits_for_handle(
        kernel,
        handle,
        default_resolved_limits_for_gateway(gateway, max_steps),
    )
    .await
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

pub(crate) fn spawn_handoff_artifacts(
    child_handle: Option<&SubRunHandle>,
    sub_run_id: &str,
    child_agent_id: Option<&str>,
    child_session_id: &str,
    parent_session_id: &str,
    parent_agent_id: &str,
) -> Vec<ArtifactRef> {
    if let Some(child_handle) = child_handle {
        let mut artifacts = vec![artifact_ref(
            "subRun",
            sub_run_id.to_string(),
            "Sub Run",
            Some(parent_session_id.to_string()),
        )];
        artifacts.extend(child_reference_artifacts(
            child_handle,
            parent_session_id,
            false,
        ));
        return artifacts;
    }

    vec![
        artifact_ref(
            "subRun",
            sub_run_id.to_string(),
            "Sub Run",
            Some(parent_session_id.to_string()),
        ),
        artifact_ref(
            "agent",
            child_agent_id.unwrap_or_default().to_string(),
            "Agent",
            Some(child_session_id.to_string()),
        ),
        artifact_ref(
            "parentSession",
            parent_session_id.to_string(),
            "Parent Session",
            Some(parent_session_id.to_string()),
        ),
        artifact_ref(
            "session",
            child_session_id.to_string(),
            "Child Session",
            Some(child_session_id.to_string()),
        ),
        artifact_ref(
            "parentAgent",
            parent_agent_id.to_string(),
            "Parent Agent",
            Some(parent_session_id.to_string()),
        ),
    ]
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

    /// 解析子 agent profile，将 ApplicationError 映射为编排层错误。
    /// NotFound → NotFound（提示用户检查 profile id），
    /// InvalidArgument → InvalidInput（参数格式问题），
    /// 其余一律降级为 Internal（避免泄露内部细节）。
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
        // 根据父级 agent 的 depth 决定 event context 类型：
        // depth == 0 是根级执行（用 root_execution_event_context），
        // 否则是子运行（用 subrun_event_context 保持 child session 血缘）。
        let event_agent = if let Some(parent_agent_id) = fact.parent_agent_id.as_deref() {
            self.kernel
                .agent()
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

    async fn reject_spawn<T>(
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
        Err(map_orchestration_error(error))
    }

    async fn fail_spawn_internal<T>(
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

    /// 三级解析确保父级 agent handle 存在：
    /// 1. 显式 agent_id → 直接查找，未找到且为 RootExecution 则自动注册
    /// 2. 无显式 id → 按 session 查找已有 root agent
    /// 3. 都没有 → 注册一个隐式 root agent（synthetic agent id）
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
            if let Some(handle) = self.kernel.agent().get_handle(&agent_id).await {
                if handle.depth == 0 && handle.resolved_limits.allowed_tools.is_empty() {
                    return ensure_handle_has_resolved_limits(
                        self.kernel.as_ref(),
                        self.kernel.gateway(),
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
                    .agent()
                    .register_root_agent(agent_id, session_id, profile_id)
                    .await
                    .map_err(|error| {
                        AgentOrchestrationError::Internal(format!(
                            "failed to register root agent for parent context: {error}"
                        ))
                    })?;
                return ensure_handle_has_resolved_limits(
                    self.kernel.as_ref(),
                    self.kernel.gateway(),
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

        if let Some(handle) = self
            .kernel
            .agent()
            .find_root_handle_for_session(&session_id)
            .await
        {
            return ensure_handle_has_resolved_limits(
                self.kernel.as_ref(),
                self.kernel.gateway(),
                handle,
                None,
            )
            .await
            .map_err(AgentOrchestrationError::Internal);
        }

        let handle = self
            .kernel
            .agent()
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
        ensure_handle_has_resolved_limits(self.kernel.as_ref(), self.kernel.gateway(), handle, None)
            .await
            .map_err(AgentOrchestrationError::Internal)
    }

    async fn enforce_spawn_budget_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
        max_spawn_per_turn: usize,
    ) -> std::result::Result<(), AgentOrchestrationError> {
        let spawned_for_turn = self
            .kernel
            .agent()
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
                return self
                    .reject_spawn(&collaboration, "profile_resolution_failed", error)
                    .await;
            },
        };

        let spawn_description = params.description.clone();
        let request = SubagentExecutionRequest {
            parent_session_id: parent_session_id.clone(),
            parent_agent_id: parent_agent_id.clone(),
            parent_turn_id: parent_turn_id.clone(),
            working_dir: ctx.working_dir().display().to_string(),
            profile,
            description: spawn_description.clone(),
            task: params.prompt,
            context: params.context,
            parent_allowed_tools: effective_tool_names_for_handle(
                &parent_handle,
                self.kernel.gateway(),
            ),
            capability_grant: params.capability_grant,
            source_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
        };
        if let Err(error) = self
            .enforce_spawn_budget_for_turn(
                &parent_agent_id,
                &request.parent_turn_id,
                runtime_config.agent.max_spawn_per_turn,
            )
            .await
        {
            return self
                .reject_spawn(&collaboration, "spawn_budget_exhausted", error)
                .await;
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
                return self
                    .fail_spawn_internal(
                        &collaboration,
                        "launch_subagent_failed",
                        error.to_string(),
                    )
                    .await;
            },
        };
        let mut child_handle_for_handoff = None;
        if let (Some(child_agent_id), Some(parent_turn_id)) =
            (accepted.agent_id.clone(), ctx.turn_id())
        {
            if let Some(child_handle) = self.kernel.agent().get_handle(&child_agent_id).await {
                let fact = {
                    let mut fact = collaboration
                        .fact(
                            AgentCollaborationActionKind::Spawn,
                            AgentCollaborationOutcomeKind::Accepted,
                        )
                        .child(&child_handle);
                    if let Some(summary) =
                        Some(spawn_description.trim()).filter(|value| !value.is_empty())
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

        let accepted_sub_run_id = accepted.turn_id.to_string();
        let accepted_child_session_id = accepted.session_id.to_string();
        let handoff_artifacts = spawn_handoff_artifacts(
            child_handle_for_handoff.as_ref(),
            &accepted_sub_run_id,
            accepted.agent_id.as_deref(),
            &accepted_child_session_id,
            &parent_session_id,
            &parent_agent_id,
        );

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
        ChildSessionNotificationKind, ResolvedExecutionLimitsSnapshot, SessionId, SpawnAgentParams,
        StorageEventPayload, ToolContext, agent::executor::SubAgentExecutor,
    };

    use super::{
        IMPLICIT_ROOT_PROFILE_ID, build_delegation_metadata, build_fresh_child_contract,
        build_resumed_child_contract, child_delivery_mailbox_envelope,
        implicit_session_root_agent_id, root_execution_event_context,
        terminal_notification_message, terminal_notification_turn_outcome,
    };
    use crate::agent::test_support::{
        TestLlmBehavior, build_agent_test_harness, build_agent_test_harness_with_agent_config,
    };

    fn assert_no_legacy_kernel_agent_methods(source: &str, file: &str) {
        let production_source = source
            .split_once("#[cfg(test)]")
            .map(|(prefix, _)| prefix)
            .unwrap_or(source);
        let forbidden = [
            ".get_agent_handle(",
            ".get_agent_lifecycle(",
            ".get_agent_turn_outcome(",
            ".deliver_to_agent(",
            ".drain_agent_inbox(",
            ".resume_agent(",
            ".collect_agent_subtree_handles(",
            ".terminate_agent_subtree(",
        ];

        for pattern in forbidden {
            assert!(
                !production_source.contains(pattern),
                "{file} should use kernel.agent() stable surface instead of legacy Kernel method \
                 {pattern}"
            );
        }
    }

    fn assert_agent_control_boundary(source: &str, file: &str) {
        let production_source = source
            .split_once("#[cfg(test)]")
            .map(|(prefix, _)| prefix)
            .unwrap_or(source);
        let direct_agent_control_count = production_source.matches(".agent_control()").count();

        if file == "terminal.rs" {
            assert_eq!(
                direct_agent_control_count, 1,
                "terminal.rs should keep exactly one direct AgentControl access for complete_turn \
                 lifecycle finalization"
            );
            assert!(
                production_source.contains(".complete_turn("),
                "terminal.rs direct AgentControl access must be reserved for complete_turn"
            );
            return;
        }

        assert_eq!(
            direct_agent_control_count, 0,
            "{file} production code should use kernel.agent() stable surface instead of direct \
             AgentControl access"
        );
    }

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
    fn agent_orchestration_sources_use_kernel_agent_surface() {
        let sources = [
            ("mod.rs", include_str!("mod.rs")),
            ("observe.rs", include_str!("observe.rs")),
            ("routing.rs", include_str!("routing.rs")),
            ("terminal.rs", include_str!("terminal.rs")),
            ("wake.rs", include_str!("wake.rs")),
        ];

        for (file, source) in sources {
            assert_no_legacy_kernel_agent_methods(source, file);
            assert_agent_control_boundary(source, file);
        }
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

    #[test]
    fn fresh_child_contract_exposes_responsibility_and_capability_limit() {
        let metadata = build_delegation_metadata(
            "审查缓存层",
            "检查缓存一致性",
            &ResolvedExecutionLimitsSnapshot {
                allowed_tools: vec!["readFile".to_string(), "grep".to_string()],
                max_steps: Some(8),
            },
            true,
        );

        let contract = build_fresh_child_contract(&metadata);

        assert_eq!(contract.origin.as_deref(), Some("child-contract:fresh"));
        assert!(contract.content.contains("审查缓存层"));
        assert!(contract.content.contains("本分支当前只允许使用这些工具"));
    }

    #[test]
    fn resumed_child_contract_keeps_branch_and_delta_instruction() {
        let metadata = build_delegation_metadata(
            "审查缓存层",
            "检查缓存一致性",
            &ResolvedExecutionLimitsSnapshot {
                allowed_tools: vec!["readFile".to_string(), "grep".to_string()],
                max_steps: Some(8),
            },
            false,
        );

        let contract =
            build_resumed_child_contract(&metadata, "补充验证过期路径", Some("优先看失效逻辑"));

        assert_eq!(contract.origin.as_deref(), Some("child-contract:resumed"));
        assert!(contract.content.contains("审查缓存层"));
        assert!(contract.content.contains("补充验证过期路径"));
        assert!(contract.content.contains("优先看失效逻辑"));
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
                    capability_grant: None,
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
            .agent()
            .query_root_status(&parent.session_id)
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
                    capability_grant: None,
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
            .agent()
            .get_handle(&child_agent_id)
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
        assert_eq!(
            child_handle
                .delegation
                .as_ref()
                .map(|metadata| metadata.responsibility_summary.as_str()),
            Some("仓库审查"),
            "fresh launch should persist the delegated responsibility branch"
        );
        assert!(
            child_handle
                .delegation
                .as_ref()
                .is_some_and(|metadata| !metadata.restricted),
            "fresh launch without grant should not mark the branch as restricted"
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
                    capability_grant: None,
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
                    capability_grant: None,
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
