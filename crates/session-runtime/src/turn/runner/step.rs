use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

use astrcode_core::{
    LlmFinishReason, LlmMessage, LlmOutput, LlmRequest, Result, ToolCallRequest, UserMessageOrigin,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::{TurnExecutionContext, TurnExecutionResources};
use crate::turn::{
    compaction_cycle::{self, ReactiveCompactContext},
    continuation_cycle::{
        OUTPUT_CONTINUATION_PROMPT, OutputContinuationDecision, continuation_transition,
        decide_output_continuation,
    },
    events::{assistant_final_event, turn_done_event, user_message_event},
    llm_cycle::{StreamedToolCallDelta, ToolCallDeltaSink},
    loop_control::{
        AUTO_CONTINUE_NUDGE, BudgetContinuationDecision, TurnLoopTransition, TurnStopCause,
        decide_budget_continuation,
    },
    request::{AssemblePromptRequest, AssemblePromptResult, assemble_prompt_request},
    tool_cycle::{
        self, BufferedToolExecution, BufferedToolExecutionRequest, ToolCycleContext,
        ToolCycleOutcome, ToolCycleResult, ToolEventEmissionMode,
    },
};

struct RuntimeStepDriver;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StreamingToolStats {
    launched_count: usize,
    matched_count: usize,
    fallback_count: usize,
    discard_count: usize,
    overlap_ms: u64,
}

#[derive(Debug, Default)]
struct StreamingToolAssembly {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    launched: bool,
}

struct SpawnedStreamingTool {
    request: ToolCallRequest,
    handle: JoinHandle<BufferedToolExecution>,
}

#[derive(Default)]
struct StreamingToolPlanner {
    gateway: Option<astrcode_kernel::KernelGateway>,
    session_state: Option<Arc<crate::SessionState>>,
    session_id: String,
    working_dir: String,
    turn_id: String,
    agent: Option<astrcode_core::AgentEventContext>,
    cancel: Option<astrcode_core::CancelToken>,
    tool_result_inline_limit: usize,
    assemblies: BTreeMap<usize, StreamingToolAssembly>,
    spawned: HashMap<String, SpawnedStreamingTool>,
    stats: StreamingToolStats,
}

struct StreamingToolFinalizeResult {
    matched_results: HashMap<String, BufferedToolExecution>,
    remaining_tool_calls: Vec<ToolCallRequest>,
    stats: StreamingToolStats,
    used_streaming_path: bool,
}

#[derive(Clone)]
struct StreamingToolPlannerHandle {
    inner: Arc<Mutex<StreamingToolPlanner>>,
}

impl StreamingToolPlannerHandle {
    fn new(resources: &TurnExecutionResources<'_>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamingToolPlanner {
                gateway: Some(resources.gateway.clone()),
                session_state: Some(Arc::clone(resources.session_state)),
                session_id: resources.session_id.to_string(),
                working_dir: resources.working_dir.to_string(),
                turn_id: resources.turn_id.to_string(),
                agent: Some(resources.agent.clone()),
                cancel: Some(resources.cancel.clone()),
                tool_result_inline_limit: resources.runtime.tool_result_inline_limit,
                ..StreamingToolPlanner::default()
            })),
        }
    }

    fn tool_delta_sink(&self) -> ToolCallDeltaSink {
        let inner = Arc::clone(&self.inner);
        Arc::new(move |delta| {
            inner
                .lock()
                .expect("streaming tool planner lock should work")
                .observe_delta(delta);
        })
    }

    fn abort_all(&self) {
        let spawned = {
            let mut planner = self
                .inner
                .lock()
                .expect("streaming tool planner lock should work");
            let discarded = planner.spawned.len();
            planner.stats.discard_count = planner.stats.discard_count.saturating_add(discarded);
            std::mem::take(&mut planner.spawned)
        };
        for (_, spawned_tool) in spawned {
            spawned_tool.handle.abort();
        }
    }

    async fn finalize(
        &self,
        final_tool_calls: &[ToolCallRequest],
        llm_finished_at: Instant,
    ) -> StreamingToolFinalizeResult {
        let (mut assemblies, mut spawned, mut stats, gateway) = {
            let mut planner = self
                .inner
                .lock()
                .expect("streaming tool planner lock should work");
            (
                std::mem::take(&mut planner.assemblies),
                std::mem::take(&mut planner.spawned),
                planner.stats,
                planner.gateway.clone(),
            )
        };

        let mut matched_results = HashMap::new();
        let mut remaining_tool_calls = Vec::new();

        for (index, call) in final_tool_calls.iter().cloned().enumerate() {
            if let Some(spawned_tool) = spawned.remove(&call.id) {
                if spawned_tool.request == call {
                    match spawned_tool.handle.await {
                        Ok(buffered) => {
                            stats.matched_count = stats.matched_count.saturating_add(1);
                            stats.overlap_ms = stats
                                .overlap_ms
                                .saturating_add(overlap_ms(&buffered, llm_finished_at));
                            matched_results.insert(call.id.clone(), buffered);
                        },
                        Err(error) => {
                            log::warn!(
                                "turn streaming tool execution join failed for {}: {error}",
                                call.id
                            );
                            stats.fallback_count = stats.fallback_count.saturating_add(1);
                            remaining_tool_calls.push(call);
                        },
                    }
                } else {
                    spawned_tool.handle.abort();
                    stats.discard_count = stats.discard_count.saturating_add(1);
                    remaining_tool_calls.push(call);
                }
                continue;
            }

            if let Some(reason) =
                fallback_reason_for_final_call(gateway.as_ref(), assemblies.get(&index), &call)
            {
                log::debug!(
                    "turn streaming tool planner fallback for {} ({}): {}",
                    call.id,
                    call.name,
                    reason
                );
                stats.fallback_count = stats.fallback_count.saturating_add(1);
            }
            remaining_tool_calls.push(call);
        }

        stats.discard_count = stats.discard_count.saturating_add(spawned.len());
        for (_, spawned_tool) in spawned.drain() {
            spawned_tool.handle.abort();
        }
        assemblies.clear();

        StreamingToolFinalizeResult {
            matched_results,
            remaining_tool_calls,
            stats,
            used_streaming_path: stats.launched_count > 0,
        }
    }
}

