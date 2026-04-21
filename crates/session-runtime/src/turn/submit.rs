use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, ApprovalPending, CancelToken, CapabilityCall, EventStore, EventTranslator,
    ExecutionAccepted, LlmMessage, Phase, PolicyContext, PromptDeclaration,
    ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
    Result, RuntimeMetricsRecorder, SessionId, TurnId, UserMessageOrigin,
};
use astrcode_kernel::CapabilityRouter;
use chrono::Utc;

use crate::{
    SessionRuntime,
    actor::SessionActor,
    run_turn,
    turn::{
        branch::SubmitTarget,
        events::user_message_event,
        finalize::{
            persist_pending_manual_compact_if_any, persist_turn_events, persist_turn_failure,
        },
        subrun_events::subrun_started_event,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitBusyPolicy {
    BranchOnBusy,
    RejectOnBusy,
}

struct SubmitPromptRequest {
    session_id: String,
    turn_id: Option<TurnId>,
    live_user_input: Option<String>,
    queued_inputs: Vec<String>,
    runtime: ResolvedRuntimeConfig,
    busy_policy: SubmitBusyPolicy,
    submission: AgentPromptSubmission,
}

struct TurnExecutionTask {
    kernel: Arc<astrcode_kernel::Kernel>,
    request: crate::turn::RunnerRequest,
    finalize: TurnFinalizeContext,
}

struct TurnCoordinator {
    kernel: Arc<astrcode_kernel::Kernel>,
    prompt_facts_provider: Arc<dyn astrcode_core::PromptFactsProvider>,
    event_store: Arc<dyn EventStore>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    submit_target: SubmitTarget,
    turn_id: TurnId,
    runtime: ResolvedRuntimeConfig,
    live_user_input: Option<String>,
    queued_inputs: Vec<String>,
    submission: AgentPromptSubmission,
}

#[derive(Clone, Default)]
pub struct AgentPromptSubmission {
    pub agent: AgentEventContext,
    pub capability_router: Option<CapabilityRouter>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub injected_messages: Vec<LlmMessage>,
    pub source_tool_call_id: Option<String>,
    pub policy_context: Option<PolicyContext>,
    pub governance_revision: Option<String>,
    pub approval: Option<Box<ApprovalPending<CapabilityCall>>>,
    pub prompt_governance: Option<astrcode_core::PromptGovernanceContext>,
}

#[derive(Debug, Clone)]
struct PersistedTurnContext {
    turn_id: String,
    agent: AgentEventContext,
    source_tool_call_id: Option<String>,
}

struct TurnFinalizeContext {
    kernel: Arc<astrcode_kernel::Kernel>,
    prompt_facts_provider: Arc<dyn astrcode_core::PromptFactsProvider>,
    event_store: Arc<dyn EventStore>,
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    actor: Arc<SessionActor>,
    session_id: String,
    turn_started_at: Instant,
    generation: u64,
    persisted: PersistedTurnContext,
}

impl TurnCoordinator {
    async fn start(self) -> Result<ExecutionAccepted> {
        let accepted = self.accepted();
        let task = self.prepare().await?;
        tokio::spawn(execute_turn_and_finalize(task));
        Ok(accepted)
    }

    fn accepted(&self) -> ExecutionAccepted {
        ExecutionAccepted {
            session_id: self.submit_target.session_id.clone(),
            turn_id: self.turn_id.clone(),
            agent_id: None,
            branched_from_session_id: self.submit_target.branched_from_session_id.clone(),
        }
    }

    async fn prepare(self) -> Result<TurnExecutionTask> {
        let Self {
            kernel,
            prompt_facts_provider,
            event_store,
            metrics,
            submit_target,
            turn_id,
            runtime,
            live_user_input,
            queued_inputs,
            submission,
        } = self;
        let cancel = CancelToken::new();
        let generation = submit_target.actor.state().prepare_execution(
            submit_target.session_id.as_str(),
            turn_id.as_str(),
            cancel.clone(),
            submit_target.turn_lease,
        )?;

        let prepared = prepare_turn_submission(
            submit_target.actor.state(),
            turn_id.as_str(),
            live_user_input,
            queued_inputs,
            submission,
        )
        .await;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = submit_target.actor.state().force_complete_execution_state();
                return Err(error);
            },
        };

        Ok(TurnExecutionTask {
            kernel: Arc::clone(&kernel),
            request: crate::turn::RunnerRequest {
                session_id: submit_target.session_id.to_string(),
                working_dir: submit_target.actor.working_dir().to_string(),
                turn_id: turn_id.to_string(),
                messages: prepared.messages,
                last_assistant_at: submit_target
                    .actor
                    .state()
                    .snapshot_projected_state()?
                    .last_assistant_at,
                session_state: Arc::clone(submit_target.actor.state()),
                runtime,
                cancel,
                agent: prepared.persisted.agent.clone(),
                prompt_facts_provider: Arc::clone(&prompt_facts_provider),
                capability_router: prepared.capability_router,
                prompt_declarations: prepared.prompt_declarations,
                prompt_governance: prepared.prompt_governance,
            },
            finalize: TurnFinalizeContext {
                kernel,
                prompt_facts_provider,
                event_store,
                metrics,
                actor: Arc::clone(&submit_target.actor),
                session_id: submit_target.session_id.to_string(),
                turn_started_at: Instant::now(),
                generation,
                persisted: prepared.persisted,
            },
        })
    }
}

