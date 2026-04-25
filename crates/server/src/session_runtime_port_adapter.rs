use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use astrcode_agent_runtime::{
    AgentRuntime, AgentRuntimeExecutionSurface, HookDispatcher, ToolDispatchRequest,
    ToolDispatcher, TurnInput, TurnOutput,
};
use astrcode_context_window::tool_result_budget::ToolResultReplacementRecord;
use astrcode_core::{
    AgentEvent, AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, AstrError, CancelToken,
    ChildSessionNotification, CompletedSubRunOutcome, DelegationMetadata, ExecutionControl,
    FailedSubRunOutcome, LlmMessage, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig,
    SessionId, StorageEvent, StorageEventPayload, StoredEvent, SubRunFailure, SubRunFailureCode,
    SubRunHandoff, SubRunResult, TurnId, UserMessageOrigin,
};
use astrcode_governance_contract::{BoundModeToolContractSnapshot, ModeId};
use astrcode_host_session::{
    CompactSessionMutationInput, EventTranslator, HookDispatch as HostSessionHookDispatch,
    InputHookApplyRequest, InputHookDecision, InputQueueProjection, InterruptSessionMutationInput,
    SessionCatalog, SubRunFinishStats, SubmitPromptMutationInput, SubmitTurnBusyPolicy,
    TurnMutationPreparation, apply_input_hooks,
};
use astrcode_llm_contract::{LlmEvent, LlmProvider};
use astrcode_runtime_contract::{
    ExecutionAccepted, ExecutionSubmissionOutcome, RuntimeEventSink, RuntimeTurnEvent,
    TurnStopCause,
};
use astrcode_tool_contract::{ToolContext, ToolEventSink, ToolExecutionResult};
use async_trait::async_trait;
use chrono::Utc;

use crate::{
    agent_control_bridge::{ServerCloseAgentSummary, ServerLiveSubRunStatus},
    agent_control_registry::{AgentControlError, AgentControlRegistry, PendingParentDelivery},
    application_error_bridge::ServerRouteError,
    capability_router::CapabilityRouter,
    mode_catalog_service::ServerModeCatalog,
    ports::{
        AppAgentPromptSubmission, RecoverableParentDelivery, ServerKernelControlError,
        SessionObserveSnapshot,
    },
    session_runtime_owner_bridge::ActiveSessionRegistry,
    session_runtime_port::SessionRuntimePort,
};

pub(crate) struct SessionRuntimePortBuildInput {
    pub session_catalog: Arc<SessionCatalog>,
    pub mode_catalog: Arc<ServerModeCatalog>,
    pub agent_control: Arc<AgentControlRegistry>,
    pub capability_router: CapabilityRouter,
    pub llm_provider: Arc<dyn LlmProvider>,
    pub active_sessions: Arc<ActiveSessionRegistry>,
    pub hook_dispatcher: Option<Arc<dyn HookDispatcher>>,
    pub owner_hook_dispatcher: Option<Arc<dyn HostSessionHookDispatch>>,
    pub hook_snapshot_id: Arc<dyn Fn() -> String + Send + Sync>,
}

pub(crate) fn build_session_runtime_port(
    input: SessionRuntimePortBuildInput,
) -> Arc<dyn SessionRuntimePort> {
    Arc::new(SessionRuntimeCompatPort {
        session_catalog: input.session_catalog,
        mode_catalog: input.mode_catalog,
        agent_control: input.agent_control,
        capability_router: input.capability_router,
        llm_provider: input.llm_provider,
        active_sessions: input.active_sessions,
        hook_dispatcher: input.hook_dispatcher,
        owner_hook_dispatcher: input.owner_hook_dispatcher,
        hook_snapshot_id: input.hook_snapshot_id,
    })
}

struct SessionRuntimeCompatPort {
    session_catalog: Arc<SessionCatalog>,
    mode_catalog: Arc<ServerModeCatalog>,
    agent_control: Arc<AgentControlRegistry>,
    capability_router: CapabilityRouter,
    llm_provider: Arc<dyn LlmProvider>,
    active_sessions: Arc<ActiveSessionRegistry>,
    hook_dispatcher: Option<Arc<dyn HookDispatcher>>,
    owner_hook_dispatcher: Option<Arc<dyn HostSessionHookDispatch>>,
    hook_snapshot_id: Arc<dyn Fn() -> String + Send + Sync>,
}

struct RouterToolDispatcher {
    capability_router: CapabilityRouter,
    working_dir: PathBuf,
    cancel: CancelToken,
    agent: astrcode_core::AgentEventContext,
    mode_tool_state: RuntimeModeToolState,
    event_sink: Arc<dyn ToolEventSink>,
}

#[derive(Clone)]
struct RuntimeModeToolState {
    inner: Arc<Mutex<RuntimeModeToolStateSnapshot>>,
}

#[derive(Clone)]
struct RuntimeModeToolStateSnapshot {
    current_mode_id: ModeId,
    bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
}

impl RuntimeModeToolState {
    fn new(
        current_mode_id: ModeId,
        bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RuntimeModeToolStateSnapshot {
                current_mode_id,
                bound_mode_tool_contract,
            })),
        }
    }

    fn snapshot(&self) -> RuntimeModeToolStateSnapshot {
        self.inner
            .lock()
            .expect("runtime mode tool state lock poisoned")
            .clone()
    }

    fn current_mode_id(&self) -> ModeId {
        self.snapshot().current_mode_id
    }

    fn replace(
        &self,
        current_mode_id: ModeId,
        bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    ) {
        *self
            .inner
            .lock()
            .expect("runtime mode tool state lock poisoned") = RuntimeModeToolStateSnapshot {
            current_mode_id,
            bound_mode_tool_contract,
        };
    }
}

struct SpawnTurnExecutionInput {
    begun: astrcode_host_session::BegunAcceptedTurn,
    working_dir: PathBuf,
    runtime: ResolvedRuntimeConfig,
    messages: Vec<LlmMessage>,
    last_assistant_at: Option<chrono::DateTime<Utc>>,
    tool_result_replacements: Vec<ToolResultReplacementRecord>,
    submission: AppAgentPromptSubmission,
}

enum LiveInputHookDecision {
    Continue { prompt_text: Option<String> },
    Handled(ExecutionSubmissionOutcome),
}

