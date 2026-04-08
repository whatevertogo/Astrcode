//! # 工具执行周期 (Tool Cycle)
//!
//! 负责执行 LLM 请求的工具调用，包括：
//! - 策略检查（Allow / Deny / Ask）
//! - 审批流程（需要用户确认的工具调用）
//! - 并发执行（只读工具可并行，写操作串行）
//! - 结果收集和事件广播
//!
//! ## 执行策略
//!
//! 工具调用分为三类：
//! - **安全调用**: 只读或幂等操作，可并发执行
//! - **不安全调用**: 有副作用的操作，需串行执行
//! - **被拒绝调用**: 策略拒绝或用户拒绝，直接返回错误结果
//!
//! ## 并发模型
//!
//! 安全工具使用 `FuturesUnordered` 并发执行，上限由 `max_tool_concurrency` 控制。
//! 不安全工具按顺序执行，避免并发写冲突。

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentState, ApprovalPending, ApprovalResolution, CancelToken,
    CapabilityCall, ExecutionOwner, LlmMessage, PolicyVerdict, Result, StorageEvent,
    ToolCallRequest, ToolEventSink, ToolExecutionResult, ToolHookResultContext,
};
use astrcode_protocol::capability::CapabilityDescriptor;
use astrcode_runtime_registry::CapabilityRouter;
use futures_util::stream::{self, StreamExt};
use tokio::sync::mpsc;

use super::AgentLoop;
use crate::hook_runtime::PreToolUseDecision;

/// 工具执行周期的最终结果。
pub(crate) enum ToolCycleOutcome {
    /// 所有工具调用均已完成，可以继续下一轮 LLM 调用
    Completed,
    /// 工具执行被用户取消（CancelToken 触发）
    Interrupted,
}

/// 待执行的工具调用，记录其在原始列表中的索引和调用详情。
struct PendingToolCall {
    /// 在原始 tool_calls 列表中的位置
    index: usize,
    /// 工具调用请求
    tool_call: ToolCallRequest,
}

/// 单个工具调用的执行结果及可能缓冲的事件。
struct CallOutcome {
    /// 工具执行结果
    result: ToolExecutionResult,
    /// 待刷入的事件列表（用于安全工具的并发执行路径）
    buffered_events: Option<Vec<StorageEvent>>,
}

struct ChannelToolEventSink {
    tx: mpsc::UnboundedSender<StorageEvent>,
}

impl ToolEventSink for ChannelToolEventSink {
    fn emit(&self, event: StorageEvent) -> Result<()> {
        self.tx.send(event).map_err(|_| {
            astrcode_core::AstrError::Internal("tool event channel closed unexpectedly".to_string())
        })
    }
}

pub(crate) struct ToolCycleContext<'a, F>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    pub(crate) agent_loop: &'a AgentLoop,
    pub(crate) capabilities: &'a CapabilityRouter,
    pub(crate) turn_id: &'a str,
    pub(crate) state: &'a AgentState,
    pub(crate) step_index: usize,
    pub(crate) agent: &'a AgentEventContext,
    pub(crate) execution_owner: &'a ExecutionOwner,
    pub(crate) messages: &'a mut Vec<LlmMessage>,
    pub(crate) on_event: &'a mut F,
    pub(crate) cancel: &'a CancelToken,
}

