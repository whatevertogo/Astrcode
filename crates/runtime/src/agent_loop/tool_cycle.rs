use std::time::Instant;

use astrcode_core::{
    ApprovalResolution, CancelToken, CapabilityCall, PolicyVerdict, Result, ToolExecutionResult,
};

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
    step_index: usize,
    messages: &mut Vec<LlmMessage>,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    cancel: &CancelToken,
) -> Result<ToolCycleOutcome> {
    for call in tool_calls {
        if cancel.is_cancelled() {
            return Ok(ToolCycleOutcome::Interrupted);
        }

        let ctx = agent_loop.tool_context(state, cancel.clone());
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        let result = if let Some(descriptor) = capabilities.descriptor(&call.name) {
            let proposed_call = CapabilityCall {
                request_id: call.id.clone(),
                descriptor,
                payload: call.args.clone(),
                metadata: serde_json::Value::Null,
            };
            match agent_loop
                .policy
                .check_capability_call(proposed_call.clone(), &policy_ctx)
                .await?
            {
                PolicyVerdict::Allow(allowed_call) => {
                    execute_tool_call(
                        capabilities,
                        normalized_tool_call(&proposed_call, allowed_call)?,
                        turn_id,
                        &ctx,
                        on_event,
                    )
                    .await?
                }
                PolicyVerdict::Deny { reason } => {
                    denied_tool_result(&call, turn_id, &reason, on_event)?;
                    denial_result(&call, reason)
                }
                PolicyVerdict::Ask(pending) => {
                    let pending_call = normalized_tool_call(&proposed_call, pending.action)?;
                    let resolution = agent_loop
                        .approval
                        .request(pending.request, cancel.clone())
                        .await?;

                    if resolution.approved {
                        execute_tool_call(capabilities, pending_call, turn_id, &ctx, on_event)
                            .await?
                    } else {
                        let reason = approval_denial_reason(&resolution);
                        denied_tool_result(&call, turn_id, &reason, on_event)?;
                        denial_result(&call, reason)
                    }
                }
            }
        } else {
            execute_raw_tool_call(capabilities, call.clone(), turn_id, &ctx, on_event).await?
        };

        messages.push(LlmMessage::Tool {
            tool_call_id: call.id,
            content: result.model_content(),
        });
    }

    Ok(ToolCycleOutcome::Completed)
}

async fn execute_tool_call(
    capabilities: &CapabilityRouter,
    call: CapabilityCall,
    turn_id: &str,
    ctx: &astrcode_core::ToolContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<ToolExecutionResult> {
    let tool_call = ToolCallRequest {
        id: call.request_id,
        name: call.descriptor.name,
        args: call.payload,
    };
    execute_raw_tool_call(capabilities, tool_call, turn_id, ctx, on_event).await
}

async fn execute_raw_tool_call(
    capabilities: &CapabilityRouter,
    tool_call: ToolCallRequest,
    turn_id: &str,
    ctx: &astrcode_core::ToolContext,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<ToolExecutionResult> {
    on_event(StorageEvent::ToolCall {
        turn_id: Some(turn_id.to_string()),
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        args: tool_call.args.clone(),
    })?;

    let start = Instant::now();

    // Yield before local IO-heavy tools so other tasks can make progress between tool calls.
    tokio::task::yield_now().await;
    let result = capabilities.execute_tool(&tool_call, ctx).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    on_event(StorageEvent::ToolResult {
        turn_id: Some(turn_id.to_string()),
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        output: result.model_content(),
        success: result.ok,
        duration_ms,
    })?;

    Ok(result)
}

fn denied_tool_result(
    call: &ToolCallRequest,
    turn_id: &str,
    reason: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::ToolCall {
        turn_id: Some(turn_id.to_string()),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.args.clone(),
    })?;
    on_event(StorageEvent::ToolResult {
        turn_id: Some(turn_id.to_string()),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        output: format!("tool execution blocked: {reason}"),
        success: false,
        duration_ms: 0,
    })
}

fn denial_result(call: &ToolCallRequest, reason: String) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        ok: false,
        output: String::new(),
        error: Some(reason),
        metadata: None,
        duration_ms: 0,
        truncated: false,
    }
}

fn approval_denial_reason(resolution: &ApprovalResolution) -> String {
    resolution
        .reason
        .clone()
        .unwrap_or_else(|| "approval denied".to_string())
}

fn normalized_tool_call(
    original: &CapabilityCall,
    updated: CapabilityCall,
) -> Result<CapabilityCall> {
    if original.request_id != updated.request_id {
        return Err(astrcode_core::AstrError::Validation(
            "policy rewrites must preserve capability request_id".to_string(),
        ));
    }

    if original.descriptor.name != updated.descriptor.name {
        return Err(astrcode_core::AstrError::Validation(
            "policy rewrites must preserve capability identity".to_string(),
        ));
    }

    Ok(CapabilityCall {
        request_id: original.request_id.clone(),
        descriptor: original.descriptor.clone(),
        payload: updated.payload,
        metadata: updated.metadata,
    })
}
