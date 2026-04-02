// Re-export capability types from the authoritative definition in the protocol crate.
// Core and all downstream crates share a single canonical type, eliminating duplicate
// definitions and the manual conversion functions that bridged them.
pub use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, DescriptorBuildError,
    PermissionHint, SideEffectLevel, StabilityLevel,
};