pub(crate) async fn execute_tool_calls<F>(
    tool_calls: Vec<ToolCallRequest>,
    ctx: ToolCycleContext<'_, F>,
) -> Result<ToolCycleOutcome>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    let ToolCycleContext {
        agent_loop,
        capabilities,
        turn_id,
        state,
        step_index,
        agent,
        execution_owner,
        messages,
        on_event,
        cancel,
    } = ctx;
    let mut safe_calls = Vec::new();
    let mut unsafe_calls = Vec::new();
    let mut outcomes = (0..tool_calls.len()).map(|_| None).collect::<Vec<_>>();

    for (index, call) in tool_calls.into_iter().enumerate() {
        if cancel.is_cancelled() {
            push_tool_messages(messages, outcomes);
            return Ok(ToolCycleOutcome::Interrupted);
        }

        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        if let Some(descriptor) = capabilities.descriptor(&call.name) {
            let proposed_call = CapabilityCall {
                request_id: call.id.clone(),
                descriptor: descriptor.clone(),
                payload: call.args.clone(),
                metadata: serde_json::Value::Null,
            };
            match agent_loop
                .policy
                .check_capability_call(proposed_call.clone(), &policy_ctx)
                .await?
            {
                PolicyVerdict::Allow(allowed_call) => {
                    let tool_call = tool_call_from_capability_call(normalized_tool_call(
                        &proposed_call,
                        allowed_call,
                    )?);
                    push_prepared_call(
                        &descriptor,
                        PendingToolCall { index, tool_call },
                        &mut safe_calls,
                        &mut unsafe_calls,
                    );
                },
                PolicyVerdict::Deny { reason } => {
                    denied_tool_result(&call, turn_id, agent, &reason, on_event)?;
                    outcomes[index] = Some(CallOutcome {
                        result: denial_result(&call, reason),
                        buffered_events: None,
                    });
                },
                PolicyVerdict::Ask(pending) => {
                    let ApprovalPending { request, action } = *pending;
                    let pending_call = normalized_tool_call(&proposed_call, action)?;
                    let resolution = agent_loop.approval.request(request, cancel.clone()).await?;

                    if resolution.approved {
                        let tool_call = tool_call_from_capability_call(pending_call);
                        push_prepared_call(
                            &descriptor,
                            PendingToolCall { index, tool_call },
                            &mut safe_calls,
                            &mut unsafe_calls,
                        );
                    } else {
                        let reason = approval_denial_reason(&resolution);
                        denied_tool_result(&call, turn_id, agent, &reason, on_event)?;
                        outcomes[index] = Some(CallOutcome {
                            result: denial_result(&call, reason),
                            buffered_events: None,
                        });
                    }
                },
            }
        } else {
            unsafe_calls.push(PendingToolCall {
                index,
                tool_call: call,
            });
        }
    }

    if !safe_calls.is_empty() {
        if cancel.is_cancelled() {
            push_tool_messages(messages, outcomes);
            return Ok(ToolCycleOutcome::Interrupted);
        }

        for (index, recorded) in execute_safe_tool_calls(
            agent_loop,
            capabilities,
            safe_calls,
            turn_id,
            state,
            agent,
            execution_owner,
            cancel,
        )
        .await?
        {
            outcomes[index] = Some(CallOutcome {
                result: recorded.result,
                buffered_events: Some(recorded.events),
            });
        }

        flush_buffered_events(&mut outcomes, on_event, cancel)?;
    }

    if !unsafe_calls.is_empty() && cancel.is_cancelled() {
        push_tool_messages(messages, outcomes);
        return Ok(ToolCycleOutcome::Interrupted);
    }

    for pending in unsafe_calls {
        if cancel.is_cancelled() {
            push_tool_messages(messages, outcomes);
            return Ok(ToolCycleOutcome::Interrupted);
        }

        let ctx = agent_loop.tool_context(state, cancel.clone(), execution_owner.clone());
        let result = execute_raw_tool_call(
            pending.tool_call,
            RawToolExecutionContext {
                agent_loop,
                capabilities,
                state,
                turn_id,
                agent,
                tool_ctx: &ctx,
                on_event,
            },
        )
        .await?;
        outcomes[pending.index] = Some(CallOutcome {
            result,
            buffered_events: None,
        });
    }

    push_tool_messages(messages, outcomes);
    Ok(ToolCycleOutcome::Completed)
}

fn push_prepared_call(
    descriptor: &CapabilityDescriptor,
    pending: PendingToolCall,
    safe_calls: &mut Vec<PendingToolCall>,
    unsafe_calls: &mut Vec<PendingToolCall>,
) {
    if descriptor.concurrency_safe {
        safe_calls.push(pending);
    } else {
        unsafe_calls.push(pending);
    }
}

