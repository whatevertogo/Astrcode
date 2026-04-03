//! # Tool Trait 与执行上下文
//!
//! 定义了工具（Tool）系统的核心抽象。Tool 是 LLM Agent 调用外部能力的统一接口。
//!
//! ## 核心概念
//!
//! - **Tool**: 可被 Agent 调用的能力单元（如文件读写、Shell 执行、代码搜索）
//! - **ToolContext**: 工具执行时的上下文信息（会话 ID、工作目录、取消令牌）
//! - **ToolCapabilityMetadata**: 工具的能力元数据（用于策略引擎的权限判断）

use std::fmt;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    CancelToken, CapabilityDescriptor, CapabilityKind, DescriptorBuildError, PermissionHint,
    Result, SideEffectLevel, StabilityLevel, ToolDefinition, ToolExecutionResult, ToolOutputDelta,
    ToolOutputStream,
};

/// Unique identifier for a session.
pub type SessionId = String;

/// 工具执行的默认最大输出大小（1 MB）
///
/// 超过此大小的输出会被截断，防止大文件导致内存溢出或网络传输问题。
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

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
    /// Maximum output size in bytes. Defaults to 1MB.
    max_output_size: usize,
    /// Optional streaming channel for long-running tools.
    ///
    /// Tools emit best-effort deltas through this sender so runtime can persist and fan them out
    /// in-order without coupling individual tools to storage or transport details.
    tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
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
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
            tool_output_sender: None,
        }
    }

    /// Sets the maximum output size in bytes.
    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
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

    /// Returns the maximum output size in bytes.
    pub fn max_output_size(&self) -> usize {
        self.max_output_size
    }

    pub fn tool_output_sender(&self) -> Option<UnboundedSender<ToolOutputDelta>> {
        self.tool_output_sender.clone()
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
            max_output_size: self.max_output_size,
            tool_output_sender: self.tool_output_sender.clone(),
        }
    }
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("working_dir", &self.working_dir)
            .field("cancel", &self.cancel)
            .field("max_output_size", &self.max_output_size)
            .field(
                "tool_output_sender",
                &self.tool_output_sender.as_ref().map(|_| "<attached>"),
            )
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
    pub permissions: Vec<PermissionHint>,
    /// The level of side effects this tool may produce.
    pub side_effect: SideEffectLevel,
    /// Whether the runtime may execute multiple calls to this capability in parallel.
    pub concurrency_safe: bool,
    /// Whether old tool results may be compacted out of request context to save tokens.
    pub compact_clearable: bool,
    /// Stability level indicating API maturity.
    pub stability: StabilityLevel,
    /// Prompt guidance that should be projected into the layered prompt system.
    pub prompt: Option<ToolPromptMetadata>,
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
            side_effect: SideEffectLevel::Workspace,
            concurrency_safe: false,
            compact_clearable: false,
            stability: StabilityLevel::Stable,
            prompt: None,
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
        self.permissions.push(PermissionHint {
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
        self.permissions.push(PermissionHint {
            name: name.into(),
            rationale: Some(rationale.into()),
        });
        self
    }

    /// Sets the side effect level for this tool.
    pub fn side_effect(mut self, side_effect: SideEffectLevel) -> Self {
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
    pub fn stability(mut self, stability: StabilityLevel) -> Self {
        self.stability = stability;
        self
    }

    /// Attaches prompt guidance to this tool descriptor.
    pub fn prompt(mut self, prompt: ToolPromptMetadata) -> Self {
        self.prompt = Some(prompt);
        self
    }

    /// Builds a [`CapabilityDescriptor`] from this metadata and the tool definition.
    pub fn build_descriptor(
        self,
        definition: ToolDefinition,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        let mut metadata = serde_json::Map::new();
        if let Some(prompt) = self.prompt {
            metadata.insert(
                "prompt".to_string(),
                serde_json::to_value(prompt)
                    .expect("tool prompt metadata should serialize into JSON"),
            );
        }

        CapabilityDescriptor::builder(definition.name, CapabilityKind::tool())
            .description(definition.description)
            .schema(definition.parameters, json!({ "type": "string" }))
            .profiles(self.profiles)
            .tags(self.tags)
            .permissions(self.permissions)
            .side_effect(self.side_effect)
            .concurrency_safe(self.concurrency_safe)
            .compact_clearable(self.compact_clearable)
            .stability(self.stability)
            .metadata(Value::Object(metadata))
            .build()
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

    /// Returns a full capability descriptor for this tool.
    ///
    /// The default implementation builds a descriptor from `definition()` and
    /// `capability_metadata()`. Override this method for advanced tools that
    /// need complete control over the descriptor.
    fn capability_descriptor(
        &self,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        self.capability_metadata()
            .build_descriptor(self.definition())
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