struct PreparedTurnSubmission {
    capability_router: Option<CapabilityRouter>,
    prompt_declarations: Vec<PromptDeclaration>,
    prompt_governance: Option<astrcode_core::PromptGovernanceContext>,
    messages: Vec<LlmMessage>,
    persisted: PersistedTurnContext,
}

async fn execute_turn_and_finalize(task: TurnExecutionTask) {
    let TurnExecutionTask {
        kernel,
        request,
        finalize,
    } = task;
    let result = run_turn(kernel, request).await;
    finalize.metrics.record_turn_execution(
        finalize.turn_started_at.elapsed().as_millis() as u64,
        result.is_ok(),
    );
    finalize_turn_execution(finalize, result).await;
}

async fn finalize_turn_execution(
    finalize: TurnFinalizeContext,
    result: Result<crate::TurnRunResult>,
) {
    let mut translator = EventTranslator::new(
        finalize
            .actor
            .state()
            .current_phase()
            .unwrap_or(Phase::Idle),
    );

    match result {
        Ok(turn_result) => {
            persist_turn_events(
                &finalize.event_store,
                finalize.actor.state(),
                &finalize.session_id,
                &mut translator,
                turn_result,
                &finalize.persisted.turn_id,
                &finalize.persisted.agent,
                finalize.persisted.source_tool_call_id.clone(),
            )
            .await;
        },
        Err(error) if error.is_cancelled() => {
            log::warn!(
                "turn execution cancelled for session '{}': {}",
                finalize.session_id,
                error
            );
        },
        Err(error) => {
            log::error!(
                "turn execution failed for session '{}': {}",
                finalize.session_id,
                error
            );
            persist_turn_failure(
                finalize.actor.state(),
                &finalize.session_id,
                &finalize.persisted.turn_id,
                finalize.persisted.agent.clone(),
                &mut translator,
                error.to_string(),
            )
            .await;
        },
    }

    let pending_manual_compact = match finalize
        .actor
        .state()
        .complete_execution_state(finalize.generation)
    {
        Ok(pending) => pending,
        Err(error) => {
            log::warn!(
                "failed to complete turn runtime state for session '{}': {}",
                finalize.session_id,
                error
            );
            None
        },
    };
    persist_pending_manual_compact_if_any(
        finalize.kernel.gateway(),
        finalize.prompt_facts_provider.as_ref(),
        &finalize.event_store,
        finalize.actor.working_dir(),
        finalize.actor.state(),
        &finalize.session_id,
        pending_manual_compact,
    )
    .await;
}