fn tool_call_from_capability_call(call: CapabilityCall) -> ToolCallRequest {
    ToolCallRequest {
        id: call.request_id,
        name: call.descriptor.name,
        args: call.payload,
    }
}

struct RecordedExecution {
    result: ToolExecutionResult,
    events: Vec<StorageEvent>,
}

#[allow(clippy::too_many_arguments)]
async fn execute_safe_tool_calls(
    agent_loop: &AgentLoop,
    capabilities: &CapabilityRouter,
    safe_calls: Vec<PendingToolCall>,
    turn_id: &str,
    state: &AgentState,
    agent: &AgentEventContext,
    execution_owner: &ExecutionOwner,
    cancel: &CancelToken,
) -> Result<Vec<(usize, RecordedExecution)>> {
    let concurrency_limit = agent_loop
        .max_tool_concurrency()
        .min(safe_calls.len().max(1));
    let results = stream::iter(safe_calls)
        .map(|pending| async move {
            let ctx = agent_loop.tool_context(state, cancel.clone(), execution_owner.clone());
            let recorded = execute_raw_tool_call_recorded(
                agent_loop,
                capabilities,
                state,
                pending.tool_call,
                turn_id,
                agent,
                &ctx,
            )
            .await?;
            Ok((pending.index, recorded))
        })
        .buffer_unordered(concurrency_limit)
        .collect::<Vec<Result<(usize, RecordedExecution)>>>()
        .await;

    results.into_iter().collect()
}

async fn execute_raw_tool_call_recorded(
    agent_loop: &AgentLoop,
    capabilities: &CapabilityRouter,
    state: &AgentState,
    tool_call: ToolCallRequest,
    turn_id: &str,
    agent: &AgentEventContext,
    ctx: &astrcode_core::ToolContext,
) -> Result<RecordedExecution> {
    let mut events = Vec::new();
    let result = execute_raw_tool_call(
        tool_call,
        RawToolExecutionContext {
            agent_loop,
            capabilities,
            state,
            turn_id,
            agent,
            tool_ctx: ctx,
            on_event: &mut |event| {
                events.push(event);
                Ok(())
            },
        },
    )
    .await?;

    Ok(RecordedExecution { result, events })
}

fn flush_buffered_events(
    outcomes: &mut [Option<CallOutcome>],
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    cancel: &CancelToken,
) -> Result<()> {
    for outcome in outcomes.iter_mut().flatten() {
        let Some(events) = outcome.buffered_events.take() else {
            continue;
        };

        for event in events {
            if let Err(error) = on_event(event) {
                cancel.cancel();
                return Err(error);
            }
        }
    }

    Ok(())
}

fn push_tool_messages(messages: &mut Vec<LlmMessage>, outcomes: Vec<Option<CallOutcome>>) {
    for outcome in outcomes.into_iter().flatten() {
        let tool_call_id = outcome.result.tool_call_id.clone();
        messages.push(LlmMessage::Tool {
            tool_call_id,
            content: outcome.result.model_content(),
        });
    }
}

struct RawToolExecutionContext<'a, F>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    agent_loop: &'a AgentLoop,
    capabilities: &'a CapabilityRouter,
    state: &'a AgentState,
    turn_id: &'a str,
    agent: &'a AgentEventContext,
    tool_ctx: &'a astrcode_core::ToolContext,
    on_event: &'a mut F,
}

