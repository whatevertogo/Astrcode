//! Runtime 侧能力注册实现。
//!
//! 该 crate 承载 `CapabilityRouter` / `ToolRegistry` 等具体实现，
//! 避免 core 持有运行时实现细节，保持 core 为契约与 DTO 层。

pub mod router;
pub mod tool;

pub use router::{CapabilityRouter, CapabilityRouterBuilder};
pub use tool::{
    ToolCapabilityInvoker, ToolRegistry, ToolRegistryBuilder, tools_into_capability_invokers,
};
