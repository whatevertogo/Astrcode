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
//! ## 核心组件
//!
//! `AgentLoop` 由以下运行时组件协同工作，每个组件各司其职：
//!
//! | 组件 | 职责 | 使用的依赖 | 说明 |
//! |------|------|------------|------|
//! | **`factory`** | LLM Provider 工厂 | `DynProviderFactory` | 根据工作目录解析配置文件，构建对应的 LLM Provider（OpenAI/Anthropic），负责 API 密钥和模型 limits 解析 |
//! | **`capabilities`** | 能力路由器/工具注册表 | `CapabilityRouter` | 将工具名称映射到具体的执行器，支持内置工具和插件工具，提供工具定义列表给 prompt 组装 |
//! | **`policy`** | 策略引擎 | `Arc<dyn PolicyEngine>` | 评估每个能力调用是否需要审批，实现细粒度的访问控制（如 allow-all/deny-all/conditional） |
//! | **`approval`** | 审批代理 | `Arc<dyn ApprovalBroker>` | 处理需要用户确认的工具调用，阻塞执行直到用户允许或拒绝（交互式审批流程） |
//! | **`prompt`** | Prompt 运行时 | `PromptRuntime` | 桥接 PromptComposer 与 loop 输入快照，按需加载 skill 内容，组装完整的系统提示词和规划结果 |
//! | **`context`** | Context 运行时 | `ContextRuntime` | 通过 pipeline stages 构建模型可见的上下文包（conversation view），处理消息裁剪、工作集注入、工具结果截断 |
//! | **`compaction`** | Compaction 运行时 | `CompactionRuntime` | 统一管理上下文压缩的触发策略/决策/重建，支持自动压缩（阈值触发）和手动压缩（用户主动触发） |
//! | **`request_assembler`** | 请求装配器 | `RequestAssembler` | 最终请求装配边界，将 prompt plan + context bundle + 工具定义组装为 LLM API 请求，并生成 prompt 快照用于指标上报 |
//!
//! ## Turn 执行流中各组件的调用顺序
//!
//! ```text
//! 1. factory.build_for_working_dir() → 构建 LLM Provider
//! 2. context.build_bundle() → 构建模型可见上下文包（含 conversation view）
//! 3. prompt.build_plan() → 组装系统提示词，生成 plan
//! 4. request_assembler.build_step_request() → 组装完整的 LLM 请求体
//! 5. policy.decide_context_strategy() → 决定是否需要压缩
//! 6. compaction.* → 若需要压缩，执行上下文压缩并重写 conversation view
//! 7. policy.check_model_request() → 策略检查/重写请求
//! 8. llm_cycle::generate_response() → 调用 LLM
//! 9. policy.* / approval.* → 工具调用前的策略检查和审批
//! 10. capabilities.execute() → 执行工具调用
//! → 循环回到步骤 2，直到 LLM 不再请求工具调用
//! ```
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

use std::{path::PathBuf, sync::Arc};

use astrcode_core::{
    AgentEventContext, AgentState, AllowAllPolicyEngine, AstrError, CancelToken,
    CapabilityDescriptor, CompactionHookContext, ExecutionOwner, HookCompactionReason, HookHandler,
    InvocationKind, LlmMessage, PolicyContext, PolicyEngine, Result, StorageEvent, StoredEvent,
    ToolContext, ToolHookContext, UserMessageOrigin,
};
use astrcode_runtime_config::{
    DEFAULT_AUTO_COMPACT_ENABLED, DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT, DEFAULT_TOOL_RESULT_MAX_BYTES, max_tool_concurrency,
};
use astrcode_runtime_llm::LlmProvider;
use astrcode_runtime_prompt::{PromptComposer, PromptDeclaration};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::{SkillCatalog, load_builtin_skills};
use chrono::Utc;

