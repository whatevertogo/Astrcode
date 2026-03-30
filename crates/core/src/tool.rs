use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    CancelToken, CapabilityDescriptor, CapabilityKind, DescriptorBuildError, PermissionHint,
    Result, SideEffectLevel, StabilityLevel, ToolDefinition, ToolExecutionResult,
};

pub type SessionId = String;

/// Default maximum output size for tool execution (1 MB)
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub session_id: SessionId,
    pub working_dir: PathBuf,
    pub cancel: CancelToken,
    /// Maximum output size in bytes. Defaults to 1MB.
    pub max_output_size: usize,
}

impl ToolContext {
    pub fn new(session_id: SessionId, working_dir: PathBuf, cancel: CancelToken) -> Self {
        Self {
            session_id,
            working_dir,
            cancel,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        }
    }

    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCapabilityMetadata {
    pub profiles: Vec<String>,
    pub tags: Vec<String>,
    pub permissions: Vec<PermissionHint>,
    pub side_effect: SideEffectLevel,
    pub stability: StabilityLevel,
}

impl Default for ToolCapabilityMetadata {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ToolCapabilityMetadata {
    pub fn builtin() -> Self {
        Self {
            profiles: vec!["coding".to_string()],
            tags: vec!["builtin".to_string()],
            permissions: Vec::new(),
            side_effect: SideEffectLevel::Workspace,
            stability: StabilityLevel::Stable,
        }
    }

    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profiles.push(profile.into());
        self
    }

    pub fn profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.profiles.extend(profiles.into_iter().map(Into::into));
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn permission(mut self, name: impl Into<String>) -> Self {
        self.permissions.push(PermissionHint {
            name: name.into(),
            rationale: None,
        });
        self
    }

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

    pub fn side_effect(mut self, side_effect: SideEffectLevel) -> Self {
        self.side_effect = side_effect;
        self
    }

    pub fn stability(mut self, stability: StabilityLevel) -> Self {
        self.stability = stability;
        self
    }

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

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    /// Keeps policy and projection metadata next to the tool implementation instead of hardcoding
    /// it in the capability adapter.
    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
    }

    /// Allows advanced tools to replace the derived descriptor entirely while still letting most
    /// tools customize only metadata through `capability_metadata()`.
    fn capability_descriptor(
        &self,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        self.capability_metadata()
            .build_descriptor(self.definition())
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult>;
}
