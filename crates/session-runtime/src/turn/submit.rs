use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, CompletedParentDeliveryPayload, EventTranslator,
    ExecutionAccepted, ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload,
    ParentDeliveryTerminalSemantics, Phase, PromptDeclaration, ResolvedExecutionLimitsSnapshot,
    ResolvedRuntimeConfig, ResolvedSubagentContextOverrides, Result, RuntimeMetricsRecorder,
    SessionId, StorageEvent, StorageEventPayload, TurnId, UserMessageOrigin,
};
use astrcode_kernel::CapabilityRouter;
use chrono::Utc;

use crate::{
    SessionRuntime, TurnOutcome,
    actor::SessionActor,
    prepare_session_execution,
    query::current_turn_messages,
    run_turn,
    state::{append_and_broadcast, complete_session_execution},
    turn::events::{error_event, user_message_event},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitBusyPolicy {
    BranchOnBusy,
    RejectOnBusy,
}

struct TurnExecutionTask {
    kernel: Arc<astrcode_kernel::Kernel>,
    request: crate::turn::RunnerRequest,
    finalize: TurnFinalizeContext,
}

#[derive(Clone, Default)]
pub struct AgentPromptSubmission {
    pub agent: AgentEventContext,
    pub capability_router: Option<CapabilityRouter>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    pub source_tool_call_id: Option<String>,
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
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    actor: Arc<SessionActor>,
    session_id: String,
    turn_started_at: Instant,
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
    let terminal_phase = terminal_phase_for_result(&result);
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
                finalize.actor.state(),
                &finalize.session_id,
                &mut translator,
                turn_result,
                &finalize.persisted,
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

    complete_session_execution(finalize.actor.state(), terminal_phase);
    if terminal_phase == Phase::Idle {
        let pending_runtime = finalize
            .actor
            .state()
            .take_pending_manual_compact()
            .ok()
            .flatten();
        if let Some(runtime) = pending_runtime {
            persist_deferred_manual_compact(
                finalize.kernel.gateway(),
                finalize.prompt_facts_provider.as_ref(),
                finalize.actor.working_dir(),
                finalize.actor.state(),
                &finalize.session_id,
                &runtime,
            )
            .await;
        }
    }
}

fn terminal_phase_for_result(result: &Result<crate::TurnRunResult>) -> Phase {
    match result {
        Ok(outcome) => match outcome.outcome {
            TurnOutcome::Completed => Phase::Idle,
            TurnOutcome::Cancelled | TurnOutcome::Error { .. } => Phase::Interrupted,
        },
        Err(_) => Phase::Interrupted,
    }
}

async fn persist_turn_events(
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
    translator: &mut EventTranslator,
    turn_result: crate::TurnRunResult,
    persisted: &PersistedTurnContext,
) {
    for event in &turn_result.events {
        if let Err(error) = append_and_broadcast(session_state, event, translator).await {
            log::error!(
                "failed to persist turn event for session '{}': {}",
                session_id,
                error
            );
            break;
        }
    }
    if let Some(event) = subrun_finished_event(
        &persisted.turn_id,
        &persisted.agent,
        &turn_result,
        persisted.source_tool_call_id.clone(),
    ) {
        if let Err(error) = append_and_broadcast(session_state, &event, translator).await {
            log::error!(
                "failed to persist subrun finished event for session '{}': {}",
                session_id,
                error
            );
        }
    }
}

async fn persist_turn_failure(
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
    turn_id: &str,
    agent: AgentEventContext,
    translator: &mut EventTranslator,
    message: String,
) {
    let failure = error_event(Some(turn_id), &agent, message, Some(Utc::now()));
    if let Err(append_error) = append_and_broadcast(session_state, &failure, translator).await {
        log::error!(
            "failed to persist turn failure for session '{}': {}",
            session_id,
            append_error
        );
    }
}

async fn persist_deferred_manual_compact(
    gateway: &astrcode_kernel::KernelGateway,
    prompt_facts_provider: &dyn astrcode_core::PromptFactsProvider,
    working_dir: &str,
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
    runtime: &ResolvedRuntimeConfig,
) {
    let events = match crate::turn::manual_compact::build_manual_compact_events(
        crate::turn::manual_compact::ManualCompactRequest {
            gateway,
            prompt_facts_provider,
            session_state,
            session_id,
            working_dir: std::path::Path::new(working_dir),
            runtime,
        },
    )
    .await
    {
        Ok(Some(events)) => events,
        Ok(None) => return,
        Err(error) => {
            log::warn!(
                "failed to build deferred compact for session '{}': {}",
                session_id,
                error
            );
            return;
        },
    };
    let mut compact_translator =
        EventTranslator::new(session_state.current_phase().unwrap_or(Phase::Idle));
    for event in &events {
        if let Err(error) =
            append_and_broadcast(session_state, event, &mut compact_translator).await
        {
            log::warn!(
                "failed to persist deferred compact for session '{}': {}",
                session_id,
                error
            );
            break;
        }
    }
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
        self.submit_prompt_inner(
            session_id,
            None,
            text,
            runtime,
            SubmitBusyPolicy::BranchOnBusy,
            submission,
        )
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
        self.submit_prompt_inner(
            session_id,
            None,
            text,
            runtime,
            SubmitBusyPolicy::RejectOnBusy,
            submission,
        )
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
        self.submit_prompt_inner(
            session_id,
            Some(turn_id),
            text,
            runtime,
            SubmitBusyPolicy::RejectOnBusy,
            submission,
        )
        .await
    }

    pub async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_inner(
            session_id,
            None,
            text,
            runtime,
            SubmitBusyPolicy::BranchOnBusy,
            submission,
        )
        .await?
        .ok_or_else(|| {
            astrcode_core::AstrError::Validation(
                "submit prompt unexpectedly rejected while branch-on-busy is enabled".to_string(),
            )
        })
    }

    async fn submit_prompt_inner(
        &self,
        session_id: &str,
        turn_id: Option<TurnId>,
        text: String,
        runtime: ResolvedRuntimeConfig,
        busy_policy: SubmitBusyPolicy,
        submission: AgentPromptSubmission,
    ) -> Result<Option<ExecutionAccepted>> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(astrcode_core::AstrError::Validation(
                "prompt must not be empty".to_string(),
            ));
        }

        let requested_session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let turn_id = turn_id
            .unwrap_or_else(|| TurnId::from(format!("turn-{}", Utc::now().timestamp_millis())));
        let cancel = CancelToken::new();
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

        let pending_reactivation_messages = submit_target
            .actor
            .state()
            .pending_reactivation_messages()?;
        let AgentPromptSubmission {
            agent,
            capability_router,
            prompt_declarations,
            resolved_limits,
            source_tool_call_id,
        } = submission;

        let user_message = user_message_event(
            turn_id.as_str(),
            &agent,
            text,
            UserMessageOrigin::User,
            Utc::now(),
        );
        prepare_session_execution(
            submit_target.actor.state(),
            submit_target.session_id.as_str(),
            turn_id.as_str(),
            cancel.clone(),
            submit_target.turn_lease,
        )?;
        *submit_target
            .actor
            .state()
            .phase
            .lock()
            .map_err(|_| astrcode_core::AstrError::LockPoisoned("session phase".to_string()))? =
            Phase::Thinking;

        let mut translator = EventTranslator::new(submit_target.actor.state().current_phase()?);
        append_and_broadcast(submit_target.actor.state(), &user_message, &mut translator).await?;
        if let Some(event) = subrun_started_event(
            turn_id.as_str(),
            &agent,
            resolved_limits.clone(),
            source_tool_call_id.clone(),
        ) {
            append_and_broadcast(submit_target.actor.state(), &event, &mut translator).await?;
        }
        let mut messages = current_turn_messages(submit_target.actor.state())?;
        if !pending_reactivation_messages.is_empty() {
            let insert_at = messages.len().saturating_sub(1);
            messages.splice(insert_at..insert_at, pending_reactivation_messages);
        }

        tokio::spawn(execute_turn_and_finalize(TurnExecutionTask {
            kernel: Arc::clone(&self.kernel),
            request: crate::turn::RunnerRequest {
                session_id: submit_target.session_id.to_string(),
                working_dir: submit_target.actor.working_dir().to_string(),
                turn_id: turn_id.to_string(),
                messages,
                last_assistant_at: submit_target
                    .actor
                    .state()
                    .snapshot_projected_state()?
                    .last_assistant_at,
                session_state: Arc::clone(submit_target.actor.state()),
                runtime,
                cancel: cancel.clone(),
                agent: agent.clone(),
                prompt_facts_provider: Arc::clone(&self.prompt_facts_provider),
                capability_router,
                prompt_declarations,
                resolved_limits: resolved_limits.clone(),
                source_tool_call_id: source_tool_call_id.clone(),
            },
            finalize: TurnFinalizeContext {
                kernel: Arc::clone(&self.kernel),
                prompt_facts_provider: Arc::clone(&self.prompt_facts_provider),
                metrics: Arc::clone(&self.metrics),
                actor: Arc::clone(&submit_target.actor),
                session_id: submit_target.session_id.to_string(),
                turn_started_at: Instant::now(),
                persisted: PersistedTurnContext {
                    turn_id: turn_id.to_string(),
                    agent: agent.clone(),
                    source_tool_call_id,
                },
            },
        }));

        Ok(Some(ExecutionAccepted {
            session_id: submit_target.session_id,
            turn_id,
            agent_id: None,
            branched_from_session_id: submit_target.branched_from_session_id,
        }))
    }
}