impl From<AgentControlError> for ServerKernelControlError {
    fn from(value: AgentControlError) -> Self {
        match value {
            AgentControlError::MaxDepthExceeded { current, max } => {
                Self::MaxDepthExceeded { current, max }
            },
            AgentControlError::MaxConcurrentExceeded { current, max } => {
                Self::MaxConcurrentExceeded { current, max }
            },
            AgentControlError::ParentAgentNotFound { agent_id } => {
                Self::ParentAgentNotFound { agent_id }
            },
        }
    }
}

#[async_trait]
impl ToolDispatcher for RouterToolDispatcher {
    async fn dispatch_tool(
        &self,
        request: ToolDispatchRequest,
    ) -> astrcode_core::Result<ToolExecutionResult> {
        let mut tool_ctx = ToolContext::new(
            request.session_id.clone().into(),
            self.working_dir.clone(),
            self.cancel.clone(),
        )
        .with_turn_id(request.turn_id)
        .with_tool_call_id(request.tool_call.id.clone())
        .with_agent_context(self.agent.clone());
        let mode_snapshot = self.mode_tool_state.snapshot();
        tool_ctx = tool_ctx.with_current_mode_id(mode_snapshot.current_mode_id);
        if let Some(snapshot) = mode_snapshot.bound_mode_tool_contract {
            tool_ctx = tool_ctx.with_bound_mode_tool_contract(snapshot);
        }
        if let Some(sender) = request.tool_output_sender {
            tool_ctx = tool_ctx.with_tool_output_sender(sender);
        }
        tool_ctx = tool_ctx.with_event_sink(Arc::clone(&self.event_sink));
        if let Some(max_inline) = self
            .capability_router
            .capability_spec(&request.tool_call.name)
            .and_then(|spec| spec.max_result_inline_size)
        {
            tool_ctx = tool_ctx.with_resolved_inline_limit(max_inline);
        }
        Ok(self
            .capability_router
            .execute_tool(&request.tool_call, &tool_ctx)
            .await)
    }
}

impl SessionRuntimeCompatPort {
    fn submit_preparation() -> TurnMutationPreparation {
        TurnMutationPreparation::external_preparation("server")
    }

