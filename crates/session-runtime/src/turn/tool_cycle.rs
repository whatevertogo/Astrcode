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

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use astrcode_core::{
    AgentEventContext, CancelToken, LlmMessage, Result, StorageEvent, ToolCallRequest, ToolContext,
    ToolEventSink, ToolExecutionResult, ToolOutputDelta, tool_result_persist::resolve_inline_limit,
};
use astrcode_kernel::KernelGateway;
use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    SessionState, SessionStateEventSink,
    turn::events::{tool_call_event, tool_result_event},
};

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
    /// 仅在 buffered 模式下返回，由 step 在 assistant 定稿后统一刷入 durable 事件流。
    pub events: Vec<StorageEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEventEmissionMode {
    Immediate,
    Buffered,
}

/// 工具执行周期的上下文参数，避免函数参数过多。
pub struct ToolCycleContext<'a> {
    pub gateway: &'a KernelGateway,
    pub session_state: &'a Arc<SessionState>,
    pub session_id: &'a str,
    pub working_dir: &'a str,
    pub turn_id: &'a str,
    pub agent: &'a AgentEventContext,
    pub cancel: &'a CancelToken,
    pub events: &'a mut Vec<StorageEvent>,
    pub max_concurrency: usize,
    pub tool_result_inline_limit: usize,
    pub event_emission_mode: ToolEventEmissionMode,
}

struct SingleToolInvocation<'a> {
    gateway: &'a KernelGateway,
    session_state: Arc<SessionState>,
    tool_call: &'a ToolCallRequest,
    session_id: &'a str,
    working_dir: &'a str,
    turn_id: &'a str,
    agent: &'a AgentEventContext,
    cancel: &'a CancelToken,
    tool_result_inline_limit: usize,
    event_emission_mode: ToolEventEmissionMode,
}

pub struct BufferedToolExecutionRequest {
    pub gateway: KernelGateway,
    pub session_state: Arc<SessionState>,
    pub tool_call: ToolCallRequest,
    pub session_id: String,
    pub working_dir: String,
    pub turn_id: String,
    pub agent: AgentEventContext,
    pub cancel: CancelToken,
    pub tool_result_inline_limit: usize,
}

pub struct BufferedToolExecution {
    pub tool_call: ToolCallRequest,
    pub result: ToolExecutionResult,
    pub events: Vec<StorageEvent>,
    pub started_at: Instant,
    pub finished_at: Instant,
}

struct BufferedToolEventSink {
    events: Arc<Mutex<Vec<StorageEvent>>>,
}

#[async_trait]
impl ToolEventSink for BufferedToolEventSink {
    async fn emit(&self, event: StorageEvent) -> Result<()> {
        self.events
            .lock()
            .expect("buffered tool event sink lock should work")
            .push(event);
        Ok(())
    }
}

struct ToolOutputForwarder {
    shutdown_tx: oneshot::Sender<()>,
    join_handle: JoinHandle<()>,
}

