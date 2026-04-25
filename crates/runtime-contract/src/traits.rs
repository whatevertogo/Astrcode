//! # 运行时接口
//!
//! 定义了运行时组件的抽象接口，用于管理 LLM 连接和生命周期。
//!
//! ## 核心接口
//!
//! - [`RuntimeHandle`][]: 运行时主句柄，提供名称、类型和关闭接口
//! - [`ManagedRuntimeComponent`][]: 可被运行时协调器管理的子组件

use astrcode_core::{AgentId, AstrError, SessionId, TurnId};
use async_trait::async_trait;

/// 运行时主句柄。
///
/// 代表一个具体的 LLM 运行时实现（如 OpenAI 兼容 API 客户端）。
/// 生命周期由组合根的运行时协调设施统一管理。
#[async_trait]
pub trait RuntimeHandle: Send + Sync {
    /// 运行时实例的名称（用于日志和错误信息）。
    fn runtime_name(&self) -> &'static str;

    /// 运行时的类型标识（如 "openai"）。
    fn runtime_kind(&self) -> &'static str;

    /// 优雅关闭运行时，释放所有连接和资源。
    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError>;
}

/// 可被运行时协调器管理的子组件。
///
/// 用于管理除主运行时之外的其他需要生命周期管理的组件
/// （如 SSE 广播器、后台任务等）。
#[async_trait]
pub trait ManagedRuntimeComponent: Send + Sync {
    /// 组件名称（用于日志和错误信息）。
    fn component_name(&self) -> String;

    /// 优雅关闭组件，释放资源。
    async fn shutdown_component(&self) -> std::result::Result<(), AstrError>;
}

/// 统一执行回执。
///
/// 替代之前的 `PromptAccepted` / `RootExecutionAccepted` / runtime 重复 receipt。
/// 内部 contract 不再分裂，HTTP 路由可按需做 DTO 投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionAccepted {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    /// 仅 root execute 等有独立 agent 时存在。
    pub agent_id: Option<AgentId>,
    /// 仅 prompt submit 分支场景存在。
    pub branched_from_session_id: Option<String>,
}

/// Prompt submit 的稳定结果。
///
/// `Accepted` 表示 host-session 已创建 turn 并进入后续 runtime 执行；
/// `Handled` 表示 input hook 已处理输入，不应创建 turn。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionSubmissionOutcome {
    Accepted(ExecutionAccepted),
    Handled {
        session_id: SessionId,
        response: String,
    },
}

impl ExecutionSubmissionOutcome {
    pub fn accepted(accepted: ExecutionAccepted) -> Self {
        Self::Accepted(accepted)
    }

    pub fn handled(session_id: SessionId, response: impl Into<String>) -> Self {
        Self::Handled {
            session_id,
            response: response.into(),
        }
    }

    pub fn accepted_ref(&self) -> Option<&ExecutionAccepted> {
        match self {
            Self::Accepted(accepted) => Some(accepted),
            Self::Handled { .. } => None,
        }
    }

    pub fn into_accepted(self) -> Option<ExecutionAccepted> {
        match self {
            Self::Accepted(accepted) => Some(accepted),
            Self::Handled { .. } => None,
        }
    }
}