    async fn begin_turn(
        &self,
        accepted: astrcode_host_session::AcceptedSubmitPrompt,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<(
        ExecutionAccepted,
        astrcode_host_session::BegunAcceptedTurn,
        Vec<LlmMessage>,
        PathBuf,
        Option<chrono::DateTime<Utc>>,
        Vec<ToolResultReplacementRecord>,
    )> {
        let session_id = accepted.summary.session_id.clone();
        let turn_id = accepted.summary.turn_id.clone();
        let loaded = self
            .session_catalog
            .ensure_loaded_session(&session_id)
            .await?;
        let working_dir = loaded.working_dir.clone();
        let projected_state = loaded.state.snapshot_projected_state()?;
        let last_assistant_at = projected_state.last_assistant_at;
        let replacements =
            tool_result_replacements_from_events(loaded.state.snapshot_recent_stored_events()?);
        let messages = build_turn_messages(
            loaded.state.current_turn_messages()?,
            accepted.live_user_input.clone(),
            accepted.queued_inputs.clone(),
            submission.injected_messages.clone(),
        );
        let cancel = CancelToken::new();
        let begun = self.session_catalog.begin_accepted_turn(
            accepted,
            submission.agent.clone(),
            cancel.clone(),
        )?;
        if let Err(error) = self
            .session_catalog
            .persist_begun_turn_inputs(&begun, submission.agent.clone())
            .await
        {
            let _ = self
                .session_catalog
                .complete_running_turn(&session_id, &turn_id);
            return Err(error);
        }
        let accepted = ExecutionAccepted {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            agent_id: None,
            branched_from_session_id: begun.summary.branched_from_session_id.clone(),
        };
        self.active_sessions.mark_running(session_id.as_str());
        Ok((
            accepted,
            begun,
            messages,
            working_dir,
            last_assistant_at,
            replacements,
        ))
    }

    fn spawn_turn_execution(&self, input: SpawnTurnExecutionInput) {
        let SpawnTurnExecutionInput {
            begun,
            working_dir,
            runtime,
            messages,
            last_assistant_at,
            tool_result_replacements,
            submission,
        } = input;
        let session_catalog = Arc::clone(&self.session_catalog);
        let capability_router = self.capability_router.clone();
        let llm_provider = Arc::clone(&self.llm_provider);
        let active_sessions = Arc::clone(&self.active_sessions);
        let mode_catalog = Arc::clone(&self.mode_catalog);
        let runtime_loop = AgentRuntime::new();
        let hook_dispatcher = self.hook_dispatcher.clone();
        let hook_snapshot_id = (self.hook_snapshot_id)();
        tokio::spawn(async move {
            let session_id = begun.summary.session_id.clone();
            let turn_id = begun.summary.turn_id.clone();
            let agent = submission.agent.clone();
            let source_tool_call_id = submission.source_tool_call_id.clone();
            let resolved_limits = submission.resolved_limits.clone();
            let resolved_overrides = submission.resolved_overrides.clone();
            let current_mode_id = submission.current_mode_id.clone();
            let bound_mode_tool_contract = submission.bound_mode_tool_contract.clone();
            let mode_tool_state =
                RuntimeModeToolState::new(current_mode_id.clone(), bound_mode_tool_contract);
            let (runtime_event_sink, runtime_event_bridge) = spawn_runtime_event_bridge(
                Arc::clone(&session_catalog),
                session_id.clone(),
                turn_id.clone(),
                agent.clone(),
            );
            let tool_event_sink = Arc::new(RuntimeToolEventSink {
                runtime_event_sink: Arc::clone(&runtime_event_sink),
                mode_catalog: Arc::clone(&mode_catalog),
                mode_tool_state: mode_tool_state.clone(),
            });

            let _ = session_catalog
                .append_subrun_started(
                    &session_id,
                    turn_id.as_str(),
                    agent.clone(),
                    resolved_limits.clone(),
                    resolved_overrides,
                    source_tool_call_id.clone(),
                )
                .await;

            let events_history_path = astrcode_support::hostpaths::project_dir(&working_dir)
                .ok()
                .map(|project_dir| {
                    project_dir
                        .join("sessions")
                        .join(session_id.as_str())
                        .join("events.jsonl")
                        .to_string_lossy()
                        .to_string()
                });

            let mut turn_input_builder = TurnInput::new(AgentRuntimeExecutionSurface {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                agent_id: agent_id_for_surface(&agent),
                model_ref: "server-owned-runtime".to_string(),
                provider_ref: "server-owned-provider".to_string(),
                tool_specs: capability_router.capability_specs(),
                hook_snapshot_id: hook_snapshot_id.clone(),
                current_mode: Some(current_mode_id.to_string()),
            })
            .with_agent(agent.clone())
            .with_messages(messages)
            .with_provider(llm_provider)
            .with_tool_dispatcher(Arc::new(RouterToolDispatcher {
                capability_router,
                working_dir: working_dir.clone(),
                cancel: CancelToken::new(),
                agent: agent.clone(),
                mode_tool_state,
                event_sink: tool_event_sink,
            }))
            .with_working_dir(working_dir)
            .with_runtime_config(runtime.clone())
            .with_last_assistant_at(last_assistant_at)
            .with_previous_tool_result_replacements(tool_result_replacements)
            .with_max_output_continuations(runtime.max_output_continuation_attempts)
            .with_events_history_path(events_history_path)
            .with_event_sink(runtime_event_sink);

            if let Some(dispatcher) = &hook_dispatcher {
                turn_input_builder = turn_input_builder.with_hook_dispatcher(dispatcher.clone());
            }

            let turn_input = turn_input_builder;

            let output = runtime_loop.execute_turn(turn_input).await;
            if let Err(error) = runtime_event_bridge.await {
                log::warn!(
                    "runtime event bridge join failed for session '{}': {}",
                    session_id,
                    error
                );
            }
            finalize_turn_execution(
                session_catalog,
                active_sessions,
                begun,
                output,
                agent,
                source_tool_call_id,
            )
            .await;
        });
    }

    async fn apply_live_input_hooks(
        &self,
        session_id: &str,
        prompt_text: Option<String>,
    ) -> astrcode_core::Result<LiveInputHookDecision> {
        let Some(prompt_text) = prompt_text else {
            return Ok(LiveInputHookDecision::Continue { prompt_text: None });
        };
        let Some(dispatcher) = self.owner_hook_dispatcher.clone() else {
            return Ok(LiveInputHookDecision::Continue {
                prompt_text: Some(prompt_text),
            });
        };

        let requested_session_id = SessionId::from(session_id.to_string());
        let current_mode = self
            .session_catalog
            .session_mode_state(&requested_session_id)
            .await?
            .current_mode_id;
        let decision = apply_input_hooks(InputHookApplyRequest {
            session_id: requested_session_id.clone(),
            source: "user".to_string(),
            text: prompt_text,
            images: Vec::new(),
            current_mode: current_mode.clone(),
            dispatcher: Some(dispatcher),
        })
        .await?;

        match decision {
            InputHookDecision::Handled {
                session_id,
                response,
                switch_mode,
            } => {
                self.apply_requested_mode_switch(&requested_session_id, &current_mode, switch_mode)
                    .await?;
                Ok(LiveInputHookDecision::Handled(
                    ExecutionSubmissionOutcome::handled(session_id, response),
                ))
            },
            InputHookDecision::Continue { text, switch_mode } => {
                self.apply_requested_mode_switch(&requested_session_id, &current_mode, switch_mode)
                    .await?;
                Ok(LiveInputHookDecision::Continue {
                    prompt_text: Some(text),
                })
            },
        }
    }

    async fn apply_requested_mode_switch(
        &self,
        session_id: &SessionId,
        current_mode: &ModeId,
        requested_mode: Option<ModeId>,
    ) -> astrcode_core::Result<()> {
        if let Some(target_mode) = requested_mode.filter(|target| target != current_mode) {
            self.mode_catalog
                .validate_transition(current_mode, &target_mode)?;
            self.session_catalog
                .switch_mode(session_id, target_mode)
                .await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_prompt_inner(
        &self,
        session_id: &str,
        requested_turn_id: Option<TurnId>,
        prompt_text: Option<String>,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
        busy_policy: SubmitTurnBusyPolicy,
    ) -> astrcode_core::Result<Option<ExecutionSubmissionOutcome>> {
        let prompt_text = match self.apply_live_input_hooks(session_id, prompt_text).await? {
            LiveInputHookDecision::Continue { prompt_text } => prompt_text,
            LiveInputHookDecision::Handled(outcome) => return Ok(Some(outcome)),
        };
        let accepted = self
            .session_catalog
            .accept_submit_prompt(
                SubmitPromptMutationInput {
                    requested_session_id: SessionId::from(session_id.to_string()),
                    requested_turn_id,
                    prompt_text: prompt_text.unwrap_or_default(),
                    queued_inputs,
                    control: None,
                    preparation: Self::submit_preparation(),
                },
                busy_policy,
            )
            .await?;
        let Some(accepted_prompt) = accepted else {
            return Ok(None);
        };
        let (accepted, begun, messages, working_dir, last_assistant_at, replacements) =
            self.begin_turn(accepted_prompt, submission.clone()).await?;
        self.spawn_turn_execution(SpawnTurnExecutionInput {
            begun,
            working_dir,
            runtime,
            messages,
            last_assistant_at,
            tool_result_replacements: replacements,
            submission,
        });
        Ok(Some(ExecutionSubmissionOutcome::accepted(accepted)))
    }
}

#[async_trait]
impl SessionRuntimePort for SessionRuntimeCompatPort {
    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionSubmissionOutcome> {
        let max_branch_depth = runtime.max_concurrent_branch_depth;
        self.submit_prompt_inner(
            session_id,
            None,
            Some(text),
            Vec::new(),
            runtime,
            submission,
            SubmitTurnBusyPolicy::BranchOnBusy { max_branch_depth },
        )
        .await?
        .ok_or_else(|| {
            AstrError::Validation(
                "submit prompt unexpectedly rejected while branch-on-busy is enabled".to_string(),
            )
        })
    }

    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()> {
        let _ = self
            .session_catalog
            .interrupt_running_turn(InterruptSessionMutationInput {
                session_id: SessionId::from(session_id.to_string()),
            })
            .await?;
        Ok(())
    }

    async fn compact_session(
        &self,
        session_id: &str,
        _runtime: ResolvedRuntimeConfig,
        instructions: Option<String>,
    ) -> astrcode_core::Result<bool> {
        Ok(self
            .session_catalog
            .request_manual_compact(CompactSessionMutationInput {
                session_id: SessionId::from(session_id.to_string()),
                control: Some(ExecutionControl {
                    manual_compact: Some(true),
                }),
                instructions,
                preparation: Self::submit_preparation(),
            })
            .await?
            .deferred)
    }

    async fn switch_mode(
        &self,
        session_id: &str,
        from: ModeId,
        to: ModeId,
    ) -> astrcode_core::Result<StoredEvent> {
        self.mode_catalog.validate_transition(&from, &to)?;
        let session_state = self
            .session_catalog
            .session_state(&SessionId::from(session_id.to_string()))
            .await?;
        let mut translator = EventTranslator::new(session_state.current_phase()?);
        session_state
            .append_and_broadcast(
                &astrcode_core::StorageEvent {
                    turn_id: None,
                    agent: astrcode_core::AgentEventContext::default(),
                    payload: astrcode_core::StorageEventPayload::ModeChanged {
                        from,
                        to,
                        timestamp: Utc::now(),
                    },
                },
                &mut translator,
            )
            .await
    }

    async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionSubmissionOutcome> {
        self.submit_prompt_for_agent(session_id, text, runtime, submission)
            .await
    }

    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionSubmissionOutcome>> {
        self.submit_prompt_inner(
            session_id,
            Some(turn_id),
            Some(text),
            Vec::new(),
            runtime,
            submission,
            SubmitTurnBusyPolicy::RejectOnBusy,
        )
        .await
    }

    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionSubmissionOutcome>> {
        self.submit_prompt_inner(
            session_id,
            Some(turn_id),
            None,
            queued_inputs,
            runtime,
            submission,
            SubmitTurnBusyPolicy::RejectOnBusy,
        )
        .await
    }

    async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot> {
        let session_state = self
            .session_catalog
            .session_state(&SessionId::from(open_session_id.to_string()))
            .await?;
        let projected = session_state.snapshot_projected_state()?;
        let input_queue_projection =
            session_state.input_queue_projection_for_agent(target_agent_id)?;
        Ok(build_agent_observe_snapshot(
            lifecycle_status,
            &projected,
            &input_queue_projection,
        ))
    }

    async fn get_handle(&self, agent_id: &str) -> Option<astrcode_host_session::SubRunHandle> {
        self.agent_control.get(agent_id).await
    }

    async fn find_root_handle_for_session(
        &self,
        session_id: &str,
    ) -> Option<astrcode_host_session::SubRunHandle> {
        self.agent_control
            .find_root_agent_for_session(session_id)
            .await
    }

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<astrcode_host_session::SubRunHandle, ServerKernelControlError> {
        self.agent_control
            .register_root_agent(agent_id, session_id, profile_id)
            .await
            .map_err(ServerKernelControlError::from)
    }

    async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        self.agent_control
            .set_resolved_limits(sub_run_or_agent_id, resolved_limits)
            .await
    }

    async fn get_lifecycle(&self, sub_run_or_agent_id: &str) -> Option<AgentLifecycleStatus> {
        self.agent_control.get_lifecycle(sub_run_or_agent_id).await
    }

    async fn get_turn_outcome(&self, sub_run_or_agent_id: &str) -> Option<AgentTurnOutcome> {
        self.agent_control
            .get_turn_outcome(sub_run_or_agent_id)
            .await
            .flatten()
    }

    async fn resume(
        &self,
        sub_run_or_agent_id: &str,
        parent_turn_id: &str,
    ) -> Option<astrcode_host_session::SubRunHandle> {
        self.agent_control
            .resume(sub_run_or_agent_id, parent_turn_id)
            .await
    }

    async fn spawn_independent_child(
        &self,
        profile: &astrcode_core::AgentProfile,
        session_id: String,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> Result<astrcode_host_session::SubRunHandle, ServerKernelControlError> {
        self.agent_control
            .spawn_with_storage(
                profile,
                session_id,
                Some(child_session_id),
                parent_turn_id,
                Some(parent_agent_id),
                astrcode_core::SubRunStorageMode::IndependentSession,
            )
            .await
            .map_err(ServerKernelControlError::from)
    }

    async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()> {
        self.agent_control
            .set_lifecycle(sub_run_or_agent_id, new_status)
            .await
    }

    async fn complete_turn(
        &self,
        sub_run_or_agent_id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        self.agent_control
            .complete_turn(sub_run_or_agent_id, outcome)
            .await
    }

    async fn set_delegation(
        &self,
        sub_run_or_agent_id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()> {
        self.agent_control
            .set_delegation(sub_run_or_agent_id, delegation)
            .await
    }

    async fn count_children_spawned_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
    ) -> usize {
        self.agent_control
            .list()
            .await
            .into_iter()
            .filter(|handle| {
                handle.parent_turn_id.as_str() == parent_turn_id
                    && handle
                        .parent_agent_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == parent_agent_id)
                    && matches!(
                        handle.lineage_kind,
                        astrcode_core::ChildSessionLineageKind::Spawn
                            | astrcode_core::ChildSessionLineageKind::Fork
                    )
            })
            .count()
    }

    async fn collect_subtree_handles(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Vec<astrcode_host_session::SubRunHandle> {
        self.agent_control
            .collect_subtree_handles(sub_run_or_agent_id)
            .await
    }

    async fn terminate_subtree(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<astrcode_host_session::SubRunHandle> {
        self.agent_control
            .terminate_subtree(sub_run_or_agent_id)
            .await
    }

    async fn deliver(&self, agent_id: &str, envelope: AgentInboxEnvelope) -> Option<()> {
        self.agent_control.push_inbox(agent_id, envelope).await
    }

    async fn drain_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        self.agent_control.drain_inbox(agent_id).await
    }

    async fn enqueue_child_delivery(
        &self,
        parent_session_id: String,
        parent_turn_id: String,
        notification: ChildSessionNotification,
    ) -> bool {
        self.agent_control
            .enqueue_parent_delivery(parent_session_id, parent_turn_id, notification)
            .await
    }

    async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<RecoverableParentDelivery>> {
        self.agent_control
            .checkout_parent_delivery_batch(parent_session_id)
            .await
            .map(map_pending_parent_deliveries)
    }

    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        self.agent_control
            .pending_parent_delivery_count(parent_session_id)
            .await
    }

    async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) {
        self.agent_control
            .requeue_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }

    async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        self.agent_control
            .consume_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }

