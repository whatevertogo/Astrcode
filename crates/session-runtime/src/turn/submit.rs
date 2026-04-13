use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, EventTranslator, ExecutionAccepted, Phase, Result, SessionId,
    StorageEvent, StorageEventPayload, TurnId, UserMessageOrigin, config::RuntimeConfig,
};
use chrono::Utc;

use crate::{
    SessionRuntime, TurnOutcome,
    factory::prepare_turn_messages,
    prepare_session_execution, run_turn,
    state::{append_and_broadcast, complete_session_execution},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitBusyPolicy {
    BranchOnBusy,
    RejectOnBusy,
}

impl SessionRuntime {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
        runtime: RuntimeConfig,
    ) -> Result<ExecutionAccepted> {
        self.submit_prompt_for_agent(session_id, text, runtime, AgentEventContext::default())
            .await
    }

    pub async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: RuntimeConfig,
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
        runtime: RuntimeConfig,
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
        runtime: RuntimeConfig,
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
        runtime: RuntimeConfig,
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

        let user_message = StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: text,
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
        };
        prepare_session_execution(
            submit_target.actor.state(),
            submit_target.session_id.as_str(),
            turn_id.as_str(),
            cancel.clone(),
            submit_target.turn_lease,
            runtime.default_token_budget,
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
        let messages = prepare_turn_messages(submit_target.actor.state())?;

        let kernel = Arc::clone(&self.kernel);
        let prompt_facts_provider = Arc::clone(&self.prompt_facts_provider);
        let metrics = Arc::clone(&self.metrics);
        let actor_for_task = Arc::clone(&submit_target.actor);
        let session_id_for_task = submit_target.session_id.clone();
        let turn_id_for_task = turn_id.clone();
        tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let request = crate::turn::RunnerRequest {
                session_id: session_id_for_task.to_string(),
                working_dir: actor_for_task.working_dir().to_string(),
                turn_id: turn_id_for_task.to_string(),
                messages,
                session_state: Arc::clone(actor_for_task.state()),
                runtime,
                cancel: cancel.clone(),
                agent: agent.clone(),
                prompt_facts_provider,
            };

            let result = run_turn(kernel, request).await;
            metrics.record_turn_execution(
                turn_started_at.elapsed().as_millis() as u64,
                result.is_ok(),
            );
            let terminal_phase = match &result {
                Ok(outcome) => match outcome.outcome {
                    TurnOutcome::Completed => Phase::Idle,
                    TurnOutcome::Cancelled => Phase::Interrupted,
                    TurnOutcome::Error { .. } => Phase::Interrupted,
                },
                Err(_) => Phase::Interrupted,
            };

            let mut translator = EventTranslator::new(
                actor_for_task
                    .state()
                    .current_phase()
                    .unwrap_or(Phase::Idle),
            );

            match result {
                Ok(turn_result) => {
                    for event in turn_result.events {
                        if let Err(error) =
                            append_and_broadcast(actor_for_task.state(), &event, &mut translator)
                                .await
                        {
                            log::error!(
                                "failed to persist turn event for session '{}': {}",
                                session_id_for_task,
                                error
                            );
                            break;
                        }
                    }
                },
                Err(error) if error.is_cancelled() => {
                    log::warn!(
                        "turn execution cancelled for session '{}': {}",
                        session_id_for_task,
                        error
                    );
                },
                Err(error) => {
                    log::error!(
                        "turn execution failed for session '{}': {}",
                        session_id_for_task,
                        error
                    );
                    let failure = StorageEvent {
                        turn_id: Some(turn_id_for_task.to_string()),
                        agent: agent.clone(),
                        payload: StorageEventPayload::Error {
                            message: error.to_string(),
                            timestamp: Some(Utc::now()),
                        },
                    };
                    if let Err(append_error) =
                        append_and_broadcast(actor_for_task.state(), &failure, &mut translator)
                            .await
                    {
                        log::error!(
                            "failed to persist turn failure for session '{}': {}",
                            session_id_for_task,
                            append_error
                        );
                    }
                },
            }

            complete_session_execution(actor_for_task.state(), terminal_phase);
            if terminal_phase == Phase::Idle
                && actor_for_task
                    .state()
                    .take_pending_manual_compact()
                    .unwrap_or(false)
            {
                let projected = match actor_for_task.state().snapshot_projected_state() {
                    Ok(projected) => projected,
                    Err(error) => {
                        log::warn!(
                            "failed to snapshot session '{}' for deferred compact: {}",
                            session_id_for_task,
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
                        astrcode_core::LlmMessage::Assistant { content, .. }
                            if !content.trim().is_empty() =>
                        {
                            Some(content.clone())
                        },
                        astrcode_core::LlmMessage::User { content, .. }
                            if !content.trim().is_empty() =>
                        {
                            Some(content.clone())
                        },
                        _ => None,
                    })
                    .unwrap_or_else(|| "compacted".to_string());
                let event = StorageEvent {
                    turn_id: None,
                    agent: AgentEventContext::default(),
                    payload: StorageEventPayload::CompactApplied {
                        trigger: astrcode_core::CompactTrigger::Manual,
                        summary,
                        preserved_recent_turns: 1,
                        pre_tokens: 0,
                        post_tokens_estimate: 0,
                        messages_removed: 0,
                        tokens_freed: 0,
                        timestamp: Utc::now(),
                    },
                };
                let mut compact_translator = EventTranslator::new(
                    actor_for_task
                        .state()
                        .current_phase()
                        .unwrap_or(Phase::Idle),
                );
                if let Err(error) =
                    append_and_broadcast(actor_for_task.state(), &event, &mut compact_translator)
                        .await
                {
                    log::warn!(
                        "failed to persist deferred compact for session '{}': {}",
                        session_id_for_task,
                        error
                    );
                }
            }
        });

        Ok(Some(ExecutionAccepted {
            session_id: submit_target.session_id,
            turn_id,
            agent_id: None,
            branched_from_session_id: submit_target.branched_from_session_id,
        }))
    }
}
