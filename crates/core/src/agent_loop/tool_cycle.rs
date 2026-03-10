use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::action::{LlmMessage, ToolCallRequest};
use crate::events::StorageEvent;
use crate::tools::registry::ToolRegistry;

pub(crate) enum ToolCycleOutcome {
    Completed,
    Interrupted,
}

pub(crate) async fn execute_tool_calls(
    tools: &ToolRegistry,
    tool_calls: Vec<ToolCallRequest>,
    messages: &mut Vec<LlmMessage>,
    on_event: &mut impl FnMut(StorageEvent),
    cancel: &CancellationToken,
) -> ToolCycleOutcome {
    for call in tool_calls {
        if cancel.is_cancelled() {
            return ToolCycleOutcome::Interrupted;
        }

        on_event(StorageEvent::ToolCall {
            tool_call_id: call.id.clone(),
            tool_name: call.name.clone(),
            args: call.args.clone(),
        });

        let start = Instant::now();
        let result = tools.execute(&call, cancel.child_token()).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        on_event(StorageEvent::ToolResult {
            tool_call_id: call.id.clone(),
            output: tool_result_output(&result),
            success: result.ok,
            duration_ms,
        });

        messages.push(LlmMessage::Tool {
            tool_call_id: call.id,
            content: result.model_content(),
        });
    }

    ToolCycleOutcome::Completed
}

fn tool_result_output(result: &crate::action::ToolExecutionResult) -> String {
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
