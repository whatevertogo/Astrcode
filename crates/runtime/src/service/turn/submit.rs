use std::time::Instant;

use astrcode_core::{
    AgentEventContext, CancelToken, ExecutionOwner, InvocationKind, StorageEvent, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{TurnOutcome, strip_token_budget_marker};
use astrcode_runtime_session::{
    complete_session_execution, normalize_session_id, prepare_session_execution, run_session_turn,
};
use chrono::Utc;
use uuid::Uuid;

use super::{BudgetSettings, branch::SubmitTarget};
use crate::{
    config::{
        resolve_continuation_min_delta_tokens, resolve_default_token_budget,
        resolve_max_continuations,
    },
    service::{
        PromptAccepted, RuntimeService, ServiceError, ServiceResult, blocking_bridge::lock_anyhow,
    },
};

impl RuntimeService {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        let runtime_config = { self.config.lock().await.runtime.clone() };
        let parsed_budget = strip_token_budget_marker(&text);
        let default_token_budget = resolve_default_token_budget(&runtime_config);
        let token_budget = parsed_budget
            .budget
            .or((default_token_budget > 0).then_some(default_token_budget));
        let text = if parsed_budget.cleaned_text.is_empty() {
            text
        } else {
            parsed_budget.cleaned_text
        };
        let budget_settings = BudgetSettings {
            continuation_min_delta_tokens: resolve_continuation_min_delta_tokens(&runtime_config),
            max_continuations: resolve_max_continuations(&runtime_config),
        };
        let turn_id = Uuid::new_v4().to_string();
        let session_id = normalize_session_id(session_id);
        let SubmitTarget {
            session_id,
            branched_from_session_id,
            session,
            turn_lease,
        } = self.resolve_submit_target(&session_id, &turn_id).await?;
        let cancel = CancelToken::new();
        prepare_session_execution(
            &session,
            &session_id,
            &turn_id,
            cancel.clone(),
            turn_lease,
            token_budget,
        )
        .map_err(ServiceError::from)?;

        let state = session.clone();
        let loop_ = self.current_loop().await;
        let text_for_task = text;
        let accepted_turn_id = turn_id.clone();
        let observability = self.observability.clone();
        let agent_control = self.agent_control.clone();
        let accepted_session_id = session_id.clone();
        let execution_owner_session_id = accepted_session_id.clone();
        tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let user_event = StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                agent: AgentEventContext::default(),
                content: text_for_task,
                timestamp: Utc::now(),
                origin: UserMessageOrigin::User,
            };
            let result = run_session_turn(
                &state,
                &loop_,
                &turn_id,
                cancel.clone(),
                user_event,
                AgentEventContext::default(),
                ExecutionOwner::root(
                    execution_owner_session_id.clone(),
                    turn_id.clone(),
                    InvocationKind::RootExecution,
                ),
                budget_settings,
            )
            .await;
            complete_session_execution(&state, &agent_control, &turn_id, result.phase).await;

            let elapsed = turn_started_at.elapsed();
            observability.record_turn_execution(elapsed, result.succeeded);
            match &result.outcome {
                Ok(TurnOutcome::Completed) => {
                    if elapsed.as_millis() >= 5_000 {
                        log::warn!(
                            "turn '{}' completed slowly in {}ms",
                            turn_id,
                            elapsed.as_millis()
                        );
                    } else {
                        log::info!("turn '{}' completed in {}ms", turn_id, elapsed.as_millis());
                    }
                },
                Ok(TurnOutcome::Cancelled) => {
                    log::info!("turn '{}' cancelled in {}ms", turn_id, elapsed.as_millis());
                },
                Ok(TurnOutcome::Error { message }) => {
                    log::warn!(
                        "turn '{}' ended with agent error in {}ms: {}",
                        turn_id,
                        elapsed.as_millis(),
                        message
                    );
                },
                Err(_) => {
                    log::warn!("turn '{}' failed in {}ms", turn_id, elapsed.as_millis());
                },
            }
        });

        Ok(PromptAccepted {
            turn_id: accepted_turn_id,
            session_id: accepted_session_id,
            branched_from_session_id,
        })
    }

    pub async fn interrupt(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        if let Some(session) = self.sessions.get(&session_id) {
            if !session.running.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(());
            }
            let Some(active_turn_id) =
                lock_anyhow(&session.active_turn_id, "session active turn").map(|g| g.clone())?
            else {
                return Ok(());
            };
            if let Ok(cancel) = lock_anyhow(&session.cancel, "session cancel") {
                cancel.cancel();
            }
            let _ = self
                .agent_control
                .cancel_for_parent_turn(&active_turn_id)
                .await;
        }
        Ok(())
    }
}
