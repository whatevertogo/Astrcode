use astrcode_core::{Result, ToolCallRequest};
use astrcode_tool_contract::{ToolExecutionResult, ToolOutputDelta};
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

/// `agent-runtime -> plugin-host` 的工具调度请求。
#[derive(Debug, Clone)]
pub struct ToolDispatchRequest {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub tool_call: ToolCallRequest,
    pub tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
}

/// runtime 消费的抽象工具调度面。
///
/// 真实工具归属 plugin-host active snapshot；runtime 只提交一次工具调用并接收
/// 纯数据结果，不持有 plugin registry 或具体 invoker。
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch_tool(&self, request: ToolDispatchRequest) -> Result<ToolExecutionResult>;
}
