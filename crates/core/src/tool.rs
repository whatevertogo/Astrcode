//! # Tool Trait 与执行上下文
//!
//! 定义了工具（Tool）系统的核心抽象。Tool 是 LLM Agent 调用外部能力的统一接口。
//!
//! ## 核心概念
//!
//! - **Tool**: 可被 Agent 调用的能力单元（如文件读写、Shell 执行、代码搜索）
//! - **ToolContext**: 工具执行时的上下文信息（会话 ID、工作目录、取消令牌）
//! - **ToolCapabilityMetadata**: 工具的能力元数据（用于策略引擎的权限判断）

use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    AgentEventContext, CancelToken, CapabilityKind, CapabilitySpec, CapabilitySpecBuildError,
    InvocationKind, InvocationMode, PermissionSpec, Result, SessionId, SideEffect, Stability,
    StorageEvent, ToolDefinition, ToolExecutionResult, ToolOutputDelta, ToolOutputStream,
    tool_result_persist::DEFAULT_TOOL_RESULT_INLINE_LIMIT,
};

/// 工具执行的默认最大输出大小（1 MB）
///
/// 超过此大小的输出会被截断，防止大文件导致内存溢出或网络传输问题。
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// 工具调用链路的稳定归属标识。
///
/// 该结构只服务 runtime 内部的控制面演进，用于把“根执行归属”和
/// “当前 sub-run 归属”显式挂到工具上下文里，避免后续长任务注册继续
/// 依赖 `parent_turn_id` 这类脆弱字符串推断。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionOwner {
    /// 根执行所在的 session。
    pub root_session_id: SessionId,
    /// 根执行所在的 turn。
    pub root_turn_id: String,
    /// 当前工具调用若属于子执行域，则记录 sub-run id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<String>,
    /// 当前归属来源。
    pub invocation_kind: InvocationKind,
}

impl ExecutionOwner {
    /// 为顶层执行构造 owner。
    pub fn root(
        root_session_id: impl Into<SessionId>,
        root_turn_id: impl Into<String>,
        invocation_kind: InvocationKind,
    ) -> Self {
        Self {
            root_session_id: root_session_id.into(),
            root_turn_id: root_turn_id.into(),
            sub_run_id: None,
            invocation_kind,
        }
    }

    /// 在现有根归属上挂接当前 sub-run。
    pub fn for_sub_run(&self, sub_run_id: impl Into<String>) -> Self {
        Self {
            root_session_id: self.root_session_id.clone(),
            root_turn_id: self.root_turn_id.clone(),
            sub_run_id: Some(sub_run_id.into()),
            invocation_kind: InvocationKind::SubRun,
        }
    }
}

/// Tool 内部产生 turn 级事件时使用的发射接口。
///
/// 子 Agent / 复合工具不能直接依赖 runtime 的会话写入实现，
/// 因此这里通过一个最小抽象把事件重新交回当前 turn 的持久化/广播链路。
pub trait ToolEventSink: Send + Sync {
    fn emit(&self, event: StorageEvent) -> Result<()>;
}