    async fn query_subrun_status(&self, agent_id: &str) -> Option<ServerLiveSubRunStatus> {
        self.agent_control
            .get(agent_id)
            .await
            .map(|handle| map_runtime_status(&handle))
    }

    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus> {
        self.agent_control
            .find_root_agent_for_session(session_id)
            .await
            .map(|handle| map_runtime_status(&handle))
    }

    async fn close_subtree(
        &self,
        agent_id: &str,
    ) -> Result<ServerCloseAgentSummary, ServerRouteError> {
        self.agent_control
            .terminate_subtree_and_collect_handles(agent_id)
            .await
            .map(|handles| ServerCloseAgentSummary {
                closed_agent_ids: handles
                    .into_iter()
                    .map(|handle| handle.agent_id.to_string())
                    .collect(),
            })
            .ok_or_else(|| ServerRouteError::not_found(format!("agent '{}' not found", agent_id)))
    }
}

struct RuntimeToolEventSink {
    runtime_event_sink: Arc<dyn RuntimeEventSink>,
    mode_catalog: Arc<ServerModeCatalog>,
    mode_tool_state: RuntimeModeToolState,
}

#[async_trait]
impl ToolEventSink for RuntimeToolEventSink {
    async fn emit(&self, event: StorageEvent) -> astrcode_core::Result<()> {
        if let StorageEventPayload::ModeChanged { from, to, .. } = &event.payload {
            let from_mode_id = from.clone();
            let to_mode_id = to.clone();
            self.mode_catalog
                .validate_transition(&from_mode_id, &to_mode_id)?;
            let current_mode_id = self.mode_tool_state.current_mode_id();
            if current_mode_id != from_mode_id {
                return Err(AstrError::Validation(format!(
                    "mode transition from '{}' does not match current runtime mode '{}'",
                    from, current_mode_id
                )));
            }
            let bound_mode_tool_contract = self
                .mode_catalog
                .bound_tool_contract_snapshot(&to_mode_id)?;
            self.mode_tool_state
                .replace(to_mode_id, Some(bound_mode_tool_contract));
        }
        self.runtime_event_sink
            .emit_event(RuntimeTurnEvent::StorageEvent {
                event: Box::new(event),
            });
        Ok(())
    }
}

