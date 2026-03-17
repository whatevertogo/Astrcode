use std::time::Instant;

use anyhow::Result as AnyhowResult;
use astrcode_core::{CancelToken, Result as CoreResult};

use super::AgentLoop;
use crate::events::StorageEvent;
use crate::projection::AgentState;
use crate::tool_registry::ToolRegistry;
use astrcode_core::{LlmMessage, ToolCallRequest};

pub(crate) enum ToolCycleOutcome {
    Completed,
    Interrupted,
}

pub(crate) async fn execute_tool_calls(
    agent_loop: &AgentLoop,
    tools: &ToolRegistry,
    tool_calls: Vec<ToolCallRequest>,
    turn_id: &str,
    state: &AgentState,
    messages: &mut Vec<LlmMessage>,
    on_event: &mut impl FnMut(StorageEvent) -> CoreResult<()>,
    cancel: &CancelToken,
) -> AnyhowResult<ToolCycleOutcome> {
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
        let result = tools.execute(&call, &ctx).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        on_event(StorageEvent::ToolResult {
            turn_id: Some(turn_id.to_string()),
            tool_call_id: call.id.clone(),
            output: tool_result_output(&result),
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

fn tool_result_output(result: &astrcode_core::ToolExecutionResult) -> String {
    if result.ok {
        result.output.clone()
    } else {
        format!(
            "tool execution failed: {}\n{}",
            result.error.as_deref().unwrap_or("unknown error"),
            result.output
        )
    }
}
