//! 运行时能力语义模型。
//!
//! `CapabilitySpec` 是 runtime 内部唯一能力模型；协议层 `CapabilityDescriptor`
//! 只应作为边界 DTO 使用。

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::ids::CapabilityName;

/// 能力类型。
///
/// 使用 enum 避免运行时内部继续通过字符串比较做分支判断。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum CapabilityKind {
    #[default]
    Tool,
    Agent,
    ContextProvider,
    MemoryProvider,
    PolicyHook,
    Renderer,
    Resource,
    Prompt,
    Custom(String),
}

impl CapabilityKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tool => "tool",
            Self::Agent => "agent",
            Self::ContextProvider => "context_provider",
            Self::MemoryProvider => "memory_provider",
            Self::PolicyHook => "policy_hook",
            Self::Renderer => "renderer",
            Self::Resource => "resource",
            Self::Prompt => "prompt",
            Self::Custom(value) => value.as_str(),
        }
    }

    pub fn is_tool(&self) -> bool {
        matches!(self, Self::Tool)
    }
}

impl From<String> for CapabilityKind {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&str> for CapabilityKind {
    fn from(value: &str) -> Self {
        match value.trim() {
            "tool" => Self::Tool,
            "agent" => Self::Agent,
            "context_provider" => Self::ContextProvider,
            "memory_provider" => Self::MemoryProvider,
            "policy_hook" => Self::PolicyHook,
            "renderer" => Self::Renderer,
            "resource" => Self::Resource,
            "prompt" => Self::Prompt,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for CapabilityKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CapabilityKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::from(String::deserialize(deserializer)?))
    }
}

/// 调用模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InvocationMode {
    #[default]
    Unary,
    Streaming,
}

/// 副作用级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    #[default]
    None,
    Local,
    Workspace,
    External,
}

/// 稳定性级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    Experimental,
    #[default]
    Stable,
    Deprecated,
}

/// 权限声明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// 运行时能力语义定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySpec {
    pub name: CapabilityName,
    #[serde(default)]
    pub kind: CapabilityKind,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    #[serde(default)]
    pub invocation_mode: InvocationMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub concurrency_safe: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact_clearable: bool,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<PermissionSpec>,
    #[serde(default)]
    pub side_effect: SideEffect,
    #[serde(default)]
    pub stability: Stability,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_result_inline_size: Option<usize>,
}

impl CapabilitySpec {
    pub fn builder(
        name: impl Into<CapabilityName>,
        kind: impl Into<CapabilityKind>,
    ) -> CapabilitySpecBuilder {
        CapabilitySpecBuilder::new(name, kind)
    }