/// Execution context provided to tools during invocation.
///
/// `ToolContext` carries session metadata, working directory, cancellation support,
/// and output size limits that tools should respect when producing results.
pub struct ToolContext {
    /// Unique session identifier.
    session_id: SessionId,
    /// Working directory that tools must operate within.
    working_dir: PathBuf,
    /// Cancellation token for cooperative cancellation.
    cancel: CancelToken,
    /// 当前工具调用所属 turn。
    ///
    /// 普通工具通常不需要感知 turn_id，但像 spawn 这类复合工具
    /// 需要把子事件重新挂回父 turn。
    turn_id: Option<String>,
    /// 当前工具调用 ID。
    ///
    /// spawn 等复合工具会把这个值落到 durable lifecycle 事件，
    /// 保证重放后仍然能还原触发链路。
    tool_call_id: Option<String>,
    /// 当前工具调用所属 Agent 元数据。
    ///
    /// 子 Agent 工具会基于父 Agent 上下文继续派生自己的 agent_id /
    /// parent_turn_id / agent_profile。
    ///
    /// 使用 Arc 避免 ToolContext 在高频 clone 时反复复制整块 AgentEventContext。
    agent: Arc<AgentEventContext>,
    /// Maximum output size in bytes. Defaults to 1MB.
    max_output_size: usize,
    /// Optional override for session-scoped persisted tool artifacts.
    ///
    /// Production runtime usually leaves this unset so storage falls back to the
    /// project bucket under `~/.astrcode/projects/...`. Tests can point it at a
    /// temp dir to avoid leaking files into the real home directory.
    session_storage_root: Option<PathBuf>,
    /// Optional streaming channel for long-running tools.
    ///
    /// Tools emit best-effort deltas through this sender so runtime can persist and fan them out
    /// in-order without coupling individual tools to storage or transport details.
    tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
    /// 工具级 turn 事件发射器。
    ///
    /// 只有像 spawn 这类会在工具内部再触发 turn/tool 事件的复合工具才会使用。
    event_sink: Option<Arc<dyn ToolEventSink>>,
    /// 工具调用链路归属。
    ///
    /// 当前只作为只读上下文向下游传播，为后续根级任务控制平面预留稳定 owner。
    execution_owner: Option<ExecutionOwner>,
    /// 当前工具的结果内联阈值（字节）。
    ///
    /// 由工具调度时从 `CapabilitySpec::max_result_inline_size` 解析填入，
    /// 未设置时使用 `DEFAULT_TOOL_RESULT_INLINE_LIMIT`。
    /// 工具执行侧用此值决定是否将结果持久化到磁盘。
    resolved_inline_limit: usize,
}

impl ToolContext {
    /// Creates a new `ToolContext` with the given session id, working directory, and cancel token.
    ///
    /// The `max_output_size` is initialized to [`DEFAULT_MAX_OUTPUT_SIZE`].
    pub fn new(session_id: SessionId, working_dir: PathBuf, cancel: CancelToken) -> Self {
        Self {
            session_id,
            working_dir,
            cancel,
            turn_id: None,
            tool_call_id: None,
            agent: Arc::new(AgentEventContext::default()),
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
            session_storage_root: None,
            tool_output_sender: None,
            event_sink: None,
            execution_owner: None,
            resolved_inline_limit: DEFAULT_TOOL_RESULT_INLINE_LIMIT,
        }
    }