impl StreamingToolPlanner {
    fn observe_delta(&mut self, delta: StreamedToolCallDelta) {
        let assembly = self.assemblies.entry(delta.index).or_default();
        if let Some(id) = delta.id {
            assembly.id = Some(id);
        }
        if let Some(name) = delta.name {
            assembly.name = Some(name);
        }
        assembly.arguments.push_str(&delta.arguments_delta);

        if assembly.launched {
            return;
        }

        let Some(id) = assembly.id.clone() else {
            return;
        };
        let Some(name) = assembly.name.clone() else {
            return;
        };

        let Some(gateway) = self.gateway.as_ref() else {
            return;
        };
        let Some(capability) = gateway.capabilities().capability_spec(&name) else {
            return;
        };
        if !capability.concurrency_safe {
            return;
        }

        let Ok(args) = serde_json::from_str::<Value>(&assembly.arguments) else {
            return;
        };

        let Some(session_state) = self.session_state.as_ref() else {
            return;
        };
        let Some(agent) = self.agent.as_ref() else {
            return;
        };
        let Some(cancel) = self.cancel.as_ref() else {
            return;
        };

        let request = ToolCallRequest {
            id: id.clone(),
            name,
            args,
        };
        let handle = tokio::spawn(tool_cycle::execute_buffered_tool_call(
            BufferedToolExecutionRequest {
                gateway: gateway.clone(),
                session_state: Arc::clone(session_state),
                tool_call: request.clone(),
                session_id: self.session_id.clone(),
                working_dir: self.working_dir.clone(),
                turn_id: self.turn_id.clone(),
                agent: agent.clone(),
                cancel: cancel.clone(),
                tool_result_inline_limit: self.tool_result_inline_limit,
            },
        ));
        assembly.launched = true;
        self.stats.launched_count = self.stats.launched_count.saturating_add(1);
        self.spawned
            .insert(id, SpawnedStreamingTool { request, handle });
    }
}

fn fallback_reason_for_final_call(
    gateway: Option<&astrcode_kernel::KernelGateway>,
    assembly: Option<&StreamingToolAssembly>,
    call: &ToolCallRequest,
) -> Option<&'static str> {
    let capability = gateway?.capabilities().capability_spec(&call.name)?;
    if !capability.concurrency_safe {
        return Some("tool is not concurrency_safe");
    }
    let assembly = assembly?;
    if assembly.id.as_deref() != Some(call.id.as_str())
        || assembly.name.as_deref() != Some(call.name.as_str())
    {
        return Some("streamed identity never stabilized");
    }
    Some("streamed arguments never formed a stable JSON payload")
}

fn overlap_ms(buffered: &BufferedToolExecution, llm_finished_at: Instant) -> u64 {
    let overlap_end = if buffered.finished_at < llm_finished_at {
        buffered.finished_at
    } else {
        llm_finished_at
    };
    if buffered.started_at >= overlap_end {
        return 0;
    }
    overlap_end.duration_since(buffered.started_at).as_millis() as u64
}

/// 单步执行的结果，决定 turn 主循环的后续走向。
pub(super) enum StepOutcome {
    /// 有工具调用，继续下一个 step。
    Continue(TurnLoopTransition),
    /// LLM 无工具调用，turn 自然结束。
    Completed(TurnStopCause),
    /// 取消信号或工具中断。
    Cancelled(TurnStopCause),
}

/// 抽象单步执行的各个阶段，方便测试时注入 mock 替代真实 LLM / 工具调用。
#[async_trait]
trait StepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult>;

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
        tool_delta_sink: Option<ToolCallDeltaSink>,
    ) -> Result<LlmOutput>;

    async fn try_reactive_compact(
        &self,
        execution: &TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<Option<compaction_cycle::RecoveryResult>>;

    async fn execute_tool_cycle(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
        tool_calls: Vec<ToolCallRequest>,
        event_emission_mode: ToolEventEmissionMode,
    ) -> Result<ToolCycleResult>;
}

