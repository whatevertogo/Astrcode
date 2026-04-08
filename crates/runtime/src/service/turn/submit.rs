//! Turn 提交：将用户 prompt 解析为 turn 并异步执行。

use std::time::Instant;

use astrcode_core::CancelToken;
use astrcode_runtime_agent_loop::{TurnOutcome, strip_token_budget_marker};
use astrcode_runtime_execution::prepare_prompt_submission;
use astrcode_runtime_session::prepare_session_execution;
use uuid::Uuid;

use super::BudgetSettings;
use crate::{
    config::{
        resolve_continuation_min_delta_tokens, resolve_default_token_budget,
        resolve_max_continuations,
    },
    service::{
        PromptAccepted, ServiceResult,
        execution::AgentExecutionServiceHandle,
        turn::{complete_session_execution, run_session_turn},
    },
};

impl AgentExecutionServiceHandle {
    /// 提交 prompt 并启动异步 turn 执行。
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        let runtime_config = { self.runtime.config.lock().await.runtime.clone() };
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
        let submit_target = self
            .runtime
            .resolve_submit_target(session_id, &turn_id)
            .await?;
        let session_id = submit_target.session_id;
        let branched_from_session_id = submit_target.branched_from_session_id;
        let session = submit_target.session;
        let turn_lease = submit_target.turn_lease;
        let cancel = CancelToken::new();
        let prepared_submission =
            prepare_prompt_submission(&session_id, &turn_id, text, token_budget);
        prepare_session_execution(
            &session,
            &session_id,
            &turn_id,
            cancel.clone(),
            turn_lease,
            token_budget,
        )?;

        let state = session.clone();
        let loop_ = self.runtime.current_loop().await;
        let accepted_turn_id = turn_id.clone();
        let observability = self.runtime.observability.clone();
        let accepted_session_id = session_id.clone();
        let user_event = prepared_submission.user_event.clone();
        let execution_owner = prepared_submission.execution_owner.clone();
        // 在 spawn 前克隆 agent_control，避免借用 `self` 逃逸到 'static 闭包
        let agent_control = self.runtime.agent_control();
        tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let result = run_session_turn(
                &state,
                &loop_,
                &turn_id,
                cancel.clone(),
                user_event,
                astrcode_core::AgentEventContext::default(),
                execution_owner,
                budget_settings,
            )
            .await;
            complete_session_execution(&state, result.phase, &agent_control).await;

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
                    log::error!("turn '{}' failed in {}ms", turn_id, elapsed.as_millis());
                },
            }
        });

        Ok(PromptAccepted {
            turn_id: accepted_turn_id,
            session_id: accepted_session_id,
            branched_from_session_id,
        })
    }
}