async fn prepare_turn_submission(
    session_state: &Arc<crate::SessionState>,
    turn_id: &str,
    live_user_input: Option<String>,
    queued_inputs: Vec<String>,
    submission: AgentPromptSubmission,
) -> Result<PreparedTurnSubmission> {
    let AgentPromptSubmission {
        agent,
        capability_router,
        prompt_declarations,
        resolved_limits,
        resolved_overrides,
        injected_messages,
        source_tool_call_id,
        policy_context: _,
        governance_revision: _,
        approval: _,
        prompt_governance,
    } = submission;

    let mut translator = EventTranslator::new(session_state.current_phase()?);
    for content in &queued_inputs {
        let queued_event = user_message_event(
            turn_id,
            &agent,
            content.clone(),
            UserMessageOrigin::QueuedInput,
            Utc::now(),
        );
        session_state
            .append_and_broadcast(&queued_event, &mut translator)
            .await?;
    }
    if let Some(text) = &live_user_input {
        let user_message = user_message_event(
            turn_id,
            &agent,
            text.clone(),
            UserMessageOrigin::User,
            Utc::now(),
        );
        session_state
            .append_and_broadcast(&user_message, &mut translator)
            .await?;
    }
    if let Some(event) = subrun_started_event(
        turn_id,
        &agent,
        resolved_limits.clone(),
        resolved_overrides.clone(),
        source_tool_call_id.clone(),
    ) {
        session_state
            .append_and_broadcast(&event, &mut translator)
            .await?;
    }
    let mut messages = session_state.current_turn_messages()?;
    if !injected_messages.is_empty() {
        let insert_at = if live_user_input.is_some() {
            messages.len().saturating_sub(1)
        } else {
            messages.len()
        };
        messages.splice(insert_at..insert_at, injected_messages);
    }

    Ok(PreparedTurnSubmission {
        capability_router,
        prompt_declarations,
        prompt_governance,
        messages,
        persisted: PersistedTurnContext {
            turn_id: turn_id.to_string(),
            agent,
            source_tool_call_id,
        },
    })
}

