use std::time::Instant;

use astrcode_core::{CancelToken, Result};

use super::AgentLoop;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;
use astrcode_core::{CapabilityRouter, LlmMessage, ToolCallRequest};

pub(crate) enum ToolCycleOutcome {
    Completed,
    Interrupted,
}

pub(crate) async fn execute_tool_calls(
    agent_loop: &AgentLoop,
    capabilities: &CapabilityRouter,
    tool_calls: Vec<ToolCallRequest>,
    turn_id: &str,
    state: &AgentState,
    messages: &mut Vec<LlmMessage>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    cancel: &CancelToken,
) -> Result<ToolCycleOutcome> {
    for call in tool_calls {
        if cancel.is_cancelled() {
            return Ok(ToolCycleOutcome::Interrupted);
        }

        on_event(StorageEvent::ToolCall {
            turn_id: Some(turn_id.to_string()),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.args.clone(),
        })?;

        let start = Instant::now();
        let ctx = agent_loop.tool_context(state, cancel.clone());

        // 让出控制权，允许其他任务运行
        // 注意：工具内部包含阻塞操作（shell、文件 I/O 等）
        // 在高并发场景下应使用 spawn_blocking，但对本地开发工具当前实现可接受
        tokio::task::yield_now().await;
        let result = capabilities.execute_tool(&call, &ctx).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        on_event(StorageEvent::ToolResult {
            turn_id: Some(turn_id.to_string()),
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            output: result.model_content(),
            success: result.ok,
            duration_ms,
        })?;

        messages.push(LlmMessage::Tool {
            tool_call_id: call.id,
            content: result.model_content(),
        });
    }

    Ok(ToolCycleOutcome::Completed)
}
