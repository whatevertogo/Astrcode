//! # Agent 循环 (Agent Loop)
//!
//! 实现单个 Agent Turn 的执行逻辑，是 Astrcode Agent 的核心驱动引擎。
//!
//! ## 职责
//!
//! `AgentLoop` 负责编排一个完整 Turn 的执行流程：
//! 1. 构建 LLM Provider（根据工作目录解析配置）
//! 2. 组装 Prompt（系统提示词 + 历史消息 + 工具描述）
//! 3. 调用 LLM 并流式接收响应
//! 4. 若有工具调用，执行工具并循环回到步骤 2
//! 5. 处理 Token 预算和自动压缩
//!
//! ## 架构约束
//!
//! - `AgentLoop` 仅依赖 `core` 定义的接口，不直接依赖 `runtime` 门面
//! - LLM Provider 通过 `ProviderFactory` 抽象，支持热替换
//! - 工具执行通过 `CapabilityRouter` 路由，支持策略检查和审批
//! - Prompt 组装通过 `PromptComposer` 独立实现，保持关注点分离

mod llm_cycle;
pub mod token_budget;
mod tool_cycle;
mod turn_runner;

use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CancelToken, CapabilityDescriptor, CapabilityRouter,
    PolicyContext, PolicyEngine, Result, StorageEvent, ToolContext,
};
use astrcode_runtime_llm::LlmProvider;
use astrcode_runtime_prompt::{PromptComposer, PromptDeclaration};
use astrcode_runtime_skill_loader::{load_builtin_skills, SkillCatalog};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;

use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::compaction_runtime::{
    AutoCompactStrategy, CompactionReason, CompactionRuntime, CompactionTailSnapshot,
    ConversationViewRebuilder, ThresholdCompactionPolicy,
};
use crate::context_pipeline::ContextRuntime;
use crate::prompt_runtime::PromptRuntime;
use crate::provider_factory::DynProviderFactory;
use crate::request_assembler::RequestAssembler;
use astrcode_core::AgentState;

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
/// - Token 预算管理和自动上下文压缩
///
/// ## 设计原则
///
/// `AgentLoop` 是纯执行引擎，不持有会话状态或持久化逻辑。
/// 它通过回调 `on_event` 发出事件，由调用方（通常是 `RuntimeService`）
/// 负责持久化和广播。这种设计使得 AgentLoop 可以在不同上下文中复用
/// （如测试、CLI、HTTP 服务）。
pub struct AgentLoop {
    /// LLM 提供者工厂，负责根据工作目录构建对应的 LLM Provider
    factory: DynProviderFactory,
    /// 能力路由器（工具注册表），将工具名映射到执行器
    capabilities: CapabilityRouter,
    /// 策略引擎，决定能力调用是否需要审批
    policy: Arc<dyn PolicyEngine>,
    /// 审批代理，处理需要用户确认的能力调用
    approval: Arc<dyn ApprovalBroker>,
    /// Prompt 运行时，桥接 PromptComposer 与 loop 输入快照。
    prompt: PromptRuntime,
    /// Context 运行时，只负责构建模型可见的上下文包。
    context: ContextRuntime,
    /// Compaction 运行时，统一承载 trigger / strategy / rebuild 协作者。
    compaction: CompactionRuntime,
    /// 最终请求装配边界。
    request_assembler: RequestAssembler,
    /// 单个 step 内允许并发执行的只读工具上限
    max_tool_concurrency: usize,
}

impl AgentLoop {
    /// 从能力路由器创建 AgentLoop
    ///
    /// 使用默认策略引擎（AllowAll）和默认审批代理（DefaultApprovalBroker）。
    /// 适用于不需要审批控制的场景。
    pub fn from_capabilities(factory: DynProviderFactory, capabilities: CapabilityRouter) -> Self {
        Self::from_capabilities_with_prompt_inputs(
            factory,
            capabilities,
            Vec::new(),
            Arc::new(SkillCatalog::new(load_builtin_skills())),
        )
    }