/// 使用真实运行时 driver 执行一个 step。
pub(super) async fn run_single_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
) -> Result<StepOutcome> {
    run_single_step_with(execution, resources, &RuntimeStepDriver).await
}

/// 单步编排：assemble → LLM → 工具 → 决定下一步走向。
/// 可注入 driver 以便测试时替换真实 LLM / 工具调用。
async fn run_single_step_with(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
) -> Result<StepOutcome> {
    let assembled = driver.assemble_prompt(execution, resources).await?;
    let streaming_planner = StreamingToolPlannerHandle::new(resources);
    let llm_result = call_llm_for_step(
        execution,
        resources,
        driver,
        assembled.llm_request,
        Some(streaming_planner.tool_delta_sink()),
    )
    .await;
    let Some(output) = (match llm_result {
        Ok(output) => output,
        Err(error) => {
            streaming_planner.abort_all();
            return Err(error);
        },
    }) else {
        streaming_planner.abort_all();
        return Ok(StepOutcome::Continue(
            TurnLoopTransition::ReactiveCompactRecovered,
        ));
    };

    let llm_finished_at = Instant::now();
    record_llm_usage(execution, &output);
    let has_tool_calls = append_assistant_output(execution, resources, &output);
    warn_if_output_truncated(resources, execution, &output);

    if !has_tool_calls {
        streaming_planner.abort_all();
        return Ok(handle_assistant_without_tool_calls(
            execution, resources, &output,
        ));
    }

    let finalized_streaming = streaming_planner
        .finalize(&output.tool_calls, llm_finished_at)
        .await;
    execution.streaming_tool_launch_count = execution
        .streaming_tool_launch_count
        .saturating_add(finalized_streaming.stats.launched_count);
    execution.streaming_tool_match_count = execution
        .streaming_tool_match_count
        .saturating_add(finalized_streaming.stats.matched_count);
    execution.streaming_tool_fallback_count = execution
        .streaming_tool_fallback_count
        .saturating_add(finalized_streaming.stats.fallback_count);
    execution.streaming_tool_discard_count = execution
        .streaming_tool_discard_count
        .saturating_add(finalized_streaming.stats.discard_count);
    execution.streaming_tool_overlap_ms = execution
        .streaming_tool_overlap_ms
        .saturating_add(finalized_streaming.stats.overlap_ms);

    let event_emission_mode = if finalized_streaming.used_streaming_path {
        ToolEventEmissionMode::Buffered
    } else {
        ToolEventEmissionMode::Immediate
    };
    let mut executed_remaining = if finalized_streaming.remaining_tool_calls.is_empty() {
        ToolCycleResult {
            outcome: ToolCycleOutcome::Completed,
            tool_messages: Vec::new(),
            raw_results: Vec::new(),
            events: Vec::new(),
        }
    } else {
        driver
            .execute_tool_cycle(
                execution,
                resources,
                finalized_streaming.remaining_tool_calls.clone(),
                event_emission_mode,
            )
            .await?
    };

    if matches!(event_emission_mode, ToolEventEmissionMode::Buffered) {
        let mut combined_events = Vec::new();
        let mut remaining_events = std::mem::take(&mut executed_remaining.events);
        let mut remaining_results = executed_remaining
            .raw_results
            .iter()
            .cloned()
            .map(|(call, result)| (call.id.clone(), (call, result)))
            .collect::<HashMap<_, _>>();
        let mut merged_raw_results = Vec::with_capacity(output.tool_calls.len());
        let mut merged_tool_messages = Vec::with_capacity(output.tool_calls.len());

        for call in &output.tool_calls {
            if let Some(buffered) = finalized_streaming.matched_results.get(&call.id) {
                combined_events.extend(buffered.events.iter().cloned());
                merged_tool_messages.push(LlmMessage::Tool {
                    tool_call_id: buffered.result.tool_call_id.clone(),
                    content: buffered.result.model_content(),
                });
                merged_raw_results.push((call.clone(), buffered.result.clone()));
                continue;
            }
            if let Some((remaining_call, result)) = remaining_results.remove(&call.id) {
                merged_tool_messages.push(LlmMessage::Tool {
                    tool_call_id: result.tool_call_id.clone(),
                    content: result.model_content(),
                });
                merged_raw_results.push((remaining_call, result));
            }
        }

        combined_events.append(&mut remaining_events);
        execution.events.extend(combined_events);
        executed_remaining.tool_messages = merged_tool_messages;
        executed_remaining.raw_results = merged_raw_results;
    }

    track_tool_results(execution, resources.working_dir, &executed_remaining);
    execution
        .messages
        .extend(executed_remaining.tool_messages.clone());

    if matches!(executed_remaining.outcome, ToolCycleOutcome::Interrupted) {
        return Ok(StepOutcome::Cancelled(TurnStopCause::Cancelled));
    }

    execution.step_index += 1;
    Ok(StepOutcome::Continue(
        TurnLoopTransition::ToolCycleCompleted,
    ))
}

