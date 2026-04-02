use std::time::Instant;

use astrcode_core::{
    ApprovalPending, ApprovalResolution, CancelToken, CapabilityCall, PolicyVerdict, Result,
    ToolExecutionResult,
};
use tokio::sync::mpsc;

use super::AgentLoop;
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;
use astrcode_core::{CapabilityRouter, LlmMessage, ToolCallRequest};

pub(crate) enum ToolCycleOutcome {
    Completed,
    Interrupted,
}

#[allow(clippy::too_many_arguments)]
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
                    let ApprovalPending { request, action } = *pending;
                    let pending_call = normalized_tool_call(&proposed_call, action)?;
                    let resolution = agent_loop.approval.request(request, cancel.clone()).await?;

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
    let (tool_output_tx, mut tool_output_rx) = mpsc::unbounded_channel();
    let tool_call_for_execution = tool_call.clone();
    let tool_ctx_for_execution = ctx.clone().with_tool_output_sender(tool_output_tx);

    // Yield before local IO-heavy tools so other tasks can make progress between tool calls.
    tokio::task::yield_now().await;
    let mut execute_tool = Some(Box::pin(async move {
        capabilities
            .execute_tool(&tool_call_for_execution, &tool_ctx_for_execution)
            .await
    }));
    let mut execution_result = None;
    let mut output_stream_open = true;

    while execution_result.is_none() || output_stream_open {
        if execution_result.is_none() {
            tokio::select! {
                result = execute_tool
                    .as_mut()
                    .expect("tool future should exist until it resolves")
                    .as_mut() => {
                    execution_result = Some(result);
                    // Drop runtime's last sender clone as soon as the tool future resolves so the
                    // receiver can observe channel closure after background reader threads drain.
                    drop(execute_tool.take());
                    // Safety: the tool future resolving guarantees all background reader threads
                    // (e.g. shell stdout/stderr) have already been joined *inside* the tool impl
                    // before it returned. Every sender clone is therefore dropped, so the channel
                    // is already closed or will close as soon as the tokio runtime flushes
                    // remaining buffered items. The drain loop below (recv() after this branch)
                    // is safe to assume no new deltas can arrive — it only empties the buffer.
                }
                maybe_delta = tool_output_rx.recv(), if output_stream_open => {
                    match maybe_delta {
                        Some(delta) => {
                            if let Err(error) = on_event(StorageEvent::ToolCallDelta {
                                turn_id: Some(turn_id.to_string()),
                                tool_call_id: delta.tool_call_id,
                                tool_name: delta.tool_name,
                                stream: delta.stream,
                                delta: delta.delta,
                            }) {
                                ctx.cancel().cancel();
                                return Err(error);
                            }
                        }
                        None => {
                            output_stream_open = false;
                        }
                    }
                }
            }
            continue;
        }

        match tool_output_rx.recv().await {
            Some(delta) => {
                if let Err(error) = on_event(StorageEvent::ToolCallDelta {
                    turn_id: Some(turn_id.to_string()),
                    tool_call_id: delta.tool_call_id,
                    tool_name: delta.tool_name,
                    stream: delta.stream,
                    delta: delta.delta,
                }) {
                    ctx.cancel().cancel();
                    return Err(error);
                }
            }
            None => {
                output_stream_open = false;
            }
        }
    }

    let mut result = execution_result.expect("tool execution future should resolve");
    result.duration_ms = start.elapsed().as_millis() as u64;
    on_event(StorageEvent::ToolResult {
        turn_id: Some(turn_id.to_string()),
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        output: result.output.clone(),
        success: result.ok,
        error: result.error.clone(),
        metadata: result.metadata.clone(),
        duration_ms: result.duration_ms,
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
        output: String::new(),
        success: false,
        error: Some(reason.to_string()),
        metadata: None,
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