    /// 从能力路由器和自定义 Prompt 输入创建 AgentLoop
    ///
    /// 允许调用方注入自定义 prompt 声明和 skill 列表，
    /// 用于扩展或覆盖默认的 prompt 行为。
    pub fn from_capabilities_with_prompt_inputs(
        factory: DynProviderFactory,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
    ) -> Self {
        let tool_names = capabilities.tool_names().to_vec();
        let prompt_capability_descriptors = capabilities.descriptors();
        Self {
            factory,
            capabilities,
            policy: Arc::new(AllowAllPolicyEngine),
            approval: Arc::new(DefaultApprovalBroker),
            prompt: PromptRuntime::new(
                PromptComposer::with_defaults(),
                tool_names,
                prompt_capability_descriptors,
                prompt_declarations,
                skill_catalog,
            ),
            context: ContextRuntime::new(DEFAULT_TOOL_RESULT_MAX_BYTES),
            compaction: CompactionRuntime::new(
                DEFAULT_AUTO_COMPACT_ENABLED,
                DEFAULT_COMPACT_KEEP_RECENT_TURNS as usize,
                DEFAULT_COMPACT_THRESHOLD_PERCENT,
                Arc::new(ThresholdCompactionPolicy::new(DEFAULT_AUTO_COMPACT_ENABLED)),
                Arc::new(AutoCompactStrategy),
                Arc::new(ConversationViewRebuilder),
            ),
            request_assembler: RequestAssembler,
            // 默认并行度统一从 runtime-config 读取，这样环境变量覆盖和
            // 直接构造 AgentLoop 的默认行为保持同一套来源。
            max_tool_concurrency: max_tool_concurrency(),
        }
    }

    /// 设置策略引擎
    ///
    /// 用于替换默认的 AllowAll 策略引擎，实现细粒度的能力调用控制。
    pub fn with_policy_engine(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = policy;
        self
    }

    /// 设置审批代理
    ///
    /// 用于替换默认的 DefaultApprovalBroker，实现用户交互式的审批流程。
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

    /// 是否启用自动上下文压缩
    pub fn with_auto_compact_enabled(mut self, auto_compact_enabled: bool) -> Self {
        self.compaction = CompactionRuntime::new(
            auto_compact_enabled,
            self.compaction.keep_recent_turns(),
            self.compaction.threshold_percent(),
            Arc::new(ThresholdCompactionPolicy::new(auto_compact_enabled)),
            self.compaction.strategy.clone(),
            self.compaction.rebuilder.clone(),
        );
        self
    }

    /// 设置压缩时保留的最近 Turn 数量
    ///
    /// 最小值会被钳制到 1，确保至少保留一个最近的 Turn 不被压缩。
    pub fn with_compact_keep_recent_turns(mut self, compact_keep_recent_turns: usize) -> Self {
        self.compaction = CompactionRuntime::new(
            self.compaction.auto_compact_enabled(),
            compact_keep_recent_turns.max(1),
            self.compaction.threshold_percent(),
            Arc::new(ThresholdCompactionPolicy::new(
                self.compaction.auto_compact_enabled(),
            )),
            self.compaction.strategy.clone(),
            self.compaction.rebuilder.clone(),
        );
        self
    }

    #[cfg(test)]
    pub(crate) fn with_prompt_composer(mut self, prompt_composer: PromptComposer) -> Self {
        self.prompt = self.prompt.with_composer(prompt_composer);
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
        self.run_turn_with_compaction_tail(
            state,
            turn_id,
            on_event,
            cancel,
            CompactionTailSnapshot::from_messages(
                &state.messages,
                self.compact_keep_recent_turns(),
            ),
        )
        .await
    }

    /// 执行 Turn 但不发送 TurnDone/TurnFailed 事件
    ///
    /// 用于内部场景（如自动压缩），调用方需要自行处理 Turn 的完成状态。
    pub async fn run_turn_without_finish(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
    ) -> Result<TurnOutcome> {
        self.run_turn_without_finish_with_compaction_tail(
            state,
            turn_id,
            on_event,
            cancel,
            CompactionTailSnapshot::from_messages(
                &state.messages,
                self.compact_keep_recent_turns(),
            ),
        )
        .await
    }

    /// Execute a turn while carrying a real tail snapshot for compaction rebuilds.
    pub async fn run_turn_with_compaction_tail(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        compaction_tail: CompactionTailSnapshot,
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(
            self,
            state,
            turn_id,
            on_event,
            cancel,
            true,
            compaction_tail,
        )
        .await
    }

    /// Internal turn execution variant used by runtime service when it has a live tail recorder.
    pub async fn run_turn_without_finish_with_compaction_tail(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        compaction_tail: CompactionTailSnapshot,
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(
            self,
            state,
            turn_id,
            on_event,
            cancel,
            false,
            compaction_tail,
        )
        .await
    }

    /// 创建工具执行上下文
    ///
    /// 包含会话 ID、工作目录和取消令牌，供工具执行时访问。
    pub(crate) fn tool_context(&self, state: &AgentState, cancel: CancelToken) -> ToolContext {
        ToolContext::new(state.session_id.clone(), state.working_dir.clone(), cancel)
    }