impl ToolOutputForwarder {
    fn spawn(
        session_state: Arc<SessionState>,
        turn_id: &str,
        agent: &AgentEventContext,
        mut tool_output_rx: mpsc::UnboundedReceiver<ToolOutputDelta>,
    ) -> Self {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let turn_id = turn_id.to_string();
        let agent = agent.clone();
        let join_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => {
                        while let Ok(delta) = tool_output_rx.try_recv() {
                            broadcast_tool_output_delta(&session_state, &turn_id, &agent, delta);
                        }
                        break;
                    }
                    maybe_delta = tool_output_rx.recv() => {
                        let Some(delta) = maybe_delta else {
                            break;
                        };
                        broadcast_tool_output_delta(&session_state, &turn_id, &agent, delta);
                    }
                }
            }
        });
        Self {
            shutdown_tx,
            join_handle,
        }
    }

    async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        if let Err(error) = self.join_handle.await {
            log::warn!("tool output forwarder join failed: {error}");
        }
    }
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

    // 收集所有 fallback 事件到局部缓冲，最后合并到 ctx.events。
    // 为什么仍保留这层缓冲：
    // 1. 并发工具执行期间不能直接借用共享的 ctx.events。
    // 2. 若即时 durable 发射失败，turn 结束阶段还能兜底落盘，避免结构事件丢失。
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
        let (result, local_events) = invoke_single_tool(SingleToolInvocation {
            gateway: ctx.gateway,
            session_state: Arc::clone(ctx.session_state),
            tool_call: &call,
            session_id: ctx.session_id,
            working_dir: ctx.working_dir,
            turn_id: ctx.turn_id,
            agent: ctx.agent,
            cancel: ctx.cancel,
            tool_result_inline_limit: ctx.tool_result_inline_limit,
            event_emission_mode: ctx.event_emission_mode,
        })
        .await;
        collected_events.extend(local_events);
        raw_results.push((call, result));
    }

    let events = if matches!(ctx.event_emission_mode, ToolEventEmissionMode::Buffered) {
        collected_events
    } else {
        ctx.events.extend(collected_events);
        Vec::new()
    };

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
        events,
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
            let tool_result_inline_limit = ctx.tool_result_inline_limit;
            let session_state = Arc::clone(ctx.session_state);

            async move {
                let (result, events) = invoke_single_tool(SingleToolInvocation {
                    gateway: &gateway,
                    session_state,
                    tool_call: &call,
                    session_id: &session_id,
                    working_dir: &working_dir,
                    turn_id: &turn_id,
                    agent: &agent,
                    cancel: &cancel,
                    tool_result_inline_limit,
                    event_emission_mode: ctx.event_emission_mode,
                })
                .await;
                (call, result, events)
            }
        })
        .buffer_unordered(concurrency_limit)
        .collect()
        .await;

    Ok(results)
}

pub async fn execute_buffered_tool_call(
    request: BufferedToolExecutionRequest,
) -> BufferedToolExecution {
    let BufferedToolExecutionRequest {
        gateway,
        session_state,
        tool_call,
        session_id,
        working_dir,
        turn_id,
        agent,
        cancel,
        tool_result_inline_limit,
    } = request;
    let started_at = Instant::now();
    let (result, events) = invoke_single_tool(SingleToolInvocation {
        gateway: &gateway,
        session_state,
        tool_call: &tool_call,
        session_id: &session_id,
        working_dir: &working_dir,
        turn_id: &turn_id,
        agent: &agent,
        cancel: &cancel,
        tool_result_inline_limit,
        event_emission_mode: ToolEventEmissionMode::Buffered,
    })
    .await;
    let finished_at = Instant::now();
    BufferedToolExecution {
        tool_call,
        result,
        events,
        started_at,
        finished_at,
    }
}

/// 底层工具调用：通过 kernel gateway 执行，记录开始/结束事件。
///
/// 返回 `(ToolExecutionResult, Vec<StorageEvent>)`，
/// 返回值中的事件仅用于“即时 durable 发射失败”时的兜底补写。
async fn invoke_single_tool(
    invocation: SingleToolInvocation<'_>,
) -> (ToolExecutionResult, Vec<StorageEvent>) {
    let SingleToolInvocation {
        gateway,
        session_state,
        tool_call,
        session_id,
        working_dir,
        turn_id,
        agent,
        cancel,
        tool_result_inline_limit,
        event_emission_mode,
    } = invocation;
    let buffered_events = Arc::new(Mutex::new(Vec::new()));
    let mut fallback_events = Vec::new();
    let start = Instant::now();
    let event_sink = match event_emission_mode {
        ToolEventEmissionMode::Immediate => SessionStateEventSink::new(Arc::clone(&session_state))
            .map(|sink| Arc::new(sink) as Arc<dyn ToolEventSink>)
            .ok(),
        ToolEventEmissionMode::Buffered => Some(Arc::new(BufferedToolEventSink {
            events: Arc::clone(&buffered_events),
        }) as Arc<dyn ToolEventSink>),
    };
    let (tool_output_tx, tool_output_rx) = mpsc::unbounded_channel::<ToolOutputDelta>();
    let tool_output_forwarder =
        ToolOutputForwarder::spawn(Arc::clone(&session_state), turn_id, agent, tool_output_rx);

    // 发出 ToolCall 开始事件
    let tool_call_event = tool_call_event(turn_id, agent, tool_call);
    emit_or_buffer_tool_event(
        &event_sink,
        &mut fallback_events,
        tool_call_event,
        "tool start",
    )
    .await;

    // 构建工具上下文
    let tool_ctx = ToolContext::new(
        session_id.to_string().into(),
        working_dir.to_string().into(),
        cancel.clone(),
    )
    .with_turn_id(turn_id.to_string())
    .with_tool_call_id(tool_call.id.clone())
    .with_agent_context(agent.clone())
    .with_resolved_inline_limit(resolve_inline_limit(
        &tool_call.name,
        gateway
            .capabilities()
            .capability_spec(&tool_call.name)
            .and_then(|spec| spec.max_result_inline_size),
        tool_result_inline_limit,
    ))
    .with_tool_output_sender(tool_output_tx.clone());
    let tool_ctx = if let Some(sink) = &event_sink {
        tool_ctx.with_event_sink(Arc::clone(sink))
    } else {
        tool_ctx
    };

    // 通过 kernel gateway 执行工具
    let result = gateway.invoke_tool(tool_call, &tool_ctx).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    // 先释放当前调用持有的上下文，再显式通知 forwarder 排空并退出。
    // 不能把“所有 sender 都 drop”当成工具结束条件，因为 sender 会被多层上下文 clone。
    drop(tool_ctx);
    drop(tool_output_tx);
    tool_output_forwarder.shutdown().await;

    // 发出 ToolResult 结束事件
    let tool_result_event = tool_result_event(
        turn_id,
        agent,
        &ToolExecutionResult {
            duration_ms,
            ..result.clone()
        },
    );
    emit_or_buffer_tool_event(
        &event_sink,
        &mut fallback_events,
        tool_result_event,
        "tool result",
    )
    .await;

    let mut events = buffered_events
        .lock()
        .expect("buffered tool events lock should work")
        .clone();
    events.extend(fallback_events);
    (result, events)
}