use crate::{
    approval_service::{ApprovalBroker, DefaultApprovalBroker},
    compaction_runtime::{
        AutoCompactStrategy, CompactionRuntime, CompactionTailSnapshot, ConversationViewRebuilder,
        DEFAULT_RECOVERY_TRUNCATE_BYTES, FsFileContentProvider, MAX_RECOVERED_FILES,
        ThresholdCompactionPolicy,
    },
    context_pipeline::ContextRuntime,
    context_window::{file_access::FileAccessTracker, merge_compact_prompt_context},
    hook_runtime::HookRuntime,
    prompt_runtime::PromptRuntime,
    provider_factory::DynProviderFactory,
    request_assembler::RequestAssembler,
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
    /// 生命周期 hook 运行时。
    hooks: HookRuntime,
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
            compaction: CompactionRuntime::with_truncate_bytes(
                DEFAULT_AUTO_COMPACT_ENABLED,
                DEFAULT_COMPACT_KEEP_RECENT_TURNS as usize,
                DEFAULT_COMPACT_THRESHOLD_PERCENT,
                DEFAULT_RECOVERY_TRUNCATE_BYTES,
                Arc::new(ThresholdCompactionPolicy::new(DEFAULT_AUTO_COMPACT_ENABLED)),
                Arc::new(AutoCompactStrategy),
                Arc::new(ConversationViewRebuilder),
                Arc::new(FsFileContentProvider),
            ),
            hooks: HookRuntime::default(),
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

    /// 注册单个生命周期 hook。
    ///
    /// 将 hook 直接挂到真实执行路径上，而不是额外引入一套事件总线。
    pub fn with_hook_handler(mut self, handler: Arc<dyn HookHandler>) -> Self {
        self.hooks.register(handler);
        self
    }

    /// 批量注册生命周期 hook。
    pub fn with_hook_handlers<I>(mut self, handlers: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn HookHandler>>,
    {
        self.hooks.register_all(handlers);
        self
    }

    /// 是否启用自动上下文压缩
    pub fn with_auto_compact_enabled(mut self, auto_compact_enabled: bool) -> Self {
        self.compaction = self.compaction.with_enabled_and_policy(
            auto_compact_enabled,
            Arc::new(ThresholdCompactionPolicy::new(auto_compact_enabled)),
        );
        self
    }

    /// 设置压缩时保留的最近 Turn 数量
    ///
    /// 最小值会被钳制到 1，确保至少保留一个最近的 Turn 不被压缩。
    pub fn with_compact_keep_recent_turns(mut self, compact_keep_recent_turns: usize) -> Self {
        let enabled = self.compaction.auto_compact_enabled();
        self.compaction = self.compaction.with_keep_recent_turns(
            compact_keep_recent_turns,
            Arc::new(ThresholdCompactionPolicy::new(enabled)),
        );
        self
    }

    #[cfg(test)]
    pub(crate) fn with_prompt_composer(mut self, prompt_composer: PromptComposer) -> Self {
        self.prompt = self.prompt.with_composer(prompt_composer);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_compaction_runtime(mut self, compaction: CompactionRuntime) -> Self {
        self.compaction = compaction;
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
        self.run_turn_with_agent_context_and_owner(
            state,
            turn_id,
            on_event,
            cancel,
            AgentEventContext::default(),
            ExecutionOwner::root(
                state.session_id.clone(),
                turn_id.to_string(),
                InvocationKind::RootExecution,
            ),
        )
        .await
    }

    /// 执行带 Agent 事件上下文的 Turn。
    ///
    /// 为后续子 Agent 执行提供统一入口，避免在每个事件构造点手写父子元数据。
    pub async fn run_turn_with_agent_context(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
    ) -> Result<TurnOutcome> {
        self.run_turn_with_agent_context_and_owner(
            state,
            turn_id,
            on_event,
            cancel,
            agent,
            ExecutionOwner::root(
                state.session_id.clone(),
                turn_id.to_string(),
                InvocationKind::RootExecution,
            ),
        )
        .await
    }

    /// 执行带 Agent 事件上下文和稳定 owner 的 Turn。
    ///
    /// owner 会继续向工具上下文透传，为后续根级任务控制平面预留稳定归属标识。
    pub async fn run_turn_with_agent_context_and_owner(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
        execution_owner: ExecutionOwner,
    ) -> Result<TurnOutcome> {
        self.run_turn_with_compaction_tail(
            state,
            turn_id,
            on_event,
            cancel,
            agent,
            execution_owner,
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
        self.run_turn_without_finish_with_agent_context_and_owner(
            state,
            turn_id,
            on_event,
            cancel,
            AgentEventContext::default(),
            ExecutionOwner::root(
                state.session_id.clone(),
                turn_id.to_string(),
                InvocationKind::RootExecution,
            ),
        )
        .await
    }

    /// 执行 Turn，但由调用方自行负责补 finish 事件。
    pub async fn run_turn_without_finish_with_agent_context(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
    ) -> Result<TurnOutcome> {
        self.run_turn_without_finish_with_agent_context_and_owner(
            state,
            turn_id,
            on_event,
            cancel,
            agent,
            ExecutionOwner::root(
                state.session_id.clone(),
                turn_id.to_string(),
                InvocationKind::RootExecution,
            ),
        )
        .await
    }

    /// 执行 Turn，但由调用方自行负责补 finish 事件，同时显式携带 owner。
    pub async fn run_turn_without_finish_with_agent_context_and_owner(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
        execution_owner: ExecutionOwner,
    ) -> Result<TurnOutcome> {
        self.run_turn_without_finish_with_compaction_tail(
            state,
            turn_id,
            on_event,
            cancel,
            agent,
            execution_owner,
            CompactionTailSnapshot::from_messages(
                &state.messages,
                self.compact_keep_recent_turns(),
            ),
        )
        .await
    }

    /// Execute a turn while carrying a real tail snapshot for compaction rebuilds.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_compaction_tail(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
        execution_owner: ExecutionOwner,
        compaction_tail: CompactionTailSnapshot,
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(turn_runner::TurnRunContext {
            agent_loop: self,
            state,
            turn_id,
            on_event,
            cancel,
            emit_turn_done: true,
            agent,
            execution_owner,
            compaction_tail,
        })
        .await
    }

    /// Internal turn execution variant used by runtime service when it has a live tail recorder.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_without_finish_with_compaction_tail(
        &self,
        state: &AgentState,
        turn_id: &str,
        on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
        cancel: CancelToken,
        agent: AgentEventContext,
        execution_owner: ExecutionOwner,
        compaction_tail: CompactionTailSnapshot,
    ) -> Result<TurnOutcome> {
        turn_runner::run_turn(turn_runner::TurnRunContext {
            agent_loop: self,
            state,
            turn_id,
            on_event,
            cancel,
            emit_turn_done: false,
            agent,
            execution_owner,
            compaction_tail,
        })
        .await
    }

    /// 创建工具执行上下文
    ///
    /// 包含会话 ID、工作目录和取消令牌，供工具执行时访问。
    pub(crate) fn tool_context(
        &self,
        state: &AgentState,
        cancel: CancelToken,
        execution_owner: ExecutionOwner,
    ) -> ToolContext {
        ToolContext::new(state.session_id.clone(), state.working_dir.clone(), cancel)
            .with_execution_owner(execution_owner)
    }

    pub(crate) fn tool_hook_context(
        &self,
        state: &AgentState,
        turn_id: &str,
        tool_call: &astrcode_core::ToolCallRequest,
    ) -> ToolHookContext {
        ToolHookContext {
            session_id: state.session_id.clone(),
            turn_id: turn_id.to_string(),
            working_dir: state.working_dir.clone(),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
        }
    }

    /// 构建压缩 hook 上下文（简化版本，不包含 messages/tools/runtime_system_prompt）。
    ///
    /// 用于 post-compact hook 和不需要完整上下文的场景。
    pub(crate) fn compaction_hook_context(
        &self,
        state: &AgentState,
        conversation: &crate::context_pipeline::ConversationView,
        reason: crate::compaction_runtime::CompactionReason,
        keep_recent_turns: usize,
    ) -> CompactionHookContext {
        CompactionHookContext {
            session_id: state.session_id.clone(),
            working_dir: state.working_dir.clone(),
            reason: match reason {
                crate::compaction_runtime::CompactionReason::Auto => HookCompactionReason::Auto,
                crate::compaction_runtime::CompactionReason::Reactive => {
                    HookCompactionReason::Reactive
                },
                crate::compaction_runtime::CompactionReason::Manual => HookCompactionReason::Manual,
            },
            keep_recent_turns,
            message_count: conversation.messages.len(),
            messages: Vec::new(),
            tools: Vec::new(),
            system_prompt: None,
        }
    }

    /// 构建压缩 hook 上下文（完整版本，包含 messages/tools/runtime_system_prompt）。
    ///
    /// 用于 pre-compact hook，允许插件检查完整上下文并做出修改决策。
    pub(crate) fn compaction_hook_context_full(
        &self,
        state: &AgentState,
        conversation: &crate::context_pipeline::ConversationView,
        reason: crate::compaction_runtime::CompactionReason,
        keep_recent_turns: usize,
        tools: &[astrcode_core::ToolDefinition],
        runtime_system_prompt: Option<&str>,
    ) -> CompactionHookContext {
        CompactionHookContext {
            session_id: state.session_id.clone(),
            working_dir: state.working_dir.clone(),
            reason: match reason {
                crate::compaction_runtime::CompactionReason::Auto => HookCompactionReason::Auto,
                crate::compaction_runtime::CompactionReason::Reactive => {
                    HookCompactionReason::Reactive
                },
                crate::compaction_runtime::CompactionReason::Manual => HookCompactionReason::Manual,
            },
            keep_recent_turns,
            message_count: conversation.messages.len(),
            messages: conversation.messages.clone(),
            tools: tools.to_vec(),
            // Hook 对外仍暴露 `system_prompt` 字段，避免插件协议再引入一层
            // “runtime vs compact” 命名迁移；内部实现统一使用 runtime_system_prompt。
            system_prompt: runtime_system_prompt.map(ToOwned::to_owned),
        }
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
        recent_stored_events: Option<&[StoredEvent]>,
    ) -> Result<Option<StorageEvent>> {
        let user_turns = count_real_user_turns(&state.messages);
        // 手动 compact 是否可执行不再由“至少两个真实用户 turn”粗暴决定，而是交给
        // compact 边界计算：只要后续能找到安全 cut point（旧 turn 或 assistant step），
        // 单 turn 长会话也允许压缩；如果没有安全边界，则返回 Ok(None)。
        // TODO:必须要当前turn结束了才允许手动压缩和自动压缩防止丢失信息
        //
        // 手动 compact 应该“尽量立刻压缩”，因此最多只保留到还能留下至少一个旧 turn
        // 可被折叠，而不是盲目复用自动 compact 的保守保留值。
        let manual_keep_recent_turns = self
            .compaction
            .keep_recent_turns()
            .min(user_turns.saturating_sub(1))
            .max(1);
        // 手动 compact 也要给 pre-hook 暴露完整上下文，否则插件在手动/自动
        // 两条路径上看到的输入会分叉，导致同一条压缩策略静默失效。
        let conversation = crate::context_pipeline::ConversationView::new(state.messages.clone());
        let tools = self.capabilities.tool_definitions();
        let decision = self
            .hooks
            .run_pre_compact(self.compaction_hook_context_full(
                state,
                &conversation,
                crate::compaction_runtime::CompactionReason::Manual,
                manual_keep_recent_turns,
                &tools,
                None,
            ))
            .await?;

        // 检查 hook 是否阻止压缩
        if !decision.allowed {
            return Err(AstrError::Validation(
                decision
                    .block_reason
                    .unwrap_or_else(|| "compaction blocked by hook".to_string()),
            ));
        }

        // 应用 hook 修改的保留轮数
        let effective_keep_turns = decision
            .override_keep_recent_turns
            .unwrap_or(manual_keep_recent_turns);

        let provider = self.build_provider(Some(state.working_dir.clone())).await?;

        // 如果 hook 提供了自定义摘要，跳过 LLM 调用
        let artifact = if let Some(custom_summary) = &decision.custom_summary {
            log::info!(
                "using custom summary from hook ({} chars)",
                custom_summary.len()
            );
            // 使用自定义摘要构建 artifact
            crate::compaction_runtime::build_artifact_from_custom_summary(
                &conversation.messages,
                custom_summary,
                effective_keep_turns,
                crate::compaction_runtime::CompactionReason::Manual,
            )
        } else {
            // hook 只能追加 compact 指令，保留默认压缩 prompt 的约束骨架。
            let compact_prompt_context =
                merge_compact_prompt_context(None, decision.additional_system_prompt.as_deref());

            self.compaction
                .compact_manual_with_keep_recent_turns(
                    provider.as_ref(),
                    &conversation,
                    compact_prompt_context.as_deref(),
                    effective_keep_turns,
                    CancelToken::new(),
                )
                .await?
        };

        let Some(mut artifact) = artifact else {
            return Ok(None);
        };
        let materialized_tail = compaction_tail.materialize();
        // manual compact 的 rebuild tail 只负责“保留最近 turn 的真实事件”，
        // 但文件恢复应尽量看完整的最近持久化事件窗口，避免 hook/策略调整了
        // preserved turns 后，把刚刚读过的文件错误地遗漏掉。
        let file_access = recent_stored_events
            .map(FileAccessTracker::from_stored_events)
            .unwrap_or_else(|| FileAccessTracker::from_stored_events(&materialized_tail));
        artifact.recovered_files = file_access.recent_files(MAX_RECOVERED_FILES);
        let tail = {
            if materialized_tail.is_empty() {
                CompactionTailSnapshot::from_messages(
                    &state.messages,
                    artifact.preserved_recent_turns,
                )
                .materialize()
            } else {
                materialized_tail
            }
        };
        artifact.record_tail_seq(&tail);
        let _rebuilt_view = self.compaction.rebuild_conversation(&artifact, &tail)?;
        self.hooks
            .run_post_compact_best_effort(astrcode_core::CompactionHookResultContext {
                compaction: self.compaction_hook_context(
                    state,
                    &crate::context_pipeline::ConversationView::new(state.messages.clone()),
                    crate::compaction_runtime::CompactionReason::Manual,
                    artifact.preserved_recent_turns,
                ),
                summary: artifact.summary.clone(),
                strategy_id: artifact.strategy_id.clone(),
                preserved_recent_turns: artifact.preserved_recent_turns,
                pre_tokens: artifact.pre_tokens,
                post_tokens_estimate: artifact.post_tokens_estimate,
                messages_removed: artifact.messages_removed,
                tokens_freed: artifact.tokens_freed,
            })
            .await;

        Ok(Some(StorageEvent::CompactApplied {
            turn_id: None,
            agent: AgentEventContext::default(),
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
        self.compaction = self
            .compaction
            .with_threshold_percent(compact_threshold_percent);
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

fn count_real_user_turns(messages: &[LlmMessage]) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
            )
        })
        .count()
}

/// 完成 Turn（发出 TurnDone 事件）
pub(crate) fn finish_turn(
    turn_id: &str,
    outcome: TurnOutcome,
    agent: AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    on_event(StorageEvent::TurnDone {
        turn_id: Some(turn_id.to_string()),
        agent,
        timestamp: Utc::now(),
        reason: Some(outcome.reason().to_string()),
    })?;
    Ok(outcome)
}

/// 完成并发出错误事件
pub(crate) fn finish_with_error(
    turn_id: &str,
    message: impl Into<String>,
    agent: AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    let message = message.into();
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        message: message.clone(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, TurnOutcome::Error { message }, agent, on_event)
}

/// 完成并发出中断事件
pub(crate) fn finish_interrupted(
    turn_id: &str,
    agent: AgentEventContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<TurnOutcome> {
    on_event(StorageEvent::Error {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        message: "interrupted".to_string(),
        timestamp: Some(Utc::now()),
    })?;
    finish_turn(turn_id, TurnOutcome::Cancelled, agent, on_event)
}

/// 创建内部错误
pub(crate) fn internal_error(error: impl std::fmt::Display) -> AstrError {
    AstrError::Internal(error.to_string())
}

#[cfg(test)]
mod tests;
