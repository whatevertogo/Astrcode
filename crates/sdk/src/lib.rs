//! # Astrcode 插件 SDK
//!
//! 本库为插件开发者提供 Rust SDK，用于编写 Astrcode 插件。
//!
//! ## 核心功能
//!
//! - **ToolHandler**: 定义工具的处理逻辑
//! - **HookRegistry**: 注册策略钩子（如权限检查）
//! - **PluginContext**: 访问插件上下文（工作目录等）
//! - **StreamWriter**: 流式响应写入

mod context;
mod error;
mod hook;
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
