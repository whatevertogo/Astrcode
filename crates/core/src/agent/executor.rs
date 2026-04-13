//! # Agent 协作执行端口
//!
//! Why: `spawn/send/observe/close` 工具属于 adapter 层，但其执行契约属于
//! 业务编排边界，必须由 core 定义，避免 application 反向依赖 adapter crate。

use async_trait::async_trait;

use crate::{
    CloseAgentParams, CollaborationResult, ObserveParams, Result, SendAgentParams,
    SpawnAgentParams, SubRunResult, ToolContext,
};

/// 子 Agent 启动执行端口。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// 启动子 Agent，返回结构化执行结果。
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}

/// 子 Agent 协作执行端口（send / close / observe）。
#[async_trait]
pub trait CollaborationExecutor: Send + Sync {
    /// 发送追加消息给既有子 Agent。
    async fn send(&self, params: SendAgentParams, ctx: &ToolContext)
    -> Result<CollaborationResult>;

    /// 关闭目标子 Agent（级联关闭其子树）。
    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;

    /// 观测目标子 Agent 快照。
    async fn observe(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult>;
}
