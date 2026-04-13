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

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, LlmMessage, Result, StorageEvent, StorageEventPayload,
    ToolCallRequest, ToolContext, ToolExecutionResult, ToolOutputDelta,
};
use astrcode_kernel::KernelGateway;
use futures_util::stream::{self, StreamExt};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{SessionState, SessionStateEventSink};

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
    pub session_state: &'a Arc<SessionState>,
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
        let (result, local_events) = invoke_single_tool(
            ctx.gateway,
            Arc::clone(ctx.session_state),
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
            let session_state = Arc::clone(ctx.session_state);

            async move {
                let (result, events) = invoke_single_tool(
                    &gateway,
                    session_state,
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
/// 返回值中的事件仅用于“即时 durable 发射失败”时的兜底补写。
#[allow(clippy::too_many_arguments)]
async fn invoke_single_tool(
    gateway: &KernelGateway,
    session_state: Arc<SessionState>,
    tool_call: &ToolCallRequest,
    session_id: &str,
    working_dir: &str,
    turn_id: &str,
    agent: &AgentEventContext,
    cancel: &CancelToken,
) -> (ToolExecutionResult, Vec<StorageEvent>) {
    let mut events = Vec::new();
    let start = Instant::now();
    let event_sink = SessionStateEventSink::new(Arc::clone(&session_state))
        .map(|sink| Arc::new(sink) as Arc<dyn astrcode_core::ToolEventSink>)
        .ok();
    let (tool_output_tx, mut tool_output_rx) = mpsc::unbounded_channel::<ToolOutputDelta>();
    let (forwarder_shutdown_tx, mut forwarder_shutdown_rx) = oneshot::channel::<()>();
    let session_state_for_forwarder = Arc::clone(&session_state);
    let agent_for_forwarder = agent.clone();
    let turn_id_for_forwarder = turn_id.to_string();
    let tool_output_forwarder: JoinHandle<()> = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut forwarder_shutdown_rx => {
                    while let Ok(delta) = tool_output_rx.try_recv() {
                        session_state_for_forwarder.broadcast_live_event(
                            astrcode_core::AgentEvent::ToolCallDelta {
                                turn_id: turn_id_for_forwarder.clone(),
                                agent: agent_for_forwarder.clone(),
                                tool_call_id: delta.tool_call_id,
                                tool_name: delta.tool_name,
                                stream: delta.stream,
                                delta: delta.delta,
                            },
                        );
                    }
                    break;
                }
                maybe_delta = tool_output_rx.recv() => {
                    let Some(delta) = maybe_delta else {
                        break;
                    };
                    session_state_for_forwarder.broadcast_live_event(
                        astrcode_core::AgentEvent::ToolCallDelta {
                            turn_id: turn_id_for_forwarder.clone(),
                            agent: agent_for_forwarder.clone(),
                            tool_call_id: delta.tool_call_id,
                            tool_name: delta.tool_name,
                            stream: delta.stream,
                            delta: delta.delta,
                        },
                    );
                }
            }
        }
    });

    // 发出 ToolCall 开始事件
    let tool_call_event = StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::ToolCall {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
        },
    };
    emit_or_buffer_tool_event(&event_sink, &mut events, tool_call_event, "tool start");

    // 构建工具上下文
    let tool_ctx = ToolContext::new(
        session_id.to_string().into(),
        working_dir.to_string().into(),
        cancel.clone(),
    )
    .with_turn_id(turn_id.to_string())
    .with_tool_call_id(tool_call.id.clone())
    .with_agent_context(agent.clone())
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
    let _ = forwarder_shutdown_tx.send(());
    if let Err(error) = tool_output_forwarder.await {
        log::warn!("tool output forwarder join failed: {error}");
    }

    // 发出 ToolResult 结束事件
    let tool_result_event = StorageEvent {
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
    };
    emit_or_buffer_tool_event(&event_sink, &mut events, tool_result_event, "tool result");

    (result, events)
}

