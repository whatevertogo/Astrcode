//! # Agent 编排子域
//!
//! 承接四工具模型（spawn / send / observe / close）的业务编排，
//! 以及父级 delivery 唤醒调度。
//!
//! `AgentOrchestrationService` 是本子域的唯一服务入口，实现
//! `SubAgentExecutor` 和 `CollaborationExecutor` 两个 trait，
//! 通过 agent 子域专用的 kernel/session 端口完成所有操作。
//!
//! 架构约束：
//! - 不持有 session shadow state
//! - 不直接依赖 adapter-*
//! - 不缓存 session 引用

mod context;
mod observe;
mod routing;
mod terminal;
#[cfg(test)]
pub(crate) mod test_support;
mod wake;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationOutcomeKind, AgentEventContext,
    AgentLifecycleStatus, AgentTurnOutcome, ArtifactRef, CloseAgentParams, CollaborationResult,
    DelegationMetadata, ObserveParams, ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
    ParentDeliveryTerminalSemantics, ProgressParentDeliveryPayload, QueuedInputEnvelope,
    ResolvedExecutionLimitsSnapshot, Result, RuntimeMetricsRecorder, SendAgentParams,
    SpawnAgentParams, SubRunHandoff, SubRunResult, ToolContext,
};
use astrcode_host_session::{CollaborationExecutor, SubAgentExecutor, SubRunHandle};
use async_trait::async_trait;
pub(crate) use context::{
    CollaborationFactRecord, ToolCollaborationContext, ToolCollaborationContextInput,
    implicit_session_root_agent_id,
};
use thiserror::Error;

