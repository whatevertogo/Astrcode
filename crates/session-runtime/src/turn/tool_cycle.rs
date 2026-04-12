//! # 工具执行周期
//!
//! 负责执行 LLM 请求中的工具调用，包括：
//! - 并发执行（只读工具可并行，写操作串行）
//! - 结果收集和事件广播
//! - 取消信号传播
//!
//! ## 并发模型
//!
//! 只读工具（`concurrency_safe`）使用 `buffer_unordered` 并发执行。
//! 有副作用的工具按顺序执行，避免并发写冲突。
//! 并发上限通过 `max_concurrency` 参数控制。
//!
//! ## 架构约束
//!
//! 所有工具调用通过 `KernelGateway` 进行，session-runtime 不直接持有 provider。
//! 策略检查（policy/approval）由 kernel gateway 在 `invoke_tool` 内部处理。

use std::time::Instant;

use astrcode_core::{
    AgentEventContext, CancelToken, LlmMessage, Result, StorageEvent, StorageEventPayload,
    ToolCallRequest, ToolContext, ToolExecutionResult,
};
use astrcode_kernel::KernelGateway;
use futures_util::stream::{self, StreamExt};

/// 工具执行周期的最终结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCycleOutcome {
    /// 所有工具调用均已完成。
    Completed,
    /// 工具执行被取消。
    Interrupted,
}

/// 工具执行周期的完整结果。
pub struct ToolCycleResult {
    pub outcome: ToolCycleOutcome,
    /// 工具结果消息，需要追加到对话历史。
    pub tool_messages: Vec<LlmMessage>,
    /// 原始调用和结果，供外部追踪（文件访问、微压缩等）。
    pub raw_results: Vec<(ToolCallRequest, ToolExecutionResult)>,
}

/// 工具执行周期的上下文参数，避免函数参数过多。
pub struct ToolCycleContext<'a> {
    pub gateway: &'a KernelGateway,
    pub session_id: &'a str,
    pub working_dir: &'a str,
    pub turn_id: &'a str,
    pub agent: &'a AgentEventContext,
    pub cancel: &'a CancelToken,
    pub events: &'a mut Vec<StorageEvent>,
    pub max_concurrency: usize,
}

/// 执行一组工具调用，支持并发安全工具并行。
///
/// 工具分为两类：
/// - **安全调用**（`concurrency_safe = true`）：并发执行，受 `max_concurrency` 限制
/// - **不安全调用**（有副作用）：按顺序执行
pub async fn execute_tool_calls(
    ctx: &mut ToolCycleContext<'_>,
    tool_calls: Vec<ToolCallRequest>,
) -> Result<ToolCycleResult> {
    let capabilities = ctx.gateway.capabilities();

    let mut safe_calls = Vec::new();
    let mut unsafe_calls = Vec::new();

    for call in tool_calls {
        if ctx.cancel.is_cancelled() {
            return Ok(interrupted_result());
        }

        let is_safe = capabilities
            .capability_spec(&call.name)
            .is_some_and(|spec| spec.concurrency_safe);

        if is_safe {
            safe_calls.push(call);
        } else {
            unsafe_calls.push(call);
        }
    }

    // 收集所有事件到局部缓冲，最后合并到 ctx.events，
    // 避免并发执行期间对 ctx.events 的借用冲突。
    let mut collected_events: Vec<StorageEvent> = Vec::new();
    let mut raw_results: Vec<(ToolCallRequest, ToolExecutionResult)> = Vec::new();

    // 并发执行安全工具
    if !safe_calls.is_empty() {
        if ctx.cancel.is_cancelled() {
            return Ok(interrupted_result());
        }
        let results = execute_concurrent_safe(ctx, safe_calls).await?;
        for (call, result, local_events) in results {
            collected_events.extend(local_events);
            raw_results.push((call, result));
        }
    }

    // 串行执行不安全工具
    for call in unsafe_calls {
        if ctx.cancel.is_cancelled() {
            // 已执行的工具事件仍需保留
            ctx.events.extend(collected_events);
            return Ok(interrupted_result());
        }
        let (result, local_events) = invoke_single_tool(
            ctx.gateway,
            &call,
            ctx.session_id,
            ctx.working_dir,
            ctx.turn_id,
            ctx.agent,
            ctx.cancel,
        )
        .await;
        collected_events.extend(local_events);
        raw_results.push((call, result));
    }

    ctx.events.extend(collected_events);

    // 构建工具结果消息
    let tool_messages: Vec<LlmMessage> = raw_results
        .iter()
        .map(|(_, result)| LlmMessage::Tool {
            tool_call_id: result.tool_call_id.clone(),
            content: result.model_content(),
        })
        .collect();

    Ok(ToolCycleResult {
        outcome: ToolCycleOutcome::Completed,
        tool_messages,
        raw_results,
    })
}

