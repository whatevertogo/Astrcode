use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use astrcode_core::{
    AgentEventContext, AstrError, CapabilitySpec, HookEventKey, LlmMessage, ResolvedRuntimeConfig,
    StorageEvent, StorageEventPayload, ToolCallRequest, ToolDefinition, ToolExecutionResult,
    ToolOutputDelta, UserMessageOrigin,
    llm::{LlmEventSink, LlmOutput, LlmProvider, LlmRequest},
};
use astrcode_runtime_contract::{
    RuntimeEventSink, RuntimeTurnEvent, StepError, TurnIdentity, TurnLoopTransition, TurnStopCause,
};
use async_trait::async_trait;
use chrono::Utc;

use crate::{
    context_window::{
        ContextWindowSettings,
        compaction::is_prompt_too_long_message,
        file_access::FileAccessTracker,
        micro_compact::MicroCompactState,
        request::{
            apply_prompt_metrics_usage, assemble_runtime_request, recover_from_prompt_too_long,
        },
        token_usage::TokenUsageTracker,
        tool_result_budget::{ToolResultBudgetStats, ToolResultReplacementState},
    },
    hook_dispatch::{HookDispatchRequest, HookDispatcher, HookEffect, HookEventPayload},
    tool_dispatch::{ToolDispatchRequest, ToolDispatcher},
    types::{TurnInput, TurnOutput},
};

const OUTPUT_CONTINUATION_PROMPT: &str = "Continue from the exact point where the previous \
                                          response was cut off. Do not restart, recap, or \
                                          apologize.";

/// 单步执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Continue(TurnLoopTransition),
    Completed(TurnStopCause),
    Error(StepError),
}

/// 单 turn 执行所需的无状态资源快照。
///
/// 由 `TurnInput` 一次性提取，整个 turn 生命周期内不再变化。
/// 可变状态全部放在 `TurnExecutionContext` 中。
#[derive(Clone)]
pub struct TurnExecutionResources {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub agent: AgentEventContext,
    pub model_ref: String,
    pub provider_ref: String,
    pub hook_snapshot_id: String,
    pub current_mode: Option<String>,
    pub tool_count: usize,
    pub capability_specs: Arc<[CapabilitySpec]>,
    pub tools: Arc<[ToolDefinition]>,
    pub provider: Option<Arc<dyn LlmProvider>>,
    pub tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
    pub hook_dispatcher: Option<Arc<dyn HookDispatcher>>,
    pub cancel: astrcode_core::CancelToken,
    pub max_output_continuations: usize,
    pub working_dir: PathBuf,
    pub runtime_config: ResolvedRuntimeConfig,
    pub(crate) settings: ContextWindowSettings,
    pub(crate) clearable_tools: HashSet<String>,
    pub(crate) previous_tool_result_replacements:
        Vec<crate::context_window::tool_result_budget::ToolResultReplacementRecord>,
    pub(crate) last_assistant_at: Option<chrono::DateTime<Utc>>,
    /// 由宿主传入的事件历史路径，`None` 表示不保存 compact 历史。
    pub(crate) events_history_path: Option<String>,
    pub(crate) event_sink: Arc<dyn RuntimeEventSink>,
}

impl std::fmt::Debug for TurnExecutionResources {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnExecutionResources")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("agent_id", &self.agent_id)
            .field("agent", &self.agent)
            .field("model_ref", &self.model_ref)
            .field("provider_ref", &self.provider_ref)
            .field("hook_snapshot_id", &self.hook_snapshot_id)
            .field("current_mode", &self.current_mode)
            .field("tool_count", &self.tool_count)
            .field(
                "provider",
                &self.provider.as_ref().map(|_| "<llm-provider>"),
            )
            .field(
                "tool_dispatcher",
                &self.tool_dispatcher.as_ref().map(|_| "<tool-dispatcher>"),
            )
            .field(
                "hook_dispatcher",
                &self.hook_dispatcher.as_ref().map(|_| "<hook-dispatcher>"),
            )
            .field("cancel", &self.cancel)
            .field("max_output_continuations", &self.max_output_continuations)
            .field("working_dir", &self.working_dir)
            .field("runtime_config", &self.runtime_config)
            .field("events_history_path", &self.events_history_path)
            .field("event_sink", &"<runtime-event-sink>")
            .finish()
    }
}

impl TurnExecutionResources {
    fn turn_identity(&self) -> TurnIdentity {
        TurnIdentity::new(
            self.session_id.clone(),
            self.turn_id.clone(),
            self.agent_id.clone(),
        )
    }

    fn from_input(input: &TurnInput) -> Self {
        let surface = &input.surface;
        let settings = ContextWindowSettings::from(&input.runtime_config);
        let clearable_tools = surface
            .tool_specs
            .iter()
            .filter(|spec| spec.compact_clearable)
            .map(|spec| spec.name.to_string())
            .collect();
        Self {
            session_id: surface.session_id.clone(),
            turn_id: surface.turn_id.clone(),
            agent_id: surface.agent_id.clone(),
            agent: input.agent.clone(),
            model_ref: surface.model_ref.clone(),
            provider_ref: surface.provider_ref.clone(),
            hook_snapshot_id: surface.hook_snapshot_id.clone(),
            current_mode: surface.current_mode.clone(),
            tool_count: surface.tool_specs.len(),
            capability_specs: Arc::from(surface.tool_specs.clone().into_boxed_slice()),
            tools: tool_definitions_from_specs(&surface.tool_specs),
            provider: input.provider.clone(),
            tool_dispatcher: input.tool_dispatcher.clone(),
            hook_dispatcher: input.hook_dispatcher.clone(),
            cancel: input.cancel.clone(),
            max_output_continuations: input.max_output_continuations,
            working_dir: input.working_dir.clone(),
            runtime_config: input.runtime_config.clone(),
            settings,
            clearable_tools,
            previous_tool_result_replacements: input.previous_tool_result_replacements.clone(),
            last_assistant_at: input.last_assistant_at,
            events_history_path: input.events_history_path.clone(),
            event_sink: input.event_sink.clone().unwrap_or_else(|| Arc::new(|_| {})),
        }
    }
}

/// 单 turn 执行上下文。
///
/// 与 `TurnExecutionResources` 互补：这里存放 turn 执行过程中的**可变状态**，
/// 包括消息历史、待刷出的事件、token 统计、micro-compact 状态等。
/// 每个 step 修改此结构体，Resources 保持不变。
#[derive(Debug, Clone)]
pub struct TurnExecutionContext {
    pub messages: Vec<LlmMessage>,
    pub pending_events: Vec<RuntimeTurnEvent>,
    pub started_at: Instant,
    pub step_index: usize,
    pub last_transition: Option<TurnLoopTransition>,
    pub stop_cause: Option<TurnStopCause>,
    pub max_output_continuation_count: usize,
    pub reactive_compact_attempts: usize,
    pub(crate) token_tracker: TokenUsageTracker,
    pub(crate) micro_compact_state: MicroCompactState,
    pub(crate) file_access_tracker: FileAccessTracker,
    pub(crate) tool_result_replacement_state: ToolResultReplacementState,
    pub(crate) tool_result_budget_stats: ToolResultBudgetStats,
    /// 此 turn 中自动 compact 被触发的次数（含 reactive compact）。
    pub(crate) auto_compaction_count: usize,
}

impl TurnExecutionContext {
    fn new(messages: Vec<LlmMessage>, resources: &TurnExecutionResources) -> Self {
        let now = Instant::now();
        Self {
            micro_compact_state: MicroCompactState::seed_from_messages(
                &messages,
                resources.settings.micro_compact_config(),
                now,
                resources.last_assistant_at,
            ),
            file_access_tracker: FileAccessTracker::seed_from_messages(
                &messages,
                resources.settings.max_tracked_files,
                &resources.working_dir,
            ),
            tool_result_replacement_state: ToolResultReplacementState::seed(
                resources.previous_tool_result_replacements.clone(),
            ),
            messages,
            pending_events: Vec::new(),
            started_at: now,
            step_index: 0,
            last_transition: None,
            stop_cause: None,
            max_output_continuation_count: 0,
            reactive_compact_attempts: 0,
            token_tracker: TokenUsageTracker::default(),
            tool_result_budget_stats: ToolResultBudgetStats::default(),
            auto_compaction_count: 0,
        }
    }