fn subrun_started_event(
    turn_id: &str,
    agent: &AgentEventContext,
    resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunStarted {
            tool_call_id: source_tool_call_id,
            resolved_overrides: ResolvedSubagentContextOverrides::default(),
            resolved_limits: resolved_limits.unwrap_or_default(),
            timestamp: Some(Utc::now()),
        },
    })
}

fn subrun_finished_event(
    turn_id: &str,
    agent: &AgentEventContext,
    turn_result: &crate::TurnRunResult,
    source_tool_call_id: Option<String>,
) -> Option<StorageEvent> {
    if agent.invocation_kind != Some(astrcode_core::InvocationKind::SubRun) {
        return None;
    }

    let summary = turn_result
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            astrcode_core::LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
                Some(content.trim().to_string())
            },
            _ => None,
        })
        .unwrap_or_else(|| match &turn_result.outcome {
            crate::TurnOutcome::Completed => "子 Agent 已完成，但没有返回可读总结。".to_string(),
            crate::TurnOutcome::Cancelled => "子 Agent 已关闭。".to_string(),
            crate::TurnOutcome::Error { message } => message.trim().to_string(),
        });

    let result = match &turn_result.outcome {
        crate::TurnOutcome::Completed => astrcode_core::SubRunResult {
            lifecycle: astrcode_core::AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Completed),
            handoff: Some(astrcode_core::SubRunHandoff {
                findings: Vec::new(),
                artifacts: Vec::new(),
                delivery: Some(ParentDelivery {
                    idempotency_key: format!(
                        "legacy-subrun-finished:{}:{}",
                        agent.sub_run_id.as_deref().unwrap_or("unknown-subrun"),
                        turn_id
                    ),
                    origin: ParentDeliveryOrigin::Fallback,
                    terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                    source_turn_id: Some(turn_id.to_string()),
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: summary,
                        findings: Vec::new(),
                        artifacts: Vec::new(),
                    }),
                }),
            }),
            failure: None,
        },
        crate::TurnOutcome::Cancelled => astrcode_core::SubRunResult {
            lifecycle: astrcode_core::AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Cancelled),
            handoff: None,
            failure: Some(astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Interrupted,
                display_message: summary,
                technical_message: "interrupted".to_string(),
                retryable: false,
            }),
        },
        crate::TurnOutcome::Error { message } => astrcode_core::SubRunResult {
            lifecycle: astrcode_core::AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(astrcode_core::AgentTurnOutcome::Failed),
            handoff: None,
            failure: Some(astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Internal,
                display_message: summary,
                technical_message: message.clone(),
                retryable: true,
            }),
        },
    };

    Some(StorageEvent {
        turn_id: Some(turn_id.to_string()),
        agent: agent.clone(),
        payload: StorageEventPayload::SubRunFinished {
            tool_call_id: source_tool_call_id,
            result,
            step_count: turn_result.summary.step_count as u32,
            estimated_tokens: turn_result.summary.total_tokens_used,
            timestamp: Some(Utc::now()),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use astrcode_core::{
        LlmFinishReason, LlmMessage, LlmOutput, LlmProvider, LlmRequest, ModelLimits,
        PromptBuildOutput, PromptBuildRequest, PromptProvider, ResourceProvider,
        ResourceReadResult, ResourceRequestContext, StorageEventPayload, UserMessageOrigin,
    };
    use astrcode_kernel::Kernel;
    use async_trait::async_trait;

    use super::*;
    use crate::{
        TurnCollaborationSummary, TurnFinishReason, TurnRunResult, TurnSummary,
        turn::{
            TurnLoopTransition, TurnStopCause,
            test_support::{
                BranchingTestEventStore, NoopMetrics, append_root_turn_event_to_actor,
                assert_contains_compact_summary, assert_contains_error_message,
                root_compact_applied_event, test_actor, test_runtime,
            },
        },
    };

    #[derive(Debug)]
    struct SummaryLlmProvider;

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
        TurnFinalizeContext {
            kernel: summary_kernel(),
            prompt_facts_provider: Arc::new(crate::turn::test_support::NoopPromptFactsProvider),
            metrics: Arc::new(NoopMetrics),
            actor,
            session_id: "session-1".to_string(),
            turn_started_at: Instant::now(),
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
            events: Vec::new(),
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
            Phase::Interrupted
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
            .request_manual_compact(ResolvedRuntimeConfig::default())
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

    #[test]
    fn subrun_lifecycle_events_ignore_non_subrun_context() {
        assert!(
            subrun_started_event("turn-1", &AgentEventContext::default(), None, None).is_none()
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
            .submit_prompt_inner(
                &session.session_id,
                None,
                "hello".to_string(),
                ResolvedRuntimeConfig::default(),
                SubmitBusyPolicy::RejectOnBusy,
                AgentPromptSubmission::default(),
            )
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
            .submit_prompt_inner(
                &session.session_id,
                None,
                "hello".to_string(),
                ResolvedRuntimeConfig {
                    max_concurrent_branch_depth: 2,
                    ..ResolvedRuntimeConfig::default()
                },
                SubmitBusyPolicy::BranchOnBusy,
                AgentPromptSubmission::default(),
            )
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
    async fn submit_prompt_inner_injects_pending_reactivation_only_once() {
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
        let session_state = runtime
            .get_session_state(&SessionId::from(session.session_id.clone()))
            .await
            .expect("session state should load");
        let mut translator = EventTranslator::new(session_state.current_phase().expect("phase"));

        append_and_broadcast(
            &session_state,
            &crate::turn::test_support::root_user_message_event("turn-0", "older question"),
            &mut translator,
        )
        .await
        .expect("older user event should append");
        append_and_broadcast(
            &session_state,
            &crate::turn::test_support::root_assistant_final_event("turn-0", "older answer"),
            &mut translator,
        )
        .await
        .expect("older assistant event should append");
        append_and_broadcast(
            &session_state,
            &root_compact_applied_event("turn-compact", "history summary", 1, 100, 40, 2, 60),
            &mut translator,
        )
        .await
        .expect("compact event should append");
        append_and_broadcast(
            &session_state,
            &crate::turn::events::user_message_event(
                "turn-compact",
                &AgentEventContext::default(),
                "Recovered file context".to_string(),
                UserMessageOrigin::ReactivationPrompt,
                Utc::now(),
            ),
            &mut translator,
        )
        .await
        .expect("reactivation event should append");

        let accepted = runtime
            .submit_prompt_inner(
                &session.session_id,
                None,
                "first after compact".to_string(),
                ResolvedRuntimeConfig::default(),
                SubmitBusyPolicy::RejectOnBusy,
                AgentPromptSubmission::default(),
            )
            .await
            .expect("submit should not error")
            .expect("submit should be accepted");
        runtime
            .wait_for_turn_terminal_snapshot(
                accepted.session_id.as_str(),
                accepted.turn_id.as_str(),
            )
            .await
            .expect("first turn should finish");

        let second = runtime
            .submit_prompt_inner(
                &session.session_id,
                None,
                "second turn".to_string(),
                ResolvedRuntimeConfig::default(),
                SubmitBusyPolicy::RejectOnBusy,
                AgentPromptSubmission::default(),
            )
            .await
            .expect("second submit should not error")
            .expect("second submit should be accepted");
        runtime
            .wait_for_turn_terminal_snapshot(second.session_id.as_str(), second.turn_id.as_str())
            .await
            .expect("second turn should finish");

        let requests = requests.lock().expect("recorded requests lock should work");
        assert_eq!(requests.len(), 2, "expected two model requests");

        assert!(matches!(
            requests[0].as_slice(),
            [
                LlmMessage::User { origin: UserMessageOrigin::CompactSummary, .. },
                LlmMessage::User { origin: UserMessageOrigin::ReactivationPrompt, content },
                LlmMessage::User { origin: UserMessageOrigin::User, content: user_content },
            ] if content == "Recovered file context" && user_content == "first after compact"
        ));
        assert!(
            requests[1].iter().all(|message| !matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::ReactivationPrompt,
                    ..
                }
            )),
            "reactivation prompt should only be injected into the first post-compact turn"
        );
    }
}