fn spawn_runtime_event_bridge(
    session_catalog: Arc<SessionCatalog>,
    session_id: SessionId,
    turn_id: TurnId,
    agent: astrcode_core::AgentEventContext,
) -> (Arc<dyn RuntimeEventSink>, tokio::task::JoinHandle<()>) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<RuntimeTurnEvent>();
    let sink = Arc::new(move |event: RuntimeTurnEvent| {
        let _ = sender.send(event);
    });
    let bridge = tokio::spawn(async move {
        let session_state = match session_catalog.session_state(&session_id).await {
            Ok(state) => Some(state),
            Err(error) => {
                log::warn!(
                    "failed to attach live runtime event bridge for session '{}': {}",
                    session_id,
                    error
                );
                None
            },
        };
        while let Some(event) = receiver.recv().await {
            if is_stale_terminal_runtime_event(&session_catalog, &session_id, &turn_id, &event)
                .await
            {
                continue;
            }
            if let Some(state) = &session_state {
                for agent_event in runtime_event_to_live_agent_events(&event, &agent) {
                    state.broadcast_live_event(agent_event);
                }
            }
            if let Err(error) = session_catalog
                .persist_runtime_turn_event(
                    astrcode_host_session::RuntimeTurnEventPersistenceInput {
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                        agent: agent.clone(),
                        runtime_event: event,
                    },
                )
                .await
            {
                log::warn!(
                    "failed to persist runtime event for session '{}' turn '{}': {}",
                    session_id,
                    turn_id,
                    error
                );
            }
        }
    });
    (sink, bridge)
}

async fn is_stale_terminal_runtime_event(
    session_catalog: &SessionCatalog,
    session_id: &SessionId,
    turn_id: &TurnId,
    event: &RuntimeTurnEvent,
) -> bool {
    if !matches!(
        event,
        RuntimeTurnEvent::TurnCompleted { .. } | RuntimeTurnEvent::TurnErrored { .. }
    ) {
        return false;
    }
    match session_catalog.session_control_state(session_id).await {
        Ok(control) => {
            control
                .active_turn_id
                .as_deref()
                .is_some_and(|active| active != turn_id.as_str())
                || control.active_turn_id.is_none()
        },
        Err(error) => {
            log::warn!(
                "failed to read active turn before terminal persistence for session '{}': {}",
                session_id,
                error
            );
            false
        },
    }
}

fn runtime_event_to_live_agent_events(
    event: &RuntimeTurnEvent,
    agent: &astrcode_core::AgentEventContext,
) -> Vec<AgentEvent> {
    match event {
        RuntimeTurnEvent::ProviderStream { identity, event } => match event {
            LlmEvent::TextDelta(delta) if !delta.is_empty() => vec![AgentEvent::ModelDelta {
                turn_id: identity.turn_id.clone(),
                agent: agent.clone(),
                delta: delta.clone(),
            }],
            LlmEvent::ThinkingDelta(delta) if !delta.is_empty() => {
                vec![AgentEvent::ThinkingDelta {
                    turn_id: identity.turn_id.clone(),
                    agent: agent.clone(),
                    delta: delta.clone(),
                }]
            },
            LlmEvent::StreamRetryStarted {
                attempt,
                max_attempts,
                reason,
            } => vec![AgentEvent::StreamRetryStarted {
                turn_id: identity.turn_id.clone(),
                agent: agent.clone(),
                attempt: *attempt,
                max_attempts: *max_attempts,
                reason: reason.clone(),
            }],
            _ => Vec::new(),
        },
        RuntimeTurnEvent::TurnCompleted { identity, .. } => vec![AgentEvent::TurnDone {
            turn_id: identity.turn_id.clone(),
            agent: agent.clone(),
        }],
        RuntimeTurnEvent::TurnErrored { identity, message } => vec![AgentEvent::Error {
            turn_id: Some(identity.turn_id.clone()),
            agent: agent.clone(),
            code: "agent_error".to_string(),
            message: message.clone(),
        }],
        RuntimeTurnEvent::StorageEvent { event } => {
            runtime_storage_event_to_live_agent_events(event, agent)
        },
        _ => Vec::new(),
    }
}

