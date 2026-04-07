//! execution owner handle 与内部执行辅助逻辑：
//! - `surface`：读取当前 runtime surface 并构造 scoped execution 输入
//! - `status`：sub-run 状态查询
//! - `subagent`：作为工具执行子 agent
//! - `root`：根执行入口

mod root;
mod status;
mod subagent;
mod surface;

use std::{
    sync::{Arc, RwLock as StdRwLock, Weak},
    time::Instant,
};

use astrcode_core::{
    AgentProfile, AstrError, CancelToken, ExecutionOrchestrationBoundary,
    LiveSubRunControlBoundary, Result, SpawnAgentParams, SubRunHandle, SubRunResult, ToolContext,
};
use astrcode_runtime_agent_loop::{TurnOutcome, strip_token_budget_marker};
use astrcode_runtime_agent_tool::{AgentProfileCatalog, SubAgentExecutor};
use astrcode_runtime_execution::{prepare_prompt_submission, resolve_interrupt_session_plan};
use astrcode_runtime_session::prepare_session_execution;
use async_trait::async_trait;
pub use root::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};
use uuid::Uuid;

use crate::{
    config::{
        resolve_continuation_min_delta_tokens, resolve_default_token_budget,
        resolve_max_continuations,
    },
    service::{
        PromptAccepted, RuntimeService, ServiceError, ServiceResult,
        blocking_bridge::{lock_anyhow, spawn_blocking_service},
        turn::{BudgetSettings, complete_session_execution, run_session_turn},
    },
};

/// bootstrap 阶段使用的延迟执行器桥。
///
/// builtin router 在 `RuntimeService` 创建前就要注册 `spawnAgent`，因此这里先占位，
/// 等 service 创建完成后再绑定真实 runtime。
#[derive(Default)]
pub(crate) struct DeferredSubAgentExecutor {
    runtime: StdRwLock<Option<Weak<RuntimeService>>>,
}

impl DeferredSubAgentExecutor {
    pub(crate) fn bind(&self, runtime: &Arc<RuntimeService>) {
        let mut guard = self
            .runtime
            .write()
            .expect("sub-agent executor binding lock should not be poisoned");
        *guard = Some(Arc::downgrade(runtime));
    }

    fn runtime(&self) -> Result<Arc<RuntimeService>> {
        let guard = self
            .runtime
            .read()
            .expect("sub-agent executor binding lock should not be poisoned");
        let Some(runtime) = guard.as_ref().and_then(Weak::upgrade) else {
            return Err(AstrError::Internal(
                "spawnAgent executor is not bound to runtime service yet".to_string(),
            ));
        };
        Ok(runtime)
    }
}

#[async_trait]
impl SubAgentExecutor for DeferredSubAgentExecutor {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let runtime = self.runtime()?;
        runtime
            .execution()
            .launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl RuntimeService {
    pub fn execution(self: &Arc<Self>) -> AgentExecutionServiceHandle {
        AgentExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    pub fn tools(self: &Arc<Self>) -> ToolExecutionServiceHandle {
        ToolExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }
}

#[async_trait]
impl SubAgentExecutor for root::AgentExecutionServiceHandle {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        self.launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl AgentProfileCatalog for root::AgentExecutionServiceHandle {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
}

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => error,
    }
}

impl AgentExecutionServiceHandle {
    pub(super) async fn load_profiles_for_working_dir(
        &self,
        working_dir: &std::path::Path,
    ) -> ServiceResult<Arc<astrcode_runtime_agent_loader::AgentProfileRegistry>> {
        let loader = self.runtime.agent_loader();
        let working_dir = working_dir.to_path_buf();
        let registry = spawn_blocking_service("load scoped agent profiles", move || {
            loader
                .load_for_working_dir(Some(&working_dir))
                .map_err(|error| {
                    ServiceError::Internal(astrcode_core::AstrError::Validation(error.to_string()))
                })
        })
        .await?;
        Ok(Arc::new(registry))
    }

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
            prepared_submission.token_budget,
        )?;