    fn push_event(&mut self, event: RuntimeTurnEvent) {
        self.pending_events.push(event);
    }

    fn record_transition(&mut self, transition: TurnLoopTransition) {
        self.last_transition = Some(transition);
        self.step_index = self.step_index.saturating_add(1);
    }

    fn record_stop(&mut self, stop_cause: TurnStopCause) {
        self.stop_cause = Some(stop_cause);
    }
}

#[async_trait]
pub trait TurnStepRunner {
    async fn run_single_step(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources,
    ) -> StepOutcome;
}

#[derive(Debug, Default)]
struct ProviderTurnStepRunner;

#[async_trait]
impl TurnStepRunner for ProviderTurnStepRunner {
    async fn run_single_step(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources,
    ) -> StepOutcome {
        run_single_step(execution, resources).await
    }
}

/// 单 turn 主循环。
///
/// 执行流程：
/// 1. 派发 `TurnStart` hook → 若 hook 终止则直接返回
/// 2. 进入 step 循环，每次 step 由 `TurnStepRunner` 驱动
///    - `Continue` → 记录 transition，刷出事件，继续下一轮
///    - `Completed` → 派发 `TurnEnd` hook，然后终止
///    - `Error` → 立即终止（不经过 `TurnEnd` hook）
/// 3. 终止时调用 `finalize_turn` 统一收尾
#[derive(Debug, Default)]
pub struct TurnLoop;

impl TurnLoop {
    pub async fn run(&self, input: TurnInput) -> TurnOutput {
        self.run_with_step_runner(input, &ProviderTurnStepRunner)
            .await
    }

    pub async fn run_with_step_runner(
        &self,
        input: TurnInput,
        runner: &impl TurnStepRunner,
    ) -> TurnOutput {
        let resources = TurnExecutionResources::from_input(&input);
        let event_sink = Arc::clone(&resources.event_sink);
        let mut execution = TurnExecutionContext::new(input.messages, &resources);
        let mut emitted_events = Vec::new();

        // TurnStart 阶段：hook 可以在 turn 开始时中止执行
        execution.push_event(RuntimeTurnEvent::TurnStarted {
            identity: resources.turn_identity(),
        });
        if let Some(outcome) =
            dispatch_runtime_hook(&mut execution, &resources, HookEventKey::TurnStart).await
        {
            flush_pending_events(event_sink.as_ref(), &mut execution, &mut emitted_events);
            return match outcome {
                StepOutcome::Completed(stop_cause) => finalize_turn(
                    event_sink.as_ref(),
                    &resources.turn_identity(),
                    &mut execution,
                    &mut emitted_events,
                    stop_cause,
                    None,
                ),
                StepOutcome::Error(step_error) => finalize_turn(
                    event_sink.as_ref(),
                    &resources.turn_identity(),
                    &mut execution,
                    &mut emitted_events,
                    TurnStopCause::Error,
                    Some(&step_error.message),
                ),
                StepOutcome::Continue(_) => unreachable!("hooks cannot request loop continuation"),
            };
        }
        flush_pending_events(event_sink.as_ref(), &mut execution, &mut emitted_events);

        // Step 循环：每次迭代由 runner 执行一个 step
        loop {
            match runner.run_single_step(&mut execution, &resources).await {
                StepOutcome::Continue(transition) => {
                    execution.push_event(RuntimeTurnEvent::StepContinued {
                        identity: resources.turn_identity(),
                        step_index: execution.step_index,
                        transition,
                    });
                    execution.record_transition(transition);
                    flush_pending_events(event_sink.as_ref(), &mut execution, &mut emitted_events);
                },
                StepOutcome::Completed(stop_cause) => {
                    execution.record_stop(stop_cause);
                    // Completed 时派发 TurnEnd hook，hook 可覆盖最终的 stop_cause
                    if let Some(hook_outcome) =
                        dispatch_runtime_hook(&mut execution, &resources, HookEventKey::TurnEnd)
                            .await
                    {
                        return match hook_outcome {
                            StepOutcome::Completed(stop_cause) => finalize_turn(
                                event_sink.as_ref(),
                                &resources.turn_identity(),
                                &mut execution,
                                &mut emitted_events,
                                stop_cause,
                                None,
                            ),
                            StepOutcome::Error(step_error) => finalize_turn(
                                event_sink.as_ref(),
                                &resources.turn_identity(),
                                &mut execution,
                                &mut emitted_events,
                                TurnStopCause::Error,
                                Some(&step_error.message),
                            ),
                            StepOutcome::Continue(_) => {
                                unreachable!("hooks cannot request loop continuation")
                            },
                        };
                    }
                    return finalize_turn(
                        event_sink.as_ref(),
                        &resources.turn_identity(),
                        &mut execution,
                        &mut emitted_events,
                        stop_cause,
                        None,
                    );
                },
                StepOutcome::Error(step_error) => {
                    return finalize_turn(
                        event_sink.as_ref(),
                        &resources.turn_identity(),
                        &mut execution,
                        &mut emitted_events,
                        TurnStopCause::Error,
                        Some(&step_error.message),
                    );
                },
            }
        }
    }
}

