use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Core,
    Plugin,
    Runtime,
    Worker,
    Supervisor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PeerDescriptor {
    pub id: String,
    pub name: String,
    pub role: PeerRole,
    pub version: String,
    #[serde(default)]
    pub supported_profiles: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Classification metadata attached to a capability descriptor.
///
/// This string-like type helps hosts and plugins with routing, policy, and presentation. It
/// should not be treated as a second invocation protocol layered on top of `{descriptor, invoke}`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct CapabilityKind(String);

impl CapabilityKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into().trim().to_string())
    }

    /// Alias for [`Self::new`]; kept for readability at call-sites that construct
    /// non-standard / user-defined capability kinds.
    pub fn custom(kind: impl Into<String>) -> Self {
        Self::new(kind)
    }

    pub fn tool() -> Self {
        Self::new("tool")
    }

    pub fn agent() -> Self {
        Self::new("agent")
    }

    pub fn context_provider() -> Self {
        Self::new("context_provider")
    }

    pub fn memory_provider() -> Self {
        Self::new("memory_provider")
    }

    pub fn policy_hook() -> Self {
        Self::new("policy_hook")
    }

    pub fn renderer() -> Self {
        Self::new("renderer")
    }

    pub fn resource() -> Self {
        Self::new("resource")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_tool(&self) -> bool {
        self.as_str() == "tool"
    }
}

impl Default for CapabilityKind {
    fn default() -> Self {
        Self::tool()
    }
}

impl From<&str> for CapabilityKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CapabilityKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for CapabilityKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectLevel {
    #[default]
    None,
    Local,
    Workspace,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StabilityLevel {
    Experimental,
    #[default]
    Stable,
    Deprecated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionHint {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    pub name: String,
    /// Routing and policy metadata for this capability.
    ///
    /// Hosts may project some kinds into specific surfaces, such as tool-call UIs, but the
    /// capability transport itself remains descriptor + invoke.
    #[serde(default)]
    pub kind: CapabilityKind,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<PermissionHint>,
    #[serde(default)]
    pub side_effect: SideEffectLevel,
    #[serde(default)]
    pub stability: StabilityLevel,
}

impl CapabilityDescriptor {
    pub fn builder(
        name: impl Into<String>,
        kind: impl Into<CapabilityKind>,
    ) -> CapabilityDescriptorBuilder {
        CapabilityDescriptorBuilder::new(name, kind)
    }

    /// Validates a descriptor that may have been constructed directly or decoded from the wire.
    ///
    /// Runtime and plugin registration paths call this so plugin authors get the same guarantees
    /// as the builder API even when they do not use the builder helpers.
    pub fn validate(&self) -> Result<(), DescriptorBuildError> {
        validate_non_empty("name", self.name.clone())?;
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
pub enum DescriptorBuildError {
    EmptyField(&'static str),
    MissingField(&'static str),
    InvalidSchema(&'static str),
    DuplicateValue { field: &'static str, value: String },
}

impl fmt::Display for DescriptorBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "descriptor field '{field}' cannot be empty"),
            Self::MissingField(field) => write!(f, "descriptor field '{field}' is required"),
            Self::InvalidSchema(field) => {
                write!(f, "descriptor field '{field}' must be a JSON object schema")
            }
            Self::DuplicateValue { field, value } => {
                write!(
                    f,
                    "descriptor field '{field}' contains duplicate value '{value}'"
                )
            }
        }
    }
}

impl std::error::Error for DescriptorBuildError {}

#[derive(Debug, Clone)]
pub struct CapabilityDescriptorBuilder {
    name: String,
    kind: CapabilityKind,
    description: Option<String>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
    streaming: bool,
    profiles: Vec<String>,
    tags: Vec<String>,
    permissions: Vec<PermissionHint>,
    side_effect: SideEffectLevel,
    stability: StabilityLevel,
}

impl CapabilityDescriptorBuilder {
    pub fn new(name: impl Into<String>, kind: impl Into<CapabilityKind>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            description: None,
            input_schema: None,
            output_schema: None,
            streaming: false,
            profiles: Vec::new(),
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffectLevel::default(),
            stability: StabilityLevel::default(),
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

    pub fn streaming(mut self, streaming: bool) -> Self {
        self.streaming = streaming;
        self
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

    pub fn permissions(mut self, permissions: Vec<PermissionHint>) -> Self {
        self.permissions.extend(permissions);
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

    pub fn build(self) -> Result<CapabilityDescriptor, DescriptorBuildError> {
        let name = validate_non_empty("name", self.name)?;
        let kind = validate_kind(self.kind)?;
        let description = validate_non_empty(
            "description",
            self.description
                .ok_or(DescriptorBuildError::MissingField("description"))?,
        )?;
        let input_schema = validate_schema(
            "input_schema",
            self.input_schema
                .ok_or(DescriptorBuildError::MissingField("input_schema"))?,
        )?;
        let output_schema = validate_schema(
            "output_schema",
            self.output_schema
                .ok_or(DescriptorBuildError::MissingField("output_schema"))?,
        )?;
        let profiles = validate_string_list("profiles", self.profiles)?;
        let tags = validate_string_list("tags", self.tags)?;
        let permissions = validate_permissions(self.permissions)?;

        Ok(CapabilityDescriptor {
            name,
            kind,
            description,
            input_schema,
            output_schema,
            streaming: self.streaming,
            profiles,
            tags,
            permissions,
            side_effect: self.side_effect,
            stability: self.stability,
        })
    }
}

fn validate_kind(value: CapabilityKind) -> Result<CapabilityKind, DescriptorBuildError> {
    Ok(CapabilityKind(validate_non_empty("kind", value.0)?))
}

fn validate_non_empty(field: &'static str, value: String) -> Result<String, DescriptorBuildError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DescriptorBuildError::EmptyField(field));
    }
    Ok(trimmed.to_string())
}

fn validate_schema(field: &'static str, value: Value) -> Result<Value, DescriptorBuildError> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(DescriptorBuildError::InvalidSchema(field))
    }
}

fn validate_string_list(
    field: &'static str,
    values: Vec<String>,
) -> Result<Vec<String>, DescriptorBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = validate_non_empty(field, value)?;
        if !seen.insert(value.clone()) {
            return Err(DescriptorBuildError::DuplicateValue { field, value });
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn validate_permissions(
    permissions: Vec<PermissionHint>,
) -> Result<Vec<PermissionHint>, DescriptorBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(permissions.len());
    for permission in permissions {
        let name = validate_non_empty("permissions", permission.name)?;
        if !seen.insert(name.clone()) {
            return Err(DescriptorBuildError::DuplicateValue {
                field: "permissions",
                value: name,
            });
        }
        normalized.push(PermissionHint {
            name,
            rationale: permission
                .rationale
                .map(|rationale| rationale.trim().to_string())
                .filter(|rationale| !rationale.is_empty()),
        });
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDescriptor {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilterDescriptor {
    pub field: String,
    pub op: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HandlerDescriptor {
    pub id: String,
    pub trigger: TriggerDescriptor,
    pub input_schema: Value,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub filters: Vec<FilterDescriptor>,
    #[serde(default)]
    pub permissions: Vec<PermissionHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDescriptor {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub context_schema: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallerRef {
    pub id: String,
    pub role: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InvocationContext {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<CallerRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetHint>,
    pub profile: String,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub metadata: Value,
}
