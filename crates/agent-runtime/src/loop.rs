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
    hook_dispatch::{HookDispatchRequest, HookDispatcher, HookEffectKind},
    provider::{LlmEventSink, LlmOutput, LlmProvider},
    tool_dispatch::{ToolDispatchRequest, ToolDispatcher},
    types::{
        RuntimeEventSink, RuntimeTurnEvent, StepError, TurnInput, TurnLoopTransition, TurnOutput,
        TurnStopCause,
    },
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
#[derive(Clone)]
pub struct TurnExecutionResources {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub agent: AgentEventContext,
    pub model_ref: String,
    pub provider_ref: String,
    pub hook_snapshot_id: String,
    pub tool_count: usize,
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
    fn turn_identity(&self) -> crate::types::TurnIdentity {
        crate::types::TurnIdentity::new(
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
            tool_count: surface.tool_specs.len(),
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
    for event in [
        HookEventKey::Context,
        HookEventKey::BeforeAgentStart,
        HookEventKey::BeforeProviderRequest,
    ] {
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

    let request = match assemble_runtime_request(execution, resources).await {
        Ok(request) => request,
        Err(error) if error.is_cancelled() => {
            return StepOutcome::Completed(TurnStopCause::Cancelled);
        },
        Err(error) => return StepOutcome::Error(StepError::from(&error)),
    };

    let output = match provider.generate(request, Some(sink)).await {
        Ok(output) => output,
        Err(error) if error.is_cancelled() => {
            return StepOutcome::Completed(TurnStopCause::Cancelled);
        },
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

    record_provider_output(execution, resources, &output);
    apply_prompt_metrics_usage(
        &mut execution.pending_events,
        execution.step_index,
        output.usage,
        output.prompt_cache_diagnostics.clone(),
    );
    execution.token_tracker.record_usage(output.usage);

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
        execution.push_event(RuntimeTurnEvent::ToolUseRequested {
            identity: resources.turn_identity(),
            tool_call_count: output.tool_calls.len(),
        });
        return StepOutcome::Completed(TurnStopCause::Completed);
    }

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
            payload: serde_json::json!({
                "agent": resources.agent.clone(),
                "stepIndex": execution.step_index,
                "messageCount": execution.messages.len(),
            }),
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
        match effect.kind {
            HookEffectKind::Continue | HookEffectKind::Diagnostic => {},
            HookEffectKind::AugmentPrompt => {
                let content = effect.message.unwrap_or_default();
                execution.messages.push(LlmMessage::User {
                    content: content.clone(),
                    origin: UserMessageOrigin::ReactivationPrompt,
                });
                execution.push_event(RuntimeTurnEvent::HookPromptAugmented {
                    identity: resources.turn_identity(),
                    event,
                    content,
                });
            },
            HookEffectKind::CancelTurn => {
                return Some(StepOutcome::Completed(TurnStopCause::Cancelled));
            },
            HookEffectKind::Block => {
                return Some(StepOutcome::Error(StepError::fatal(
                    effect
                        .message
                        .unwrap_or_else(|| "hook blocked execution".to_string()),
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
    let (tool_output_sender, mut tool_output_receiver) =
        tokio::sync::mpsc::unbounded_channel::<ToolOutputDelta>();

    // Phase 1: Pre-dispatch hooks and event emissions (sequential, order-preserving)
    let mut futures = Vec::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        if resources.cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }
        if let Some(outcome) =
            dispatch_runtime_hook(execution, resources, HookEventKey::ToolCall).await
        {
            return Err(step_outcome_to_error(outcome));
        }
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
                        args: tool_call.args.clone(),
                    },
                }),
            },
        );
        futures.push(dispatcher.dispatch_tool(ToolDispatchRequest {
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            agent_id: resources.agent_id.clone(),
            tool_call: tool_call.clone(),
            tool_output_sender: Some(tool_output_sender.clone()),
        }));
    }
    drop(tool_output_sender);

    // Phase 2: Execute all tool dispatches concurrently (I/O-bound parallelism)
    let mut results_future = Box::pin(futures_util::future::join_all(futures));
    let results = loop {
        tokio::select! {
            Some(delta) = tool_output_receiver.recv() => {
                record_tool_output_delta(execution, resources, delta);
            },
            results = &mut results_future => {
                while let Ok(delta) = tool_output_receiver.try_recv() {
                    record_tool_output_delta(execution, resources, delta);
                }
                break results;
            },
        }
    };

    // Phase 3: Process results in order (sequential, preserving message ordering)
    for (tool_call, result) in tool_calls.iter().zip(results) {
        let result = match result {
            Ok(r) => r,
            Err(e) if e.is_cancelled() => return Err(AstrError::Cancelled),
            Err(e) => return Err(e),
        };
        record_tool_result(execution, resources, tool_call, result);
        if let Some(outcome) =
            dispatch_runtime_hook(execution, resources, HookEventKey::ToolResult).await
        {
            return Err(step_outcome_to_error(outcome));
        }
    }
    Ok(())
}

fn step_outcome_to_error(outcome: StepOutcome) -> AstrError {
    match outcome {
        StepOutcome::Error(step_error) => AstrError::Internal(step_error.message),
        StepOutcome::Completed(TurnStopCause::Cancelled) => AstrError::Cancelled,
        StepOutcome::Completed(stop_cause) => {
            AstrError::Internal(format!("hook terminated tool dispatch with {stop_cause:?}"))
        },
        StepOutcome::Continue(transition) => {
            AstrError::Internal(format!("hook unexpectedly requested {transition:?}"))
        },
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

fn flush_pending_events(
    event_sink: &dyn crate::types::RuntimeEventSink,
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
    event_sink: &dyn crate::types::RuntimeEventSink,
    identity: &crate::types::TurnIdentity,
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
        ToolExecutionResult, TurnTerminalKind,
    };
    use async_trait::async_trait;

    use super::{
        StepOutcome, TurnExecutionContext, TurnExecutionResources, TurnLoop, TurnStepRunner,
    };
    use crate::{
        hook_dispatch::{HookDispatchOutcome, HookDispatchRequest, HookDispatcher, HookEffect},
        provider::{
            LlmEventSink, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest, ModelLimits,
        },
        tool_dispatch::{ToolDispatchRequest, ToolDispatcher},
        types::{
            AgentRuntimeExecutionSurface, RuntimeTurnEvent, StepError, TurnInput,
            TurnLoopTransition, TurnStopCause,
        },
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
                sink(crate::provider::LlmEvent::TextDelta("delta".to_string()));
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
                sink(crate::provider::LlmEvent::TextDelta("live".to_string()));
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
                let _ = sender.send(astrcode_core::ToolOutputDelta {
                    tool_call_id: request.tool_call.id.clone(),
                    tool_name: request.tool_call.name.clone(),
                    stream: astrcode_core::ToolOutputStream::Stdout,
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
        payloads: Mutex<Vec<serde_json::Value>>,
        cancel_before_provider: bool,
        augment_context: bool,
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
                .push(request.payload);
            let effects = match request.event {
                HookEventKey::Context if self.augment_context => {
                    vec![HookEffect::augment_prompt("extra runtime context")]
                },
                HookEventKey::BeforeProviderRequest if self.cancel_before_provider => {
                    vec![HookEffect::cancel_turn("blocked by hook")]
                },
                _ => vec![HookEffect::continue_flow()],
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
        let payloads = hook_dispatcher
            .payloads
            .lock()
            .expect("hook payload buffer poisoned");
        assert_eq!(
            payloads[0]["agent"]["childSessionId"],
            serde_json::json!("child-session-1")
        );
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