use crate::{
    AgentKernelPort, AgentSessionPort,
    config::ConfigService,
    execution::{
        LaunchedSubagent, ProfileResolutionService, SubagentExecutionRequest, launch_subagent,
    },
    governance_surface::{
        GOVERNANCE_POLICY_REVISION, GovernanceSurfaceAssembler, build_delegation_metadata,
    },
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
    agent_id: impl Into<astrcode_core::AgentId>,
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
pub(crate) const AGENT_COLLABORATION_POLICY_REVISION: &str = GOVERNANCE_POLICY_REVISION;
const MAX_OBSERVE_GUARD_ENTRIES: usize = 1024;


pub(crate) async fn persist_resolved_limits_for_handle(
    kernel: &dyn AgentKernelPort,
    handle: SubRunHandle,
    resolved_limits: ResolvedExecutionLimitsSnapshot,
) -> std::result::Result<SubRunHandle, String> {
    if kernel
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
    kernel: &dyn AgentKernelPort,
    handle: SubRunHandle,
    delegation: DelegationMetadata,
) -> std::result::Result<SubRunHandle, String> {
    if kernel
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

/// 将 child terminal notification 包装为 durable input queue 信封。
///
/// 提取 notification 中 delivery 的消息文本作为信封内容，
/// 同时携带 sender 的 lifecycle status、turn outcome 和 open session id，
/// 供父级 wake turn 消费时了解子代理的最新状态。
pub(crate) fn child_delivery_input_queue_envelope(
    notification: &astrcode_core::ChildSessionNotification,
    target_agent_id: String,
) -> QueuedInputEnvelope {
    QueuedInputEnvelope {
        delivery_id: notification.notification_id.clone(),
        from_agent_id: notification.child_ref.agent_id().to_string(),
        to_agent_id: target_agent_id,
        message: terminal_notification_message(notification),
        queued_at: chrono::Utc::now(),
        sender_lifecycle_status: notification.child_ref.status,
        sender_last_turn_outcome: terminal_notification_turn_outcome(notification),
        sender_open_session_id: notification.child_ref.open_session_id.to_string(),
    }
}

/// 从 notification 的 delivery payload 中提取可读消息文本。
///
/// 优先使用 delivery.payload.message()，为空时回退到默认提示。
/// 这是终端通知、durable input queue、wake turn 共享的消息提取逻辑。
pub(crate) fn terminal_notification_message(
    notification: &astrcode_core::ChildSessionNotification,
) -> String {
    notification
        .delivery
        .as_ref()
        .map(|delivery| delivery.payload.message().trim())
        .filter(|message| !message.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "子 Agent 未提供可读交付。".to_string())
}

/// 从 child notification 推断 sender 的 last turn outcome。
///
/// 仅在 child 处于 Idle 状态时才有有效的 outcome（表示已完成一轮 turn）。
/// 根据 delivery payload 类型映射：Completed → Completed, Failed → Failed, CloseRequest →
/// Cancelled。 用于 durable input queue 信封中的 sender 状态追踪。
pub(crate) fn terminal_notification_turn_outcome(
    notification: &astrcode_core::ChildSessionNotification,
) -> Option<AgentTurnOutcome> {
    if !matches!(notification.child_ref.status, AgentLifecycleStatus::Idle) {
        return None;
    }
    if let Some(delivery) = &notification.delivery {
        return match delivery.payload {
            astrcode_core::ParentDeliveryPayload::Completed(_) => Some(AgentTurnOutcome::Completed),
            astrcode_core::ParentDeliveryPayload::Failed(_) => Some(AgentTurnOutcome::Failed),
            astrcode_core::ParentDeliveryPayload::CloseRequest(_) => {
                Some(AgentTurnOutcome::Cancelled)
            },
            astrcode_core::ParentDeliveryPayload::Progress(_) => None,
        };
    }
    match notification.kind {
        astrcode_core::ChildSessionNotificationKind::Delivered => Some(AgentTurnOutcome::Completed),
        astrcode_core::ChildSessionNotificationKind::Failed => Some(AgentTurnOutcome::Failed),
        astrcode_core::ChildSessionNotificationKind::Closed => Some(AgentTurnOutcome::Cancelled),
        _ => None,
    }
}

pub(crate) fn child_open_session_id(child: &SubRunHandle) -> String {
    child.open_session_id().to_string()
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

/// 构建子代理协作的 artifact 引用列表（subRun + agent + parentSession + session + parentAgent）。
///
/// `include_parent_sub_run` 控制 是否包含 parentSubRun artifact，
/// spawn handoff 时不需要（因为已有 subRun），其他场景需要。
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
    child_handle: &SubRunHandle,
    parent_session_id: &str,
) -> Vec<ArtifactRef> {
    let mut artifacts = vec![artifact_ref(
        "subRun",
        child_handle.sub_run_id.clone(),
        "Sub Run",
        Some(parent_session_id.to_string()),
    )];
    artifacts.extend(child_reference_artifacts(
        child_handle,
        parent_session_id,
        false,
    ));
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
/// 持有 agent 子域专用的 kernel/session 端口，
/// 不持有 session shadow state，不缓存 session 引用。
#[derive(Clone)]
pub struct AgentOrchestrationService {
    kernel: Arc<dyn AgentKernelPort>,
    session_runtime: Arc<dyn AgentSessionPort>,
    config_service: Arc<ConfigService>,
    profiles: Arc<ProfileResolutionService>,
    governance_surface: Arc<GovernanceSurfaceAssembler>,
    task_registry: Arc<TaskRegistry>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    observe_guard: Arc<Mutex<ObserveGuardState>>,
}

impl AgentOrchestrationService {
    pub fn new(
        kernel: Arc<dyn AgentKernelPort>,
        session_runtime: Arc<dyn AgentSessionPort>,
        config_service: Arc<ConfigService>,
        profiles: Arc<ProfileResolutionService>,
        governance_surface: Arc<GovernanceSurfaceAssembler>,
        task_registry: Arc<TaskRegistry>,
        metrics: Arc<dyn RuntimeMetricsRecorder>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            config_service,
            profiles,
            governance_surface,
            task_registry,
            metrics,
            observe_guard: Arc::new(Mutex::new(ObserveGuardState::default())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObserveSnapshotSignature {
    lifecycle_status: AgentLifecycleStatus,
    last_turn_outcome: Option<AgentTurnOutcome>,
    phase: String,
    turn_count: u32,
    active_task: Option<String>,
    last_output_tail: Option<String>,
    last_turn_tail: Vec<String>,
}

#[derive(Debug, Clone)]
struct ObserveGuardEntry {
    sequence: u64,
    signature: ObserveSnapshotSignature,
}

#[derive(Debug, Default)]
struct ObserveGuardState {
    next_sequence: u64,
    entries: HashMap<String, ObserveGuardEntry>,
}

impl ObserveGuardState {
    fn is_unchanged(&self, key: &str, signature: &ObserveSnapshotSignature) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| &entry.signature == signature)
    }

    fn remember(&mut self, key: String, signature: ObserveSnapshotSignature) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.insert(
            key.clone(),
            ObserveGuardEntry {
                sequence,
                signature,
            },
        );
        self.evict_oldest_if_needed(&key);
    }

    fn evict_oldest_if_needed(&mut self, keep_key: &str) {
        if self.entries.len() <= MAX_OBSERVE_GUARD_ENTRIES {
            return;
        }
        let Some(oldest_key) = self
            .entries
            .iter()
            .filter(|(key, _)| key.as_str() != keep_key)
            .min_by_key(|(_, entry)| entry.sequence)
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        self.entries.remove(&oldest_key);
    }
}

// ── 实现 SubAgentExecutor（供 spawn 工具使用）──────────────────────

#[async_trait]
impl SubAgentExecutor for AgentOrchestrationService {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let parent_handle = self
            .ensure_parent_agent_handle(ctx)
            .await
            .map_err(map_orchestration_error)?;
        let collaboration = self
            .tool_collaboration_context(ctx)
            .await
            .map_err(map_orchestration_error)?
            .with_parent_agent_id(Some(parent_handle.agent_id.to_string()));
        let parent_agent_id = parent_handle.agent_id.to_string();
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
            mode_id: collaboration.mode_id().clone(),
            profile,
            description: spawn_description.clone(),
            task: params.prompt,
            context: params.context,
            source_tool_call_id: ctx.tool_call_id().map(ToString::to_string),
        };
        if let Err(error) = self
            .enforce_spawn_budget_for_turn(
                &parent_agent_id,
                &request.parent_turn_id,
                collaboration.policy().max_spawn_per_turn,
            )
            .await
        {
            return self
                .reject_spawn(&collaboration, "spawn_budget_exhausted", error)
                .await;
        }

        let launched = match launch_subagent(
            self.kernel.as_ref(),
            self.session_runtime.as_ref(),
            self.governance_surface.as_ref(),
            request,
            runtime_config.clone(),
            &self.metrics,
        )
        .await
        {
            Ok(launched) => launched,
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
        let LaunchedSubagent { accepted, handle } = launched;
        if let Some(parent_turn_id) = ctx.turn_id() {
            let fact = {
                let mut fact = collaboration
                    .fact(
                        AgentCollaborationActionKind::Spawn,
                        AgentCollaborationOutcomeKind::Accepted,
                    )
                    .child(&handle);
                if let Some(summary) =
                    Some(spawn_description.trim()).filter(|value| !value.is_empty())
                {
                    fact = fact.summary(summary.to_string());
                }
                fact
            };
            self.record_fact_best_effort(&runtime_config, fact).await;
            self.spawn_child_turn_terminal_watcher(
                handle.clone(),
                accepted.session_id.to_string(),
                accepted.turn_id.to_string(),
                parent_session_id.clone(),
                parent_turn_id.to_string(),
                collaboration.source_tool_call_id(),
            );
        }

        let handoff_artifacts = spawn_handoff_artifacts(&handle, &parent_session_id);

        Ok(SubRunResult::Running {
            handoff: SubRunHandoff {
                findings: Vec::new(),
                artifacts: handoff_artifacts,
                delivery: Some(ParentDelivery {
                    idempotency_key: format!("subrun-started:{}", handle.sub_run_id),
                    origin: ParentDeliveryOrigin::Explicit,
                    terminal_semantics: ParentDeliveryTerminalSemantics::NonTerminal,
                    source_turn_id: None,
                    payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                        message: if params.description.trim().is_empty() {
                            "子 Agent 已启动。".to_string()
                        } else {
                            format!("子 Agent 已启动：{}", params.description.trim())
                        },
                    }),
                }),
            },
        })
    }
}