impl SessionRuntime {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_for_agent(session_id, text, runtime, AgentPromptSubmission::default())
            .await
    }

    pub async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_inner(SubmitPromptRequest {
            session_id: session_id.to_string(),
            turn_id: None,
            live_user_input: Some(text),
            queued_inputs: Vec::new(),
            runtime,
            busy_policy: SubmitBusyPolicy::BranchOnBusy,
            submission,
        })
        .await
        .and_then(|accepted| {
            accepted.ok_or_else(|| {
                astrcode_core::AstrError::Validation(
                    "submit prompt unexpectedly rejected while branch-on-busy is enabled"
                        .to_string(),
                )
            })
        })
    }

    pub async fn try_submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<Option<ExecutionAccepted>> {
        self.submit_prompt_inner(SubmitPromptRequest {
            session_id: session_id.to_string(),
            turn_id: None,
            live_user_input: Some(text),
            queued_inputs: Vec::new(),
            runtime,
            busy_policy: SubmitBusyPolicy::RejectOnBusy,
            submission,
        })
        .await
    }

    pub async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<Option<ExecutionAccepted>> {
        self.submit_prompt_inner(SubmitPromptRequest {
            session_id: session_id.to_string(),
            turn_id: Some(turn_id),
            live_user_input: Some(text),
            queued_inputs: Vec::new(),
            runtime,
            busy_policy: SubmitBusyPolicy::RejectOnBusy,
            submission,
        })
        .await
    }

    pub async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_inner(SubmitPromptRequest {
            session_id: session_id.to_string(),
            turn_id: None,
            live_user_input: Some(text),
            queued_inputs: Vec::new(),
            runtime,
            busy_policy: SubmitBusyPolicy::BranchOnBusy,
            submission,
        })
        .await?
        .ok_or_else(|| {
            astrcode_core::AstrError::Validation(
                "submit prompt unexpectedly rejected while branch-on-busy is enabled".to_string(),
            )
        })
    }

    pub async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<Option<ExecutionAccepted>> {
        self.submit_prompt_inner(SubmitPromptRequest {
            session_id: session_id.to_string(),
            turn_id: Some(turn_id),
            live_user_input: None,
            queued_inputs,
            runtime,
            busy_policy: SubmitBusyPolicy::RejectOnBusy,
            submission,
        })
        .await
    }

    async fn submit_prompt_inner(
        &self,
        request: SubmitPromptRequest,
    ) -> Result<Option<ExecutionAccepted>> {
        let SubmitPromptRequest {
            session_id,
            turn_id,
            live_user_input,
            queued_inputs,
            runtime,
            busy_policy,
            submission,
        } = request;
        let live_user_input = live_user_input
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let queued_inputs = queued_inputs
            .into_iter()
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();
        if live_user_input.is_none() && queued_inputs.is_empty() {
            return Err(astrcode_core::AstrError::Validation(
                "turn submission must include live user input or queued inputs".to_string(),
            ));
        }

        let requested_session_id = SessionId::from(crate::state::normalize_session_id(&session_id));
        let turn_id = turn_id.unwrap_or_else(|| TurnId::from(astrcode_core::generate_turn_id()));
        let submit_target = match busy_policy {
            SubmitBusyPolicy::BranchOnBusy => Some(
                self.resolve_submit_target(
                    &requested_session_id,
                    turn_id.as_str(),
                    runtime.max_concurrent_branch_depth,
                )
                .await?,
            ),
            SubmitBusyPolicy::RejectOnBusy => {
                self.try_resolve_submit_target_without_branch(
                    &requested_session_id,
                    turn_id.as_str(),
                )
                .await?
            },
        };
        let Some(submit_target) = submit_target else {
            return Ok(None);
        };

        Ok(Some(
            TurnCoordinator {
                kernel: Arc::clone(&self.kernel),
                prompt_facts_provider: Arc::clone(&self.prompt_facts_provider),
                event_store: Arc::clone(&self.event_store),
                metrics: Arc::clone(&self.metrics),
                submit_target,
                turn_id,
                runtime,
                live_user_input,
                queued_inputs,
                submission,
            }
            .start()
            .await?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use astrcode_core::{
        CancelToken, LlmFinishReason, LlmMessage, LlmOutput, LlmProvider, LlmRequest, ModelLimits,
        PromptBuildOutput, PromptBuildRequest, PromptProvider, ResourceProvider,
        ResourceReadResult, ResourceRequestContext, SessionTurnLease, StorageEventPayload,
        UserMessageOrigin,
    };
    use astrcode_kernel::Kernel;
    use async_trait::async_trait;

    use super::*;
    use crate::{
        TurnCollaborationSummary, TurnFinishReason, TurnOutcome, TurnRunResult, TurnSummary,
        turn::{
            TurnLoopTransition, TurnStopCause,
            events::turn_done_event,
            subrun_events::subrun_finished_event,
            test_support::{
                BranchingTestEventStore, NoopMetrics, append_root_turn_event_to_actor,
                assert_contains_compact_summary, assert_contains_error_message, test_actor,
                test_runtime,
            },
        },
    };

    #[derive(Debug)]
    struct SummaryLlmProvider;

    struct StubTurnLease;

    impl SessionTurnLease for StubTurnLease {}

    #[async_trait]
    impl LlmProvider for SummaryLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> Result<LlmOutput> {
            Ok(LlmOutput {
                content: "<analysis>ok</analysis><summary>manual compact summary</summary>"
                    .to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::Stop,
            })
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 64_000,
                max_output_tokens: 8_000,
            }
        }
    }

    #[derive(Debug)]
    struct TestPromptProvider;

    #[async_trait]
    impl PromptProvider for TestPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                prompt_cache_hints: Default::default(),
                cache_metrics: Default::default(),
                metadata: serde_json::Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct TestResourceProvider;

    #[async_trait]
    impl ResourceProvider for TestResourceProvider {
        async fn read_resource(
            &self,
            _uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: "noop://resource".to_string(),
                content: serde_json::Value::Null,
                metadata: serde_json::Value::Null,
            })
        }
    }

    fn summary_kernel() -> Arc<Kernel> {
        Arc::new(
            Kernel::builder()
                .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
                .with_llm_provider(Arc::new(SummaryLlmProvider))
                .with_prompt_provider(Arc::new(TestPromptProvider))
                .with_resource_provider(Arc::new(TestResourceProvider))
                .build()
                .expect("kernel should build"),
        )
    }

    fn finalize_context(actor: Arc<SessionActor>) -> TurnFinalizeContext {
        let generation = actor
            .state()
            .prepare_execution(
                "session-1",
                "turn-1",
                CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("turn runtime should prepare for finalize");
        TurnFinalizeContext {
            kernel: summary_kernel(),
            prompt_facts_provider: Arc::new(crate::turn::test_support::NoopPromptFactsProvider),
            event_store: Arc::new(BranchingTestEventStore::default()),
            metrics: Arc::new(NoopMetrics),
            actor,
            session_id: "session-1".to_string(),
            turn_started_at: Instant::now(),
            generation,
            persisted: PersistedTurnContext {
                turn_id: "turn-1".to_string(),
                agent: AgentEventContext::default(),
                source_tool_call_id: None,
            },
        }
    }

    #[derive(Debug)]
    struct RecordingLlmProvider {
        requests: Arc<Mutex<Vec<Vec<LlmMessage>>>>,
    }

    #[async_trait]
    impl LlmProvider for RecordingLlmProvider {
        async fn generate(
            &self,
            request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> Result<LlmOutput> {
            self.requests
                .lock()
                .expect("recorded requests lock should work")
                .push(request.messages.clone());
            Ok(LlmOutput {
                content: "answer".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::Stop,
            })
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 64_000,
                max_output_tokens: 8_000,
            }
        }
    }

    fn completed_turn_result() -> TurnRunResult {
        TurnRunResult {
            outcome: TurnOutcome::Completed,
            messages: Vec::new(),
            events: vec![turn_done_event(
                "turn-1",
                &AgentEventContext::default(),
                Some("completed".to_string()),
                chrono::Utc::now(),
            )],
            summary: TurnSummary {
                finish_reason: TurnFinishReason::NaturalEnd,
                stop_cause: TurnStopCause::Completed,
                last_transition: Some(TurnLoopTransition::ToolCycleCompleted),
                wall_duration: Duration::default(),
                step_count: 1,
                continuation_count: 0,
                total_tokens_used: 0,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                auto_compaction_count: 0,
                reactive_compact_count: 0,
                max_output_continuation_count: 0,
                tool_result_replacement_count: 0,
                tool_result_reapply_count: 0,
                tool_result_bytes_saved: 0,
                tool_result_over_budget_message_count: 0,
                streaming_tool_launch_count: 0,
                streaming_tool_match_count: 0,
                streaming_tool_fallback_count: 0,
                streaming_tool_discard_count: 0,
                streaming_tool_overlap_ms: 0,
                collaboration: TurnCollaborationSummary::default(),
            },
        }
    }

    #[tokio::test]
    async fn finalize_turn_execution_records_failure_event_and_interrupts_session() {
        let actor = test_actor().await;

        finalize_turn_execution(
            finalize_context(Arc::clone(&actor)),
            Err(astrcode_core::AstrError::Internal("boom".to_string())),
        )
        .await;

        assert_eq!(
            actor
                .state()
                .current_phase()
                .expect("phase should be readable"),
            Phase::Idle
        );
        let stored = actor
            .state()
            .snapshot_recent_stored_events()
            .expect("stored events should be available");
        assert_contains_error_message(&stored, "internal error: boom");
    }

    #[tokio::test]
    async fn finalize_turn_execution_persists_deferred_manual_compact_after_success() {
        let actor = test_actor().await;
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_user_message_event("turn-1", "hello"),
        )
        .await;
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_assistant_final_event("turn-1", "latest answer"),
        )
        .await;
        actor
            .state()
            .request_manual_compact(crate::state::PendingManualCompactRequest {
                runtime: ResolvedRuntimeConfig::default(),
                instructions: None,
            })
            .expect("manual compact flag should set");

        finalize_turn_execution(
            finalize_context(Arc::clone(&actor)),
            Ok(completed_turn_result()),
        )
        .await;

        assert_eq!(
            actor
                .state()
                .current_phase()
                .expect("phase should be readable"),
            Phase::Idle
        );
        let stored = actor
            .state()
            .snapshot_recent_stored_events()
            .expect("stored events should be available");
        assert_contains_compact_summary(&stored, "manual compact summary");
    }

    #[tokio::test]
    async fn finalize_turn_execution_persists_deferred_manual_compact_after_interrupt() {
        let actor = test_actor().await;
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_user_message_event("turn-1", "hello"),
        )
        .await;
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_assistant_final_event("turn-1", "latest answer"),
        )
        .await;
        actor
            .state()
            .request_manual_compact(crate::state::PendingManualCompactRequest {
                runtime: ResolvedRuntimeConfig::default(),
                instructions: None,
            })
            .expect("manual compact flag should set");

        finalize_turn_execution(
            finalize_context(Arc::clone(&actor)),
            Err(astrcode_core::AstrError::Internal("boom".to_string())),
        )
        .await;

        assert_eq!(
            actor
                .state()
                .current_phase()
                .expect("phase should be readable"),
            Phase::Idle
        );
        let stored = actor
            .state()
            .snapshot_recent_stored_events()
            .expect("stored events should be available");
        assert_contains_error_message(&stored, "internal error: boom");
        assert_contains_compact_summary(&stored, "manual compact summary");
    }

    #[test]
    fn subrun_lifecycle_events_ignore_non_subrun_context() {
        assert!(
            subrun_started_event("turn-1", &AgentEventContext::default(), None, None, None)
                .is_none()
        );
        assert!(
            subrun_finished_event(
                "turn-1",
                &AgentEventContext::default(),
                &completed_turn_result(),
                None,
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn submit_prompt_inner_returns_none_when_reject_on_busy() {
        let event_store = Arc::new(BranchingTestEventStore::default());
        let runtime = test_runtime(event_store.clone());
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");
        event_store.push_busy("turn-busy");

        let result = runtime
            .submit_prompt_inner(SubmitPromptRequest {
                session_id: session.session_id.clone(),
                turn_id: None,
                live_user_input: Some("hello".to_string()),
                queued_inputs: Vec::new(),
                runtime: ResolvedRuntimeConfig::default(),
                busy_policy: SubmitBusyPolicy::RejectOnBusy,
                submission: AgentPromptSubmission::default(),
            })
            .await
            .expect("submit should not error");

        assert!(result.is_none(), "reject-on-busy should not branch");
        assert_eq!(
            runtime.list_sessions(),
            vec![SessionId::from(session.session_id)]
        );
    }

    #[tokio::test]
    async fn submit_prompt_inner_branches_when_branch_on_busy() {
        let event_store = Arc::new(BranchingTestEventStore::default());
        let runtime = test_runtime(event_store.clone());
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");
        event_store.push_busy("turn-busy");

        let accepted = runtime
            .submit_prompt_inner(SubmitPromptRequest {
                session_id: session.session_id.clone(),
                turn_id: None,
                live_user_input: Some("hello".to_string()),
                queued_inputs: Vec::new(),
                runtime: ResolvedRuntimeConfig {
                    max_concurrent_branch_depth: 2,
                    ..ResolvedRuntimeConfig::default()
                },
                busy_policy: SubmitBusyPolicy::BranchOnBusy,
                submission: AgentPromptSubmission::default(),
            })
            .await
            .expect("submit should not error")
            .expect("branch-on-busy should always accept");

        assert_eq!(
            accepted.branched_from_session_id.as_deref(),
            Some(session.session_id.as_str())
        );
        assert_ne!(accepted.session_id.as_str(), session.session_id.as_str());
        let loaded_sessions = runtime.list_sessions();
        assert_eq!(
            loaded_sessions.len(),
            2,
            "branch submit should load a second session"
        );
        assert!(
            loaded_sessions.contains(&SessionId::from(session.session_id.clone())),
            "source session should stay loaded"
        );
        assert!(
            loaded_sessions.contains(&accepted.session_id),
            "branched session should be loaded"
        );

        let stored = event_store.stored_events_for(accepted.session_id.as_str());
        assert!(stored.iter().any(|stored| matches!(
            &stored.event.payload,
            StorageEventPayload::SessionStart {
                parent_session_id,
                ..
            } if parent_session_id.as_deref() == Some(session.session_id.as_str())
        )));

        let _ = runtime
            .wait_for_turn_terminal_snapshot(
                accepted.session_id.as_str(),
                accepted.turn_id.as_str(),
            )
            .await
            .expect("background turn should settle before test exits");
    }

    #[tokio::test]
    async fn submit_prompt_inner_appends_queued_inputs_before_live_user_prompt() {
        let requests = Arc::new(Mutex::new(Vec::<Vec<LlmMessage>>::new()));
        let kernel = Arc::new(
            Kernel::builder()
                .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
                .with_llm_provider(Arc::new(RecordingLlmProvider {
                    requests: Arc::clone(&requests),
                }))
                .with_prompt_provider(Arc::new(TestPromptProvider))
                .with_resource_provider(Arc::new(TestResourceProvider))
                .build()
                .expect("kernel should build"),
        );
        let event_store = Arc::new(BranchingTestEventStore::default());
        let runtime = SessionRuntime::new(
            kernel,
            Arc::new(crate::turn::test_support::NoopPromptFactsProvider),
            event_store,
            Arc::new(NoopMetrics),
        );
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");

        let accepted = runtime
            .submit_prompt_inner(SubmitPromptRequest {
                session_id: session.session_id.clone(),
                turn_id: None,
                live_user_input: Some("live user input".to_string()),
                queued_inputs: vec![
                    "queued child result".to_string(),
                    "queued reactivation context".to_string(),
                ],
                runtime: ResolvedRuntimeConfig::default(),
                busy_policy: SubmitBusyPolicy::RejectOnBusy,
                submission: AgentPromptSubmission::default(),
            })
            .await
            .expect("submit should not error")
            .expect("submit should be accepted");
        runtime
            .wait_for_turn_terminal_snapshot(
                accepted.session_id.as_str(),
                accepted.turn_id.as_str(),
            )
            .await
            .expect("turn should finish");

        let requests = requests.lock().expect("recorded requests lock should work");
        assert_eq!(requests.len(), 1, "expected one model request");

        assert!(matches!(
            requests[0].as_slice(),
            [
                LlmMessage::User {
                    content: first_queued,
                    origin: UserMessageOrigin::QueuedInput,
                },
                LlmMessage::User {
                    content: second_queued,
                    origin: UserMessageOrigin::QueuedInput,
                },
                LlmMessage::User {
                    content: user_content,
                    origin: UserMessageOrigin::User,
                }
            ] if first_queued == "queued child result"
                && second_queued == "queued reactivation context"
                && user_content == "live user input"
        ));
    }

    #[tokio::test]
    async fn submit_prompt_inner_inserts_injected_messages_before_live_user_prompt() {
        let requests = Arc::new(Mutex::new(Vec::<Vec<LlmMessage>>::new()));
        let kernel = Arc::new(
            Kernel::builder()
                .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
                .with_llm_provider(Arc::new(RecordingLlmProvider {
                    requests: Arc::clone(&requests),
                }))
                .with_prompt_provider(Arc::new(TestPromptProvider))
                .with_resource_provider(Arc::new(TestResourceProvider))
                .build()
                .expect("kernel should build"),
        );
        let event_store = Arc::new(BranchingTestEventStore::default());
        let runtime = SessionRuntime::new(
            kernel,
            Arc::new(crate::turn::test_support::NoopPromptFactsProvider),
            event_store,
            Arc::new(NoopMetrics),
        );
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");

        let accepted = runtime
            .submit_prompt_inner(SubmitPromptRequest {
                session_id: session.session_id.clone(),
                turn_id: None,
                live_user_input: Some("child task".to_string()),
                queued_inputs: Vec::new(),
                runtime: ResolvedRuntimeConfig::default(),
                busy_policy: SubmitBusyPolicy::RejectOnBusy,
                submission: AgentPromptSubmission {
                    injected_messages: vec![
                        LlmMessage::User {
                            content: "parent turn".to_string(),
                            origin: UserMessageOrigin::User,
                        },
                        LlmMessage::Assistant {
                            content: "parent answer".to_string(),
                            tool_calls: Vec::new(),
                            reasoning: None,
                        },
                    ],
                    ..AgentPromptSubmission::default()
                },
            })
            .await
            .expect("submit should not error")
            .expect("submit should be accepted");
        runtime
            .wait_for_turn_terminal_snapshot(
                accepted.session_id.as_str(),
                accepted.turn_id.as_str(),
            )
            .await
            .expect("turn should finish");

        let requests = requests.lock().expect("recorded requests lock should work");
        assert!(matches!(
            requests[0].as_slice(),
            [
                LlmMessage::User {
                    content: inherited_user,
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant { content: inherited_answer, .. },
                LlmMessage::User {
                    content: child_task,
                    origin: UserMessageOrigin::User,
                },
            ] if inherited_user == "parent turn"
                && inherited_answer == "parent answer"
                && child_task == "child task"
        ));
    }

    #[test]
    fn subrun_started_event_persists_resolved_overrides_snapshot() {
        let event = subrun_started_event(
            "turn-1",
            &AgentEventContext::sub_run(
                "agent-child",
                "turn-parent",
                "explore",
                "subrun-1",
                None,
                astrcode_core::SubRunStorageMode::IndependentSession,
                Some("session-child".into()),
            ),
            None,
            Some(ResolvedSubagentContextOverrides {
                include_compact_summary: true,
                fork_mode: Some(astrcode_core::ForkMode::LastNTurns(3)),
                ..ResolvedSubagentContextOverrides::default()
            }),
            None,
        )
        .expect("subrun event should be built");

        assert!(matches!(
            event.payload,
            StorageEventPayload::SubRunStarted { resolved_overrides, .. }
                if resolved_overrides.include_compact_summary
                    && resolved_overrides.fork_mode
                        == Some(astrcode_core::ForkMode::LastNTurns(3))
        ));
    }
}
