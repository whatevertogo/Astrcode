use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, EventTranslator, ExecutionAccepted, Phase,
    ResolvedRuntimeConfig, Result, RuntimeMetricsRecorder, SessionId, TurnId, UserMessageOrigin,
};
use chrono::Utc;

use crate::{
    SessionRuntime, TurnOutcome,
    actor::SessionActor,
    prepare_session_execution,
    query::current_turn_messages,
    run_turn,
    state::{append_and_broadcast, complete_session_execution},
    turn::events::{CompactAppliedStats, compact_applied_event, error_event, user_message_event},
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

struct TurnFinalizeContext {
    metrics: Arc<dyn RuntimeMetricsRecorder>,
    actor: Arc<SessionActor>,
    session_id: String,
    turn_id: String,
    agent: AgentEventContext,
    turn_started_at: Instant,
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
                &finalize.turn_id,
                finalize.agent.clone(),
                &mut translator,
                error.to_string(),
            )
            .await;
        },
    }

    complete_session_execution(finalize.actor.state(), terminal_phase);
    if terminal_phase == Phase::Idle
        && finalize
            .actor
            .state()
            .take_pending_manual_compact()
            .unwrap_or(false)
    {
        persist_deferred_manual_compact(finalize.actor.state(), &finalize.session_id).await;
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
) {
    for event in turn_result.events {
        if let Err(error) = append_and_broadcast(session_state, &event, translator).await {
            log::error!(
                "failed to persist turn event for session '{}': {}",
                session_id,
                error
            );
            break;
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
    session_state: &Arc<crate::SessionState>,
    session_id: &str,
) {
    let projected = match session_state.snapshot_projected_state() {
        Ok(projected) => projected,
        Err(error) => {
            log::warn!(
                "failed to snapshot session '{}' for deferred compact: {}",
                session_id,
                error
            );
            return;
        },
    };
    let summary = projected
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            astrcode_core::LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
                Some(content.clone())
            },
            astrcode_core::LlmMessage::User { content, .. } if !content.trim().is_empty() => {
                Some(content.clone())
            },
            _ => None,
        })
        .unwrap_or_else(|| "compacted".to_string());
    let event = compact_applied_event(
        None,
        &AgentEventContext::default(),
        astrcode_core::CompactTrigger::Manual,
        summary,
        CompactAppliedStats {
            preserved_recent_turns: 1,
            pre_tokens: 0,
            post_tokens_estimate: 0,
            messages_removed: 0,
            tokens_freed: 0,
        },
        Utc::now(),
    );
    let mut compact_translator =
        EventTranslator::new(session_state.current_phase().unwrap_or(Phase::Idle));
    if let Err(error) = append_and_broadcast(session_state, &event, &mut compact_translator).await {
        log::warn!(
            "failed to persist deferred compact for session '{}': {}",
            session_id,
            error
        );
    }
}

impl SessionRuntime {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_for_agent(session_id, text, runtime, AgentEventContext::default())
            .await
    }

    pub async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        agent: AgentEventContext,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_inner(
            session_id,
            None,
            text,
            runtime,
            agent,
            SubmitBusyPolicy::BranchOnBusy,
        )
        .await?
        .ok_or_else(|| {
            astrcode_core::AstrError::Validation(
                "submit prompt unexpectedly rejected while branch-on-busy is enabled".to_string(),
            )
        })
    }

    pub async fn try_submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        agent: AgentEventContext,
    ) -> Result<Option<ExecutionAccepted>> {
        self.submit_prompt_inner(
            session_id,
            None,
            text,
            runtime,
            agent,
            SubmitBusyPolicy::RejectOnBusy,
        )
        .await
    }

    pub async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        agent: AgentEventContext,
    ) -> Result<Option<ExecutionAccepted>> {
        self.submit_prompt_inner(
            session_id,
            Some(turn_id),
            text,
            runtime,
            agent,
            SubmitBusyPolicy::RejectOnBusy,
        )
        .await
    }

    async fn submit_prompt_inner(
        &self,
        session_id: &str,
        turn_id: Option<TurnId>,
        text: String,
        runtime: ResolvedRuntimeConfig,
        agent: AgentEventContext,
        busy_policy: SubmitBusyPolicy,
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
        let messages = current_turn_messages(submit_target.actor.state())?;

        tokio::spawn(execute_turn_and_finalize(TurnExecutionTask {
            kernel: Arc::clone(&self.kernel),
            request: crate::turn::RunnerRequest {
                session_id: submit_target.session_id.to_string(),
                working_dir: submit_target.actor.working_dir().to_string(),
                turn_id: turn_id.to_string(),
                messages,
                session_state: Arc::clone(submit_target.actor.state()),
                runtime,
                cancel: cancel.clone(),
                agent: agent.clone(),
                prompt_facts_provider: Arc::clone(&self.prompt_facts_provider),
            },
            finalize: TurnFinalizeContext {
                metrics: Arc::clone(&self.metrics),
                actor: Arc::clone(&submit_target.actor),
                session_id: submit_target.session_id.to_string(),
                turn_id: turn_id.to_string(),
                agent: agent.clone(),
                turn_started_at: Instant::now(),
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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use astrcode_core::StorageEventPayload;

    use super::*;
    use crate::{
        TurnCollaborationSummary, TurnFinishReason, TurnRunResult, TurnSummary,
        turn::test_support::{
            BranchingTestEventStore, NoopMetrics, append_root_turn_event_to_actor,
            assert_contains_compact_summary, assert_contains_error_message, test_actor,
            test_runtime,
        },
    };

    fn finalize_context(actor: Arc<SessionActor>) -> TurnFinalizeContext {
        TurnFinalizeContext {
            metrics: Arc::new(NoopMetrics),
            actor,
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            agent: AgentEventContext::default(),
            turn_started_at: Instant::now(),
        }
    }

    fn completed_turn_result() -> TurnRunResult {
        TurnRunResult {
            outcome: TurnOutcome::Completed,
            messages: Vec::new(),
            events: Vec::new(),
            summary: TurnSummary {
                finish_reason: TurnFinishReason::NaturalEnd,
                wall_duration: Duration::default(),
                step_count: 1,
                total_tokens_used: 0,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                auto_compaction_count: 0,
                reactive_compact_count: 0,
                collaboration: TurnCollaborationSummary::default(),
            },
        }
    }

    #[tokio::test]
    async fn finalize_turn_execution_records_failure_event_and_interrupts_session() {
        let actor = test_actor();

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
        let actor = test_actor();
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
            .request_manual_compact()
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
        assert_contains_compact_summary(&stored, "latest answer");
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
                AgentEventContext::default(),
                SubmitBusyPolicy::RejectOnBusy,
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
                AgentEventContext::default(),
                SubmitBusyPolicy::BranchOnBusy,
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
}
