use astrcode_core::{
    CloseAgentParams, CollaborationResult, DeliverToParentParams, ObserveParams, Result,
    ResumeAgentParams, SendAgentParams, ToolContext, WaitAgentParams,
};
use async_trait::async_trait;

/// 协作工具执行器抽象。
///
/// 与 `SubAgentExecutor` 拆开是因为两者的职责粒度完全不同：
/// - `SubAgentExecutor` 只管"创建并启动"，是一锤子买卖；
/// - `CollaborationExecutor` 管理已存在 agent 的整个协作生命周期（发消息/观测/关闭/恢复/交付）。
///
/// 真实实现由 runtime 边界注入，本 crate 不感知 session 调度细节。
#[async_trait]
pub trait CollaborationExecutor: Send + Sync {
    /// 向既有 child agent 追加消息。
    ///
    /// 消息会进入 child agent 的 inbox，由其下一轮 LLM 调用消费。
    async fn send(&self, params: SendAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 等待指定 child agent 到达可消费状态（终态或下一次交付）。
    ///
    /// 调用方（父 agent）会阻塞在此，直到条件满足。
    async fn wait(&self, params: WaitAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 关闭指定 child agent。
    ///
    /// runtime 层负责级联关闭逻辑（默认关闭整棵子树）。
    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 恢复已完成的 child agent 继续协作。
    ///
    /// 复用同一 child session，不创建新会话——这保证了上下文连续性。
    async fn resume(
        &self,
        params: ResumeAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 向直接父 agent 交付结果。
    ///
    /// 只能由 child session 调用，交付目标固定为直接父 agent（不可跨级）。
    async fn deliver(
        &self,
        params: DeliverToParentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    ///
    /// 只返回直接子 agent 的快照，融合 live lifecycle、对话投影和 mailbox 派生信息。
    async fn observe(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;
}
