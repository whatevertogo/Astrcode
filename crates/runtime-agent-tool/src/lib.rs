//! # Agent as Tool
//!
//! 将子 Agent 的生命周期操作封装为 LLM 可调用的内置工具。
//!
//! ## 架构分层
//!
//! - **Tool 层**（本 crate）：定义 JSON schema、参数校验、结果映射； 通过 `SubAgentExecutor` /
//!   `CollaborationExecutor` 两个 trait 把真实执行委托给 runtime， 不直接依赖
//!   `RuntimeService`，避免把 runtime 细节扩散到 Tool crate。
//! - **Runtime 层**：实现上述两个 trait，负责创建 session、调度 event、管理 inbox。
//!
//! ## 四工具公开面
//!
//! | 工具 | 用途 | 执行器 |
//! |------|------|--------|
//! | `spawn` | 创建并启动子 Agent | `SubAgentExecutor` |
//! | `send` | 向子 Agent 发送消息 | `CollaborationExecutor` |
//! | `observe` | 获取子 Agent 状态快照 | `CollaborationExecutor` |
//! | `close` | 终止子 Agent 及子树 | `CollaborationExecutor` |

mod close_tool;
mod collab_result_mapping;
mod collaboration_executor;
mod executor;
mod observe_tool;
mod result_mapping;
mod send_tool;
mod spawn_tool;

pub use astrcode_core::{
    CloseAgentParams, CollaborationResult, CollaborationResultKind, ObserveParams, SendAgentParams,
    SpawnAgentParams,
};
pub use close_tool::CloseAgentTool;
pub use collaboration_executor::CollaborationExecutor;
pub use executor::SubAgentExecutor;
pub use observe_tool::ObserveAgentTool;
pub use send_tool::SendAgentTool;
pub use spawn_tool::SpawnAgentTool;

#[cfg(test)]
mod tests;
