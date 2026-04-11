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
//! | `send` | 向子 Agent / 父 Agent 发送消息 | `CollaborationExecutor` |
//! | `observe` | 获取子 Agent 状态快照 | `CollaborationExecutor` |
//! | `close` | 终止子 Agent 及子树 | `CollaborationExecutor` |
//!
//! ## 内部保留工具（不在公开 prompt 中暴露）
//!
//! | 工具 | 用途 |
//! |------|------|
//! | `waitAgent` | 阻塞等待子 Agent 状态变化（过渡期内部保留） |
//! | `resumeAgent` | 恢复已完成的子 Agent（过渡期内部保留） |
//! | `deliverToParent` | 子 Agent 向父 Agent 交付结果（过渡期内部保留） |
//!
//! ## 设计约束
//!
//! - `agentId` 是 LLM 不可编造的稳定标识，必须逐字复用 tool result 中的原始值。
//! - 参数校验在工具层尽早完成，避免无意义请求下沉到 runtime。

mod close_tool;
mod collab_result_mapping;
mod collaboration_executor;
mod deliver_tool;
mod executor;
mod observe_tool;
mod result_mapping;
mod resume_tool;
mod send_tool;
mod spawn_tool;
mod wait_tool;

pub use astrcode_core::{
    CloseAgentParams, CollaborationResult, CollaborationResultKind, DeliverToParentParams,
    ObserveParams, ResumeAgentParams, SendAgentParams, SpawnAgentParams, WaitAgentParams,
    WaitUntil,
};
pub use close_tool::CloseAgentTool;
pub use collaboration_executor::CollaborationExecutor;
pub use deliver_tool::DeliverToParentTool;
pub use executor::SubAgentExecutor;
pub use observe_tool::ObserveAgentTool;
pub use resume_tool::ResumeAgentTool;
pub use send_tool::SendAgentTool;
pub use spawn_tool::SpawnAgentTool;
pub use wait_tool::WaitAgentTool;

#[cfg(test)]
mod tests;