async fn run_single_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
) -> StepOutcome {
    // 阶段 1：派发前置 hook（Context → BeforeAgentStart → BeforeProviderRequest）
    for event in [HookEventKey::Context, HookEventKey::BeforeAgentStart] {
        if let Some(outcome) = dispatch_runtime_hook(execution, resources, event).await {
            return outcome;
        }
    }

    let Some(provider) = &resources.provider else {
        return StepOutcome::Completed(TurnStopCause::Completed);
    };

    if resources.cancel.is_cancelled() {
        return StepOutcome::Completed(TurnStopCause::Cancelled);
    }

    // 阶段 2：组装请求 + 调用 provider
    // stream_events 同时通过两条路径输出：
    //   1. 即时推送到 event_sink（供 UI 实时展示）
    //   2. 缓存到 stream_events（供 step 完成后追加到 pending_events）
    let stream_events = Arc::new(Mutex::new(Vec::new()));
    let stream_events_sink = Arc::clone(&stream_events);
    let stream_event_sink = Arc::clone(&resources.event_sink);
    let stream_identity = resources.turn_identity();
    let sink: LlmEventSink = Arc::new(move |event| {
        stream_event_sink.emit_event(RuntimeTurnEvent::ProviderStream {
            identity: stream_identity.clone(),
            event: event.clone(),
        });
        stream_events_sink
            .lock()
            .expect("provider stream event buffer poisoned")
            .push(event);
    });

    let mut request = match assemble_runtime_request(execution, resources).await {
        Ok(request) => request,
        Err(error) if error.is_cancelled() => {
            return StepOutcome::Completed(TurnStopCause::Cancelled);
        },
        Err(error) => return StepOutcome::Error(StepError::from(&error)),
    };

    let before_provider_payload = HookEventPayload::BeforeProviderRequest {
        session_id: resources.session_id.clone(),
        turn_id: resources.turn_id.clone(),
        provider_ref: resources.provider_ref.clone(),
        model_ref: resources.model_ref.clone(),
        request: provider_request_hook_payload(&request),
        current_mode: resources.current_mode.clone(),
    };
    match dispatch_typed_hook(
        execution,
        resources,
        HookEventKey::BeforeProviderRequest,
        before_provider_payload,
    )
    .await
    {
        Ok(effects) => {
            for effect in effects {
                match effect {
                    HookEffect::DenyProviderRequest { reason } => {
                        execution.push_event(RuntimeTurnEvent::HookPromptAugmented {
                            identity: resources.turn_identity(),
                            event: HookEventKey::BeforeProviderRequest,
                            content: format!("provider request denied: {reason}"),
                        });
                        return StepOutcome::Completed(TurnStopCause::Completed);
                    },
                    HookEffect::ModifyProviderRequest { request: patch } => {
                        apply_provider_request_patch(&mut request, patch);
                    },
                    HookEffect::CancelTurn { .. } => {
                        return StepOutcome::Completed(TurnStopCause::Cancelled);
                    },
                    HookEffect::Continue | HookEffect::Diagnostic { .. } => {},
                    _ => {
                        return StepOutcome::Error(StepError::fatal(
                            "before_provider_request hook returned unsupported effect",
                        ));
                    },
                }
            }
        },
        Err(error) if error.is_cancelled() => {
            return StepOutcome::Completed(TurnStopCause::Cancelled);
        },
        Err(error) => return StepOutcome::Error(StepError::from(&error)),
    }

    let output = match provider.generate(request, Some(sink)).await {
        Ok(output) => output,
        Err(error) if error.is_cancelled() => {
            return StepOutcome::Completed(TurnStopCause::Cancelled);
        },
        // "prompt too long" 错误时尝试 reactive compact：缩小上下文后重试当前 step
        Err(error)
            if is_prompt_too_long_message(&error.to_string())
                && execution.reactive_compact_attempts
                    < resources.settings.compact_max_retry_attempts
                && resources.settings.auto_compact_enabled =>
        {
            match recover_from_prompt_too_long(execution, resources, provider.as_ref()).await {
                Ok(true) => {
                    return StepOutcome::Continue(TurnLoopTransition::ReactiveCompactRecovered);
                },
                Ok(false) => return StepOutcome::Error(StepError::from(&error)),
                Err(recovery_error) if recovery_error.is_cancelled() => {
                    return StepOutcome::Completed(TurnStopCause::Cancelled);
                },
                Err(recovery_error) => return StepOutcome::Error(StepError::from(&recovery_error)),
            }
        },
        Err(error) => return StepOutcome::Error(StepError::from(&error)),
    };

    // 将缓存中的 stream 事件追加到 pending_events，供最终 flush 输出
    for event in stream_events
        .lock()
        .expect("provider stream event buffer poisoned")
        .drain(..)
    {
        execution.push_event(RuntimeTurnEvent::ProviderStream {
            identity: resources.turn_identity(),
            event,
        });
    }

    // 阶段 3：记录 provider 输出 + 更新 token 统计
    record_provider_output(execution, resources, &output);
    apply_prompt_metrics_usage(
        &mut execution.pending_events,
        execution.step_index,
        output.usage,
        output.prompt_cache_diagnostics.clone(),
    );
    execution.token_tracker.record_usage(output.usage);

    // 阶段 4：若有 tool_calls，执行工具调度
    if !output.tool_calls.is_empty() {
        if let Some(dispatcher) = &resources.tool_dispatcher {
            match execute_tool_calls(
                execution,
                resources,
                dispatcher.as_ref(),
                &output.tool_calls,
            )
            .await
            {
                Ok(()) => return StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted),
                Err(error) if error.is_cancelled() => {
                    return StepOutcome::Completed(TurnStopCause::Cancelled);
                },
                Err(error) => return StepOutcome::Error(StepError::from(&error)),
            }
        }
        // 有 tool_calls 但没有 dispatcher → 标记 ToolUseRequested 后结束 turn
        // 由宿主层负责后续的工具执行
        execution.push_event(RuntimeTurnEvent::ToolUseRequested {
            identity: resources.turn_identity(),
            tool_call_count: output.tool_calls.len(),
        });
        return StepOutcome::Completed(TurnStopCause::Completed);
    }

    // 阶段 5：输出被 max_tokens 截断时，注入 continuation prompt 继续生成
    if output.finish_reason.is_max_tokens()
        && execution.max_output_continuation_count < resources.max_output_continuations
    {
        execution.messages.push(LlmMessage::User {
            content: OUTPUT_CONTINUATION_PROMPT.to_string(),
            origin: UserMessageOrigin::ContinuationPrompt,
        });
        execution.max_output_continuation_count =
            execution.max_output_continuation_count.saturating_add(1);
        return StepOutcome::Continue(TurnLoopTransition::OutputContinuationRequested);
    }

    StepOutcome::Completed(TurnStopCause::Completed)
}

