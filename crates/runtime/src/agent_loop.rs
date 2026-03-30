//! # Agent 循环
//!
//! 实现单个 Agent Turn 的执行逻辑。

mod llm_cycle;
mod tool_cycle;
mod turn_runner;

use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CancelToken, CapabilityRouter, PolicyContext, PolicyEngine,
    Result, ToolContext, DEFAULT_MAX_OUTPUT_SIZE,
};
use chrono::Utc;
use std::sync::Arc;

use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::prompt::PromptComposer;
use crate::provider_factory::DynProviderFactory;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;

/// Agent 循环
///
/// 负责执行单个 Agent Turn，包含：
/// - LLM 调用（获取助手响应）
/// - 工具执行（调用外部能力）
/// - 策略检查（审批流程）
pub struct AgentLoop {
    /// LLM 提供者工厂
    factory: DynProviderFactory,
    /// 能力路由器（工具注册表）
    capabilities: CapabilityRouter,
    /// 策略引擎
    policy: Arc<dyn PolicyEngine>,
    /// 审批代理
    approval: Arc<dyn ApprovalBroker>,
    /// 最大步数限制（防止无限循环）
    max_steps: Option<usize>,
    /// Prompt 组装器
    prompt_composer: PromptComposer,
}

impl AgentLoop {
    /// 从能力路由器创建 AgentLoop
    pub fn from_capabilities(factory: DynProviderFactory, capabilities: CapabilityRouter) -> Self {
        Self {
            factory,
            capabilities,
            policy: Arc::new(AllowAllPolicyEngine),
            approval: Arc::new(DefaultApprovalBroker),
            max_steps: None,
            prompt_composer: PromptComposer::with_defaults(),
        }
    }

    /// 设置策略引擎
    pub fn with_policy_engine(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = policy;
        self
    }

    /// 设置审批代理
    pub fn with_approval_broker(mut self, approval: Arc<dyn ApprovalBroker>) -> Self {
        self.approval = approval;
        self
    }

    /// 设置最大步数
    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = Some(max_steps);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_prompt_composer(mut self, prompt_composer: PromptComposer) -> Self {
        self.prompt_composer = prompt_composer;
        self
    }

    /// 执行一个 Agent Turn
    ///
    /// ## 执行流程
    ///
    /// 1. 组装 Prompt（包含历史消息和系统提示）
    /// 2. 调用 LLM 获取助手响应
    /// 3. 如果响应包含工具调用，执行工具
    /// 4. 将工具结果反馈给 LLM
    /// 5. 重复直到没有更多工具调用
    ///
    /// ## 事件
    ///
    /// 每个重要步骤都通过 `on_event` 回调发出 `StorageEvent`。
    ///
    /// ## IO
    ///
    /// AgentLoop 本身不执行文件 IO，只进行 LLM 调用和工具执行。
    pub async fn run_turn(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
    ) -> Result<()> {
        turn_runner::run_turn(self, state, turn_id, on_event, cancel).await
    }

    /// 创建工具执行上下文
    pub(crate) fn tool_context(&self, state: &AgentState, cancel: CancelToken) -> ToolContext {
        ToolContext {
            session_id: state.session_id.clone(),
            working_dir: state.working_dir.clone(),
            cancel,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        }
    }

    /// 创建策略上下文
    pub(crate) fn policy_context(
        &self,
        state: &AgentState,
        turn_id: &str,
        step_index: usize,
    ) -> PolicyContext {
        PolicyContext {
            session_id: state.session_id.clone(),
            turn_id: turn_id.to_string(),
            step_index,
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            profile: "coding".to_string(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// 完成 Turn（发出 TurnDone 事件）
pub(crate) fn finish_turn(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::TurnDone {
        turn_id: Some(turn_id.to_string()),
        timestamp: Utc::now(),
    })
}

/// 完成并发出错误事件
pub(crate) fn finish_with_error(
    turn_id: &str,
    message: impl Into<String>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        message: message.into(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, on_event)
}

/// 完成并发出中断事件
pub(crate) fn finish_interrupted(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    finish_with_error(turn_id, "interrupted", on_event)
}

/// 创建内部错误
pub(crate) fn internal_error(error: impl std::fmt::Display) -> AstrError {
    AstrError::Internal(error.to_string())
}

#[cfg(test)]
mod tests;
