use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, CapabilityKind, LlmFinishReason, LlmMessage,
    LlmOutput, LlmRequest, LlmUsage, PromptFactsProvider, ResolvedRuntimeConfig,
    StorageEventPayload, Tool, ToolCallRequest, ToolContext, ToolDefinition, ToolExecutionResult,
    UserMessageOrigin,
};
use astrcode_kernel::KernelGateway;
use async_trait::async_trait;
use serde_json::json;

use super::{
    StepOutcome, TurnExecutionContext, TurnExecutionResources,
    driver::StepDriver,
    run_single_step_with,
    streaming_tools::{
        StreamingToolAssembly, StreamingToolFallbackReason, fallback_reason_for_final_call,
    },
};
use crate::{
    SessionState,
    context_window::token_usage::PromptTokenSnapshot,
    turn::{
        compaction_cycle,
        events::{prompt_metrics_event, tool_call_event, tool_result_event},
        llm_cycle::{StreamedToolCallDelta, ToolCallDeltaSink},
        loop_control::{AUTO_CONTINUE_NUDGE, TurnLoopTransition, TurnStopCause},
        request::AssemblePromptResult,
        runner::TurnExecutionRequestView,
        test_support::{
            NoopPromptFactsProvider, assert_contains_compact_summary, assert_has_assistant_final,
            assert_has_turn_done, root_compact_applied_event, test_gateway, test_session_state,
        },
        tool_cycle::{ToolCycleOutcome, ToolCycleResult, ToolEventEmissionMode},
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
    assemble_result: Mutex<Option<astrcode_core::Result<AssemblePromptResult>>>,
    llm_result: Mutex<Option<astrcode_core::Result<LlmOutput>>>,
    reactive_compact_result:
        Mutex<Option<astrcode_core::Result<Option<compaction_cycle::RecoveryResult>>>>,
    tool_cycle_result: Mutex<Option<astrcode_core::Result<ToolCycleResult>>>,
}

#[async_trait]
impl StepDriver for ScriptedStepDriver {
    async fn assemble_prompt(
        &self,
        _execution: &mut TurnExecutionContext,
        _resources: &TurnExecutionResources<'_>,
    ) -> astrcode_core::Result<AssemblePromptResult> {
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
    ) -> astrcode_core::Result<LlmOutput> {
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
    ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
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
    ) -> astrcode_core::Result<ToolCycleResult> {
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
        tool_result_budget_stats: crate::turn::tool_result_budget::ToolResultBudgetStats::default(),
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
    ) -> std::result::Result<astrcode_core::CapabilitySpec, astrcode_core::CapabilitySpecBuildError>
    {
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
    ) -> astrcode_core::Result<ToolExecutionResult> {
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

#[derive(Debug)]
struct StreamingUnsafeProbeTool;

#[async_trait]
impl Tool for StreamingUnsafeProbeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "streaming_unsafe_probe".to_string(),
            description: "unsafe probe for streamed tool execution".to_string(),
            parameters: json!({"type": "object"}),
        }
    }

    fn capability_spec(
        &self,
    ) -> std::result::Result<astrcode_core::CapabilitySpec, astrcode_core::CapabilitySpecBuildError>
    {
        astrcode_core::CapabilitySpec::builder("streaming_unsafe_probe", CapabilityKind::Tool)
            .description("unsafe probe for streamed tool execution")
            .schema(json!({"type": "object"}), json!({"type": "string"}))
            .concurrency_safe(false)
            .build()
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "streaming_unsafe_probe".to_string(),
            ok: true,
            output: "unsafe result".to_string(),
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

fn tool_result(tool_call_id: &str, tool_name: &str, output: &str) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: tool_name.to_string(),
        ok: true,
        output: output.to_string(),
        error: None,
        metadata: None,
        duration_ms: 0,
        truncated: false,
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
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message("find the answer")]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
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
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
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
        }) if content == crate::turn::continuation_cycle::OUTPUT_CONTINUATION_PROMPT
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

#[tokio::test]
async fn run_single_step_does_not_launch_non_concurrency_safe_streaming_tool() {
    struct UnsafeStreamingDriver {
        tool_cycle_calls: AtomicUsize,
        event_modes: Mutex<Vec<ToolEventEmissionMode>>,
    }

    #[async_trait]
    impl StepDriver for UnsafeStreamingDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message(
                "find the unsafe answer",
            )]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            if let Some(sink) = tool_delta_sink {
                sink(StreamedToolCallDelta {
                    index: 0,
                    id: Some("call-unsafe-1".to_string()),
                    name: Some("streaming_unsafe_probe".to_string()),
                    arguments_delta: r#"{"path":"README.md"}"#.to_string(),
                });
            }
            Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-unsafe-1".to_string(),
                    name: "streaming_unsafe_probe".to_string(),
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
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
            event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
            self.tool_cycle_calls.fetch_add(1, Ordering::SeqCst);
            self.event_modes
                .lock()
                .expect("event mode lock should work")
                .push(event_emission_mode);
            Ok(ToolCycleResult {
                outcome: ToolCycleOutcome::Completed,
                tool_messages: Vec::new(),
                raw_results: Vec::new(),
                events: Vec::new(),
            })
        }
    }

    let kernel =
        crate::turn::test_support::test_kernel_with_tool(Arc::new(StreamingUnsafeProbeTool), 8192);
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
    let driver = UnsafeStreamingDriver {
        tool_cycle_calls: AtomicUsize::new(0),
        event_modes: Mutex::new(Vec::new()),
    };

    let outcome = run_single_step_with(&mut execution, &resources, &driver)
        .await
        .expect("step should succeed");

    assert!(matches!(
        outcome,
        StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted)
    ));
    assert_eq!(execution.streaming_tool_launch_count, 0);
    assert_eq!(driver.tool_cycle_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        driver
            .event_modes
            .lock()
            .expect("event mode lock should work")
            .as_slice(),
        &[ToolEventEmissionMode::Immediate]
    );
}

