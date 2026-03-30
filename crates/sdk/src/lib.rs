mod context;
mod error;
mod hook;
mod macros;
mod stream;
#[cfg(test)]
mod tests;
mod tool;

pub use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, DescriptorBuildError,
    PermissionHint, SideEffectLevel, StabilityLevel,
};
pub use context::PluginContext;
pub use error::{SdkError, ToolSerdeStage};
pub use hook::{
    HookRegistry, HookShortCircuit, PolicyDecision, PolicyHook, PolicyHookChain,
    RegisteredPolicyHook,
};
pub use serde::{de::DeserializeOwned, Serialize};
pub use stream::{StreamChunk, StreamWriter};
pub use tool::{DynToolHandler, ToolFuture, ToolHandler, ToolRegistration, ToolResult};