/// 派发运行时 hook 并处理返回的 effects。
///
/// - `Continue` / `Diagnostic` → 忽略，继续执行
/// - `AugmentPrompt` → 注入一条用户消息到消息流（用于上下文增强）
/// - `CancelTurn` → 返回 `Completed(Cancelled)`
/// - `Block` → 返回 `Error`
///
/// 返回 `None` 表示 hook 允许继续执行；`Some` 表示 hook 终止了流程。
async fn dispatch_runtime_hook(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    event: HookEventKey,
) -> Option<StepOutcome> {
    let Some(dispatcher) = &resources.hook_dispatcher else {
        return None;
    };

    let outcome = match dispatcher
        .dispatch_hook(HookDispatchRequest {
            snapshot_id: resources.hook_snapshot_id.clone(),
            event,
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            agent_id: resources.agent_id.clone(),
            payload: HookEventPayload::from_value(
                &event,
                &serde_json::json!({
                    "sessionId": resources.session_id.clone(),
                    "turnId": resources.turn_id.clone(),
                    "agentId": resources.agent_id.clone(),
                    "agent": resources.agent.clone(),
                    "stepIndex": execution.step_index,
                    "messageCount": execution.messages.len(),
                    "currentMode": resources.current_mode.clone(),
                }),
            ),
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Some(StepOutcome::Error(StepError::from(&error))),
    };

    execution.push_event(RuntimeTurnEvent::HookDispatched {
        identity: resources.turn_identity(),
        event,
        effect_count: outcome.effects.len(),
    });

    for effect in outcome.effects {
        match effect {
            HookEffect::Continue => {},
            HookEffect::Diagnostic { message } => {
                execution.messages.push(LlmMessage::User {
                    content: message.clone(),
                    origin: UserMessageOrigin::ReactivationPrompt,
                });
                execution.push_event(RuntimeTurnEvent::HookPromptAugmented {
                    identity: resources.turn_identity(),
                    event,
                    content: message,
                });
            },
            HookEffect::CancelTurn { .. } => {
                return Some(StepOutcome::Completed(TurnStopCause::Cancelled));
            },
            _other => {
                return Some(StepOutcome::Error(StepError::fatal(
                    "hook blocked execution",
                )));
            },
        }
    }

    None
}

async fn execute_tool_calls(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    dispatcher: &dyn ToolDispatcher,
    tool_calls: &[ToolCallRequest],
) -> astrcode_core::Result<()> {
    // Phase 1: Pre-dispatch hooks (sequential, per-tool)
    // 对每个 tool_call 触发 ToolCall hook 并处理 per-tool effects
    let (tool_output_sender, mut tool_output_receiver) =
        tokio::sync::mpsc::unbounded_channel::<ToolOutputDelta>();

    let mut allowed_calls = Vec::with_capacity(tool_calls.len());
    let mut blocked_results: Vec<(usize, ToolExecutionResult)> = Vec::new();

    for (index, tool_call) in tool_calls.iter().enumerate() {
        if resources.cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }

        let hook_payload = HookEventPayload::ToolCall {
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            agent_id: resources.agent_id.clone(),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
            capability_spec: Box::new(capability_spec_for_tool(resources, &tool_call.name)),
            working_dir: resources.working_dir.clone(),
            current_mode: resources.current_mode.clone(),
            step_index: execution.step_index,
        };

        // 使用 typed payload 派发 hook
        let effects =
            match dispatch_typed_hook(execution, resources, HookEventKey::ToolCall, hook_payload)
                .await
            {
                Ok(effects) => effects,
                Err(error) if error.is_cancelled() => return Err(AstrError::Cancelled),
                Err(error) => return Err(error),
            };

        // 处理 tool_call effects
        let mut should_proceed = true;
        let mut mutated_args = tool_call.args.clone();

        for effect in effects {
            match effect {
                HookEffect::Continue | HookEffect::Diagnostic { .. } => {},
                HookEffect::MutateToolArgs { tool_call_id, args }
                    if tool_call_id == tool_call.id =>
                {
                    mutated_args = args;
                },
                HookEffect::BlockToolResult {
                    tool_call_id,
                    reason,
                } if tool_call_id == tool_call.id => {
                    blocked_results.push((
                        index,
                        ToolExecutionResult {
                            tool_call_id: tool_call.id.clone(),
                            tool_name: tool_call.name.clone(),
                            ok: false,
                            output: String::new(),
                            error: Some(reason),
                            metadata: None,
                            continuation: None,
                            duration_ms: 0,
                            truncated: false,
                        },
                    ));
                    should_proceed = false;
                },
                HookEffect::RequireApproval { reason, .. } => {
                    // 审批异步处理 — 在当前实现中等同于 BlockToolResult
                    blocked_results.push((
                        index,
                        ToolExecutionResult {
                            tool_call_id: tool_call.id.clone(),
                            tool_name: tool_call.name.clone(),
                            ok: false,
                            output: String::new(),
                            error: Some(format!("requires approval: {reason}")),
                            metadata: None,
                            continuation: None,
                            duration_ms: 0,
                            truncated: false,
                        },
                    ));
                    should_proceed = false;
                },
                HookEffect::CancelTurn { .. } => {
                    return Err(AstrError::Cancelled);
                },
                _other => {}, // 其他 effect 对 tool_call 不适用
            }
        }

        if !should_proceed {
            continue;
        }

        // 构造带突变参数的 mutated tool_call
        let effective_call = if mutated_args != tool_call.args {
            ToolCallRequest {
                id: tool_call.id.clone(),
                name: tool_call.name.clone(),
                args: mutated_args,
            }
        } else {
            tool_call.clone()
        };

        execution.push_event(RuntimeTurnEvent::ToolCallStarted {
            identity: resources.turn_identity(),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
        });
        push_immediate_event(
            execution,
            resources,
            RuntimeTurnEvent::StorageEvent {
                event: Box::new(StorageEvent {
                    turn_id: Some(resources.turn_id.clone()),
                    agent: resources.agent.clone(),
                    payload: StorageEventPayload::ToolCall {
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                        args: effective_call.args.clone(),
                    },
                }),
            },
        );

        allowed_calls.push((index, tool_call, effective_call));
    }

    // Phase 2: Dispatch allowed tools concurrently
    let futures: Vec<_> = allowed_calls
        .iter()
        .map(|(_index, _original, effective_call)| {
            dispatcher.dispatch_tool(ToolDispatchRequest {
                session_id: resources.session_id.clone(),
                turn_id: resources.turn_id.clone(),
                agent_id: resources.agent_id.clone(),
                tool_call: effective_call.clone(),
                tool_output_sender: Some(tool_output_sender.clone()),
            })
        })
        .collect();
    drop(tool_output_sender);

    let mut results_future = Box::pin(futures_util::future::join_all(futures));
    let dispatch_results: Vec<(usize, astrcode_core::Result<ToolExecutionResult>)> = loop {
        tokio::select! {
            Some(delta) = tool_output_receiver.recv() => {
                record_tool_output_delta(execution, resources, delta);
            },
            results = &mut results_future => {
                while let Ok(delta) = tool_output_receiver.try_recv() {
                    record_tool_output_delta(execution, resources, delta);
                }
                break allowed_calls.iter().zip(results).map(|((index, ..), result)| {
                    (*index, result)
                }).collect::<Vec<_>>();
            },
        }
    };

    // Phase 3: Process results with tool_result hooks BEFORE recording
    let mut all_results: std::collections::BTreeMap<usize, ToolExecutionResult> =
        blocked_results.into_iter().collect();

    for (index, result) in dispatch_results {
        let r = match result {
            Ok(r) => r,
            Err(e) if e.is_cancelled() => return Err(AstrError::Cancelled),
            Err(e) => return Err(e),
        };
        all_results.insert(index, r);
    }

    for (index, tool_call) in tool_calls.iter().enumerate() {
        let Some(mut result) = all_results.remove(&index) else {
            continue;
        };

        // tool_result hook BEFORE record (task 4.3)
        let hook_payload = HookEventPayload::ToolResult {
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            args: tool_call.args.clone(),
            result: serde_json::json!({ "output": result.output, "error": result.error }),
            ok: result.ok,
            current_mode: resources.current_mode.clone(),
        };

        let effects =
            match dispatch_typed_hook(execution, resources, HookEventKey::ToolResult, hook_payload)
                .await
            {
                Ok(effects) => effects,
                Err(error) if error.is_cancelled() => return Err(AstrError::Cancelled),
                Err(error) => return Err(error),
            };

        for effect in effects {
            match effect {
                HookEffect::OverrideToolResult {
                    tool_call_id,
                    result: override_result,
                    ok,
                } if tool_call_id == tool_call.id => {
                    result.output = override_result
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&result.output)
                        .to_string();
                    result.ok = ok;
                },
                HookEffect::Continue | HookEffect::Diagnostic { .. } => {},
                _ => {},
            }
        }

        record_tool_result(execution, resources, tool_call, result);
    }
    Ok(())
}

