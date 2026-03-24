use serde::{Deserialize, Serialize};

use crate::{AstrError, CapabilityDescriptor};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginType {
    Tool,
    Orchestrator,
    Provider,
    Hook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub plugin_type: Vec<PluginType>,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub executable: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub repository: Option<String>,
}

impl PluginManifest {
    pub fn from_toml(s: &str) -> std::result::Result<Self, AstrError> {
        toml::from_str(s).map_err(|error| {
            AstrError::Validation(format!("failed to parse plugin manifest TOML: {error}"))
        })
    }
}
