use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CancelToken, ExecutionOwner, InvocationKind, StorageEvent,
    UserMessageOrigin, replay_records,
};
use astrcode_runtime_agent_loop::{TurnOutcome, strip_token_budget_marker};
use astrcode_runtime_session::{
    complete_session_execution, normalize_session_id, prepare_session_execution, run_session_turn,
};
use chrono::Utc;
use uuid::Uuid;

use super::{
    PromptAccepted, ReplayPath, RuntimeService, ServiceResult, SessionReplay,
    blocking_bridge::lock_anyhow, session::load_events,
};
use crate::{
    config::{
        resolve_continuation_min_delta_tokens, resolve_default_token_budget,
        resolve_max_continuations,
    },
    service::turn::BudgetSettings,
};

/// 执行服务：封装 turn 执行提交、中断与回放路径。
///
/// 该组件让 RuntimeService 从执行细节中解耦，
/// 后续可独立演进执行策略（如调度、限流、回放缓存策略）。
pub(super) struct ExecutionService<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> ExecutionService<'a> {
    pub(super) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(super) async fn submit_prompt(
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
        let session_id = normalize_session_id(session_id);
        let submit_target = self
            .runtime
            .resolve_submit_target(&session_id, &turn_id)
            .await?;
        let session_id = submit_target.session_id;
        let branched_from_session_id = submit_target.branched_from_session_id;
        let session = submit_target.session;
        let turn_lease = submit_target.turn_lease;
        let cancel = CancelToken::new();
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
        let text_for_task = text;
        let accepted_turn_id = turn_id.clone();
        let observability = self.runtime.observability.clone();
        let agent_control = self.runtime.agent_control.clone();
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

    pub(super) async fn interrupt(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        if let Some(session) = self.runtime.sessions.get(&session_id) {
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
                .runtime
                .agent_control
                .cancel_for_parent_turn(&active_turn_id)
                .await;
        }
        Ok(())
    }

    pub(super) async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        let session_id = normalize_session_id(session_id);
        let state = self.runtime.ensure_session_loaded(&session_id).await?;

        let receiver = state.broadcaster.subscribe();
        let started_at = Instant::now();
        let replay_result = match state.recent_records_after(last_event_id)? {
            Some(history) => Ok((history, ReplayPath::Cache)),
            None => load_events(Arc::clone(&self.runtime.session_manager), &session_id)
                .await
                .map(|events| {
                    (
                        replay_records(&events, last_event_id),
                        ReplayPath::DiskFallback,
                    )
                }),
        };
        let elapsed = started_at.elapsed();
        match &replay_result {
            Ok((history, path)) => {
                self.runtime.observability.record_sse_catch_up(
                    elapsed,
                    true,
                    path.clone(),
                    history.len(),
                );
                if matches!(path, ReplayPath::DiskFallback) {
                    log::warn!(
                        "session '{}' replay used durable fallback and recovered {} events in {}ms",
                        session_id,
                        history.len(),
                        elapsed.as_millis()
                    );
                }
            },
            Err(error) => {
                self.runtime.observability.record_sse_catch_up(
                    elapsed,
                    false,
                    ReplayPath::DiskFallback,
                    0,
                );
                log::error!(
                    "failed to replay session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            },
        }
        let (history, _) = replay_result?;
        Ok(SessionReplay { history, receiver })
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use astrcode_core::{AgentMode, AgentProfile};
    use astrcode_runtime_agent_loop::{AgentLoop, ProviderFactory};
    use async_trait::async_trait;

    use super::*;
    use crate::{
        llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits},
        service::RuntimeService,
        test_support::{TestEnvGuard, empty_capabilities},
    };

    struct StaticProviderFactory {
        provider: Arc<dyn LlmProvider>,
    }

    impl ProviderFactory for StaticProviderFactory {
        fn build_for_working_dir(
            &self,
            _working_dir: Option<std::path::PathBuf>,
        ) -> astrcode_core::Result<Arc<dyn LlmProvider>> {
            Ok(Arc::clone(&self.provider))
        }
    }

    struct DelayedProvider {
        delay: Duration,
    }

    #[async_trait]
    impl LlmProvider for DelayedProvider {
        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 4_096,
            }
        }

        async fn generate(
            &self,
            request: LlmRequest,
            _sink: Option<EventSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            tokio::select! {
                _ = crate::llm::cancelled(request.cancel.clone()) => Err(astrcode_core::AstrError::LlmInterrupted),
                _ = tokio::time::sleep(self.delay) => Ok(LlmOutput {
                    content: "done".to_string(),
                    ..LlmOutput::default()
                }),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execution_service_submit_and_interrupt_work_without_turn_facade() {
        let _guard = TestEnvGuard::new();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let service = Arc::new(
            RuntimeService::from_capabilities(empty_capabilities()).expect("service should build"),
        );

        // 用延迟 provider 保证 turn 处于运行态，便于验证 interrupt 级联到子 Agent。
        let loop_ = AgentLoop::from_capabilities(
            Arc::new(StaticProviderFactory {
                provider: Arc::new(DelayedProvider {
                    delay: Duration::from_secs(30),
                }),
            }),
            empty_capabilities(),
        );
        *service.loop_.write().await = Arc::new(loop_);

        let session = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        let execution = ExecutionService::new(service.as_ref());
        let accepted = execution
            .submit_prompt(&session.session_id, "hello".to_string())
            .await
            .expect("prompt should be accepted");

        let control = service.agent_control();
        let child = control
            .spawn(
                &AgentProfile {
                    id: "review".to_string(),
                    name: "Review".to_string(),
                    description: "review".to_string(),
                    mode: AgentMode::SubAgent,
                    system_prompt: None,
                    allowed_tools: vec!["readFile".to_string()],
                    disallowed_tools: Vec::new(),
                    max_steps: Some(3),
                    token_budget: Some(1_000),
                    model_preference: None,
                },
                &session.session_id,
                Some(accepted.turn_id.clone()),
                None,
            )
            .await
            .expect("child spawn should succeed");
        let _ = control.mark_running(&child.agent_id).await;

        execution
            .interrupt(&session.session_id)
            .await
            .expect("interrupt should succeed");

        let child_handle = control
            .wait(&child.agent_id)
            .await
            .expect("child should still exist");
        assert_eq!(child_handle.status, astrcode_core::AgentStatus::Cancelled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execution_service_replay_returns_history_without_replay_facade() {
        let _guard = TestEnvGuard::new();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let service =
            RuntimeService::from_capabilities(empty_capabilities()).expect("service should build");

        let session = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");
        let execution = ExecutionService::new(&service);
        let replay = execution
            .replay(&session.session_id, None)
            .await
            .expect("replay should succeed");

        assert!(
            !replay.history.is_empty(),
            "fresh session replay should at least include session start event"
        );
    }
}
