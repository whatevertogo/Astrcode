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

mod mailbox;
mod observe;
mod routing;
mod terminal;
#[cfg(test)]
mod test_support;
mod turn_watch;
mod wake;

use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentEventContext, AgentLifecycleStatus, AgentMailboxEnvelope, AgentTurnOutcome, ArtifactRef,
    CloseAgentParams, CollaborationResult, ObserveParams, Result, RuntimeMetricsRecorder,
    SendAgentParams, SpawnAgentParams, SubRunHandle, SubRunHandoff, SubRunResult, ToolContext,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use async_trait::async_trait;
use thiserror::Error;

use crate::{
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
    profiles: Arc<ProfileResolutionService>,
    task_registry: Arc<TaskRegistry>,
    default_token_budget: Option<u64>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
}

impl AgentOrchestrationService {
    pub fn new(
        kernel: Arc<Kernel>,
        session_runtime: Arc<SessionRuntime>,
        profiles: Arc<ProfileResolutionService>,
        task_registry: Arc<TaskRegistry>,
        default_token_budget: Option<u64>,
        metrics: Arc<dyn RuntimeMetricsRecorder>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            profiles,
            task_registry,
            default_token_budget,
            metrics,
        }
    }

    /// 返回默认 RuntimeConfig 用于 wake / resume 场景。
    fn default_runtime_config(&self) -> astrcode_core::config::RuntimeConfig {
        astrcode_core::config::RuntimeConfig {
            default_token_budget: self.default_token_budget,
            ..Default::default()
        }
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
}

// ── 实现 SubAgentExecutor（供 spawn 工具使用）──────────────────────

#[async_trait]
impl astrcode_core::SubAgentExecutor for AgentOrchestrationService {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let parent_agent_id = ctx.agent_context().agent_id.clone().unwrap_or_default();
        let parent_turn_id = ctx.turn_id().unwrap_or("unknown-turn").to_string();
        let parent_session_id = ctx.session_id().to_string();
        let profile_id = params
            .r#type
            .clone()
            .unwrap_or_else(|| "explore".to_string());
        let profile = self
            .resolve_subagent_profile(ctx.working_dir(), &profile_id)
            .map_err(map_orchestration_error)?;

        let request = SubagentExecutionRequest {
            parent_session_id: parent_session_id.clone(),
            parent_agent_id,
            parent_turn_id,
            working_dir: ctx.working_dir().display().to_string(),
            profile,
            task: params.prompt,
            context: params.context,
        };

        let accepted = launch_subagent(
            &self.kernel,
            &self.session_runtime,
            request,
            self.default_runtime_config(),
            &self.metrics,
        )
        .await
        .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;
        if let (Some(child_agent_id), Some(parent_turn_id)) =
            (accepted.agent_id.clone(), ctx.turn_id())
        {
            if let Some(child_handle) = self.kernel.get_agent_handle(&child_agent_id).await {
                self.spawn_child_terminal_watcher(
                    child_handle,
                    accepted.session_id.to_string(),
                    accepted.turn_id.to_string(),
                    parent_session_id.clone(),
                    parent_turn_id.to_string(),
                    ctx.tool_call_id().map(ToString::to_string),
                );
            }
        }

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
                artifacts: vec![
                    ArtifactRef {
                        kind: "subRun".to_string(),
                        id: accepted.turn_id.to_string(),
                        label: "Sub Run".to_string(),
                        session_id: Some(parent_session_id),
                        storage_seq: None,
                        uri: None,
                    },
                    ArtifactRef {
                        kind: "agent".to_string(),
                        id: accepted.agent_id.clone().unwrap_or_default().to_string(),
                        label: "Agent".to_string(),
                        session_id: Some(accepted.session_id.to_string()),
                        storage_seq: None,
                        uri: None,
                    },
                    ArtifactRef {
                        kind: "parentSession".to_string(),
                        id: ctx.session_id().to_string(),
                        label: "Parent Session".to_string(),
                        session_id: Some(ctx.session_id().to_string()),
                        storage_seq: None,
                        uri: None,
                    },
                    ArtifactRef {
                        kind: "session".to_string(),
                        id: accepted.session_id.to_string(),
                        label: "Child Session".to_string(),
                        session_id: Some(accepted.session_id.to_string()),
                        storage_seq: None,
                        uri: None,
                    },
                    ArtifactRef {
                        kind: "parentAgent".to_string(),
                        id: ctx.agent_context().agent_id.clone().unwrap_or_default(),
                        label: "Parent Agent".to_string(),
                        session_id: Some(ctx.session_id().to_string()),
                        storage_seq: None,
                        uri: None,
                    },
                ],
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
        AgentLifecycleStatus, ChildAgentRef, ChildSessionLineageKind, ChildSessionNotification,
        ChildSessionNotificationKind,
    };

    use super::{
        child_delivery_mailbox_envelope, root_execution_event_context,
        terminal_notification_message, terminal_notification_turn_outcome,
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
}
