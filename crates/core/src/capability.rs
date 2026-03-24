use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityNamespace {
    pub name: String,
}

impl CapabilityNamespace {
    pub fn from_capability_name(name: &str) -> Self {
        // Keep the full input when no separator exists so malformed or legacy names remain visible
        // to callers instead of being silently normalized away.
        let namespace = name
            .split_once('.')
            .map_or(name, |(namespace, _)| namespace);
        Self {
            name: namespace.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    #[default]
    Tool,
    Agent,
    ContextProvider,
    MemoryProvider,
    PolicyHook,
    Renderer,
    Resource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionHint {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityDescriptor {
    pub name: String,
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
    pub fn namespace(&self) -> CapabilityNamespace {
        CapabilityNamespace::from_capability_name(&self.name)
    }
}