fn runtime_storage_event_to_live_agent_events(
    event: &astrcode_core::StorageEvent,
    fallback_agent: &astrcode_core::AgentEventContext,
) -> Vec<AgentEvent> {
    let Some(turn_id) = event.turn_id.clone() else {
        return Vec::new();
    };
    let agent = if event.agent.is_empty() {
        fallback_agent.clone()
    } else {
        event.agent.clone()
    };
    match &event.payload {
        StorageEventPayload::ToolCall {
            tool_call_id,
            tool_name,
            args,
        } => vec![AgentEvent::ToolCallStart {
            turn_id,
            agent,
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            input: args.clone(),
        }],
        StorageEventPayload::ToolCallDelta {
            tool_call_id,
            tool_name,
            stream,
            delta,
        } if !delta.is_empty() => vec![AgentEvent::ToolCallDelta {
            turn_id,
            agent,
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            stream: *stream,
            delta: delta.clone(),
        }],
        StorageEventPayload::ToolResult {
            tool_call_id,
            tool_name,
            output,
            success,
            error,
            metadata,
            continuation,
            duration_ms,
        } => vec![AgentEvent::ToolCallResult {
            turn_id,
            agent,
            result: ToolExecutionResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                ok: *success,
                output: output.clone(),
                error: error.clone(),
                metadata: metadata.clone(),
                continuation: continuation.clone(),
                duration_ms: *duration_ms,
                truncated: false,
            },
        }],
        _ => Vec::new(),
    }
}

async fn finalize_turn_execution(
    session_catalog: Arc<SessionCatalog>,
    active_sessions: Arc<ActiveSessionRegistry>,
    begun: astrcode_host_session::BegunAcceptedTurn,
    output: TurnOutput,
    agent: astrcode_core::AgentEventContext,
    source_tool_call_id: Option<String>,
) {
    let session_id = begun.summary.session_id.clone();
    let turn_id = begun.summary.turn_id.clone();
    let _ = session_catalog
        .append_subrun_finished(
            &session_id,
            turn_id.as_str(),
            agent,
            build_subrun_result(&output),
            SubRunFinishStats {
                step_count: output.step_count as u32,
                estimated_tokens: 0,
            },
            source_tool_call_id,
        )
        .await;

    if let Err(error) = session_catalog.complete_running_turn(&session_id, &turn_id) {
        log::warn!(
            "failed to complete running turn state for session '{}': {}",
            session_id,
            error
        );
    }
    active_sessions.mark_idle(session_id.as_str());
}

fn build_turn_messages(
    mut messages: Vec<LlmMessage>,
    live_user_input: Option<String>,
    queued_inputs: Vec<String>,
    injected_messages: Vec<LlmMessage>,
) -> Vec<LlmMessage> {
    for content in queued_inputs {
        messages.push(LlmMessage::User {
            content,
            origin: UserMessageOrigin::QueuedInput,
        });
    }
    if let Some(text) = live_user_input {
        messages.push(LlmMessage::User {
            content: text,
            origin: UserMessageOrigin::User,
        });
    }
    if !injected_messages.is_empty() {
        let insert_at = if messages.last().is_some_and(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
            )
        }) {
            messages.len().saturating_sub(1)
        } else {
            messages.len()
        };
        messages.splice(insert_at..insert_at, injected_messages);
    }
    messages
}

fn tool_result_replacements_from_events(
    events: Vec<StoredEvent>,
) -> Vec<ToolResultReplacementRecord> {
    events
        .into_iter()
        .filter_map(|stored| match stored.event.payload {
            StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id,
                persisted_output,
                replacement,
                original_bytes,
            } => Some(ToolResultReplacementRecord {
                tool_call_id,
                persisted_output,
                replacement,
                original_bytes,
            }),
            _ => None,
        })
        .collect()
}

fn build_agent_observe_snapshot(
    lifecycle_status: AgentLifecycleStatus,
    projected: &astrcode_host_session::AgentState,
    input_queue_projection: &InputQueueProjection,
) -> SessionObserveSnapshot {
    SessionObserveSnapshot {
        phase: projected.phase,
        turn_count: projected.turn_count as u32,
        active_task: active_task_summary(lifecycle_status, projected, input_queue_projection),
        last_output_tail: extract_last_output(&projected.messages),
        last_turn_tail: extract_last_turn_tail(&projected.messages),
    }
}

fn extract_last_output(messages: &[LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
            Some(truncate_text(content, 200))
        },
        _ => None,
    })
}

fn active_task_summary(
    lifecycle_status: AgentLifecycleStatus,
    projected: &astrcode_host_session::AgentState,
    input_queue_projection: &InputQueueProjection,
) -> Option<String> {
    if !input_queue_projection.active_delivery_ids.is_empty() {
        return extract_last_turn_tail(&projected.messages)
            .into_iter()
            .next();
    }
    if matches!(
        lifecycle_status,
        AgentLifecycleStatus::Pending | AgentLifecycleStatus::Running
    ) {
        return projected
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                LlmMessage::User {
                    content,
                    origin: UserMessageOrigin::User,
                } => summarize_inline_text(content, 120),
                _ => None,
            });
    }
    None
}

fn extract_last_turn_tail(messages: &[LlmMessage]) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter_map(|message| match message {
            LlmMessage::User { content, .. }
            | LlmMessage::Assistant { content, .. }
            | LlmMessage::Tool { content, .. } => summarize_inline_text(content, 120),
        })
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn summarize_inline_text(content: &str, limit: usize) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_text(trimmed, limit))
}

fn truncate_text(content: &str, limit: usize) -> String {
    if content.chars().count() <= limit {
        return content.to_string();
    }
    let prefix = content.chars().take(limit).collect::<String>();
    format!("{prefix}...")
}

fn build_subrun_result(output: &TurnOutput) -> SubRunResult {
    match output.stop_cause.unwrap_or(TurnStopCause::Completed) {
        TurnStopCause::Completed => SubRunResult::Completed {
            outcome: CompletedSubRunOutcome::Completed,
            handoff: SubRunHandoff {
                findings: Vec::new(),
                artifacts: Vec::new(),
                delivery: None,
            },
        },
        TurnStopCause::Cancelled => SubRunResult::Failed {
            outcome: FailedSubRunOutcome::Cancelled,
            failure: SubRunFailure {
                code: SubRunFailureCode::Interrupted,
                display_message: "child agent cancelled".to_string(),
                technical_message: output
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "child agent cancelled".to_string()),
                retryable: false,
            },
        },
        TurnStopCause::Error => SubRunResult::Failed {
            outcome: FailedSubRunOutcome::Failed,
            failure: SubRunFailure {
                code: SubRunFailureCode::Internal,
                display_message: output
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "child agent failed".to_string()),
                technical_message: output
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "child agent failed".to_string()),
                retryable: true,
            },
        },
    }
}