async fn execute_raw_tool_call<F>(
    tool_call: ToolCallRequest,
    ctx: RawToolExecutionContext<'_, F>,
) -> Result<ToolExecutionResult>
where
    F: FnMut(StorageEvent) -> Result<()>,
{
    let RawToolExecutionContext {
        agent_loop,
        capabilities,
        state,
        turn_id,
        agent,
        tool_ctx,
        on_event,
    } = ctx;
    let tool_call = match agent_loop
        .hooks
        .run_pre_tool_use(agent_loop.tool_hook_context(state, turn_id, &tool_call))
        .await?
    {
        PreToolUseDecision::Continue(tool) => ToolCallRequest {
            id: tool.tool_call_id,
            name: tool.tool_name,
            args: tool.args,
        },
        PreToolUseDecision::Blocked { reason, tool } => {
            let blocked_call = ToolCallRequest {
                id: tool.tool_call_id.clone(),
                name: tool.tool_name.clone(),
                args: tool.args.clone(),
            };
            denied_tool_result(&blocked_call, turn_id, agent, &reason, on_event)?;
            return Ok(denial_result(&blocked_call, reason));
        },
    };

    on_event(StorageEvent::ToolCall {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        args: tool_call.args.clone(),
    })?;

    let start = Instant::now();
    let (tool_output_tx, mut tool_output_rx) = mpsc::unbounded_channel();
    let (tool_event_tx, mut tool_event_rx) = mpsc::unbounded_channel();
    let tool_call_for_execution = tool_call.clone();
    let tool_ctx_for_execution = tool_ctx
        .clone()
        .with_turn_id(turn_id.to_string())
        .with_tool_call_id(tool_call.id.clone())
        .with_agent_context(agent.clone())
        .with_tool_output_sender(tool_output_tx)
        .with_event_sink(Arc::new(ChannelToolEventSink { tx: tool_event_tx }));

    // Yield before local IO-heavy tools so other tasks can make progress between tool calls.
    tokio::task::yield_now().await;
    let mut execute_tool = Some(Box::pin(async move {
        capabilities
            .execute_tool(&tool_call_for_execution, &tool_ctx_for_execution)
            .await
    }));
    let mut execution_result = None;
    let mut output_stream_open = true;
    let mut event_stream_open = true;

    while execution_result.is_none() || output_stream_open || event_stream_open {
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
                                agent: agent.clone(),
                                tool_call_id: delta.tool_call_id,
                                tool_name: delta.tool_name,
                                stream: delta.stream,
                                delta: delta.delta,
                            }) {
                                tool_ctx.cancel().cancel();
                                return Err(error);
                            }
                        }
                        None => {
                            output_stream_open = false;
                        }
                    }
                }
                maybe_event = tool_event_rx.recv(), if event_stream_open => {
                    match maybe_event {
                        Some(event) => {
                            if let Err(error) = on_event(event) {
                                tool_ctx.cancel().cancel();
                                return Err(error);
                            }
                        }
                        None => {
                            event_stream_open = false;
                        }
                    }
                }
            }
            continue;
        }

        tokio::select! {
            maybe_delta = tool_output_rx.recv(), if output_stream_open => {
                match maybe_delta {
                    Some(delta) => {
                        if let Err(error) = on_event(StorageEvent::ToolCallDelta {
                            turn_id: Some(turn_id.to_string()),
                            agent: agent.clone(),
                            tool_call_id: delta.tool_call_id,
                            tool_name: delta.tool_name,
                            stream: delta.stream,
                            delta: delta.delta,
                        }) {
                            tool_ctx.cancel().cancel();
                            return Err(error);
                        }
                    },
                    None => {
                        output_stream_open = false;
                    },
                }
            }
            maybe_event = tool_event_rx.recv(), if event_stream_open => {
                match maybe_event {
                    Some(event) => {
                        if let Err(error) = on_event(event) {
                            tool_ctx.cancel().cancel();
                            return Err(error);
                        }
                    }
                    None => {
                        event_stream_open = false;
                    }
                }
            }
        }
    }

    let mut result = execution_result.expect("tool execution future should resolve");
    result.duration_ms = start.elapsed().as_millis() as u64;
    let hook_result = ToolHookResultContext {
        tool: agent_loop.tool_hook_context(state, turn_id, &tool_call),
        result: result.clone(),
    };
    if result.ok {
        agent_loop
            .hooks
            .run_post_tool_use_best_effort(hook_result)
            .await;
    } else {
        agent_loop
            .hooks
            .run_post_tool_failure_best_effort(hook_result)
            .await;
    }
    on_event(StorageEvent::ToolResult {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
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
    agent: &AgentEventContext,
    reason: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
) -> Result<()> {
    on_event(StorageEvent::ToolCall {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.args.clone(),
    })?;
    on_event(StorageEvent::ToolResult {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
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