        let state = session.clone();
        let loop_ = self.runtime.current_loop().await;
        let accepted_turn_id = turn_id.clone();
        let observability = self.runtime.observability.clone();
        let agent_control = self.runtime.agent_control.clone();
        let accepted_session_id = session_id.clone();
        let user_event = prepared_submission.user_event.clone();
        let execution_owner = prepared_submission.execution_owner.clone();
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

    pub async fn interrupt_session(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = astrcode_runtime_session::normalize_session_id(session_id);
        if let Some(session) = self.runtime.sessions.get(&session_id) {
            let is_running = session.running.load(std::sync::atomic::Ordering::SeqCst);
            let active_turn_id =
                lock_anyhow(&session.active_turn_id, "session active turn").map(|g| g.clone())?;
            let interrupt_plan =
                resolve_interrupt_session_plan(is_running, active_turn_id.as_deref());
            if !interrupt_plan.should_cancel_session {
                return Ok(());
            }
            if let Ok(cancel) = lock_anyhow(&session.cancel, "session cancel") {
                cancel.cancel();
            }
            if let Some(active_turn_id) = interrupt_plan.active_turn_id.as_deref() {
                let _ = self
                    .runtime
                    .agent_control
                    .cancel_for_parent_turn(active_turn_id)
                    .await;
            }
        }
        Ok(())
    }

    pub async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<Option<SubRunHandle>> {
        let normalized_session_id = astrcode_runtime_session::normalize_session_id(session_id);
        Ok(self
            .runtime
            .agent_control
            .get(sub_run_id)
            .await
            .filter(|handle| {
                let handle_session_id =
                    astrcode_runtime_session::normalize_session_id(&handle.session_id);
                let child_session_id = handle
                    .child_session_id
                    .as_deref()
                    .map(astrcode_runtime_session::normalize_session_id);
                handle_session_id == normalized_session_id
                    || child_session_id.as_deref() == Some(normalized_session_id.as_str())
            }))
    }
}

#[async_trait]
impl ExecutionOrchestrationBoundary for AgentExecutionServiceHandle {
    async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> std::result::Result<astrcode_core::PromptAccepted, AstrError> {
        AgentExecutionServiceHandle::submit_prompt(self, session_id, text)
            .await
            .map(|accepted| astrcode_core::PromptAccepted {
                turn_id: accepted.turn_id,
                session_id: accepted.session_id,
                branched_from_session_id: accepted.branched_from_session_id,
            })
            .map_err(service_error_to_astr)
    }

    async fn interrupt_session(&self, session_id: &str) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::interrupt_session(self, session_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn execute_root_agent(
        &self,
        agent_id: String,
        task: String,
        context: Option<String>,
        max_steps: Option<u32>,
        context_overrides: Option<astrcode_core::SubagentContextOverrides>,
        working_dir: std::path::PathBuf,
    ) -> std::result::Result<astrcode_core::RootExecutionAccepted, AstrError> {
        AgentExecutionServiceHandle::execute_root_agent(
            self,
            agent_id,
            task,
            context,
            max_steps,
            context_overrides,
            working_dir,
        )
        .await
        .map(|accepted| astrcode_core::RootExecutionAccepted {
            session_id: accepted.session_id,
            turn_id: accepted.turn_id,
            agent_id: accepted.agent_id,
        })
        .map_err(service_error_to_astr)
    }

    async fn launch_subagent(
        &self,
        params: SpawnAgentParams,
        ctx: &ToolContext,
    ) -> std::result::Result<SubRunResult, AstrError> {
        AgentExecutionServiceHandle::launch_subagent(self, params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

#[async_trait]
impl LiveSubRunControlBoundary for AgentExecutionServiceHandle {
    async fn get_subrun_handle(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError> {
        AgentExecutionServiceHandle::get_subrun_handle(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn cancel_subrun(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<(), AstrError> {
        AgentExecutionServiceHandle::cancel_subrun(self, session_id, sub_run_id)
            .await
            .map_err(service_error_to_astr)
    }

    async fn list_profiles(&self) -> std::result::Result<Vec<AgentProfile>, AstrError> {
        Ok(self
            .runtime
            .agent_profiles()
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests;