fn agent_id_for_surface(agent: &astrcode_core::AgentEventContext) -> String {
    agent
        .agent_id
        .clone()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "root-agent".to_string())
}

fn map_runtime_status(value: &astrcode_host_session::SubRunHandle) -> ServerLiveSubRunStatus {
    ServerLiveSubRunStatus {
        sub_run_id: value.sub_run_id.to_string(),
        agent_id: value.agent_id.to_string(),
        agent_profile: value.agent_profile.clone(),
        session_id: value.session_id.to_string(),
        child_session_id: value.child_session_id.clone().map(Into::into),
        depth: value.depth,
        parent_agent_id: value.parent_agent_id.clone().map(Into::into),
        lifecycle: value.lifecycle,
        last_turn_outcome: value.last_turn_outcome,
        resolved_limits: value.resolved_limits.clone(),
    }
}

fn map_pending_parent_deliveries(
    deliveries: Vec<PendingParentDelivery>,
) -> Vec<RecoverableParentDelivery> {
    deliveries
        .into_iter()
        .map(|value| RecoverableParentDelivery {
            delivery_id: value.delivery_id,
            parent_session_id: value.parent_session_id,
            parent_turn_id: value.parent_turn_id,
            queued_at_ms: value.queued_at_ms,
            notification: value.notification,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_adapter_tools::builtin_tools::{
        enter_plan_mode::EnterPlanModeTool, exit_plan_mode::ExitPlanModeTool,
        upsert_session_plan::UpsertSessionPlanTool,
    };
    use astrcode_agent_runtime::{RuntimeEventSink, ToolDispatchRequest, ToolDispatcher};
    use astrcode_core::{
        AgentEvent, AgentEventContext, CancelToken, CapabilityInvoker, StorageEvent,
        StorageEventPayload, ToolCallRequest, mode::ModeId as StoredModeId,
    };
    use astrcode_governance_contract::ModeId;
    use astrcode_tool_contract::ToolOutputStream;

    use super::{
        RouterToolDispatcher, RuntimeModeToolState, RuntimeToolEventSink, RuntimeTurnEvent,
        runtime_event_to_live_agent_events,
    };
    use crate::{
        capability_router::CapabilityRouter, mode::builtin_mode_specs,
        mode_catalog_service::ServerModeCatalog, tool_capability_invoker::ToolCapabilityInvoker,
    };

    #[test]
    fn runtime_storage_tool_delta_maps_to_live_agent_event() {
        let agent = AgentEventContext::root_execution("agent-root", "default");
        let events = runtime_event_to_live_agent_events(
            &RuntimeTurnEvent::StorageEvent {
                event: Box::new(StorageEvent {
                    turn_id: Some("turn-1".to_string()),
                    agent: agent.clone(),
                    payload: StorageEventPayload::ToolCallDelta {
                        tool_call_id: "call-1".to_string(),
                        tool_name: "shell_command".to_string(),
                        stream: ToolOutputStream::Stdout,
                        delta: "live\n".to_string(),
                    },
                }),
            },
            &agent,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEvent::ToolCallDelta {
                turn_id,
                tool_call_id,
                tool_name,
                stream,
                delta,
                ..
            } if turn_id == "turn-1"
                && tool_call_id == "call-1"
                && tool_name == "shell_command"
                && *stream == ToolOutputStream::Stdout
                && delta == "live\n"
        ));
    }

    #[tokio::test]
    async fn router_tool_dispatcher_attaches_event_sink_for_enter_plan_mode() {
        let capability_router = CapabilityRouter::builder()
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(EnterPlanModeTool))
                    .expect("enterPlanMode should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .build()
            .expect("capability router should build");
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let runtime_event_sink: Arc<dyn RuntimeEventSink> =
            Arc::new(move |event: RuntimeTurnEvent| {
                let _ = event_tx.send(event);
            });
        let mode_catalog = builtin_server_mode_catalog();
        let mode_tool_state = RuntimeModeToolState::new(ModeId::code(), None);
        let dispatcher = RouterToolDispatcher {
            capability_router,
            working_dir: std::env::temp_dir(),
            cancel: CancelToken::new(),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            mode_tool_state: mode_tool_state.clone(),
            event_sink: Arc::new(RuntimeToolEventSink {
                runtime_event_sink,
                mode_catalog,
                mode_tool_state,
            }),
        };

        let result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "enterPlanMode".to_string(),
                    args: serde_json::json!({
                        "reason": "need a plan"
                    }),
                },
                tool_output_sender: None,
            })
            .await
            .expect("tool dispatch should succeed");

        assert!(result.ok, "enterPlanMode should not fail without a sink");
        let event = event_rx.recv().await.expect("mode event should emit");
        assert!(matches!(
            event,
            RuntimeTurnEvent::StorageEvent { event }
                if matches!(
                    &event.payload,
                    StorageEventPayload::ModeChanged { from, to, .. }
                        if *from == StoredModeId::code() && *to == StoredModeId::plan()
                )
        ));
    }

    #[tokio::test]
    async fn router_tool_dispatcher_updates_mode_contract_after_enter_plan_mode() {
        let capability_router = CapabilityRouter::builder()
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(EnterPlanModeTool))
                    .expect("enterPlanMode should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(UpsertSessionPlanTool))
                    .expect("upsertSessionPlan should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(ExitPlanModeTool))
                    .expect("exitPlanMode should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .build()
            .expect("capability router should build");
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let runtime_event_sink: Arc<dyn RuntimeEventSink> =
            Arc::new(move |event: RuntimeTurnEvent| {
                let _ = event_tx.send(event);
            });
        let mode_catalog = builtin_server_mode_catalog();
        let mode_tool_state = RuntimeModeToolState::new(ModeId::code(), None);
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let dispatcher = RouterToolDispatcher {
            capability_router,
            working_dir: temp.path().to_path_buf(),
            cancel: CancelToken::new(),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            mode_tool_state: mode_tool_state.clone(),
            event_sink: Arc::new(RuntimeToolEventSink {
                runtime_event_sink,
                mode_catalog,
                mode_tool_state,
            }),
        };

        dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-enter".to_string(),
                    name: "enterPlanMode".to_string(),
                    args: serde_json::json!({ "reason": "need a plan" }),
                },
                tool_output_sender: None,
            })
            .await
            .expect("enterPlanMode dispatch should succeed");
        let event = event_rx.recv().await.expect("mode event should emit");
        assert!(matches!(
            event,
            RuntimeTurnEvent::StorageEvent { event }
                if matches!(
                    &event.payload,
                    StorageEventPayload::ModeChanged { from, to, .. }
                        if *from == StoredModeId::code() && *to == StoredModeId::plan()
                )
        ));

        let upsert_result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-upsert".to_string(),
                    name: "upsertSessionPlan".to_string(),
                    args: serde_json::json!({
                        "title": "Cleanup crates",
                        "content": "# Plan: Cleanup crates\n\n## Context\n- inspect first\n\n## Goal\n- produce a plan\n\n## Implementation Steps\n- update the code\n\n## Verification\n- run tests",
                        "status": "draft"
                    }),
                },
                tool_output_sender: None,
            })
            .await
            .expect("upsertSessionPlan dispatch should succeed");
        assert!(upsert_result.ok);
        let upsert_metadata = upsert_result
            .metadata
            .as_ref()
            .expect("upsertSessionPlan metadata should exist");
        assert_eq!(
            upsert_metadata["artifactType"],
            serde_json::json!("canonical-plan")
        );

        let exit_result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-exit".to_string(),
                    name: "exitPlanMode".to_string(),
                    args: serde_json::json!({}),
                },
                tool_output_sender: None,
            })
            .await
            .expect("exitPlanMode dispatch should succeed");
        assert!(exit_result.ok);
        let exit_metadata = exit_result
            .metadata
            .as_ref()
            .expect("exitPlanMode metadata should exist");
        assert_eq!(
            exit_metadata["schema"],
            serde_json::json!("sessionPlanExitReviewPending")
        );
    }

    #[tokio::test]
    async fn router_tool_dispatcher_updates_mode_context_after_exit_plan_mode() {
        let capability_router = CapabilityRouter::builder()
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(EnterPlanModeTool))
                    .expect("enterPlanMode should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(UpsertSessionPlanTool))
                    .expect("upsertSessionPlan should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .register_invoker(Arc::new(
                ToolCapabilityInvoker::new(Arc::new(ExitPlanModeTool))
                    .expect("exitPlanMode should register"),
            ) as Arc<dyn CapabilityInvoker>)
            .build()
            .expect("capability router should build");
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let runtime_event_sink: Arc<dyn RuntimeEventSink> =
            Arc::new(move |event: RuntimeTurnEvent| {
                let _ = event_tx.send(event);
            });
        let mode_catalog = builtin_server_mode_catalog();
        let plan_contract = mode_catalog
            .bound_tool_contract_snapshot(&ModeId::plan())
            .expect("plan mode contract should exist");
        let mode_tool_state = RuntimeModeToolState::new(ModeId::plan(), Some(plan_contract));
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let dispatcher = RouterToolDispatcher {
            capability_router,
            working_dir: temp.path().to_path_buf(),
            cancel: CancelToken::new(),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            mode_tool_state: mode_tool_state.clone(),
            event_sink: Arc::new(RuntimeToolEventSink {
                runtime_event_sink,
                mode_catalog,
                mode_tool_state,
            }),
        };

        dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-upsert".to_string(),
                    name: "upsertSessionPlan".to_string(),
                    args: serde_json::json!({
                        "title": "Cleanup crates",
                        "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Scope\n- runtime and adapter cleanup\n\n## Non-Goals\n- change transport protocol\n\n## Existing Code To Reuse\n- reuse current capability routing\n\n## Implementation Steps\n- audit crate dependencies\n- update the dispatcher context\n\n## Verification\n- run targeted Rust checks\n\n## Open Questions\n- none",
                        "status": "draft"
                    }),
                },
                tool_output_sender: None,
            })
            .await
            .expect("upsertSessionPlan dispatch should succeed");

        let review_result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-exit-review".to_string(),
                    name: "exitPlanMode".to_string(),
                    args: serde_json::json!({}),
                },
                tool_output_sender: None,
            })
            .await
            .expect("first exitPlanMode dispatch should succeed");
        assert_eq!(
            review_result
                .metadata
                .as_ref()
                .expect("review metadata should exist")["schema"],
            serde_json::json!("sessionPlanExitReviewPending")
        );

        let exit_result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-exit-final".to_string(),
                    name: "exitPlanMode".to_string(),
                    args: serde_json::json!({}),
                },
                tool_output_sender: None,
            })
            .await
            .expect("second exitPlanMode dispatch should succeed");
        assert_eq!(
            exit_result
                .metadata
                .as_ref()
                .expect("exit metadata should exist")["schema"],
            serde_json::json!("sessionPlanExit")
        );
        let exit_event = event_rx.recv().await.expect("exit mode event should emit");
        assert!(matches!(
            exit_event,
            RuntimeTurnEvent::StorageEvent { event }
                if matches!(
                    &event.payload,
                    StorageEventPayload::ModeChanged { from, to, .. }
                        if *from == StoredModeId::plan() && *to == StoredModeId::code()
                )
        ));

        let reenter_result = dispatcher
            .dispatch_tool(ToolDispatchRequest {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                agent_id: "agent-root".to_string(),
                tool_call: ToolCallRequest {
                    id: "call-reenter".to_string(),
                    name: "enterPlanMode".to_string(),
                    args: serde_json::json!({ "reason": "revise again" }),
                },
                tool_output_sender: None,
            })
            .await
            .expect("enterPlanMode dispatch should succeed after exit");
        assert_eq!(
            reenter_result
                .metadata
                .as_ref()
                .expect("reenter metadata should exist")["modeChanged"],
            serde_json::json!(true)
        );
        let reenter_event = event_rx
            .recv()
            .await
            .expect("reenter mode event should emit");
        assert!(matches!(
            reenter_event,
            RuntimeTurnEvent::StorageEvent { event }
                if matches!(
                    &event.payload,
                    StorageEventPayload::ModeChanged { from, to, .. }
                        if *from == StoredModeId::code() && *to == StoredModeId::plan()
                )
        ));
    }

    fn builtin_server_mode_catalog() -> Arc<ServerModeCatalog> {
        ServerModeCatalog::from_mode_specs(builtin_mode_specs(), Vec::new())
            .expect("server mode catalog should build")
    }
}
