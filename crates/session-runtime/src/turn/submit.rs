use std::sync::Arc;

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

impl SessionRuntime {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
        runtime: RuntimeConfig,
    ) -> Result<ExecutionAccepted> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(astrcode_core::AstrError::Validation(
                "prompt must not be empty".to_string(),
            ));
        }

        let requested_session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let turn_id = TurnId::from(format!("turn-{}", Utc::now().timestamp_millis()));
        let cancel = CancelToken::new();
        let submit_target = self
            .resolve_submit_target(
                &requested_session_id,
                turn_id.as_str(),
                runtime.max_concurrent_branch_depth,
            )
            .await?;

        let user_message = StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: AgentEventContext::default(),
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
        let actor_for_task = Arc::clone(&submit_target.actor);
        let session_id_for_task = submit_target.session_id.clone();
        let turn_id_for_task = turn_id.clone();
        tokio::spawn(async move {
            let request = crate::turn::RunnerRequest {
                session_id: session_id_for_task.to_string(),
                working_dir: actor_for_task.working_dir().to_string(),
                turn_id: turn_id_for_task.to_string(),
                messages,
                runtime,
                cancel: cancel.clone(),
                agent: AgentEventContext::default(),
                prompt_facts_provider,
            };

            let result = run_turn(kernel, request).await;
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
                Err(error) => {
                    log::error!(
                        "turn execution failed for session '{}': {}",
                        session_id_for_task,
                        error
                    );
                    let failure = StorageEvent {
                        turn_id: Some(turn_id_for_task.to_string()),
                        agent: AgentEventContext::default(),
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
        });

        Ok(ExecutionAccepted {
            session_id: submit_target.session_id,
            turn_id,
            agent_id: None,
            branched_from_session_id: submit_target.branched_from_session_id,
        })
    }
}