    pub fn validate(&self) -> std::result::Result<(), CapabilitySpecBuildError> {
        validate_capability_name(self.name.clone())?;
        validate_kind(self.kind.clone())?;
        validate_non_empty("description", self.description.clone())?;
        validate_schema("input_schema", self.input_schema.clone())?;
        validate_schema("output_schema", self.output_schema.clone())?;
        validate_string_list("profiles", self.profiles.clone())?;
        validate_string_list("tags", self.tags.clone())?;
        validate_permissions(self.permissions.clone())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySpecBuildError {
    EmptyField(&'static str),
    MissingField(&'static str),
    InvalidSchema(&'static str),
    DuplicateValue { field: &'static str, value: String },
}

impl fmt::Display for CapabilitySpecBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "capability field '{field}' cannot be empty"),
            Self::MissingField(field) => write!(f, "capability field '{field}' is required"),
            Self::InvalidSchema(field) => {
                write!(f, "capability field '{field}' must be a JSON object schema")
            },
            Self::DuplicateValue { field, value } => {
                write!(
                    f,
                    "capability field '{field}' contains duplicate value '{value}'"
                )
            },
        }
    }
}

impl std::error::Error for CapabilitySpecBuildError {}

#[derive(Debug, Clone)]
pub struct CapabilitySpecBuilder {
    name: CapabilityName,
    kind: CapabilityKind,
    description: Option<String>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
    invocation_mode: InvocationMode,
    concurrency_safe: bool,
    compact_clearable: bool,
    profiles: Vec<String>,
    tags: Vec<String>,
    permissions: Vec<PermissionSpec>,
    side_effect: SideEffect,
    stability: Stability,
    metadata: Value,
    max_result_inline_size: Option<usize>,
}

impl CapabilitySpecBuilder {
    pub fn new(name: impl Into<CapabilityName>, kind: impl Into<CapabilityKind>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            description: None,
            input_schema: None,
            output_schema: None,
            invocation_mode: InvocationMode::default(),
            concurrency_safe: false,
            compact_clearable: false,
            profiles: Vec::new(),
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffect::default(),
            stability: Stability::default(),
            metadata: Value::Null,
            max_result_inline_size: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn input_schema(mut self, input_schema: Value) -> Self {
        self.input_schema = Some(input_schema);
        self
    }

    pub fn output_schema(mut self, output_schema: Value) -> Self {
        self.output_schema = Some(output_schema);
        self
    }

    pub fn schema(mut self, input_schema: Value, output_schema: Value) -> Self {
        self.input_schema = Some(input_schema);
        self.output_schema = Some(output_schema);
        self
    }

    pub fn invocation_mode(mut self, invocation_mode: InvocationMode) -> Self {
        self.invocation_mode = invocation_mode;
        self
    }

    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = concurrency_safe;
        self
    }

    pub fn compact_clearable(mut self, compact_clearable: bool) -> Self {
        self.compact_clearable = compact_clearable;
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

    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn permissions(mut self, permissions: Vec<PermissionSpec>) -> Self {
        self.permissions.extend(permissions);
        self
    }

    pub fn side_effect(mut self, side_effect: SideEffect) -> Self {
        self.side_effect = side_effect;
        self
    }

    pub fn stability(mut self, stability: Stability) -> Self {
        self.stability = stability;
        self
    }

    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn max_result_inline_size(mut self, size: usize) -> Self {
        self.max_result_inline_size = Some(size);
        self
    }

    pub fn build(self) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
        let name = validate_capability_name(self.name)?;
        let kind = validate_kind(self.kind)?;
        let description = validate_non_empty(
            "description",
            self.description
                .ok_or(CapabilitySpecBuildError::MissingField("description"))?,
        )?;
        let input_schema = validate_schema(
            "input_schema",
            self.input_schema
                .ok_or(CapabilitySpecBuildError::MissingField("input_schema"))?,
        )?;
        let output_schema = validate_schema(
            "output_schema",
            self.output_schema
                .ok_or(CapabilitySpecBuildError::MissingField("output_schema"))?,
        )?;
        let profiles = validate_string_list("profiles", self.profiles)?;
        let tags = validate_string_list("tags", self.tags)?;
        let permissions = validate_permissions(self.permissions)?;

        Ok(CapabilitySpec {
            name,
            kind,
            description,
            input_schema,
            output_schema,
            invocation_mode: self.invocation_mode,
            concurrency_safe: self.concurrency_safe,
            compact_clearable: self.compact_clearable,
            profiles,
            tags,
            permissions,
            side_effect: self.side_effect,
            stability: self.stability,
            metadata: self.metadata,
            max_result_inline_size: self.max_result_inline_size,
        })
    }
}

fn validate_capability_name(
    value: CapabilityName,
) -> std::result::Result<CapabilityName, CapabilitySpecBuildError> {
    let normalized = validate_non_empty("name", value.into_string())?;
    Ok(CapabilityName::from(normalized))
}

fn validate_kind(
    value: CapabilityKind,
) -> std::result::Result<CapabilityKind, CapabilitySpecBuildError> {
    if value.as_str().trim().is_empty() {
        return Err(CapabilitySpecBuildError::EmptyField("kind"));
    }
    Ok(value)
}

fn validate_non_empty(
    field: &'static str,
    value: String,
) -> std::result::Result<String, CapabilitySpecBuildError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CapabilitySpecBuildError::EmptyField(field));
    }
    Ok(trimmed.to_string())
}

fn validate_schema(
    field: &'static str,
    value: Value,
) -> std::result::Result<Value, CapabilitySpecBuildError> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(CapabilitySpecBuildError::InvalidSchema(field))
    }
}

fn validate_string_list(
    field: &'static str,
    values: Vec<String>,
) -> std::result::Result<Vec<String>, CapabilitySpecBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = validate_non_empty(field, value)?;
        if !seen.insert(value.clone()) {
            return Err(CapabilitySpecBuildError::DuplicateValue { field, value });
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn validate_permissions(
    permissions: Vec<PermissionSpec>,
) -> std::result::Result<Vec<PermissionSpec>, CapabilitySpecBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(permissions.len());
    for permission in permissions {
        let name = validate_non_empty("permissions", permission.name)?;
        if !seen.insert(name.clone()) {
            return Err(CapabilitySpecBuildError::DuplicateValue {
                field: "permissions",
                value: name,
            });
        }
        normalized.push(PermissionSpec {
            name,
            rationale: permission
                .rationale
                .map(|rationale| rationale.trim().to_string())
                .filter(|rationale| !rationale.is_empty()),
        });
    }
    Ok(normalized)
}

fn is_false(value: &bool) -> bool {
    !*value
}