    /// 创建策略上下文
    ///
    /// 包含会话 ID、Turn ID、Step 索引和工作目录，供策略引擎评估能力调用。
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

    pub fn max_tool_concurrency(&self) -> usize {
        self.max_tool_concurrency
    }

    pub async fn build_provider(
        &self,
        working_dir: Option<PathBuf>,
    ) -> Result<Arc<dyn LlmProvider>> {
        llm_cycle::build_provider(self.factory.clone(), working_dir).await
    }

    /// Run a user-initiated compaction over an idle projected session.
    ///
    /// The runtime service uses this instead of calling `auto_compact` directly so manual compact
    /// shares the same compaction strategy surface as the live agent loop.
    pub async fn manual_compact_event(
        &self,
        state: &AgentState,
        compaction_tail: CompactionTailSnapshot,
    ) -> Result<Option<StorageEvent>> {
        let provider = self.build_provider(Some(state.working_dir.clone())).await?;
        let artifact = self
            .compaction
            .compact(
                provider.as_ref(),
                &crate::context_pipeline::ConversationView::new(state.messages.clone()),
                None,
                CompactionReason::Manual,
                CancelToken::new(),
            )
            .await?;

        let Some(artifact) = artifact else {
            return Ok(None);
        };
        let tail = {
            let materialized = compaction_tail.materialize();
            if materialized.is_empty() {
                CompactionTailSnapshot::from_messages(
                    &state.messages,
                    artifact.preserved_recent_turns,
                )
                .materialize()
            } else {
                materialized
            }
        };
        let _rebuilt_view = self.compaction.rebuild_conversation(&artifact, &tail)?;

        Ok(Some(StorageEvent::CompactApplied {
            turn_id: None,
            trigger: artifact.trigger.as_trigger(),
            summary: artifact.summary,
            preserved_recent_turns: artifact.preserved_recent_turns.min(u32::MAX as usize) as u32,
            pre_tokens: artifact.pre_tokens.min(u32::MAX as usize) as u32,
            post_tokens_estimate: artifact.post_tokens_estimate.min(u32::MAX as usize) as u32,
            messages_removed: artifact.messages_removed.min(u32::MAX as usize) as u32,
            tokens_freed: artifact.tokens_freed.min(u32::MAX as usize) as u32,
            timestamp: Utc::now(),
        }))
    }

    pub fn auto_compact_enabled(&self) -> bool {
        self.compaction.auto_compact_enabled()
    }

    pub fn compact_threshold_percent(&self) -> u8 {
        self.compaction.threshold_percent()
    }

    pub fn tool_result_max_bytes(&self) -> usize {
        self.context.tool_result_max_bytes()
    }

    /// 设置单个工具结果的最大展示字节数
    ///
    /// 该值实际上由 ContextRuntime 持有，此处保留 builder 方法以保持
    /// RuntimeService 装配层的调用兼容性。
    pub fn with_tool_result_max_bytes(mut self, tool_result_max_bytes: usize) -> Self {
        self.context = ContextRuntime::new(tool_result_max_bytes);
        self
    }

    /// 设置触发压缩的上下文窗口百分比
    ///
    /// 该值实际上由 CompactionRuntime 持有，此处保留 builder 方法以保持
    /// RuntimeService 装配层的调用兼容性。
    pub fn with_compact_threshold_percent(mut self, compact_threshold_percent: u8) -> Self {
        self.compaction = CompactionRuntime::new(
            self.compaction.auto_compact_enabled(),
            self.compaction.keep_recent_turns(),
            compact_threshold_percent,
            self.compaction.policy.clone(),
            self.compaction.strategy.clone(),
            self.compaction.rebuilder.clone(),
        );
        self
    }

    pub fn compact_keep_recent_turns(&self) -> usize {
        self.compaction.keep_recent_turns()
    }

    /// 暴露当前 prompt 可见的 capability 描述符。
    ///
    /// 输入候选接口需要复用和 prompt 一致的 capability surface，
    /// 这样前端看到的工具候选不会和模型真实可见的工具列表漂移。
    pub fn capability_descriptors(&self) -> &[CapabilityDescriptor] {
        self.prompt.capability_descriptors()
    }

    /// 暴露统一 skill 目录。
    pub fn skill_catalog(&self) -> Arc<SkillCatalog> {
        self.prompt.skill_catalog()
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