    /// Sets the maximum output size in bytes.
    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
        self
    }

    /// Overrides the root directory used for session-scoped persisted tool artifacts.
    pub fn with_session_storage_root(mut self, session_storage_root: PathBuf) -> Self {
        self.session_storage_root = Some(session_storage_root);
        self
    }

    /// Attaches a sender used for best-effort tool output streaming.
    ///
    /// Runtime injects this when it wants a tool to publish incremental stdout/stderr updates
    /// before the final `ToolExecutionResult` is available.
    pub fn with_tool_output_sender(
        mut self,
        tool_output_sender: UnboundedSender<ToolOutputDelta>,
    ) -> Self {
        self.tool_output_sender = Some(tool_output_sender);
        self
    }

    /// 为工具上下文注入当前 turn_id。
    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    /// 为工具上下文注入当前 tool_call_id。
    pub fn with_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.tool_call_id = Some(tool_call_id.into());
        self
    }

    /// 为工具上下文注入当前 Agent 元数据。
    pub fn with_agent_context(mut self, agent: AgentEventContext) -> Self {
        self.agent = Arc::new(agent);
        self
    }

    /// 为工具上下文注入 turn 事件发射器。
    pub fn with_event_sink(mut self, event_sink: Arc<dyn ToolEventSink>) -> Self {
        self.event_sink = Some(event_sink);
        self
    }

    /// 为工具上下文注入执行 owner。
    pub fn with_execution_owner(mut self, execution_owner: ExecutionOwner) -> Self {
        self.execution_owner = Some(execution_owner);
        self
    }

    /// 设置当前工具的结果内联阈值。
    ///
    /// 由工具调度时从 `CapabilitySpec::max_result_inline_size` 解析填入。
    pub fn with_resolved_inline_limit(mut self, limit: usize) -> Self {
        self.resolved_inline_limit = limit;
        self
    }

    /// Returns the session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the working directory path.
    pub fn working_dir(&self) -> &std::path::Path {
        &self.working_dir
    }

    /// Returns a reference to the cancellation token.
    pub fn cancel(&self) -> &CancelToken {
        &self.cancel
    }

    /// 返回当前 turn_id（若有）。
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    /// 返回当前 tool_call_id（若有）。
    pub fn tool_call_id(&self) -> Option<&str> {
        self.tool_call_id.as_deref()
    }

    /// 返回当前 Agent 元数据。
    pub fn agent_context(&self) -> &AgentEventContext {
        self.agent.as_ref()
    }

    /// Returns the maximum output size in bytes.
    pub fn max_output_size(&self) -> usize {
        self.max_output_size
    }

    pub fn session_storage_root(&self) -> Option<&std::path::Path> {
        self.session_storage_root.as_deref()
    }

    pub fn tool_output_sender(&self) -> Option<UnboundedSender<ToolOutputDelta>> {
        self.tool_output_sender.clone()
    }

    pub fn event_sink(&self) -> Option<Arc<dyn ToolEventSink>> {
        self.event_sink.clone()
    }

    /// 返回当前执行 owner。
    pub fn execution_owner(&self) -> Option<&ExecutionOwner> {
        self.execution_owner.as_ref()
    }

    /// 返回当前工具的结果内联阈值（字节）。
    pub fn resolved_inline_limit(&self) -> usize {
        self.resolved_inline_limit
    }

    /// Emits a tool delta to the runtime if streaming is enabled.
    ///
    /// This is intentionally best-effort: losing a live UI update must not fail the tool itself,
    /// because the final persisted `ToolExecutionResult` is still the source of truth.
    pub fn emit_tool_delta(&self, delta: ToolOutputDelta) -> bool {
        self.tool_output_sender
            .as_ref()
            .is_some_and(|sender| sender.send(delta).is_ok())
    }

    pub fn emit_stdout(
        &self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        delta: impl Into<String>,
    ) -> bool {
        self.emit_tool_delta(ToolOutputDelta {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            stream: ToolOutputStream::Stdout,
            delta: delta.into(),
        })
    }

    pub fn emit_stderr(
        &self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        delta: impl Into<String>,
    ) -> bool {
        self.emit_tool_delta(ToolOutputDelta {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            stream: ToolOutputStream::Stderr,
            delta: delta.into(),
        })
    }
}

impl Clone for ToolContext {
    fn clone(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            working_dir: self.working_dir.clone(),
            cancel: self.cancel.clone(),
            turn_id: self.turn_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
            agent: self.agent.clone(),
            max_output_size: self.max_output_size,
            session_storage_root: self.session_storage_root.clone(),
            tool_output_sender: self.tool_output_sender.clone(),
            event_sink: self.event_sink.clone(),
            execution_owner: self.execution_owner.clone(),
            resolved_inline_limit: self.resolved_inline_limit,
        }
    }
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("working_dir", &self.working_dir)
            .field("cancel", &self.cancel)
            .field("turn_id", &self.turn_id)
            .field("agent", self.agent.as_ref())
            .field("max_output_size", &self.max_output_size)
            .field("session_storage_root", &self.session_storage_root)
            .field(
                "tool_output_sender",
                &self.tool_output_sender.as_ref().map(|_| "<attached>"),
            )
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<attached>"),
            )
            .field("execution_owner", &self.execution_owner)
            .field("resolved_inline_limit", &self.resolved_inline_limit)
            .finish()
    }
}

