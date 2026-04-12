use astrcode_core::{Result, SpawnAgentParams, SubRunResult, ToolContext};
use async_trait::async_trait;

/// 子 Agent 执行器抽象。
///
/// 设计意图：Tool crate 只关心"能不能启动一个子 Agent"，
/// 不关心 session 创建、event 调度等 runtime 内部细节。
/// 真实实现由 runtime 边界注入，保持编译隔离。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// 启动子 Agent，返回执行结果。
    ///
    /// 调用方（spawn_tool）会在调用前将 `tool_call_id` 注入 `ToolContext`，
    /// 以便 runtime 层将子会话与发起它的 tool_call 关联。
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}
