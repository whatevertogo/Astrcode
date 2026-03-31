use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    CancelToken, CapabilityDescriptor, CapabilityKind, DescriptorBuildError, PermissionHint,
    Result, SideEffectLevel, StabilityLevel, ToolDefinition, ToolExecutionResult,
};

/// Unique identifier for a session.
pub type SessionId = String;

/// Default maximum output size for tool execution (1 MB)
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// Execution context provided to tools during invocation.
///
/// `ToolContext` carries session metadata, working directory, cancellation support,
/// and output size limits that tools should respect when producing results.
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// Unique session identifier.
    session_id: SessionId,
    /// Working directory that tools must operate within.
    working_dir: PathBuf,
    /// Cancellation token for cooperative cancellation.
    cancel: CancelToken,
    /// Maximum output size in bytes. Defaults to 1MB.
    max_output_size: usize,
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
        }
    }

    /// Sets the maximum output size in bytes.
    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
        self
    }

    /// Returns the session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the working directory path.
    pub fn working_dir(&self) -> &PathBuf {
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
}

/// Metadata describing the capability profiles, permissions, and stability of a tool.
///
/// This struct is used by tools to declare their operational characteristics, which
/// the policy engine and capability router use to make access control decisions.
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
    /// Stability level indicating API maturity.
    pub stability: StabilityLevel,
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
            stability: StabilityLevel::Stable,
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

    /// Sets the stability level for this tool.
    pub fn stability(mut self, stability: StabilityLevel) -> Self {
        self.stability = stability;
        self
    }

    /// Builds a [`CapabilityDescriptor`] from this metadata and the tool definition.
    pub fn build_descriptor(
        self,
        definition: ToolDefinition,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        CapabilityDescriptor::builder(definition.name, CapabilityKind::tool())
            .description(definition.description)
            .schema(definition.parameters, json!({ "type": "string" }))
            .profiles(self.profiles)
            .tags(self.tags)
            .permissions(self.permissions)
            .side_effect(self.side_effect)
            .stability(self.stability)
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
