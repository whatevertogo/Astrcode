//! # 运行时接口
//!
//! 定义了运行时组件的抽象接口，用于管理 LLM 连接和生命周期。
//!
//! ## 核心接口
//!
//! - [`RuntimeHandle`][]: 运行时主句柄，提供名称、类型和关闭接口
//! - [`ManagedRuntimeComponent`][]: 可被运行时协调器管理的子组件

use async_trait::async_trait;

use crate::{
    AgentId, AgentProfile, AstrError, SessionEventRecord, SessionId, SessionMeta, SubRunResult,
    SubagentContextOverrides, TurnId, agent::lineage::SubRunHandle, tool::ToolContext,
};

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

/// 会话边界：负责 durable truth 与会话目录语义。
#[async_trait]
pub trait SessionTruthBoundary: Send + Sync {
    async fn create_session(
        &self,
        working_dir: &std::path::Path,
    ) -> std::result::Result<SessionMeta, AstrError>;

    async fn list_sessions(&self) -> std::result::Result<Vec<SessionMeta>, AstrError>;

    async fn load_history(
        &self,
        session_id: &SessionId,
    ) -> std::result::Result<Vec<SessionEventRecord>, AstrError>;
}

/// 执行边界：负责 submit/interrupt/root-execute。
///
/// `launch_subagent` 已迁入 [`LiveSubRunControlBoundary`]，
/// 因为它依赖 live child ownership、tool context 和 active control tree，
/// 而不是纯 root orchestration。
#[async_trait]
pub trait ExecutionOrchestrationBoundary: Send + Sync {
    async fn submit_prompt(
        &self,
        session_id: &SessionId,
        text: String,
    ) -> std::result::Result<ExecutionAccepted, AstrError>;

    async fn interrupt_session(&self, session_id: &SessionId)
    -> std::result::Result<(), AstrError>;

    async fn execute_root_agent(
        &self,
        agent_id: AgentId,
        task: String,
        context: Option<String>,
        context_overrides: Option<SubagentContextOverrides>,
        working_dir: std::path::PathBuf,
    ) -> std::result::Result<ExecutionAccepted, AstrError>;
}

/// 主循环边界：负责单次 turn 的模型/工具循环。
#[async_trait]
pub trait LoopRunnerBoundary: Send + Sync {
    async fn run_session_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> std::result::Result<(), AstrError>;
}

/// live 子执行控制平面边界。
///
/// 包含 subrun 句柄查询、取消、agent 启动和 profile 枚举。
#[async_trait]
pub trait LiveSubRunControlBoundary: Send + Sync {
    async fn get_subrun_handle(
        &self,
        session_id: &SessionId,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError>;

    async fn cancel_subrun(
        &self,
        session_id: &SessionId,
        sub_run_id: &str,
    ) -> std::result::Result<(), AstrError>;

    /// 启动子 agent 执行。
    ///
    /// 从 `ExecutionOrchestrationBoundary` 迁入，因为该操作依赖
    /// live child ownership、tool context 和 active control tree。
    async fn launch_subagent(
        &self,
        params: crate::SpawnAgentParams,
        ctx: &ToolContext,
    ) -> std::result::Result<SubRunResult, AstrError>;

    async fn list_profiles(&self) -> std::result::Result<Vec<AgentProfile>, AstrError>;
}