// ── 实现 CollaborationExecutor（供 send/close/observe 工具使用）─────

#[async_trait]
impl CollaborationExecutor for AgentOrchestrationService {
    async fn send(
        &self,
        params: SendAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        self.route_send(params, ctx)
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
        CancelToken, ChildAgentRef, ChildExecutionIdentity, ChildSessionLineageKind,
        ChildSessionNotification, ChildSessionNotificationKind, ParentExecutionRef,
        ResolvedExecutionLimitsSnapshot, SessionId, SpawnAgentParams, StorageEventPayload,
        ToolContext,
    };
    use astrcode_host_session::SubAgentExecutor;

    use super::{
        IMPLICIT_ROOT_PROFILE_ID, build_delegation_metadata, child_delivery_input_queue_envelope,
        context::implicit_session_root_agent_id, root_execution_event_context,
        terminal_notification_message, terminal_notification_turn_outcome,
    };
    use crate::{
        agent::test_support::{
            TestLlmBehavior, build_agent_test_harness, build_agent_test_harness_with_agent_config,
        },
        governance_surface::{build_fresh_child_contract, build_resumed_child_contract},
    };

    fn assert_no_removed_kernel_agent_methods(source: &str, file: &str) {
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
                "{file} should use kernel.agent() stable surface instead of removed Kernel method \
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

        assert_eq!(
            direct_agent_control_count, 0,
            "{file} production code should depend on the agent-domain port surface instead of \
             direct AgentControl access"
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
            assert_no_removed_kernel_agent_methods(source, file);
            assert_agent_control_boundary(source, file);
        }
    }

