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
mod wake;

use std::sync::Arc;

use astrcode_core::{
    AgentLifecycleStatus, AgentMode, ArtifactRef, CloseAgentParams, CollaborationResult,
    ObserveParams, Result, RuntimeMetricsRecorder, SendAgentParams, SpawnAgentParams,
    SubRunHandoff, SubRunResult, ToolContext,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use async_trait::async_trait;
use thiserror::Error;

use crate::execution::{SubagentExecutionRequest, launch_subagent};

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
    default_token_budget: Option<u64>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
}

impl AgentOrchestrationService {
    pub fn new(
        kernel: Arc<Kernel>,
        session_runtime: Arc<SessionRuntime>,
        default_token_budget: Option<u64>,
        metrics: Arc<dyn RuntimeMetricsRecorder>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
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

        // 构造 AgentProfile
        let profile = astrcode_core::AgentProfile {
            id: profile_id.clone(),
            name: profile_id.clone(),
            description: params.description.clone(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            model_preference: None,
        };

        let request = SubagentExecutionRequest {
            parent_session_id: parent_session_id.clone(),
            parent_agent_id,
            parent_turn_id,
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