/// Metadata describing the capability profiles, permissions, and stability of a tool.
///
/// This struct is used by tools to declare their operational characteristics, which
/// the policy engine and capability router use to make access control decisions.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolPromptMetadata {
    pub summary: String,
    pub guide: String,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub prompt_tags: Vec<String>,
    #[serde(default)]
    pub always_include: bool,
}

impl ToolPromptMetadata {
    pub fn new(summary: impl Into<String>, guide: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            guide: guide.into(),
            caveats: Vec::new(),
            examples: Vec::new(),
            prompt_tags: Vec::new(),
            always_include: false,
        }
    }

    pub fn caveat(mut self, caveat: impl Into<String>) -> Self {
        self.caveats.push(caveat.into());
        self
    }

    pub fn example(mut self, example: impl Into<String>) -> Self {
        self.examples.push(example.into());
        self
    }

    pub fn prompt_tag(mut self, prompt_tag: impl Into<String>) -> Self {
        self.prompt_tags.push(prompt_tag.into());
        self
    }

    pub fn always_include(mut self, always_include: bool) -> Self {
        self.always_include = always_include;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCapabilityMetadata {
    /// Capability profiles that this tool belongs to (e.g., "coding", "analysis").
    pub profiles: Vec<String>,
    /// Descriptive tags for categorization and discovery.
    pub tags: Vec<String>,
    /// Permission hints indicating what resources or actions this tool may access.
    pub permissions: Vec<PermissionSpec>,
    /// 调用模式，替代旧的 `streaming: bool`。
    pub invocation_mode: InvocationMode,
    /// The level of side effects this tool may produce.
    pub side_effect: SideEffect,
    /// Whether the runtime may execute multiple calls to this capability in parallel.
    pub concurrency_safe: bool,
    /// Whether old tool results may be compacted out of request context to save tokens.
    pub compact_clearable: bool,
    /// Stability level indicating API maturity.
    pub stability: Stability,
    /// Prompt guidance that should be projected into the layered prompt system.
    pub prompt: Option<ToolPromptMetadata>,
    /// 工具结果内联阈值（字节）。
    /// 超过此大小的结果在执行时持久化到磁盘。
    /// None 时使用系统默认阈值（DEFAULT_TOOL_RESULT_INLINE_LIMIT = 32KB）。
    pub max_result_inline_size: Option<usize>,
}

impl Default for ToolCapabilityMetadata {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ToolCapabilityMetadata {
    /// Creates a new metadata instance with default builtin values.
    ///
    /// The defaults are: profile "coding", tag "builtin", no permissions,
    /// side effect level `Workspace`, and stability `Stable`.
    pub fn builtin() -> Self {
        Self {
            profiles: vec!["coding".to_string()],
            tags: vec!["builtin".to_string()],
            permissions: Vec::new(),
            invocation_mode: InvocationMode::Unary,
            side_effect: SideEffect::Workspace,
            concurrency_safe: false,
            compact_clearable: false,
            stability: Stability::Stable,
            prompt: None,
            max_result_inline_size: None,
        }
    }

    /// Adds a single capability profile.
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profiles.push(profile.into());
        self
    }

    /// Adds multiple capability profiles.
    pub fn profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.profiles.extend(profiles.into_iter().map(Into::into));
        self
    }

    /// Adds a single descriptive tag.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Adds multiple descriptive tags.
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// Adds a permission hint without a rationale.
    pub fn permission(mut self, name: impl Into<String>) -> Self {
        self.permissions.push(PermissionSpec {
            name: name.into(),
            rationale: None,
        });
        self
    }

    /// Adds a permission hint with an explanatory rationale.
    pub fn permission_with_rationale(
        mut self,
        name: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Self {
        self.permissions.push(PermissionSpec {
            name: name.into(),
            rationale: Some(rationale.into()),
        });
        self
    }

    /// 设置调用模式（Unary / Streaming）。
    pub fn invocation_mode(mut self, invocation_mode: InvocationMode) -> Self {
        self.invocation_mode = invocation_mode;
        self
    }

    /// Sets the side effect level for this tool.
    pub fn side_effect(mut self, side_effect: SideEffect) -> Self {
        self.side_effect = side_effect;
        self
    }

    /// Marks whether the tool is safe to run concurrently with other safe tools.
    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = concurrency_safe;
        self
    }

    /// Marks whether historical results from this tool may be cleared from model context.
    pub fn compact_clearable(mut self, compact_clearable: bool) -> Self {
        self.compact_clearable = compact_clearable;
        self
    }

    /// Sets the stability level for this tool.
    pub fn stability(mut self, stability: Stability) -> Self {
        self.stability = stability;
        self
    }

    /// Attaches prompt guidance to this tool descriptor.
    pub fn prompt(mut self, prompt: ToolPromptMetadata) -> Self {
        self.prompt = Some(prompt);
        self
    }

    /// Sets the maximum inline size for tool results (bytes).
    pub fn max_result_inline_size(mut self, size: usize) -> Self {
        self.max_result_inline_size = Some(size);
        self
    }

    /// Builds a [`CapabilitySpec`] from this metadata and the tool definition.
    pub fn build_spec(
        self,
        definition: ToolDefinition,
    ) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
        let mut metadata = serde_json::Map::new();
        if let Some(prompt) = self.prompt {
            metadata.insert(
                "prompt".to_string(),
                serde_json::to_value(prompt)
                    // 提示词元数据必须作为 descriptor 的一部分向上游显式报错，
                    // 不能在库层 panic 吞掉调用方的构建上下文。
                    .map_err(|_| CapabilitySpecBuildError::InvalidSchema("metadata"))?,
            );
        }

        let builder = CapabilitySpec::builder(definition.name, CapabilityKind::Tool)
            .description(definition.description)
            .schema(definition.parameters, json!({ "type": "string" }))
            .invocation_mode(self.invocation_mode)
            .profiles(self.profiles)
            .tags(self.tags)
            .permissions(self.permissions)
            .side_effect(self.side_effect)
            .concurrency_safe(self.concurrency_safe)
            .compact_clearable(self.compact_clearable)
            .stability(self.stability)
            .metadata(Value::Object(metadata));
        let builder = match self.max_result_inline_size {
            Some(size) => builder.max_result_inline_size(size),
            None => builder,
        };
        builder.build()
    }
}

/// Trait that all tools must implement.
///
/// A `Tool` provides a named operation that can be invoked by the agent loop.
/// Implementors must be `Send + Sync` to support concurrent execution.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the tool's definition including name, description, and parameter schema.
    fn definition(&self) -> ToolDefinition;

    /// Returns capability metadata for policy and routing decisions.
    ///
    /// The default implementation returns builtin defaults. Override this method
    /// to customize the tool's operational characteristics.
    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
    }

    /// Returns a full capability spec for this tool.
    ///
    /// The default implementation builds a spec from `definition()` and
    /// `capability_metadata()`. Override this method for advanced tools that
    /// need complete control over the capability spec.
    fn capability_spec(&self) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
        self.capability_metadata().build_spec(self.definition())
    }

    /// Executes the tool with the given arguments and context.
    ///
    /// # Arguments
    /// * `tool_call_id` - Unique identifier for this tool call.
    /// * `input` - JSON arguments parsed from the agent's tool call request.
    /// * `ctx` - Execution context providing session info, working directory, and cancellation.
    ///
    /// # Returns
    /// `Ok(ToolExecutionResult)` on success, or `Err` for system-level failures.
    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult>;
}
