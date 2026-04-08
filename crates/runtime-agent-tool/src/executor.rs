use astrcode_core::{Result, SpawnAgentParams, SubRunResult, ToolContext};
use async_trait::async_trait;

/// 子 Agent 执行器抽象。
///
/// 真实执行器由 runtime 提供，这里只定义 Tool 所需的最小边界。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// 启动子 Agent。
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}
