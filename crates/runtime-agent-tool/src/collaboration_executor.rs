use astrcode_core::{
    CloseAgentParams, CollaborationResult, DeliverToParentParams, Result, ResumeAgentParams,
    SendAgentParams, ToolContext, WaitAgentParams,
};
use async_trait::async_trait;

/// 协作工具执行器抽象。
///
/// 真实执行器由 runtime 提供，这里只定义 Tool 所需的最小边界。
/// 与 `SubAgentExecutor` 拆开是因为协作操作和 spawn 的生命周期完全不同：
/// spawn 负责创建新 agent，而协作操作面向已存在的 agent。
#[async_trait]
pub trait CollaborationExecutor: Send + Sync {
    /// 向既有 child agent 追加消息。
    async fn send(&self, params: SendAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 等待指定 child agent 到达可消费状态。
    async fn wait(&self, params: WaitAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 关闭指定 child agent。
    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 恢复已完成的 child agent 继续协作。
    async fn resume(
        &self,
        params: ResumeAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 向直接父 agent 交付结果。
    async fn deliver(
        &self,
        params: DeliverToParentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;
}
