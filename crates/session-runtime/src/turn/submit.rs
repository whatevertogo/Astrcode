use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, ApprovalPending, CancelToken, CapabilityCall,
    CompletedParentDeliveryPayload, EventStore, EventTranslator, ExecutionAccepted, LlmMessage,
    ParentDelivery, ParentDeliveryOrigin, ParentDeliveryPayload, ParentDeliveryTerminalSemantics,
    Phase, PolicyContext, PromptDeclaration, ResolvedExecutionLimitsSnapshot,
    ResolvedRuntimeConfig, ResolvedSubagentContextOverrides, Result, RuntimeMetricsRecorder,
    SessionId, StorageEvent, StorageEventPayload, StoredEvent, TurnId, UserMessageOrigin,
};
use astrcode_kernel::CapabilityRouter;
use chrono::Utc;

use crate::{
    SessionRuntime, TurnOutcome,
    actor::SessionActor,
    checkpoint_if_compacted, prepare_session_execution,
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
                &finalize.event_store,
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
    persist_pending_manual_compact_if_any(
        finalize.kernel.gateway(),
        finalize.prompt_facts_provider.as_ref(),
        &finalize.event_store,
        finalize.actor.working_dir(),
        finalize.actor.state(),
        &finalize.session_id,
    )
    .await;
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
    event_store: &Arc<dyn EventStore>,
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
    translator: &mut EventTranslator,
    turn_result: crate::TurnRunResult,
    persisted: &PersistedTurnContext,
) {
    let mut persisted_events = Vec::<StoredEvent>::new();
    for event in &turn_result.events {
        match append_and_broadcast(session_state, event, translator).await {
            Ok(stored) => persisted_events.push(stored),
            Err(error) => {
                log::error!(
                    "failed to persist turn event for session '{}': {}",
                    session_id,
                    error
                );
                break;
            },
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
    checkpoint_if_compacted(
        event_store,
        &SessionId::from(session_id.to_string()),
        session_state,
        &persisted_events,
    )
    .await;
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
    event_store: &Arc<dyn EventStore>,
    working_dir: &str,
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
    request: &crate::state::PendingManualCompactRequest,
) {
    session_state.set_compacting(true);
    let built = crate::turn::manual_compact::build_manual_compact_events(
        crate::turn::manual_compact::ManualCompactRequest {
            gateway,
            prompt_facts_provider,
            session_state,
            session_id,
            working_dir: std::path::Path::new(working_dir),
            runtime: &request.runtime,
            trigger: astrcode_core::CompactTrigger::Deferred,
            instructions: request.instructions.as_deref(),
        },
    )
    .await;
    session_state.set_compacting(false);
    let events = match built {
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
    let mut persisted = Vec::<StoredEvent>::with_capacity(events.len());
    for event in &events {
        match append_and_broadcast(session_state, event, &mut compact_translator).await {
            Ok(stored) => persisted.push(stored),
            Err(error) => {
                log::warn!(
                    "failed to persist deferred compact for session '{}': {}",
                    session_id,
                    error
                );
                break;
            },
        }
    }
    checkpoint_if_compacted(
        event_store,
        &SessionId::from(session_id.to_string()),
        session_state,
        &persisted,
    )
    .await;
}

pub(crate) async fn persist_pending_manual_compact_if_any(
    gateway: &astrcode_kernel::KernelGateway,
    prompt_facts_provider: &dyn astrcode_core::PromptFactsProvider,
    event_store: &Arc<dyn EventStore>,
    working_dir: &str,
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
) {
    let pending_runtime = session_state.take_pending_manual_compact().ok().flatten();
    if let Some(request) = pending_runtime {
        persist_deferred_manual_compact(
            gateway,
            prompt_facts_provider,
            event_store,
            working_dir,
            session_state,
            session_id,
            &request,
        )
        .await;
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
        for content in &queued_inputs {
            let queued_event = user_message_event(
                turn_id.as_str(),
                &agent,
                content.clone(),
                UserMessageOrigin::QueuedInput,
                Utc::now(),
            );
            append_and_broadcast(submit_target.actor.state(), &queued_event, &mut translator)
                .await?;
        }
        if let Some(text) = &live_user_input {
            let user_message = user_message_event(
                turn_id.as_str(),
                &agent,
                text.clone(),
                UserMessageOrigin::User,
                Utc::now(),
            );
            append_and_broadcast(submit_target.actor.state(), &user_message, &mut translator)
                .await?;
        }
        if let Some(event) = subrun_started_event(
            turn_id.as_str(),
            &agent,
            resolved_limits.clone(),
            resolved_overrides.clone(),
            source_tool_call_id.clone(),
        ) {
            append_and_broadcast(submit_target.actor.state(), &event, &mut translator).await?;
        }
        let mut messages = current_turn_messages(submit_target.actor.state())?;
        if !injected_messages.is_empty() {
            let insert_at = if live_user_input.is_some() {
                messages.len().saturating_sub(1)
            } else {
                messages.len()
            };
            messages.splice(insert_at..insert_at, injected_messages);
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
                prompt_governance,
            },
            finalize: TurnFinalizeContext {
                kernel: Arc::clone(&self.kernel),
                prompt_facts_provider: Arc::clone(&self.prompt_facts_provider),
                event_store: Arc::clone(&self.event_store),
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
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
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
            resolved_overrides: resolved_overrides.unwrap_or_default(),
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
        crate::TurnOutcome::Completed => astrcode_core::SubRunResult::Completed {
            outcome: astrcode_core::CompletedSubRunOutcome::Completed,
            handoff: astrcode_core::SubRunHandoff {
                findings: Vec::new(),
                artifacts: Vec::new(),
                delivery: Some(ParentDelivery {
                    idempotency_key: format!(
                        "subrun-finished:{}:{}",
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
            },
        },
        crate::TurnOutcome::Cancelled => astrcode_core::SubRunResult::Failed {
            outcome: astrcode_core::FailedSubRunOutcome::Cancelled,
            failure: astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Interrupted,
                display_message: summary,
                technical_message: "interrupted".to_string(),
                retryable: false,
            },
        },
        crate::TurnOutcome::Error { message } => astrcode_core::SubRunResult::Failed {
            outcome: astrcode_core::FailedSubRunOutcome::Failed,
            failure: astrcode_core::SubRunFailure {
                code: astrcode_core::SubRunFailureCode::Internal,
                display_message: summary,
                technical_message: message.clone(),
                retryable: true,
            },
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
                assert_contains_compact_summary, assert_contains_error_message, test_actor,
                test_runtime,
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
            Phase::Interrupted
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