fn handle_assistant_without_tool_calls(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> StepOutcome {
    match decide_output_continuation(
        output,
        execution.max_output_continuation_count,
        resources.runtime,
    ) {
        OutputContinuationDecision::Continue => {
            execution.max_output_continuation_count =
                execution.max_output_continuation_count.saturating_add(1);
            append_internal_user_message(
                execution,
                resources,
                OUTPUT_CONTINUATION_PROMPT,
                UserMessageOrigin::ContinuationPrompt,
            );
            execution.step_index += 1;
            return StepOutcome::Continue(continuation_transition());
        },
        OutputContinuationDecision::Stop(stop_cause) => {
            append_turn_done_event(execution, resources, stop_cause);
            return StepOutcome::Completed(stop_cause);
        },
        OutputContinuationDecision::NotNeeded => {},
    }

    match decide_budget_continuation(
        output,
        execution.step_index,
        execution.continuation_count,
        resources.runtime,
        resources.gateway.model_limits(),
        execution.token_tracker.budget_tokens(0),
    ) {
        BudgetContinuationDecision::Continue => {
            append_internal_user_message(
                execution,
                resources,
                AUTO_CONTINUE_NUDGE,
                UserMessageOrigin::AutoContinueNudge,
            );
            execution.step_index += 1;
            StepOutcome::Continue(TurnLoopTransition::BudgetAllowsContinuation)
        },
        BudgetContinuationDecision::Stop(stop_cause) => {
            append_turn_done_event(execution, resources, stop_cause);
            StepOutcome::Completed(stop_cause)
        },
        BudgetContinuationDecision::NotNeeded => {
            append_turn_done_event(execution, resources, TurnStopCause::Completed);
            StepOutcome::Completed(TurnStopCause::Completed)
        },
    }
}

/// 调用 LLM，遇到 prompt too long 时尝试 reactive compact 恢复。
/// 恢复成功返回 `Ok(None)`（消息已更新，主循环应 continue 重新组装请求）。
async fn call_llm_for_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
    llm_request: LlmRequest,
    tool_delta_sink: Option<ToolCallDeltaSink>,
) -> Result<Option<LlmOutput>> {
    match driver
        .call_llm(resources, llm_request, tool_delta_sink)
        .await
    {
        Ok(output) => Ok(Some(output)),
        Err(error) => {
            if error.is_cancelled() {
                return Err(error);
            }
            if error.is_prompt_too_long()
                && execution.reactive_compact_attempts
                    < compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS
            {
                execution.reactive_compact_attempts += 1;
                log::warn!(
                    "turn {} step {}: prompt too long, reactive compact ({}/{})",
                    resources.turn_id,
                    execution.step_index,
                    execution.reactive_compact_attempts,
                    compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS,
                );

                let recovery = driver.try_reactive_compact(execution, resources).await?;

                if let Some(result) = recovery {
                    execution.events.extend(result.events);
                    execution.messages = result.messages;
                    return Ok(None);
                }
            }
            Err(error)
        },
    }
}

fn record_llm_usage(execution: &mut TurnExecutionContext, output: &LlmOutput) {
    execution.token_tracker.record_usage(output.usage);
    if let Some(usage) = &output.usage {
        execution.total_cache_read_tokens = execution
            .total_cache_read_tokens
            .saturating_add(usage.cache_read_input_tokens as u64);
        execution.total_cache_creation_tokens = execution
            .total_cache_creation_tokens
            .saturating_add(usage.cache_creation_input_tokens as u64);
    }
}