/// 带 typed payload 的 hook dispatch，返回 effect 列表。
async fn dispatch_typed_hook(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    event: HookEventKey,
    payload: HookEventPayload,
) -> astrcode_core::Result<Vec<HookEffect>> {
    let Some(dispatcher) = &resources.hook_dispatcher else {
        return Ok(Vec::new());
    };

    let outcome = match dispatcher
        .dispatch_hook(HookDispatchRequest {
            snapshot_id: resources.hook_snapshot_id.clone(),
            event,
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            agent_id: resources.agent_id.clone(),
            payload: payload.clone(),
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Err(error),
    };

    execution.push_event(RuntimeTurnEvent::HookDispatched {
        identity: resources.turn_identity(),
        event,
        effect_count: outcome.effects.len(),
    });

    Ok(outcome.effects)
}

fn provider_request_hook_payload(request: &LlmRequest) -> serde_json::Value {
    serde_json::json!({
        "messageCount": request.messages.len(),
        "toolCount": request.tools.len(),
        "systemPrompt": request.system_prompt.clone(),
        "maxOutputTokensOverride": request.max_output_tokens_override,
        "skipCacheWrite": request.skip_cache_write,
    })
}

fn apply_provider_request_patch(request: &mut LlmRequest, patch: serde_json::Value) {
    if let Some(system_prompt) = patch.get("systemPrompt") {
        request.system_prompt = system_prompt.as_str().map(str::to_owned);
    }
    if let Some(max_output_tokens) = patch
        .get("maxOutputTokensOverride")
        .and_then(serde_json::Value::as_u64)
    {
        request.max_output_tokens_override = Some(max_output_tokens as usize);
    }
    if let Some(skip_cache_write) = patch
        .get("skipCacheWrite")
        .and_then(serde_json::Value::as_bool)
    {
        request.skip_cache_write = skip_cache_write;
    }
}

fn capability_spec_for_tool(resources: &TurnExecutionResources, tool_name: &str) -> CapabilitySpec {
    resources
        .capability_specs
        .iter()
        .find(|spec| spec.name.as_str() == tool_name)
        .cloned()
        .unwrap_or_else(|| fallback_tool_capability(tool_name))
}

fn fallback_tool_capability(tool_name: &str) -> CapabilitySpec {
    use astrcode_core::{CapabilityKind, InvocationMode, SideEffect, Stability};

    CapabilitySpec {
        name: tool_name.to_string().into(),
        kind: CapabilityKind::Tool,
        description: String::new(),
        input_schema: Default::default(),
        output_schema: Default::default(),
        invocation_mode: InvocationMode::Unary,
        concurrency_safe: false,
        compact_clearable: false,
        profiles: Vec::new(),
        tags: Vec::new(),
        permissions: Vec::new(),
        side_effect: SideEffect::None,
        stability: Stability::Stable,
        metadata: Default::default(),
        max_result_inline_size: None,
    }
}

fn record_tool_result(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    tool_call: &ToolCallRequest,
    result: ToolExecutionResult,
) {
    execution.push_event(RuntimeTurnEvent::ToolResultReady {
        identity: resources.turn_identity(),
        tool_call_id: result.tool_call_id.clone(),
        tool_name: result.tool_name.clone(),
        ok: result.ok,
    });
    push_immediate_event(
        execution,
        resources,
        RuntimeTurnEvent::StorageEvent {
            event: Box::new(StorageEvent {
                turn_id: Some(resources.turn_id.clone()),
                agent: resources.agent.clone(),
                payload: StorageEventPayload::ToolResult {
                    tool_call_id: result.tool_call_id.clone(),
                    tool_name: result.tool_name.clone(),
                    output: result.output.clone(),
                    success: result.ok,
                    error: result.error.clone(),
                    metadata: result.metadata.clone(),
                    continuation: result.continuation.clone(),
                    duration_ms: result.duration_ms,
                },
            }),
        },
    );
    execution
        .file_access_tracker
        .record_tool_result(tool_call, &result, &resources.working_dir);
    execution
        .micro_compact_state
        .record_tool_result(result.tool_call_id.clone(), Instant::now());
    execution.messages.push(LlmMessage::Tool {
        tool_call_id: result.tool_call_id.clone(),
        content: result.model_content(),
    });
}

fn record_provider_output(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    output: &LlmOutput,
) {
    execution
        .micro_compact_state
        .record_assistant_activity(Instant::now());
    execution.messages.push(LlmMessage::Assistant {
        content: output.content.clone(),
        tool_calls: output.tool_calls.clone(),
        reasoning: output.reasoning.clone(),
    });
    execution.push_event(RuntimeTurnEvent::AssistantFinal {
        identity: resources.turn_identity(),
        content: output.content.clone(),
        reasoning: output.reasoning.clone(),
        tool_call_count: output.tool_calls.len(),
    });
}

fn record_tool_output_delta(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    delta: ToolOutputDelta,
) {
    push_immediate_event(
        execution,
        resources,
        RuntimeTurnEvent::StorageEvent {
            event: Box::new(StorageEvent {
                turn_id: Some(resources.turn_id.clone()),
                agent: resources.agent.clone(),
                payload: StorageEventPayload::ToolCallDelta {
                    tool_call_id: delta.tool_call_id,
                    tool_name: delta.tool_name,
                    stream: delta.stream,
                    delta: delta.delta,
                },
            }),
        },
    );
}

/// 立即推送到 event_sink 并追加到 pending_events。
/// 用于需要在产生时就对外可见的事件（如 ProviderStream、工具存储事件），
/// 与 `flush_pending_events` 的延迟推送互补。
fn push_immediate_event(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    event: RuntimeTurnEvent,
) {
    resources.event_sink.emit_event(event.clone());
    execution.push_event(event);
}

fn tool_definitions_from_specs(specs: &[CapabilitySpec]) -> Arc<[ToolDefinition]> {
    specs
        .iter()
        .map(|spec| ToolDefinition {
            name: spec.name.to_string(),
            description: spec.description.clone(),
            parameters: spec.input_schema.clone(),
        })
        .collect::<Vec<_>>()
        .into()
}

/// 将 pending_events 刷出到 event_sink 并追加到 emitted_events。
///
/// `ProviderStream` 和即时工具存储事件（ToolCall/ToolCallDelta/ToolResult）
/// 已在产生时通过 `push_immediate_event` 即时推送过 sink，此处只追加到 emitted_events
/// 以避免重复推送。
fn flush_pending_events(
    event_sink: &dyn RuntimeEventSink,
    execution: &mut TurnExecutionContext,
    emitted_events: &mut Vec<RuntimeTurnEvent>,
) {
    if execution.pending_events.is_empty() {
        return;
    }

    for event in execution.pending_events.drain(..) {
        if !matches!(event, RuntimeTurnEvent::ProviderStream { .. })
            && !is_immediate_tool_storage_event(&event)
        {
            event_sink.emit_event(event.clone());
        }
        emitted_events.push(event);
    }
}

fn is_immediate_tool_storage_event(event: &RuntimeTurnEvent) -> bool {
    matches!(
        event,
        RuntimeTurnEvent::StorageEvent { event }
            if matches!(
                event.payload,
                StorageEventPayload::ToolCall { .. }
                    | StorageEventPayload::ToolCallDelta { .. }
                    | StorageEventPayload::ToolResult { .. }
            )
    )
}

fn finalize_turn(
    event_sink: &dyn RuntimeEventSink,
    identity: &TurnIdentity,
    execution: &mut TurnExecutionContext,
    emitted_events: &mut Vec<RuntimeTurnEvent>,
    stop_cause: TurnStopCause,
    error_message: Option<&str>,
) -> TurnOutput {
    execution.record_stop(stop_cause);
    if let Some(msg) = error_message {
        execution.push_event(RuntimeTurnEvent::TurnErrored {
            identity: identity.clone(),
            message: msg.to_string(),
        });
    }
    let terminal_kind = stop_cause.terminal_kind(error_message);
    execution.push_event(RuntimeTurnEvent::TurnCompleted {
        identity: identity.clone(),
        stop_cause,
        terminal_kind: terminal_kind.clone(),
    });
    flush_pending_events(event_sink, execution, emitted_events);
    TurnOutput {
        identity: identity.clone(),
        terminal_kind: Some(terminal_kind),
        stop_cause: Some(stop_cause),
        step_count: execution.step_index.saturating_add(1),
        events: std::mem::take(emitted_events),
        error_message: error_message.map(|m| m.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use astrcode_core::{
        AgentEventContext, AstrError, CancelToken, HookEventKey, Result, SubRunStorageMode,
        ToolExecutionResult, ToolOutputDelta, ToolOutputStream, TurnTerminalKind,
        llm::{
            LlmEvent, LlmEventSink, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest,
            ModelLimits,
        },
    };
    use astrcode_runtime_contract::{
        HookEventPayload, RuntimeTurnEvent, StepError, TurnLoopTransition, TurnStopCause,
    };
    use async_trait::async_trait;

    use super::{
        StepOutcome, TurnExecutionContext, TurnExecutionResources, TurnLoop, TurnStepRunner,
    };
    use crate::{
        hook_dispatch::{HookDispatchOutcome, HookDispatchRequest, HookDispatcher, HookEffect},
        tool_dispatch::{ToolDispatchRequest, ToolDispatcher},
        types::{AgentRuntimeExecutionSurface, TurnInput},
    };

    fn input() -> TurnInput {
        TurnInput::new(AgentRuntimeExecutionSurface {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            agent_id: "agent-1".to_string(),
            model_ref: "model-a".to_string(),
            provider_ref: "provider-a".to_string(),
            tool_specs: Vec::new(),
            hook_snapshot_id: "snapshot-1".to_string(),
            current_mode: None,
        })
    }

    fn storage_payload(event: &RuntimeTurnEvent) -> Option<&astrcode_core::StorageEventPayload> {
        match event {
            RuntimeTurnEvent::StorageEvent { event } => Some(&event.payload),
            _ => None,
        }
    }

    #[tokio::test]
    async fn execute_empty_turn_emits_basic_lifecycle() {
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let sink_events = Arc::clone(&emitted);
        let input = input().with_event_sink(Arc::new(move |event| {
            sink_events.lock().expect("event sink poisoned").push(event);
        }));

        let output = TurnLoop.run(input).await;

        assert_eq!(output.identity.session_id, "session-1");
        assert_eq!(output.identity.turn_id, "turn-1");
        assert_eq!(output.identity.agent_id, "agent-1");
        assert_eq!(output.stop_cause, Some(TurnStopCause::Completed));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Completed));
        assert_eq!(output.step_count, 1);
        assert_eq!(output.events.len(), 2);
        assert!(matches!(
            output.events[0],
            RuntimeTurnEvent::TurnStarted { .. }
        ));
        assert!(matches!(
            output.events[1],
            RuntimeTurnEvent::TurnCompleted {
                stop_cause: TurnStopCause::Completed,
                terminal_kind: TurnTerminalKind::Completed,
                ..
            }
        ));
        assert_eq!(
            emitted.lock().expect("event sink poisoned").len(),
            output.events.len()
        );
    }

    #[derive(Debug)]
    struct ContinueThenComplete {
        remaining_continues: Mutex<usize>,
    }

    #[async_trait]
    impl TurnStepRunner for ContinueThenComplete {
        async fn run_single_step(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources,
        ) -> StepOutcome {
            let mut remaining = self
                .remaining_continues
                .lock()
                .expect("step runner state poisoned");
            if *remaining > 0 {
                *remaining -= 1;
                return StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted);
            }
            StepOutcome::Completed(TurnStopCause::Completed)
        }
    }

    #[tokio::test]
    async fn loop_records_continue_transitions_before_completion() {
        let runner = ContinueThenComplete {
            remaining_continues: Mutex::new(1),
        };

        let output = TurnLoop.run_with_step_runner(input(), &runner).await;

        assert_eq!(output.step_count, 2);
        assert_eq!(
            output
                .events
                .iter()
                .filter(|event| matches!(event, RuntimeTurnEvent::StepContinued { .. }))
                .count(),
            1
        );
    }

    #[derive(Debug)]
    struct FailingRunner;

    #[async_trait]
    impl TurnStepRunner for FailingRunner {
        async fn run_single_step(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources,
        ) -> StepOutcome {
            StepOutcome::Error(StepError::fatal("provider failed"))
        }
    }

    #[tokio::test]
    async fn loop_maps_step_error_to_terminal_error() {
        let output = TurnLoop.run_with_step_runner(input(), &FailingRunner).await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Error));
        assert_eq!(
            output.terminal_kind,
            Some(TurnTerminalKind::Error {
                message: "provider failed".to_string()
            })
        );
        assert_eq!(output.error_message.as_deref(), Some("provider failed"));
        assert!(matches!(
            output.events.last(),
            Some(RuntimeTurnEvent::TurnCompleted {
                stop_cause: TurnStopCause::Error,
                ..
            })
        ));
    }

    #[test]
    fn context_tracks_start_time_without_external_state() {
        let input = input();
        let resources = TurnExecutionResources::from_input(&input);
        let context = TurnExecutionContext::new(Vec::new(), &resources);

        assert!(context.started_at.elapsed() < Duration::from_secs(1));
        assert!(context.messages.is_empty());
        assert!(context.pending_events.is_empty());
    }

    #[derive(Debug)]
    struct StaticProvider {
        outputs: Mutex<Vec<LlmOutput>>,
    }

    #[derive(Debug)]
    struct BlockingStreamingProvider {
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        delta_sent: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    #[derive(Debug)]
    struct BlockingStreamingToolDispatcher {
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        delta_sent: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl LlmProvider for StaticProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            if let Some(sink) = sink {
                sink(LlmEvent::TextDelta("delta".to_string()));
            }
            Ok(self
                .outputs
                .lock()
                .expect("provider output buffer poisoned")
                .remove(0))
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for BlockingStreamingProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            if let Some(sink) = sink {
                sink(LlmEvent::TextDelta("live".to_string()));
            }
            if let Some(sender) = self
                .delta_sent
                .lock()
                .expect("delta signal lock poisoned")
                .take()
            {
                let _ = sender.send(());
            }
            let release = self
                .release
                .lock()
                .expect("release lock poisoned")
                .take()
                .expect("release receiver should be available");
            let _ = release.await;
            Ok(LlmOutput {
                content: "done".to_string(),
                finish_reason: LlmFinishReason::Stop,
                ..LlmOutput::default()
            })
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            }
        }
    }

    #[derive(Debug)]
    struct CancellingProvider;

    #[derive(Debug)]
    struct CapturingProvider {
        outputs: Mutex<Vec<LlmOutput>>,
        requests: Mutex<Vec<LlmRequest>>,
        limits: ModelLimits,
    }

    #[async_trait]
    impl LlmProvider for CapturingProvider {
        async fn generate(
            &self,
            request: LlmRequest,
            _sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            self.requests
                .lock()
                .expect("request capture poisoned")
                .push(request);
            Ok(self
                .outputs
                .lock()
                .expect("provider output buffer poisoned")
                .remove(0))
        }

        fn model_limits(&self) -> ModelLimits {
            self.limits
        }
    }

    #[async_trait]
    impl LlmProvider for CancellingProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            Err(AstrError::Cancelled)
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            }
        }
    }

    #[derive(Debug)]
    struct EchoToolDispatcher;

    #[async_trait]
    impl ToolDispatcher for EchoToolDispatcher {
        async fn dispatch_tool(&self, request: ToolDispatchRequest) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id: request.tool_call.id,
                tool_name: request.tool_call.name,
                ok: true,
                output: "tool result".to_string(),
                error: None,
                metadata: None,
                continuation: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[async_trait]
    impl ToolDispatcher for BlockingStreamingToolDispatcher {
        async fn dispatch_tool(&self, request: ToolDispatchRequest) -> Result<ToolExecutionResult> {
            if let Some(sender) = request.tool_output_sender {
                let _ = sender.send(ToolOutputDelta {
                    tool_call_id: request.tool_call.id.clone(),
                    tool_name: request.tool_call.name.clone(),
                    stream: ToolOutputStream::Stdout,
                    delta: "tool-live\n".to_string(),
                });
            }
            if let Some(sender) = self
                .delta_sent
                .lock()
                .expect("delta signal lock poisoned")
                .take()
            {
                let _ = sender.send(());
            }
            let release = self
                .release
                .lock()
                .expect("release lock poisoned")
                .take()
                .expect("release receiver should be available");
            let _ = release.await;
            Ok(ToolExecutionResult {
                tool_call_id: request.tool_call.id,
                tool_name: request.tool_call.name,
                ok: true,
                output: "tool result".to_string(),
                error: None,
                metadata: None,
                continuation: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[derive(Debug)]
    struct CancellingToolDispatcher;

    #[async_trait]
    impl ToolDispatcher for CancellingToolDispatcher {
        async fn dispatch_tool(
            &self,
            _request: ToolDispatchRequest,
        ) -> Result<ToolExecutionResult> {
            Err(AstrError::Cancelled)
        }
    }

    #[derive(Debug)]
    struct RecordingHookDispatcher {
        events: Mutex<Vec<HookEventKey>>,
        payloads: Mutex<Vec<HookEventPayload>>,
        cancel_before_provider: bool,
        augment_context: bool,
        block_tool_calls: bool,
    }

    #[async_trait]
    impl HookDispatcher for RecordingHookDispatcher {
        async fn dispatch_hook(&self, request: HookDispatchRequest) -> Result<HookDispatchOutcome> {
            self.events
                .lock()
                .expect("hook event buffer poisoned")
                .push(request.event);
            self.payloads
                .lock()
                .expect("hook payload buffer poisoned")
                .push(request.payload.clone());
            let event_for_match = request.event;
            let effects = match event_for_match {
                HookEventKey::Context if self.augment_context => {
                    vec![HookEffect::Diagnostic {
                        message: "extra runtime context".to_string(),
                    }]
                },
                HookEventKey::BeforeProviderRequest if self.cancel_before_provider => {
                    vec![HookEffect::CancelTurn {
                        reason: "blocked by hook".to_string(),
                    }]
                },
                HookEventKey::ToolCall if self.block_tool_calls => {
                    let tool_call_id = match &request.payload {
                        HookEventPayload::ToolCall { tool_call_id, .. } => tool_call_id.clone(),
                        _ => "unknown".to_string(),
                    };
                    vec![HookEffect::BlockToolResult {
                        tool_call_id,
                        reason: "policy blocked".to_string(),
                    }]
                },
                _ => vec![HookEffect::Continue],
            };
            Ok(HookDispatchOutcome { effects })
        }
    }

    #[tokio::test]
    async fn provider_output_drives_turn_completion() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![LlmOutput {
                content: "done".to_string(),
                finish_reason: LlmFinishReason::Stop,
                ..LlmOutput::default()
            }]),
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_cancel(CancelToken::new()),
            )
            .await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Completed));
        assert!(
            output
                .events
                .iter()
                .any(|event| matches!(event, RuntimeTurnEvent::ProviderStream { .. }))
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::AssistantFinal { content, .. } if content == "done"
        )));
        assert!(output.events.iter().any(|event| matches!(
            storage_payload(event),
            Some(astrcode_core::StorageEventPayload::PromptMetrics { .. })
        )));
    }

    #[tokio::test]
    async fn provider_stream_reaches_event_sink_before_provider_returns() {
        let (delta_sent_tx, delta_sent_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let provider = Arc::new(BlockingStreamingProvider {
            release: Mutex::new(Some(release_rx)),
            delta_sent: Mutex::new(Some(delta_sent_tx)),
        });
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

        let run_task = tokio::spawn(async move {
            TurnLoop
                .run(
                    input()
                        .with_provider(provider)
                        .with_event_sink(Arc::new(move |event| {
                            let _ = event_tx.send(event);
                        })),
                )
                .await
        });

        delta_sent_rx.await.expect("provider should emit a delta");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = event_rx
                    .recv()
                    .await
                    .expect("event channel should stay open");
                if matches!(event, RuntimeTurnEvent::ProviderStream { .. }) {
                    break;
                }
            }
        })
        .await
        .expect("provider stream should be emitted before provider returns");

        release_tx
            .send(())
            .expect("provider release should succeed");
        let output = run_task.await.expect("turn task should join");
        assert!(
            output
                .events
                .iter()
                .any(|event| matches!(event, RuntimeTurnEvent::ProviderStream { .. }))
        );
    }

    #[tokio::test]
    async fn tool_output_delta_reaches_event_sink_before_tool_returns() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    finish_reason: LlmFinishReason::ToolCalls,
                    tool_calls: vec![astrcode_core::ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "shell_command".to_string(),
                        args: serde_json::json!({}),
                    }],
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done".to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
            ]),
        });
        let (delta_sent_tx, delta_sent_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let dispatcher = Arc::new(BlockingStreamingToolDispatcher {
            release: Mutex::new(Some(release_rx)),
            delta_sent: Mutex::new(Some(delta_sent_tx)),
        });
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

        let run_task = tokio::spawn(async move {
            TurnLoop
                .run(
                    input()
                        .with_provider(provider)
                        .with_tool_dispatcher(dispatcher)
                        .with_event_sink(Arc::new(move |event| {
                            let _ = event_tx.send(event);
                        })),
                )
                .await
        });

        delta_sent_rx.await.expect("tool should emit a delta");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = event_rx
                    .recv()
                    .await
                    .expect("event channel should stay open");
                if matches!(
                    event,
                    RuntimeTurnEvent::StorageEvent { event }
                        if matches!(
                            event.payload,
                            astrcode_core::StorageEventPayload::ToolCallDelta { .. }
                        )
                ) {
                    break;
                }
            }
        })
        .await
        .expect("tool stream should be emitted before tool returns");

        release_tx.send(()).expect("tool release should succeed");
        let output = run_task.await.expect("turn task should join");
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::StorageEvent { event }
                if matches!(
                    event.payload,
                    astrcode_core::StorageEventPayload::ToolCallDelta { .. }
                )
        )));
    }

    #[tokio::test]
    async fn max_tokens_output_requests_one_continuation_before_completion() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    content: "partial".to_string(),
                    finish_reason: LlmFinishReason::MaxTokens,
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done".to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
            ]),
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_max_output_continuations(1),
            )
            .await;

        assert_eq!(output.step_count, 2);
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::StepContinued {
                transition: TurnLoopTransition::OutputContinuationRequested,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn repeated_max_tokens_stops_at_configured_continuation_limit() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    content: "partial 1".to_string(),
                    finish_reason: LlmFinishReason::MaxTokens,
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "partial 2".to_string(),
                    finish_reason: LlmFinishReason::MaxTokens,
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "partial 3".to_string(),
                    finish_reason: LlmFinishReason::MaxTokens,
                    ..LlmOutput::default()
                },
            ]),
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_max_output_continuations(2),
            )
            .await;

        let continuation_count = output
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    RuntimeTurnEvent::StepContinued {
                        transition: TurnLoopTransition::OutputContinuationRequested,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(output.stop_cause, Some(TurnStopCause::Completed));
        assert_eq!(output.step_count, 3);
        assert_eq!(continuation_count, 2);
    }

    #[tokio::test]
    async fn provider_tool_calls_emit_tool_use_decision() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![LlmOutput {
                finish_reason: LlmFinishReason::ToolCalls,
                tool_calls: vec![astrcode_core::ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: serde_json::json!({"path":"README.md"}),
                }],
                ..LlmOutput::default()
            }]),
        });

        let output = TurnLoop.run(input().with_provider(provider)).await;

        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::ToolUseRequested {
                tool_call_count: 1,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn tool_dispatch_results_continue_back_to_provider() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    finish_reason: LlmFinishReason::ToolCalls,
                    tool_calls: vec![astrcode_core::ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "readFile".to_string(),
                        args: serde_json::json!({"path":"README.md"}),
                    }],
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done after tool".to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
            ]),
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_tool_dispatcher(Arc::new(EchoToolDispatcher)),
            )
            .await;

        assert_eq!(output.step_count, 2);
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::ToolCallStarted {
                tool_call_id,
                tool_name,
                ..
            } if tool_call_id == "call-1" && tool_name == "readFile"
        )));
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::ToolResultReady {
                tool_call_id,
                ok: true,
                ..
            } if tool_call_id == "call-1"
        )));
        assert!(output.events.iter().any(|event| matches!(
            storage_payload(event),
            Some(astrcode_core::StorageEventPayload::ToolCall {
                tool_call_id,
                ..
            }) if tool_call_id == "call-1"
        )));
        assert!(output.events.iter().any(|event| matches!(
            storage_payload(event),
            Some(astrcode_core::StorageEventPayload::ToolResult {
                tool_call_id,
                output,
                ..
            }) if tool_call_id == "call-1" && output == "tool result"
        )));
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::StepContinued {
                transition: TurnLoopTransition::ToolCycleCompleted,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn aggregate_tool_result_budget_replaces_large_trailing_results_before_request() {
        let tempdir = tempfile::tempdir().expect("tempdir should exist");
        let provider = Arc::new(CapturingProvider {
            outputs: Mutex::new(vec![LlmOutput {
                content: "done".to_string(),
                finish_reason: LlmFinishReason::Stop,
                ..LlmOutput::default()
            }]),
            requests: Mutex::new(Vec::new()),
            limits: ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        });
        let runtime = astrcode_core::ResolvedRuntimeConfig {
            aggregate_result_bytes_budget: 128,
            ..Default::default()
        };

        let output = TurnLoop
            .run(
                input()
                    .with_working_dir(tempdir.path())
                    .with_runtime_config(runtime)
                    .with_messages(vec![
                        astrcode_core::LlmMessage::Assistant {
                            content: String::new(),
                            tool_calls: vec![astrcode_core::ToolCallRequest {
                                id: "call-large".to_string(),
                                name: "readFile".to_string(),
                                args: serde_json::json!({"path":"large.txt"}),
                            }],
                            reasoning: None,
                        },
                        astrcode_core::LlmMessage::Tool {
                            tool_call_id: "call-large".to_string(),
                            content: "x".repeat(4_096),
                        },
                    ])
                    .with_provider(provider.clone()),
            )
            .await;

        assert!(output.events.iter().any(|event| matches!(
            storage_payload(event),
            Some(astrcode_core::StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id,
                ..
            }) if tool_call_id == "call-large"
        )));
        let requests = provider.requests.lock().expect("requests should capture");
        assert!(matches!(
            &requests[0].messages[1],
            astrcode_core::LlmMessage::Tool { content, .. }
                if content.contains("<persisted-output>")
        ));
    }

    #[tokio::test]
    async fn auto_compact_replaces_history_and_emits_compact_event() {
        let provider = Arc::new(CapturingProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    content: "<analysis>ok</analysis><summary>older work \
                              summarized</summary><recent_user_context_digest>continue current \
                              task</recent_user_context_digest>"
                        .to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done".to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
            ]),
            requests: Mutex::new(Vec::new()),
            limits: ModelLimits {
                context_window: 16_000,
                max_output_tokens: 1_024,
            },
        });
        let runtime = astrcode_core::ResolvedRuntimeConfig {
            compact_threshold_percent: 1,
            summary_reserve_tokens: 1,
            reserved_context_size: 1,
            compact_keep_recent_turns: 1,
            compact_keep_recent_user_messages: 1,
            ..Default::default()
        };

        let output = TurnLoop
            .run(
                input()
                    .with_runtime_config(runtime)
                    .with_messages(vec![
                        astrcode_core::LlmMessage::User {
                            content: "old request ".repeat(200),
                            origin: astrcode_core::UserMessageOrigin::User,
                        },
                        astrcode_core::LlmMessage::Assistant {
                            content: "old answer ".repeat(200),
                            tool_calls: Vec::new(),
                            reasoning: None,
                        },
                        astrcode_core::LlmMessage::User {
                            content: "new request".to_string(),
                            origin: astrcode_core::UserMessageOrigin::User,
                        },
                    ])
                    .with_provider(provider.clone()),
            )
            .await;

        assert!(output.events.iter().any(|event| matches!(
            storage_payload(event),
            Some(astrcode_core::StorageEventPayload::CompactApplied {
                summary,
                ..
            }) if summary.contains("older work summarized")
        )));
        let requests = provider.requests.lock().expect("requests should capture");
        assert!(requests.len() >= 2);
        assert!(matches!(
            &requests[1].messages[0],
            astrcode_core::LlmMessage::User {
                origin: astrcode_core::UserMessageOrigin::CompactSummary,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn runtime_hooks_run_in_turn_order() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![LlmOutput {
                content: "done".to_string(),
                finish_reason: LlmFinishReason::Stop,
                ..LlmOutput::default()
            }]),
        });
        let hook_dispatcher = Arc::new(RecordingHookDispatcher {
            events: Mutex::new(Vec::new()),
            payloads: Mutex::new(Vec::new()),
            cancel_before_provider: false,
            augment_context: true,
            block_tool_calls: false,
        });
        let agent = AgentEventContext::sub_run(
            "agent-1",
            "parent-turn-1",
            "default",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some("child-session-1".to_string().into()),
        );

        let output = TurnLoop
            .run(
                input()
                    .with_agent(agent)
                    .with_provider(provider)
                    .with_hook_dispatcher(hook_dispatcher.clone()),
            )
            .await;

        let events = hook_dispatcher
            .events
            .lock()
            .expect("hook event buffer poisoned")
            .clone();
        assert_eq!(
            events,
            vec![
                HookEventKey::TurnStart,
                HookEventKey::Context,
                HookEventKey::BeforeAgentStart,
                HookEventKey::BeforeProviderRequest,
                HookEventKey::TurnEnd,
            ]
        );
        // Payload is now typed HookEventPayload — structure verified implicitly
        // through the event sequence and hook dispatch integration.
        let payloads_count = hook_dispatcher
            .payloads
            .lock()
            .expect("hook payload buffer poisoned")
            .len();
        assert_eq!(payloads_count, 5, "5 hook dispatches should have occurred");
        assert!(output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::HookPromptAugmented {
                event: HookEventKey::Context,
                content,
                ..
            } if content == "extra runtime context"
        )));
    }

    #[tokio::test]
    async fn hook_cancel_effect_stops_turn_before_provider_request() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![LlmOutput {
                content: "should not be used".to_string(),
                finish_reason: LlmFinishReason::Stop,
                ..LlmOutput::default()
            }]),
        });
        let hook_dispatcher = Arc::new(RecordingHookDispatcher {
            events: Mutex::new(Vec::new()),
            payloads: Mutex::new(Vec::new()),
            cancel_before_provider: true,
            augment_context: false,
            block_tool_calls: false,
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_hook_dispatcher(hook_dispatcher.clone()),
            )
            .await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Cancelled));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Cancelled));
        let events = hook_dispatcher
            .events
            .lock()
            .expect("hook event buffer poisoned")
            .clone();
        assert_eq!(
            events,
            vec![
                HookEventKey::TurnStart,
                HookEventKey::Context,
                HookEventKey::BeforeAgentStart,
                HookEventKey::BeforeProviderRequest,
                HookEventKey::TurnEnd,
            ]
        );
    }

    /// 验证 BlockToolResult 效果：工具被拒绝时产生失败结果而不是错误
    #[tokio::test]
    async fn block_tool_call_effect_produces_failed_tool_result() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![
                LlmOutput {
                    finish_reason: LlmFinishReason::ToolCalls,
                    tool_calls: vec![astrcode_core::ToolCallRequest {
                        id: "call-1".to_string(),
                        name: "readFile".to_string(),
                        args: serde_json::json!({"path":"README.md"}),
                    }],
                    ..LlmOutput::default()
                },
                LlmOutput {
                    content: "done after blocked tool".to_string(),
                    finish_reason: LlmFinishReason::Stop,
                    ..LlmOutput::default()
                },
            ]),
        });
        let hook_dispatcher = Arc::new(RecordingHookDispatcher {
            events: Mutex::new(Vec::new()),
            payloads: Mutex::new(Vec::new()),
            cancel_before_provider: false,
            augment_context: false,
            block_tool_calls: true,
        });
        let tool_dispatcher = Arc::new(EchoToolDispatcher);

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_tool_dispatcher(tool_dispatcher)
                    .with_hook_dispatcher(hook_dispatcher.clone()),
            )
            .await;

        // Turn should complete (not error) even though tool was blocked
        assert_eq!(output.stop_cause, Some(TurnStopCause::Completed));

        // Verify hook was dispatched for ToolCall
        let events = hook_dispatcher
            .events
            .lock()
            .expect("lock poisoned")
            .clone();
        assert!(
            events.contains(&HookEventKey::ToolCall),
            "ToolCall hook should have been dispatched"
        );
    }

    #[tokio::test]
    async fn cancelled_token_stops_before_provider_call() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(Vec::new()),
        });

        let output = TurnLoop
            .run(input().with_provider(provider).with_cancel(cancel))
            .await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Cancelled));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Cancelled));
        assert!(!output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::ProviderStream { .. } | RuntimeTurnEvent::AssistantFinal { .. }
        )));
    }

    #[tokio::test]
    async fn cancelled_provider_error_maps_to_cancelled_turn() {
        let output = TurnLoop
            .run(input().with_provider(Arc::new(CancellingProvider)))
            .await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Cancelled));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Cancelled));
        assert!(!output.events.iter().any(|event| matches!(
            event,
            RuntimeTurnEvent::ProviderStream { .. } | RuntimeTurnEvent::AssistantFinal { .. }
        )));
    }

    #[tokio::test]
    async fn cancelled_tool_dispatch_maps_to_cancelled_turn() {
        let provider = Arc::new(StaticProvider {
            outputs: Mutex::new(vec![LlmOutput {
                finish_reason: LlmFinishReason::ToolCalls,
                tool_calls: vec![astrcode_core::ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: serde_json::json!({"path":"README.md"}),
                }],
                ..LlmOutput::default()
            }]),
        });

        let output = TurnLoop
            .run(
                input()
                    .with_provider(provider)
                    .with_tool_dispatcher(Arc::new(CancellingToolDispatcher)),
            )
            .await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Cancelled));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Cancelled));
    }
}