/// 并发执行多个只读工具调用。
///
/// 每个并发 future 有自己的局部事件 Vec，完成后统一合并。
async fn execute_concurrent_safe(
    ctx: &ToolCycleContext<'_>,
    safe_calls: Vec<ToolCallRequest>,
) -> Result<Vec<(ToolCallRequest, ToolExecutionResult, Vec<StorageEvent>)>> {
    let concurrency_limit = ctx.max_concurrency.min(safe_calls.len().max(1));

    let results = stream::iter(safe_calls)
        .map(|call| {
            let gateway = ctx.gateway.clone();
            let session_id = ctx.session_id.to_string();
            let working_dir = ctx.working_dir.to_string();
            let turn_id = ctx.turn_id.to_string();
            let agent = ctx.agent.clone();
            let cancel = ctx.cancel.clone();

            async move {
                let (result, events) = invoke_single_tool(
                    &gateway,
                    &call,
                    &session_id,
                    &working_dir,
                    &turn_id,
                    &agent,
                    &cancel,
                )
                .await;
                (call, result, events)
            }
        })
        .buffer_unordered(concurrency_limit)
        .collect()
        .await;

    Ok(results)
}

/// 底层工具调用：通过 kernel gateway 执行，记录开始/结束事件。
///
/// 返回 `(ToolExecutionResult, Vec<StorageEvent>)`，
/// 事件包含 ToolCall 开始和 ToolResult 结束，由调用方合并到主事件流。
#[allow(clippy::too_many_arguments)]
async fn invoke_single_tool(
    gateway: &KernelGateway,
    tool_call: &ToolCallRequest,
    session_id: &str,
    working_dir: &str,
    turn_id: &str,
    agent: &AgentEventContext,
    cancel: &CancelToken,
) -> (ToolExecutionResult, Vec<StorageEvent>) {
    let mut events = Vec::new();
    let start = Instant::now();

    // 发出 ToolCall 开始事件
    events.push(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolCall {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
        },
    });

    // 构建工具上下文
    let tool_ctx = ToolContext::new(
        session_id.to_string().into(),
        working_dir.to_string().into(),
        cancel.clone(),
    );

    // 通过 kernel gateway 执行工具
    let result = gateway.invoke_tool(tool_call, &tool_ctx).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    // 发出 ToolResult 结束事件
    events.push(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolResult {
            tool_call_id: result.tool_call_id.clone(),
            tool_name: result.tool_name.clone(),
            output: result.output.clone(),
            success: result.ok,
            error: result.error.clone(),
            metadata: result.metadata.clone(),
            duration_ms,
        },
    });

    (result, events)
}

fn interrupted_result() -> ToolCycleResult {
    ToolCycleResult {
        outcome: ToolCycleOutcome::Interrupted,
        tool_messages: Vec::new(),
        raw_results: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_cycle_outcome_equality() {
        assert_eq!(ToolCycleOutcome::Completed, ToolCycleOutcome::Completed);
        assert_eq!(ToolCycleOutcome::Interrupted, ToolCycleOutcome::Interrupted);
        assert_ne!(ToolCycleOutcome::Completed, ToolCycleOutcome::Interrupted);
    }
}