#[tokio::test]
async fn run_single_step_discards_provisional_tool_when_final_plan_changes() {
    struct FinalPlanChangedDriver {
        tool_cycle_calls: AtomicUsize,
        captured_calls: Mutex<Vec<Vec<ToolCallRequest>>>,
    }

    #[async_trait]
    impl StepDriver for FinalPlanChangedDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message("find the answer")]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
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
                    args: json!({"path": "src/main.rs"}),
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
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
            self.tool_cycle_calls.fetch_add(1, Ordering::SeqCst);
            self.captured_calls
                .lock()
                .expect("captured calls lock should work")
                .push(tool_calls.clone());
            let result = tool_result(
                tool_calls[0].id.as_str(),
                tool_calls[0].name.as_str(),
                "fallback final result",
            );
            Ok(ToolCycleResult {
                outcome: ToolCycleOutcome::Completed,
                tool_messages: Vec::new(),
                raw_results: vec![(tool_calls[0].clone(), result)],
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
    let driver = FinalPlanChangedDriver {
        tool_cycle_calls: AtomicUsize::new(0),
        captured_calls: Mutex::new(Vec::new()),
    };

    let outcome = run_single_step_with(&mut execution, &resources, &driver)
        .await
        .expect("step should succeed");

    assert!(matches!(
        outcome,
        StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted)
    ));
    assert_eq!(execution.streaming_tool_launch_count, 1);
    assert_eq!(driver.tool_cycle_calls.load(Ordering::SeqCst), 1);
    let captured_calls = driver
        .captured_calls
        .lock()
        .expect("captured calls lock should work");
    assert_eq!(captured_calls.len(), 1);
    assert_eq!(captured_calls[0].len(), 1);
    assert_eq!(captured_calls[0][0].args, json!({"path": "src/main.rs"}));
    assert_eq!(execution.streaming_tool_discard_count, 1);
    assert_eq!(execution.streaming_tool_fallback_count, 1);
    assert!(
        execution.messages.iter().any(|message| matches!(
            message,
            LlmMessage::Tool { tool_call_id, content }
                if tool_call_id == "call-stream-1" && content == "fallback final result"
        )),
        "final fallback path should append exactly one final tool result"
    );
}

