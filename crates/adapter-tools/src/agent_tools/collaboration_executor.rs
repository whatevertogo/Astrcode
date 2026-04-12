use astrcode_core::{
    CloseAgentParams, CollaborationResult, ObserveParams, Result, SendAgentParams, ToolContext,
};
use async_trait::async_trait;

/// 协作工具执行器抽象。
///
/// 与 `SubAgentExecutor` 拆开是因为两者的职责粒度完全不同：
/// - `SubAgentExecutor` 只管"创建并启动"，是一锤子买卖；
/// - `CollaborationExecutor` 管理已存在 agent 的协作生命周期（发消息/观测/关闭）。
///
/// 真实实现由 runtime 边界注入，本 crate 不感知 session 调度细节。
#[async_trait]
pub trait CollaborationExecutor: Send + Sync {
    /// 向既有 child agent 追加消息。
    ///
    /// 消息会进入 child agent 的 inbox，由其下一轮 LLM 调用消费。
    async fn send(&self, params: SendAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 关闭指定 child agent。
    ///
    /// runtime 层负责级联关闭逻辑（关闭整棵子树）。
    async fn close(
        &self,
        params: CloseAgentParams,
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
