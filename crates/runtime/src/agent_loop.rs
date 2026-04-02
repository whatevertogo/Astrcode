//! # Agent 循环
//!
//! 实现单个 Agent Turn 的执行逻辑。

pub(crate) mod compaction;
mod llm_cycle;
pub(crate) mod microcompact;
pub(crate) mod token_budget;
pub(crate) mod token_usage;
mod tool_cycle;
mod turn_runner;

use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CancelToken, CapabilityRouter, PolicyContext, PolicyEngine,
    Result, ToolContext,
};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;

use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::llm::LlmProvider;
use crate::prompt::PromptComposer;
use crate::provider_factory::DynProviderFactory;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;

use crate::builtin_skills::builtin_skills;
use crate::prompt::{PromptDeclaration, SkillSpec};

use astrcode_runtime_config::{
    max_tool_concurrency, DEFAULT_AUTO_COMPACT_ENABLED, DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT, DEFAULT_TOOL_RESULT_MAX_BYTES,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// LLM 返回纯文本（无 tool_calls），自然结束。
    Completed,
    /// 用户取消或 CancelToken 触发。
    Cancelled,
    /// 不可恢复错误。
    Error { message: String },
}

impl TurnOutcome {
    fn reason(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Error { .. } => "error",
        }
    }
}

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
    /// Prompt 组装器
    prompt_composer: PromptComposer,
    /// Prompt 构建时可见的能力描述符。
    prompt_capability_descriptors: Vec<astrcode_core::CapabilityDescriptor>,
    /// 归一化后的扩展 prompt 声明。
    prompt_declarations: Vec<PromptDeclaration>,
    /// 当前运行时启用的高层 skill 指南。
    prompt_skills: Vec<SkillSpec>,
    /// 单个 step 内允许并发执行的只读工具上限。
    max_tool_concurrency: usize,
    /// Whether request-level automatic compaction may run before a model step.
    auto_compact_enabled: bool,
    /// Percentage of the effective context window at which compaction starts.
    compact_threshold_percent: u8,
    /// Maximum bytes from a single tool result that may be shown to the model.
    tool_result_max_bytes: usize,
    /// Number of recent user turns that stay verbatim during compaction.
    compact_keep_recent_turns: usize,
}

impl AgentLoop {
    /// 从能力路由器创建 AgentLoop
    pub fn from_capabilities(factory: DynProviderFactory, capabilities: CapabilityRouter) -> Self {
        Self::from_capabilities_with_prompt_inputs(
            factory,
            capabilities,
            Vec::new(),
            builtin_skills(),
        )
    }

    pub fn from_capabilities_with_prompt_inputs(
        factory: DynProviderFactory,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
    ) -> Self {
        let prompt_capability_descriptors = capabilities.descriptors();
        Self {
            factory,
            capabilities,
            policy: Arc::new(AllowAllPolicyEngine),
            approval: Arc::new(DefaultApprovalBroker),
            prompt_composer: PromptComposer::with_defaults(),
            prompt_capability_descriptors,
            prompt_declarations,
            prompt_skills,
            // 默认并行度统一从 runtime-config 读取，这样环境变量覆盖和
            // 直接构造 AgentLoop 的默认行为保持同一套来源。
            max_tool_concurrency: max_tool_concurrency(),
            auto_compact_enabled: DEFAULT_AUTO_COMPACT_ENABLED,
            compact_threshold_percent: DEFAULT_COMPACT_THRESHOLD_PERCENT,
            tool_result_max_bytes: DEFAULT_TOOL_RESULT_MAX_BYTES,
            compact_keep_recent_turns: DEFAULT_COMPACT_KEEP_RECENT_TURNS as usize,
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

    /// 设置并发安全工具的最大并行度。
    ///
    /// 最小值会被钳制到 1，避免配置错误把安全组完全禁用成不可执行状态。
    pub fn with_max_tool_concurrency(mut self, max_tool_concurrency: usize) -> Self {
        self.max_tool_concurrency = max_tool_concurrency.max(1);
        self
    }

    pub fn with_auto_compact_enabled(mut self, auto_compact_enabled: bool) -> Self {
        self.auto_compact_enabled = auto_compact_enabled;
        self
    }

    pub fn with_compact_threshold_percent(mut self, compact_threshold_percent: u8) -> Self {
        self.compact_threshold_percent = compact_threshold_percent.clamp(1, 100);
        self
    }

    pub fn with_tool_result_max_bytes(mut self, tool_result_max_bytes: usize) -> Self {
        self.tool_result_max_bytes = tool_result_max_bytes.max(1);
        self
    }

    pub fn with_compact_keep_recent_turns(mut self, compact_keep_recent_turns: usize) -> Self {
        self.compact_keep_recent_turns = compact_keep_recent_turns.max(1);
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
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(self, state, turn_id, on_event, cancel, true).await
    }

    pub(crate) async fn run_turn_without_finish(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(self, state, turn_id, on_event, cancel, false).await
    }

    /// 创建工具执行上下文
    pub(crate) fn tool_context(&self, state: &AgentState, cancel: CancelToken) -> ToolContext {
        ToolContext::new(state.session_id.clone(), state.working_dir.clone(), cancel)
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

    pub(crate) fn max_tool_concurrency(&self) -> usize {
        self.max_tool_concurrency
    }

    pub(crate) async fn build_provider(
        &self,
        working_dir: Option<PathBuf>,
    ) -> Result<Arc<dyn LlmProvider>> {
        llm_cycle::build_provider(self.factory.clone(), working_dir).await
    }

    pub(crate) fn auto_compact_enabled(&self) -> bool {
        self.auto_compact_enabled
    }

    pub(crate) fn compact_threshold_percent(&self) -> u8 {
        self.compact_threshold_percent
    }

    pub(crate) fn tool_result_max_bytes(&self) -> usize {
        self.tool_result_max_bytes
    }

    pub(crate) fn compact_keep_recent_turns(&self) -> usize {
        self.compact_keep_recent_turns
    }
}

/// 完成 Turn（发出 TurnDone 事件）
pub(crate) fn finish_turn(
    turn_id: &str,
    outcome: TurnOutcome,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    on_event(StorageEvent::TurnDone {
        turn_id: Some(turn_id.to_string()),
        timestamp: Utc::now(),
        reason: Some(outcome.reason().to_string()),
    })?;
    Ok(outcome)
}

/// 完成并发出错误事件
pub(crate) fn finish_with_error(
    turn_id: &str,
    message: impl Into<String>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    let message = message.into();
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        message: message.clone(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, TurnOutcome::Error { message }, on_event)
}

/// 完成并发出中断事件
pub(crate) fn finish_interrupted(
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        message: "interrupted".to_string(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, TurnOutcome::Cancelled, on_event)
}

/// 创建内部错误
pub(crate) fn internal_error(error: impl std::fmt::Display) -> AstrError {
    AstrError::Internal(error.to_string())
}

#[cfg(test)]
mod tests;
