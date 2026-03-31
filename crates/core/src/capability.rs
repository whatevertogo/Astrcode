use serde::{Deserialize, Serialize};

/// Namespace extracted from a capability's dotted name.
///
/// `"tool.read_file"` → namespace `"tool"`.
/// `"shell"` → namespace `"shell"` (no separator → full name).
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

// Re-export capability types from the authoritative definition in the protocol crate.
// Core and all downstream crates share a single canonical type, eliminating duplicate
// definitions and the manual conversion functions that bridged them.
pub use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, DescriptorBuildError,
    PermissionHint, SideEffectLevel, StabilityLevel,
};

#[cfg(test)]
mod tests {
    use super::CapabilityNamespace;

    #[test]
    fn namespace_extracts_prefix_before_dot() {
        let ns = CapabilityNamespace::from_capability_name("tool.read_file");
        assert_eq!(ns.name, "tool");
    }

    #[test]
    fn namespace_returns_full_name_when_no_dot() {
        let ns = CapabilityNamespace::from_capability_name("shell");
        assert_eq!(ns.name, "shell");
    }
}