fn broadcast_tool_output_delta(
    session_state: &SessionState,
    turn_id: &str,
    agent: &AgentEventContext,
    delta: ToolOutputDelta,
) {
    session_state.broadcast_live_event(astrcode_core::AgentEvent::ToolCallDelta {
        turn_id: turn_id.to_string(),
        agent: agent.clone(),
        tool_call_id: delta.tool_call_id,
        tool_name: delta.tool_name,
        stream: delta.stream,
        delta: delta.delta,
    });
}

async fn emit_or_buffer_tool_event(
    event_sink: &Option<Arc<dyn astrcode_core::ToolEventSink>>,
    events: &mut Vec<StorageEvent>,
    event: StorageEvent,
    label: &str,
) {
    if let Some(sink) = event_sink {
        if let Err(error) = sink.emit(event.clone()).await {
            log::warn!("failed to emit {label} immediately, buffering fallback event: {error}");
            events.push(event);
        }
    } else {
        events.push(event);
    }
}

fn interrupted_result() -> ToolCycleResult {
    ToolCycleResult {
        outcome: ToolCycleOutcome::Interrupted,
        tool_messages: Vec::new(),
        raw_results: Vec::new(),
        events: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{
        CapabilityKind, StorageEventPayload, Tool, ToolDefinition, ToolOutputStream,
    };
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tokio::time::{Duration, timeout};

    use super::*;
    use crate::turn::test_support::{test_kernel_with_tool, test_session_state};

    #[test]
    fn tool_cycle_outcome_equality() {
        assert_eq!(ToolCycleOutcome::Completed, ToolCycleOutcome::Completed);
        assert_eq!(ToolCycleOutcome::Interrupted, ToolCycleOutcome::Interrupted);
        assert_ne!(ToolCycleOutcome::Completed, ToolCycleOutcome::Interrupted);
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ObservedToolContext {
        turn_id: Option<String>,
        agent_id: Option<String>,
        agent_profile: Option<String>,
    }

    #[derive(Debug)]
    struct ContextProbeTool {
        observed: Arc<Mutex<Vec<ObservedToolContext>>>,
    }

    #[async_trait]
    impl Tool for ContextProbeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "context_probe".to_string(),
                description: "records tool context".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(
            &self,
        ) -> std::result::Result<
            astrcode_core::CapabilitySpec,
            astrcode_core::CapabilitySpecBuildError,
        > {
            astrcode_core::CapabilitySpec::builder("context_probe", CapabilityKind::Tool)
                .description("records tool context")
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            self.observed
                .lock()
                .expect("observed lock should work")
                .push(ObservedToolContext {
                    turn_id: ctx.turn_id().map(ToString::to_string),
                    agent_id: ctx.agent_context().agent_id.clone(),
                    agent_profile: ctx.agent_context().agent_profile.clone(),
                });
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "context_probe".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[derive(Debug)]
    struct StreamingProbeTool;

    #[async_trait]
    impl Tool for StreamingProbeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "streaming_probe".to_string(),
                description: "emits durable and live probe events".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(
            &self,
        ) -> std::result::Result<
            astrcode_core::CapabilitySpec,
            astrcode_core::CapabilitySpecBuildError,
        > {
            astrcode_core::CapabilitySpec::builder("streaming_probe", CapabilityKind::Tool)
                .description("emits durable and live probe events")
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            let turn_id = ctx
                .turn_id()
                .expect("streaming probe should receive turn id")
                .to_string();
            let sink = ctx
                .event_sink()
                .expect("streaming probe should receive tool event sink");
            sink.emit(crate::turn::events::tool_call_delta_event(
                &turn_id,
                ctx.agent_context(),
                tool_call_id.clone(),
                "streaming_probe".to_string(),
                ToolOutputStream::Stdout,
                "durable-delta".to_string(),
            ))
            .await?;
            assert!(
                ctx.emit_stdout(tool_call_id.clone(), "streaming_probe", "live-stdout"),
                "streaming probe should be able to emit live stdout"
            );
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "streaming_probe".to_string(),
                ok: true,
                output: "done".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[tokio::test]
    async fn invoke_single_tool_preserves_turn_and_agent_context() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let kernel = test_kernel_with_tool(
            Arc::new(ContextProbeTool {
                observed: Arc::clone(&observed),
            }),
            8192,
        );
        let tool_call = ToolCallRequest {
            id: "call-1".to_string(),
            name: "context_probe".to_string(),
            args: json!({}),
        };
        let agent = AgentEventContext::root_execution("root-agent:session-1", "default");
        let session_state = test_session_state();

        let cancel = CancelToken::new();
        let (result, _) = invoke_single_tool(SingleToolInvocation {
            gateway: kernel.gateway(),
            session_state,
            tool_call: &tool_call,
            session_id: "session-1",
            working_dir: ".",
            turn_id: "turn-1",
            agent: &agent,
            cancel: &cancel,
            tool_result_inline_limit: 32 * 1024,
            event_emission_mode: ToolEventEmissionMode::Immediate,
        })
        .await;

        assert!(result.ok, "tool invocation should succeed: {result:?}");
        let observed = observed.lock().expect("observed lock should work");
        assert_eq!(
            observed.as_slice(),
            &[ObservedToolContext {
                turn_id: Some("turn-1".to_string()),
                agent_id: Some("root-agent:session-1".to_string()),
                agent_profile: Some("default".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn invoke_single_tool_emits_structured_and_live_events_immediately() {
        let kernel = test_kernel_with_tool(Arc::new(StreamingProbeTool), 8192);
        let tool_call = ToolCallRequest {
            id: "call-live".to_string(),
            name: "streaming_probe".to_string(),
            args: json!({}),
        };
        let agent = AgentEventContext::root_execution("root-agent:session-1", "default");
        let session_state = test_session_state();
        let mut live_receiver = session_state.subscribe_live();

        let cancel = CancelToken::new();
        let (result, fallback_events) = invoke_single_tool(SingleToolInvocation {
            gateway: kernel.gateway(),
            session_state: Arc::clone(&session_state),
            tool_call: &tool_call,
            session_id: "session-1",
            working_dir: ".",
            turn_id: "turn-live",
            agent: &agent,
            cancel: &cancel,
            tool_result_inline_limit: 32 * 1024,
            event_emission_mode: ToolEventEmissionMode::Immediate,
        })
        .await;

        assert!(result.ok, "tool invocation should succeed: {result:?}");
        assert!(
            fallback_events.is_empty(),
            "immediate event emission should avoid fallback buffering: {fallback_events:?}"
        );

        let stored = session_state
            .snapshot_recent_stored_events()
            .expect("snapshot recent stored events should work");
        assert!(
            stored.iter().any(|event| matches!(
                &event.event.payload,
                StorageEventPayload::ToolCall {
                    tool_call_id,
                    tool_name,
                    ..
                } if tool_call_id == "call-live" && tool_name == "streaming_probe"
            )),
            "tool start should be durably recorded immediately"
        );
        assert!(
            stored.iter().any(|event| matches!(
                &event.event.payload,
                StorageEventPayload::ToolCallDelta {
                    tool_call_id,
                    tool_name,
                    delta,
                    ..
                } if tool_call_id == "call-live"
                    && tool_name == "streaming_probe"
                    && delta == "durable-delta"
            )),
            "tool internal durable delta should flow through event sink"
        );
        assert!(
            stored.iter().any(|event| matches!(
                &event.event.payload,
                StorageEventPayload::ToolResult {
                    tool_call_id,
                    tool_name,
                    output,
                    ..
                } if tool_call_id == "call-live"
                    && tool_name == "streaming_probe"
                    && output == "done"
            )),
            "tool result should be durably recorded immediately"
        );

        let live_event = timeout(Duration::from_secs(1), live_receiver.recv())
            .await
            .expect("live receiver should get stdout delta in time")
            .expect("live receiver should stay open");
        assert!(
            matches!(
                live_event,
                astrcode_core::AgentEvent::ToolCallDelta {
                    turn_id,
                    tool_call_id,
                    tool_name,
                    stream: ToolOutputStream::Stdout,
                    delta,
                    ..
                } if turn_id == "turn-live"
                    && tool_call_id == "call-live"
                    && tool_name == "streaming_probe"
                    && delta == "live-stdout"
            ),
            "stdout delta should go through the live channel immediately"
        );
    }

    #[tokio::test]
    async fn invoke_single_tool_buffers_structured_events_when_requested() {
        let kernel = test_kernel_with_tool(Arc::new(StreamingProbeTool), 8192);
        let tool_call = ToolCallRequest {
            id: "call-buffered".to_string(),
            name: "streaming_probe".to_string(),
            args: json!({}),
        };
        let agent = AgentEventContext::root_execution("root-agent:session-1", "default");
        let session_state = test_session_state();
        let mut live_receiver = session_state.subscribe_live();

        let cancel = CancelToken::new();
        let (result, buffered_events) = invoke_single_tool(SingleToolInvocation {
            gateway: kernel.gateway(),
            session_state: Arc::clone(&session_state),
            tool_call: &tool_call,
            session_id: "session-1",
            working_dir: ".",
            turn_id: "turn-buffered",
            agent: &agent,
            cancel: &cancel,
            tool_result_inline_limit: 32 * 1024,
            event_emission_mode: ToolEventEmissionMode::Buffered,
        })
        .await;

        assert!(result.ok, "tool invocation should succeed: {result:?}");
        assert!(
            buffered_events.iter().any(|event| matches!(
                &event.payload,
                StorageEventPayload::ToolCall {
                    tool_call_id,
                    tool_name,
                    ..
                } if tool_call_id == "call-buffered" && tool_name == "streaming_probe"
            )),
            "buffered mode should keep tool start in local event buffer"
        );
        assert!(
            buffered_events.iter().any(|event| matches!(
                &event.payload,
                StorageEventPayload::ToolCallDelta {
                    tool_call_id,
                    tool_name,
                    delta,
                    ..
                } if tool_call_id == "call-buffered"
                    && tool_name == "streaming_probe"
                    && delta == "durable-delta"
            )),
            "buffered mode should preserve tool-emitted durable deltas"
        );
        assert!(
            session_state
                .snapshot_recent_stored_events()
                .expect("snapshot recent stored events should work")
                .is_empty(),
            "buffered mode should not immediately append durable events"
        );

        let live_event = timeout(Duration::from_secs(1), live_receiver.recv())
            .await
            .expect("live receiver should get stdout delta in time")
            .expect("live receiver should stay open");
        assert!(
            matches!(
                live_event,
                astrcode_core::AgentEvent::ToolCallDelta {
                    turn_id,
                    tool_call_id,
                    tool_name,
                    stream: ToolOutputStream::Stdout,
                    delta,
                    ..
                } if turn_id == "turn-buffered"
                    && tool_call_id == "call-buffered"
                    && tool_name == "streaming_probe"
                    && delta == "live-stdout"
            ),
            "buffered mode should keep live stdout forwarding"
        );
    }
}
