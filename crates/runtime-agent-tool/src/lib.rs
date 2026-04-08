//! # Agent as Tool
//!
//! 提供 `spawnAgent` 工具的稳定抽象：
//! - 对 LLM 暴露统一的工具定义和参数 schema
//! - 将真实执行委托给运行时注入的 `SubAgentExecutor`
//! - 不直接依赖 `RuntimeService`，避免把 runtime 细节扩散到 Tool crate
//!
//! 协作工具（sendAgent / waitAgent / closeAgent / resumeAgent / deliverToParent）
//! 遵循相同模式，将执行委托给 `CollaborationExecutor`。

mod close_tool;
mod collab_result_mapping;
mod collaboration_executor;
mod deliver_tool;
mod executor;
mod result_mapping;
mod resume_tool;
mod send_tool;
mod spawn_tool;
mod wait_tool;

pub use astrcode_core::{
    CloseAgentParams, CollaborationResult, CollaborationResultKind, DeliverToParentParams,
    ResumeAgentParams, SendAgentParams, SpawnAgentParams, WaitAgentParams, WaitUntil,
};
pub use close_tool::CloseAgentTool;
pub use collaboration_executor::CollaborationExecutor;
pub use deliver_tool::DeliverToParentTool;
pub use executor::SubAgentExecutor;
pub use resume_tool::ResumeAgentTool;
pub use send_tool::SendAgentTool;
pub use spawn_tool::SpawnAgentTool;
pub use wait_tool::WaitAgentTool;

#[cfg(test)]
mod tests;