    #[test]
    fn child_delivery_input_queue_envelope_reuses_terminal_projection_fields() {
        let notification = ChildSessionNotification {
            notification_id: "delivery-1".to_string().into(),
            child_ref: ChildAgentRef {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child".to_string().into(),
                    session_id: "session-parent".to_string().into(),
                    sub_run_id: "subrun-child".to_string().into(),
                },
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-parent".to_string().into()),
                    parent_sub_run_id: Some("subrun-parent".to_string().into()),
                },
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                open_session_id: "session-child".to_string().into(),
            },
            kind: ChildSessionNotificationKind::Delivered,
            source_tool_call_id: None,
            delivery: Some(astrcode_core::ParentDelivery {
                idempotency_key: "delivery-1".to_string(),
                origin: astrcode_core::ParentDeliveryOrigin::Explicit,
                terminal_semantics: astrcode_core::ParentDeliveryTerminalSemantics::Terminal,
                source_turn_id: Some("turn-child".to_string()),
                payload: astrcode_core::ParentDeliveryPayload::Completed(
                    astrcode_core::CompletedParentDeliveryPayload {
                        message: "final reply".to_string(),
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    },
                ),
            }),
        };

        let envelope =
            child_delivery_input_queue_envelope(&notification, "agent-parent".to_string());

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
    fn fresh_child_contract_exposes_responsibility_boundary() {
        let metadata = build_delegation_metadata(
            "审查缓存层",
            "检查缓存一致性",
            &ResolvedExecutionLimitsSnapshot,
            true,
        );

        let contract = build_fresh_child_contract(&metadata);

        assert_eq!(contract.origin.as_deref(), Some("child-contract:fresh"));
        assert!(contract.content.contains("审查缓存层"));
        assert!(contract.content.contains("Fresh-child rule"));
    }

    #[test]
    fn resumed_child_contract_keeps_branch_and_delta_instruction() {
        let metadata = build_delegation_metadata(
            "审查缓存层",
            "检查缓存一致性",
            &ResolvedExecutionLimitsSnapshot,
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
                },
                &ctx,
            )
            .await
            .expect("subagent should launch with implicit root parent");

        let parent_agent_artifact = result
            .handoff()
            .expect("handoff should exist")
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "parentAgent")
            .expect("parent agent artifact should exist");
        let expected_parent_agent_id = implicit_session_root_agent_id(&parent.session_id);
        assert_eq!(parent_agent_artifact.id, expected_parent_agent_id);

        let root_status = harness
            .session_runtime
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
            .session_runtime
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

        let handoff = result.handoff().cloned().expect("handoff should exist");
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
            .session_runtime
            .agent()
            .get_handle(&child_agent_id)
            .await
            .expect("child handle should exist");
        assert_eq!(
            child_handle.session_id.to_string(),
            parent.session_id,
            "independent child should remain attached to parent session in control tree"
        );
        assert_eq!(
            child_handle
                .child_session_id
                .as_ref()
                .map(|id| id.to_string()),
            Some(child_session_id.clone()),
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
                .is_some_and(|metadata| metadata.reuse_scope_summary.contains("同一责任分支")),
            "fresh launch should persist a reusable branch boundary summary"
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
    async fn launch_uses_stable_child_subrun_id_in_spawn_handoff() {
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
            .session_runtime
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
        .with_agent_context(root_execution_event_context("root-agent", "root-profile"))
        .with_tool_call_id("call-spawn".to_string());

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
            .expect("subagent should launch");

        let handoff = result.handoff().cloned().expect("handoff should exist");
        let child_agent_id = handoff
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "agent")
            .map(|artifact| artifact.id.clone())
            .expect("child agent artifact should exist");
        let sub_run_artifact = handoff
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "subRun")
            .expect("subRun artifact should exist");
        let child_handle = harness
            .session_runtime
            .agent()
            .get_handle(&child_agent_id)
            .await
            .expect("child handle should exist");

        assert_eq!(
            sub_run_artifact.id,
            child_handle.sub_run_id.as_str(),
            "spawn handoff must expose the stable child subRunId instead of the initial child \
             turn id"
        );
        let expected_delivery_id = format!("subrun-started:{}", child_handle.sub_run_id);
        assert_eq!(
            handoff
                .delivery
                .as_ref()
                .map(|delivery| delivery.idempotency_key.as_str()),
            Some(expected_delivery_id.as_str()),
            "spawn progress delivery key must be derived from the stable child subRunId"
        );

        let spawn_fact_sub_run_id = harness
            .session_runtime
            .replay_stored_events(&SessionId::from(parent.session_id.clone()))
            .await
            .expect("parent events should replay")
            .into_iter()
            .find_map(|stored| match stored.event.payload {
                StorageEventPayload::AgentCollaborationFact { fact, .. }
                    if fact.action == AgentCollaborationActionKind::Spawn
                        && fact.outcome == AgentCollaborationOutcomeKind::Accepted
                        && fact.source_tool_call_id.as_deref() == Some("call-spawn") =>
                {
                    fact.child_identity
                        .as_ref()
                        .map(|identity| identity.sub_run_id.to_string())
                },
                _ => None,
            })
            .expect("spawn accepted fact should exist");

        assert_eq!(
            spawn_fact_sub_run_id,
            child_handle.sub_run_id.as_str(),
            "spawn fact and handoff must agree on the same stable child subRunId"
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
            .session_runtime
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