#[tokio::test]
async fn run_single_step_merges_buffered_events_and_results_in_final_tool_order() {
    struct MergeOrderingDriver;

    #[async_trait]
    impl StepDriver for MergeOrderingDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message("find the answer")]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            if let Some(sink) = tool_delta_sink {
                sink(StreamedToolCallDelta {
                    index: 1,
                    id: Some("call-stream-2".to_string()),
                    name: Some("streaming_safe_probe".to_string()),
                    arguments_delta: r#"{"path":"README.md"}"#.to_string(),
                });
            }
            Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-remain-1".to_string(),
                        name: "dummy_tool".to_string(),
                        args: json!({"query": "alpha"}),
                    },
                    ToolCallRequest {
                        id: "call-stream-2".to_string(),
                        name: "streaming_safe_probe".to_string(),
                        args: json!({"path": "README.md"}),
                    },
                ],
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::ToolCalls,
            })
        }

        async fn try_reactive_compact(
            &self,
            _execution: &TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            resources: &TurnExecutionResources<'_>,
            tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
            let result = tool_result(
                tool_calls[0].id.as_str(),
                tool_calls[0].name.as_str(),
                "remaining result",
            );
            Ok(ToolCycleResult {
                outcome: ToolCycleOutcome::Completed,
                tool_messages: Vec::new(),
                raw_results: vec![(tool_calls[0].clone(), result.clone())],
                events: vec![
                    tool_call_event(resources.turn_id, resources.agent, &tool_calls[0]),
                    tool_result_event(resources.turn_id, resources.agent, &result),
                ],
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

    let outcome = run_single_step_with(&mut execution, &resources, &MergeOrderingDriver)
        .await
        .expect("step should succeed");

    assert!(matches!(
        outcome,
        StepOutcome::Continue(TurnLoopTransition::ToolCycleCompleted)
    ));

    let tool_messages = execution
        .messages
        .iter()
        .filter_map(|message| match message {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => Some((tool_call_id.clone(), content.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tool_messages,
        vec![
            ("call-remain-1".to_string(), "remaining result".to_string()),
            (
                "call-stream-2".to_string(),
                "streamed safe result".to_string()
            ),
        ]
    );

    let tool_event_ids = execution
        .events
        .iter()
        .filter_map(|event| match &event.payload {
            StorageEventPayload::ToolCall { tool_call_id, .. }
            | StorageEventPayload::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tool_event_ids,
        vec![
            "call-remain-1".to_string(),
            "call-remain-1".to_string(),
            "call-stream-2".to_string(),
            "call-stream-2".to_string(),
        ]
    );
}

#[cfg(not(debug_assertions))]
#[tokio::test]
async fn run_single_step_returns_internal_error_when_buffered_merge_loses_tool_result() {
    struct MissingResultDriver;

    #[async_trait]
    impl StepDriver for MissingResultDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message("find the answer")]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            if let Some(sink) = tool_delta_sink {
                sink(StreamedToolCallDelta {
                    index: 1,
                    id: Some("call-stream-2".to_string()),
                    name: Some("streaming_safe_probe".to_string()),
                    arguments_delta: r#"{"path":"README.md"}"#.to_string(),
                });
            }
            Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-remain-1".to_string(),
                        name: "dummy_tool".to_string(),
                        args: json!({"query": "alpha"}),
                    },
                    ToolCallRequest {
                        id: "call-stream-2".to_string(),
                        name: "streaming_safe_probe".to_string(),
                        args: json!({"path": "README.md"}),
                    },
                ],
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::ToolCalls,
            })
        }

        async fn try_reactive_compact(
            &self,
            _execution: &TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
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

    let error = match run_single_step_with(&mut execution, &resources, &MissingResultDriver).await {
        Ok(_) => panic!("missing remaining tool result should fail fast"),
        Err(error) => error,
    };

    assert!(matches!(error, AstrError::Internal(message) if message.contains("call-remain-1")));
}

#[cfg(debug_assertions)]
#[tokio::test]
#[should_panic(expected = "merge dropped tool calls")]
async fn run_single_step_panics_when_buffered_merge_loses_tool_result_in_debug() {
    struct MissingResultDriver;

    #[async_trait]
    impl StepDriver for MissingResultDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<AssemblePromptResult> {
            Ok(assembled_prompt(vec![user_message("find the answer")]))
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
            tool_delta_sink: Option<ToolCallDeltaSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            if let Some(sink) = tool_delta_sink {
                sink(StreamedToolCallDelta {
                    index: 1,
                    id: Some("call-stream-2".to_string()),
                    name: Some("streaming_safe_probe".to_string()),
                    arguments_delta: r#"{"path":"README.md"}"#.to_string(),
                });
            }
            Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-remain-1".to_string(),
                        name: "dummy_tool".to_string(),
                        args: json!({"query": "alpha"}),
                    },
                    ToolCallRequest {
                        id: "call-stream-2".to_string(),
                        name: "streaming_safe_probe".to_string(),
                        args: json!({"path": "README.md"}),
                    },
                ],
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::ToolCalls,
            })
        }

        async fn try_reactive_compact(
            &self,
            _execution: &TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> astrcode_core::Result<Option<compaction_cycle::RecoveryResult>> {
            Ok(None)
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
            _event_emission_mode: ToolEventEmissionMode,
        ) -> astrcode_core::Result<ToolCycleResult> {
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

    let _ = run_single_step_with(&mut execution, &resources, &MissingResultDriver).await;
}

#[test]
fn fallback_reason_reports_identity_never_stabilized() {
    let kernel = crate::turn::test_support::test_kernel_with_tool(
        Arc::new(StreamingSafeProbeTool {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        8192,
    );
    let call = ToolCallRequest {
        id: "call-1".to_string(),
        name: "streaming_safe_probe".to_string(),
        args: json!({"path": "README.md"}),
    };
    let assembly = StreamingToolAssembly::for_test(
        Some("other-call".to_string()),
        Some("streaming_safe_probe".to_string()),
        r#"{"path":"README.md"}"#,
    );

    assert_eq!(
        fallback_reason_for_final_call(Some(kernel.gateway()), Some(&assembly), &call),
        Some(StreamingToolFallbackReason::IdentityNeverStabilized)
    );
}

#[test]
fn fallback_reason_reports_unstable_json_payload() {
    let kernel = crate::turn::test_support::test_kernel_with_tool(
        Arc::new(StreamingSafeProbeTool {
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        8192,
    );
    let call = ToolCallRequest {
        id: "call-1".to_string(),
        name: "streaming_safe_probe".to_string(),
        args: json!({"path": "README.md"}),
    };
    let assembly = StreamingToolAssembly::for_test(
        Some("call-1".to_string()),
        Some("streaming_safe_probe".to_string()),
        r#"{"path":"README.md""#,
    );

    assert_eq!(
        fallback_reason_for_final_call(Some(kernel.gateway()), Some(&assembly), &call),
        Some(StreamingToolFallbackReason::ArgumentsNeverFormedStableJson)
    );
}