fn emit_or_buffer_tool_event(
    event_sink: &Option<Arc<dyn astrcode_core::ToolEventSink>>,
    events: &mut Vec<StorageEvent>,
    event: StorageEvent,
    label: &str,
) {
    if let Some(sink) = event_sink {
        if let Err(error) = sink.emit(event.clone()) {
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
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{
        AgentStateProjector, CapabilityKind, EventLogWriter, LlmProvider, LlmRequest, ModelLimits,
        Phase, PromptBuildOutput, PromptBuildRequest, PromptProvider, ResourceProvider,
        ResourceReadResult, ResourceRequestContext, StorageEventPayload, StoreResult, Tool,
        ToolDefinition, ToolOutputStream,
    };
    use astrcode_kernel::{Kernel, ToolCapabilityInvoker};
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tokio::time::{Duration, timeout};

    use super::*;
    use crate::SessionWriter;

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
            sink.emit(StorageEvent {
                turn_id: Some(turn_id),
                agent: ctx.agent_context().clone(),
                payload: StorageEventPayload::ToolCallDelta {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: "streaming_probe".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "durable-delta".to_string(),
                },
            })?;
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

    #[derive(Debug)]
    struct NoopLlmProvider;

    #[async_trait]
    impl LlmProvider for NoopLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> Result<astrcode_core::LlmOutput> {
            Err(astrcode_core::AstrError::Validation(
                "noop llm provider should not execute in this test".to_string(),
            ))
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 8192,
                max_output_tokens: 4096,
            }
        }
    }

    #[derive(Debug)]
    struct NoopPromptProvider;

    #[async_trait]
    impl PromptProvider for NoopPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                metadata: Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct NoopResourceProvider;

    #[async_trait]
    impl ResourceProvider for NoopResourceProvider {
        async fn read_resource(
            &self,
            _uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: "noop://resource".to_string(),
                content: Value::Null,
                metadata: Value::Null,
            })
        }
    }

    fn test_kernel_with_tool(tool: Arc<dyn Tool>) -> Kernel {
        let router = astrcode_kernel::CapabilityRouter::builder()
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(tool).expect("tool invoker should build"),
            ))
            .build()
            .expect("router should build");
        Kernel::builder()
            .with_capabilities(router)
            .with_llm_provider(Arc::new(NoopLlmProvider))
            .with_prompt_provider(Arc::new(NoopPromptProvider))
            .with_resource_provider(Arc::new(NoopResourceProvider))
            .build()
            .expect("kernel should build")
    }

    #[derive(Debug, Default)]
    struct NoopEventLogWriter {
        next_seq: u64,
    }

    impl EventLogWriter for NoopEventLogWriter {
        fn append(&mut self, event: &StorageEvent) -> StoreResult<astrcode_core::StoredEvent> {
            self.next_seq += 1;
            Ok(astrcode_core::StoredEvent {
                storage_seq: self.next_seq,
                event: event.clone(),
            })
        }
    }

    fn test_session_state() -> Arc<SessionState> {
        Arc::new(SessionState::new(
            Phase::Idle,
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter::default()))),
            AgentStateProjector::default(),
            Vec::new(),
            Vec::new(),
        ))
    }

    #[tokio::test]
    async fn invoke_single_tool_preserves_turn_and_agent_context() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let kernel = test_kernel_with_tool(Arc::new(ContextProbeTool {
            observed: Arc::clone(&observed),
        }));
        let tool_call = ToolCallRequest {
            id: "call-1".to_string(),
            name: "context_probe".to_string(),
            args: json!({}),
        };
        let agent = AgentEventContext::root_execution("root-agent:session-1", "default");
        let session_state = test_session_state();

        let (result, _) = invoke_single_tool(
            kernel.gateway(),
            session_state,
            &tool_call,
            "session-1",
            ".",
            "turn-1",
            &agent,
            &CancelToken::new(),
        )
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
        let kernel = test_kernel_with_tool(Arc::new(StreamingProbeTool));
        let tool_call = ToolCallRequest {
            id: "call-live".to_string(),
            name: "streaming_probe".to_string(),
            args: json!({}),
        };
        let agent = AgentEventContext::root_execution("root-agent:session-1", "default");
        let session_state = test_session_state();
        let mut live_receiver = session_state.subscribe_live();

        let (result, fallback_events) = invoke_single_tool(
            kernel.gateway(),
            Arc::clone(&session_state),
            &tool_call,
            "session-1",
            ".",
            "turn-live",
            &agent,
            &CancelToken::new(),
        )
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
}
