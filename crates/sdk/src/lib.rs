//! # Astrcode 插件 SDK
//!
//! 本库为插件开发者提供 Rust SDK，用于编写 Astrcode 插件。
//!
//! ## 架构定位
//!
//! SDK 是插件与 Astrcode 运行时之间的桥梁。插件通过 SDK 注册工具、
//! 定义策略钩子、访问调用上下文和发送流式响应，而无需直接依赖
//! `core` 或 `runtime` crate。
//!
//! ## 核心功能
//!
//! - **ToolHandler**: 定义工具的处理逻辑，支持类型安全的输入/输出
//! - **HookRegistry**: 注册策略钩子（如权限检查、路径白名单）
//! - **PluginContext**: 访问插件调用上下文（工作目录、编辑器状态等）
//! - **StreamWriter**: 流式响应写入，支持增量输出到前端
//!
//! ## 快速开始
//!
//! ```ignore
//! use astrcode_sdk::{ToolHandler, ToolRegistration, ToolFuture, PluginContext, StreamWriter};
//! use astrcode_sdk::{CapabilityDescriptor, CapabilityKind, SideEffectLevel};
//! use serde::{Deserialize, Serialize};
//!
//! // 1. 定义工具的输入/输出类型
//! #[derive(Deserialize)]
//! struct GreetInput { name: String }
//!
//! #[derive(Serialize)]
//! struct GreetOutput { message: String }
//!
//! // 2. 实现 ToolHandler
//! struct GreetTool;
//!
//! impl ToolHandler<GreetInput, GreetOutput> for GreetTool {
//!     fn descriptor(&self) -> CapabilityDescriptor {
//!         CapabilityDescriptor::builder()
//!             .name("greet")
//!             .kind(CapabilityKind::Tool)
//!             .description("向指定用户打招呼")
//!             .side_effect_level(SideEffectLevel::None)
//!             .build()
//!             .unwrap()
//!     }
//!
//!     fn execute(&self, input: GreetInput, _ctx: PluginContext, _stream: StreamWriter) -> ToolFuture<'_, GreetOutput> {
//!         Box::pin(async move {
//!             Ok(GreetOutput { message: format!("Hello, {}!", input.name) })
//!         })
//!     }
//! }
//!
//! // 3. 注册工具
//! let registration = ToolRegistration::new(GreetTool);
//! ```
//!
//! ## Crate 依赖
//!
//! SDK 依赖 `protocol`（DTO 类型）和 `core`（接口定义），
//! 但插件作者无需关心这些内部依赖，SDK 会 re-export 必要的类型。

mod context;
mod error;
mod hook;
mod stream;
#[cfg(test)]
mod tests;
mod tool;

// Re-export protocol types that plugin authors commonly need.
pub use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, DescriptorBuildError,
    PermissionHint, SideEffectLevel, StabilityLevel,
};
// Re-export SDK core types.
pub use context::PluginContext;
pub use error::{SdkError, ToolSerdeStage};
pub use hook::{
    HookRegistry, HookShortCircuit, PolicyDecision, PolicyHook, PolicyHookChain,
    RegisteredPolicyHook,
};
// Re-export serde for convenience, so plugin authors don't need a separate dependency.
pub use serde::{de::DeserializeOwned, Serialize};
pub use stream::{StreamChunk, StreamWriter};
pub use tool::{DynToolHandler, ToolFuture, ToolHandler, ToolRegistration, ToolResult};
