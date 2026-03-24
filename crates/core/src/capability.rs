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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub streaming: bool,
}

impl CapabilityDescriptor {
    pub fn namespace(&self) -> CapabilityNamespace {
        CapabilityNamespace::from_capability_name(&self.name)
    }
}