/// 将 LLM 输出追加到 messages 和 events，返回是否包含工具调用。
fn append_assistant_output(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> bool {
    let content = output.content.trim().to_string();
    let has_tool_calls = !output.tool_calls.is_empty();
    execution.messages.push(LlmMessage::Assistant {
        content: content.clone(),
        tool_calls: output.tool_calls.clone(),
        reasoning: output.reasoning.clone(),
    });
    execution
        .micro_compact_state
        .record_assistant_activity(Instant::now());
    execution.events.push(assistant_final_event(
        resources.turn_id,
        resources.agent,
        content,
        output
            .reasoning
            .as_ref()
            .map(|reasoning| reasoning.content.clone()),
        output
            .reasoning
            .as_ref()
            .and_then(|reasoning| reasoning.signature.clone()),
        Some(Utc::now()),
    ));
    has_tool_calls
}

/// 追加 TurnDone 事件（仅在 LLM 无工具调用、turn 自然结束时）。
fn append_turn_done_event(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    stop_cause: TurnStopCause,
) {
    execution.events.push(turn_done_event(
        resources.turn_id,
        resources.agent,
        stop_cause.turn_done_reason().map(ToString::to_string),
        Utc::now(),
    ));
}

fn append_internal_user_message(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    content: &str,
    origin: UserMessageOrigin,
) {
    execution.messages.push(LlmMessage::User {
        content: content.to_string(),
        origin,
    });
    execution.events.push(user_message_event(
        resources.turn_id,
        resources.agent,
        content.to_string(),
        origin,
        Utc::now(),
    ));
}

/// max_tokens 截断只记 warning，不改变流程（下一轮 prompt 预算仍会正确估算）。
fn warn_if_output_truncated(
    resources: &TurnExecutionResources<'_>,
    execution: &TurnExecutionContext,
    output: &LlmOutput,
) {
    if matches!(output.finish_reason, LlmFinishReason::MaxTokens) {
        log::warn!(
            "turn {} step {}: LLM output truncated by max_tokens",
            resources.turn_id,
            execution.step_index
        );
    }
}

/// 双重追踪：file_access_tracker（prune pass 用）+ micro_compact_state（idle 清理用）。
fn track_tool_results(
    execution: &mut TurnExecutionContext,
    working_dir: &str,
    tool_result: &ToolCycleResult,
) {
    for (call, result) in &tool_result.raw_results {
        execution
            .file_access_tracker
            .record_tool_result(call, result, Path::new(working_dir));
        execution
            .micro_compact_state
            .record_tool_result(result.tool_call_id.clone(), Instant::now());
    }
}

#[async_trait]
impl StepDriver for RuntimeStepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult> {
        let assembled = assemble_prompt_request(AssemblePromptRequest {
            gateway: resources.gateway,
            prompt_facts_provider: resources.prompt_facts_provider,
            session_id: resources.session_id,
            turn_id: resources.turn_id,
            working_dir: Path::new(resources.working_dir),
            messages: std::mem::take(&mut execution.messages),
            cancel: resources.cancel.clone(),
            agent: resources.agent,
            step_index: execution.step_index,
            token_tracker: &execution.token_tracker,
            tools: resources.tools.clone(),
            settings: &resources.settings,
            clearable_tools: &resources.clearable_tools,
            micro_compact_state: &mut execution.micro_compact_state,
            file_access_tracker: &execution.file_access_tracker,
            session_state: resources.session_state,
            tool_result_replacement_state: &mut execution.tool_result_replacement_state,
            prompt_declarations: resources.prompt_declarations,
        })
        .await?;
        execution.messages = assembled.messages.clone();
        if assembled.auto_compacted {
            execution.auto_compaction_count += 1;
        }
        execution.tool_result_replacement_count = execution
            .tool_result_replacement_count
            .saturating_add(assembled.tool_result_budget_stats.replacement_count);
        execution.tool_result_reapply_count = execution
            .tool_result_reapply_count
            .saturating_add(assembled.tool_result_budget_stats.reapply_count);
        execution.tool_result_bytes_saved = execution
            .tool_result_bytes_saved
            .saturating_add(assembled.tool_result_budget_stats.bytes_saved);
        execution.tool_result_over_budget_message_count = execution
            .tool_result_over_budget_message_count
            .saturating_add(assembled.tool_result_budget_stats.over_budget_message_count);
        execution.events.extend(assembled.events.iter().cloned());
        Ok(assembled)
    }

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
        tool_delta_sink: Option<ToolCallDeltaSink>,
    ) -> Result<LlmOutput> {
        crate::turn::llm_cycle::call_llm_streaming(
            resources.gateway,
            llm_request,
            resources.turn_id,
            resources.agent,
            resources.session_state,
            resources.cancel,
            tool_delta_sink,
        )
        .await
    }

    async fn try_reactive_compact(
        &self,
        execution: &TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<Option<compaction_cycle::RecoveryResult>> {
        compaction_cycle::try_reactive_compact(&ReactiveCompactContext {
            gateway: resources.gateway,
            prompt_facts_provider: resources.prompt_facts_provider,
            messages: &execution.messages,
            session_id: resources.session_id,
            working_dir: resources.working_dir,
            turn_id: resources.turn_id,
            step_index: execution.step_index,
            agent: resources.agent,
            cancel: resources.cancel.clone(),
            settings: &resources.settings,
            file_access_tracker: &execution.file_access_tracker,
        })
        .await
    }

    async fn execute_tool_cycle(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
        tool_calls: Vec<ToolCallRequest>,
        event_emission_mode: ToolEventEmissionMode,
    ) -> Result<ToolCycleResult> {
        tool_cycle::execute_tool_calls(
            &mut ToolCycleContext {
                gateway: resources.gateway,
                session_state: resources.session_state,
                session_id: resources.session_id,
                working_dir: resources.working_dir,
                turn_id: resources.turn_id,
                agent: resources.agent,
                cancel: resources.cancel,
                events: &mut execution.events,
                max_concurrency: resources.runtime.max_tool_concurrency,
                tool_result_inline_limit: resources.runtime.tool_result_inline_limit,
                event_emission_mode,
            },
            tool_calls,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use astrcode_core::{
        AgentEventContext, AstrError, CancelToken, CapabilityKind, LlmMessage, LlmUsage,
        PromptFactsProvider, ResolvedRuntimeConfig, StorageEventPayload, Tool, ToolContext,
        ToolDefinition, ToolExecutionResult, UserMessageOrigin,
    };
    use astrcode_kernel::KernelGateway;
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{
        SessionState,
        context_window::token_usage::PromptTokenSnapshot,
        turn::{
            events::prompt_metrics_event,
            runner::TurnExecutionRequestView,
            test_support::{
                NoopPromptFactsProvider, assert_contains_compact_summary,
                assert_has_assistant_final, assert_has_turn_done, root_compact_applied_event,
                test_gateway, test_session_state,
            },
        },
    };

    #[derive(Default)]
    struct DriverCallCounts {
        assemble: AtomicUsize,
        llm: AtomicUsize,
        reactive_compact: AtomicUsize,
        tool_cycle: AtomicUsize,
    }

    struct ScriptedStepDriver {
        counts: DriverCallCounts,
        assemble_result: Mutex<Option<Result<AssemblePromptResult>>>,
        llm_result: Mutex<Option<Result<LlmOutput>>>,
        reactive_compact_result: Mutex<Option<Result<Option<compaction_cycle::RecoveryResult>>>>,
        tool_cycle_result: Mutex<Option<Result<ToolCycleResult>>>,
    }

    #[async_trait]
    impl StepDriver for ScriptedStepDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> Result<AssemblePromptResult> {
            self.counts.assemble.fetch_add(1, Ordering::SeqCst);
            self.assemble_result
                .lock()
                .expect("assemble result lock should work")
                .take()
                .expect("assemble result should be scripted")
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            _tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> Result<LlmOutput> {
            self.counts.llm.fetch_add(1, Ordering::SeqCst);
            self.llm_result
                .lock()
                .expect("llm result lock should work")
                .take()
                .expect("llm result should be scripted")
        }

        async fn try_reactive_compact(
            &self,
            _execution: &TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> Result<Option<compaction_cycle::RecoveryResult>> {
            self.counts.reactive_compact.fetch_add(1, Ordering::SeqCst);
            self.reactive_compact_result
                .lock()
                .expect("reactive compact result lock should work")
                .take()
                .expect("reactive compact result should be scripted")
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> Result<ToolCycleResult> {
            self.counts.tool_cycle.fetch_add(1, Ordering::SeqCst);
            self.tool_cycle_result
                .lock()
                .expect("tool cycle result lock should work")
                .take()
                .expect("tool cycle result should be scripted")
        }
    }

    fn user_message(content: &str) -> LlmMessage {
        LlmMessage::User {
            content: content.to_string(),
            origin: UserMessageOrigin::User,
        }
    }

    fn assembled_prompt(messages: Vec<LlmMessage>) -> AssemblePromptResult {
        AssemblePromptResult {
            llm_request: LlmRequest::new(
                messages.clone(),
                vec![ToolDefinition {
                    name: "dummy_tool".to_string(),
                    description: "dummy".to_string(),
                    parameters: json!({"type": "object"}),
                }],
                CancelToken::new(),
            )
            .with_system("system"),
            messages,
            events: vec![prompt_metrics_event(
                "turn-1",
                &AgentEventContext::default(),
                0,
                PromptTokenSnapshot {
                    context_tokens: 10,
                    budget_tokens: 10,
                    context_window: 100,
                    effective_window: 90,
                    threshold_tokens: 80,
                },
                0,
            )],
            auto_compacted: false,
            tool_result_budget_stats:
                crate::turn::tool_result_budget::ToolResultBudgetStats::default(),
        }
    }

    fn test_resources<'a>(
        gateway: &'a KernelGateway,
        session_state: &'a Arc<SessionState>,
        runtime: &'a ResolvedRuntimeConfig,
        cancel: &'a CancelToken,
        agent: &'a AgentEventContext,
        prompt_facts_provider: &'a dyn PromptFactsProvider,
    ) -> TurnExecutionResources<'a> {
        TurnExecutionResources::new(
            gateway,
            TurnExecutionRequestView {
                prompt_facts_provider,
                session_id: "session-1",
                working_dir: ".",
                turn_id: "turn-1",
                session_state,
                runtime,
                cancel,
                agent,
                prompt_declarations: &[],
            },
        )
    }

    #[derive(Debug)]
    struct StreamingSafeProbeTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for StreamingSafeProbeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "streaming_safe_probe".to_string(),
                description: "safe probe for streamed tool execution".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(
            &self,
        ) -> std::result::Result<
            astrcode_core::CapabilitySpec,
            astrcode_core::CapabilitySpecBuildError,
        > {
            astrcode_core::CapabilitySpec::builder("streaming_safe_probe", CapabilityKind::Tool)
                .description("safe probe for streamed tool execution")
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .concurrency_safe(true)
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "streaming_safe_probe".to_string(),
                ok: true,
                output: "streamed safe result".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[tokio::test]
    async fn run_single_step_returns_completed_when_llm_has_no_tool_calls() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: "assistant reply".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: Some(LlmUsage {
                    input_tokens: 11,
                    output_tokens: 7,
                    cache_creation_input_tokens: 3,
                    cache_read_input_tokens: 2,
                }),
                finish_reason: LlmFinishReason::Stop,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should succeed");

        assert!(matches!(
            outcome,
            StepOutcome::Completed(TurnStopCause::Completed)
        ));
        assert_eq!(execution.step_index, 0);
        assert_eq!(execution.total_cache_read_tokens, 2);
        assert_eq!(execution.total_cache_creation_tokens, 3);
        assert_eq!(driver.counts.assemble.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.llm.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 0);
        assert!(matches!(
            execution.messages.last(),
            Some(LlmMessage::Assistant { content, .. }) if content == "assistant reply"
        ));
        assert_has_turn_done(&execution.events);
    }

    #[tokio::test]
    async fn run_single_step_returns_cancelled_when_tool_cycle_interrupts() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "dummy_tool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::ToolCalls,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(Some(Ok(ToolCycleResult {
                outcome: ToolCycleOutcome::Interrupted,
                tool_messages: Vec::new(),
                raw_results: Vec::new(),
                events: Vec::new(),
            }))),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should succeed");

        assert!(matches!(
            outcome,
            StepOutcome::Cancelled(TurnStopCause::Cancelled)
        ));
        assert_eq!(execution.step_index, 0);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 1);
        assert_has_assistant_final(&execution.events);
    }

    #[tokio::test]
    async fn run_single_step_reuses_streamed_safe_tool_execution_when_final_call_matches() {
        struct StreamingDriver {
            tool_cycle_calls: AtomicUsize,
        }

        #[async_trait]
        impl StepDriver for StreamingDriver {
            async fn assemble_prompt(
                &self,
                _execution: &mut TurnExecutionContext,
                _resources: &TurnExecutionResources<'_>,
            ) -> Result<AssemblePromptResult> {
                Ok(assembled_prompt(vec![user_message("find the answer")]))
            }

            async fn call_llm(
                &self,
                _resources: &TurnExecutionResources<'_>,
                _llm_request: LlmRequest,
                tool_delta_sink: Option<ToolCallDeltaSink>,
            ) -> Result<LlmOutput> {
                if let Some(sink) = tool_delta_sink {
                    sink(StreamedToolCallDelta {
                        index: 0,
                        id: Some("call-stream-1".to_string()),
                        name: Some("streaming_safe_probe".to_string()),
                        arguments_delta: r#"{"path":"README.md"}"#.to_string(),
                    });
                }
                Ok(LlmOutput {
                    content: String::new(),
                    tool_calls: vec![ToolCallRequest {
                        id: "call-stream-1".to_string(),
                        name: "streaming_safe_probe".to_string(),
                        args: json!({"path": "README.md"}),
                    }],
                    reasoning: None,
                    usage: None,
                    finish_reason: LlmFinishReason::ToolCalls,
                })
            }

            async fn try_reactive_compact(
                &self,
                _execution: &TurnExecutionContext,
                _resources: &TurnExecutionResources<'_>,
            ) -> Result<Option<compaction_cycle::RecoveryResult>> {
                Ok(None)
            }

            async fn execute_tool_cycle(
                &self,
                _execution: &mut TurnExecutionContext,
                _resources: &TurnExecutionResources<'_>,
                _tool_calls: Vec<ToolCallRequest>,
                _event_emission_mode: ToolEventEmissionMode,
            ) -> Result<ToolCycleResult> {
                self.tool_cycle_calls.fetch_add(1, Ordering::SeqCst);
                Ok(ToolCycleResult {
                    outcome: ToolCycleOutcome::Completed,
                    tool_messages: Vec::new(),
                    raw_results: Vec::new(),
                    events: Vec::new(),
                })
            }
        }

        let probe_calls = Arc::new(AtomicUsize::new(0));
        let kernel = crate::turn::test_support::test_kernel_with_tool(
            Arc::new(StreamingSafeProbeTool {
                calls: Arc::clone(&probe_calls),
            }),
            8192,
        );
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            kernel.gateway(),
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        let driver = StreamingDriver {
            tool_cycle_calls: AtomicUsize::new(0),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should succeed");

        assert!(matches!(
            outcome,
            StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted)
        ));
        assert_eq!(execution.step_index, 1);
        assert_eq!(probe_calls.load(Ordering::SeqCst), 1);
        assert_eq!(driver.tool_cycle_calls.load(Ordering::SeqCst), 0);
        assert_eq!(execution.streaming_tool_launch_count, 1);
        assert_eq!(execution.streaming_tool_match_count, 1);
        assert_eq!(execution.streaming_tool_fallback_count, 0);
        assert_eq!(execution.streaming_tool_discard_count, 0);
        assert!(
            execution.messages.iter().any(|message| matches!(
                message,
                LlmMessage::Tool { tool_call_id, content }
                    if tool_call_id == "call-stream-1" && content == "streamed safe result"
            )),
            "matched streamed tool result should be appended without fallback tool cycle"
        );
    }

    #[tokio::test]
    async fn run_single_step_returns_continue_after_reactive_compact_recovery() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let original_messages = vec![user_message("message before compact")];
        let mut execution = TurnExecutionContext::new(&resources, original_messages, None);
        let recovered_messages = vec![
            user_message("compacted summary"),
            LlmMessage::Assistant {
                content: "recovered context".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message(
                "message before compact",
            )])))),
            llm_result: Mutex::new(Some(Err(AstrError::LlmRequestFailed {
                status: 400,
                body: "prompt too long for provider".to_string(),
            }))),
            reactive_compact_result: Mutex::new(Some(Ok(Some(compaction_cycle::RecoveryResult {
                messages: recovered_messages.clone(),
                events: vec![root_compact_applied_event(
                    "turn-1",
                    "compacted",
                    1,
                    100,
                    60,
                    2,
                    40,
                )],
            })))),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should recover via reactive compact");

        assert!(matches!(
            outcome,
            StepOutcome::Continue(TurnLoopTransition::ReactiveCompactRecovered)
        ));
        assert_eq!(execution.step_index, 0);
        assert_eq!(execution.reactive_compact_attempts, 1);
        assert_eq!(driver.counts.llm.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.reactive_compact.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 0);
        assert_eq!(execution.messages, recovered_messages);
        let stored_like = execution
            .events
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, event)| astrcode_core::StoredEvent {
                storage_seq: index as u64 + 1,
                event,
            })
            .collect::<Vec<_>>();
        assert_contains_compact_summary(&stored_like, "compacted");
        assert!(
            execution
                .events
                .iter()
                .all(|event| !matches!(&event.payload, StorageEventPayload::AssistantFinal { .. })),
            "recovery path should continue without persisting a failed assistant reply"
        );
    }

    #[tokio::test]
    async fn run_single_step_injects_auto_continue_nudge_after_prior_loop_activity() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig {
            max_continuations: 2,
            ..ResolvedRuntimeConfig::default()
        };
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        execution.step_index = 1;
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: "brief follow-up".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: Some(LlmUsage {
                    input_tokens: 32,
                    output_tokens: 12,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
                finish_reason: LlmFinishReason::Stop,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should inject auto-continue nudge");

        assert!(matches!(
            outcome,
            StepOutcome::Continue(TurnLoopTransition::BudgetAllowsContinuation)
        ));
        assert!(matches!(
            execution.messages.last(),
            Some(LlmMessage::User {
                origin: UserMessageOrigin::AutoContinueNudge,
                content,
            }) if content == AUTO_CONTINUE_NUDGE
        ));
        assert!(
            execution.events.iter().any(|event| matches!(
                &event.payload,
                StorageEventPayload::UserMessage { origin, content, .. }
                    if *origin == UserMessageOrigin::AutoContinueNudge && content == AUTO_CONTINUE_NUDGE
            )),
            "auto-continue should append a durable internal user message event"
        );
    }

    #[tokio::test]
    async fn run_single_step_continues_after_max_tokens_without_tool_calls() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig {
            max_output_continuation_attempts: 2,
            ..ResolvedRuntimeConfig::default()
        };
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: "partial answer".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: Some(LlmUsage {
                    input_tokens: 40,
                    output_tokens: 32,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
                finish_reason: LlmFinishReason::MaxTokens,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should continue after truncated output");

        assert!(matches!(
            outcome,
            StepOutcome::Continue(TurnLoopTransition::OutputContinuationRequested)
        ));
        assert_eq!(execution.max_output_continuation_count, 1);
        assert!(matches!(
            execution.messages.last(),
            Some(LlmMessage::User {
                origin: UserMessageOrigin::ContinuationPrompt,
                content,
            }) if content == OUTPUT_CONTINUATION_PROMPT
        ));
    }

    #[tokio::test]
    async fn run_single_step_stops_when_max_tokens_continuation_limit_is_reached() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig {
            max_output_continuation_attempts: 1,
            ..ResolvedRuntimeConfig::default()
        };
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")], None);
        execution.max_output_continuation_count = 1;
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: "partial answer".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: Some(LlmUsage {
                    input_tokens: 40,
                    output_tokens: 32,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
                finish_reason: LlmFinishReason::MaxTokens,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should stop when truncated output continuation limit is reached");

        assert!(matches!(
            outcome,
            StepOutcome::Completed(TurnStopCause::MaxOutputContinuationLimitReached)
        ));
        assert!(
            execution.events.iter().any(|event| matches!(
                &event.payload,
                StorageEventPayload::TurnDone { reason, .. }
                    if reason.as_deref() == Some("token_exceeded")
            )),
            "limit stop should persist token_exceeded as stable turn-done reason"
        );
    }
}
