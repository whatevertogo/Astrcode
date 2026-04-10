use std::{path::PathBuf, sync::Arc, time::Instant};

use astrcode_core::{
    AgentMode, CancelToken, ExecutionAccepted, InvocationKind, SessionTurnAcquireResult,
    SubagentContextOverrides,
};
use astrcode_runtime_execution::{
    build_root_spawn_params, prepare_root_execution_launch, validate_root_execution_storage_mode,
};
use astrcode_runtime_session::prepare_session_execution;
use uuid::Uuid;

use crate::service::{
    RuntimeService, ServiceError, ServiceResult,
    blocking_bridge::spawn_blocking_service,
    turn::{BudgetSettings, RuntimeTurnInput, complete_session_execution, run_session_turn},
};

/// 面向 API / Tool 的 Agent Profile 摘要。
// TODO: 未来可能需要重新添加 max_steps 和 token_budget 参数来限制子智能体执行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// 面向 API / Tool 的工具摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
    pub streaming: bool,
}

/// Agent 执行服务句柄。
#[derive(Clone)]
pub struct AgentExecutionServiceHandle {
    pub(crate) runtime: Arc<RuntimeService>,
}

/// Tool 执行服务句柄。
#[derive(Clone)]
pub struct ToolExecutionServiceHandle {
    pub(crate) runtime: Arc<RuntimeService>,
}

impl AgentExecutionServiceHandle {
    pub fn control(&self) -> astrcode_runtime_agent_control::AgentControl {
        self.runtime.agent_control.clone()
    }

    pub(crate) async fn current_loop(&self) -> Arc<astrcode_runtime_agent_loop::AgentLoop> {
        self.runtime.loop_surface().current_loop().await
    }

    pub(crate) fn collaboration_executor(&self) -> Arc<super::DeferredCollaborationExecutor> {
        Arc::clone(&self.runtime.collaboration_executor)
    }

    pub fn list_profiles(&self) -> Vec<AgentProfileSummary> {
        let mut profiles = self
            .runtime
            .agent_profiles()
            .list()
            .into_iter()
            .map(|profile| AgentProfileSummary {
                id: profile.id.clone(),
                name: profile.name.clone(),
                description: profile.description.clone(),
                mode: profile.mode,
                allowed_tools: profile.allowed_tools.clone(),
                disallowed_tools: profile.disallowed_tools.clone(),
                // TODO: 未来可能需要添加 max_steps 和 token_budget
            })
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        profiles
    }

    pub async fn execute_root_agent(
        &self,
        agent_id: String,
        task: String,
        context: Option<String>,
        context_overrides: Option<SubagentContextOverrides>,
        working_dir: PathBuf,
    ) -> ServiceResult<ExecutionAccepted> {
        let params = build_root_spawn_params(agent_id, task, context);
        let runtime = &self.runtime;
        let profiles = self.load_profiles_for_working_dir(&working_dir).await?;
        let profile_id = params.r#type.as_deref().unwrap_or("explore");
        let profile = profiles.get(profile_id).cloned().ok_or_else(|| {
            ServiceError::InvalidInput(format!("unknown agent profile '{profile_id}'"))
        })?;
        astrcode_runtime_execution::ensure_root_execution_mode(&profile)?;
        let mut request =
            astrcode_runtime_execution::AgentExecutionRequest::from_spawn_agent_params(
                &params,
                context_overrides,
            );
        match request.context_overrides.as_mut() {
            Some(overrides) => {
                if overrides.storage_mode.is_none() {
                    overrides.storage_mode = Some(astrcode_core::SubRunStorageMode::SharedSession);
                }
            },
            None => {
                request.context_overrides = Some(SubagentContextOverrides {
                    storage_mode: Some(astrcode_core::SubRunStorageMode::SharedSession),
                    ..SubagentContextOverrides::default()
                });
            },
        }
        let prepared_execution = self.prepare_scoped_execution_request(
            InvocationKind::RootExecution,
            &profile,
            request,
            self.snapshot_execution_surface().await,
            None,
        )?;
        validate_root_execution_storage_mode(
            prepared_execution
                .execution_spec
                .resolved_overrides
                .storage_mode,
        )?;

        let session_meta = runtime.sessions().create(working_dir).await?;
        let session_state = runtime
            .ensure_session_loaded(&session_meta.session_id)
            .await?;
        let session_manager = Arc::clone(&runtime.session_manager);
        let session_cancel = CancelToken::new();
        let turn_id = Uuid::new_v4().to_string();
        let session_id_for_lease = session_meta.session_id.clone();
        let turn_id_for_lease = turn_id.clone();
        let turn_lease_result =
            spawn_blocking_service("acquire root execution turn lease", move || {
                session_manager
                    .try_acquire_turn(&session_id_for_lease, &turn_id_for_lease)
                    .map_err(ServiceError::from)
            })
            .await?;
        let turn_lease = match turn_lease_result {
            SessionTurnAcquireResult::Acquired(turn_lease) => Ok(turn_lease),
            SessionTurnAcquireResult::Busy(active_turn) => Err(ServiceError::Conflict(format!(
                "session '{}' is already executing turn '{}'",
                session_meta.session_id, active_turn.turn_id
            ))),
        }?;
        let root_agent_id = format!("root-agent-{}", Uuid::new_v4());
        let budget_settings = BudgetSettings {
            continuation_min_delta_tokens: crate::config::resolve_continuation_min_delta_tokens(
                &prepared_execution.runtime_config,
            ),
            max_continuations: crate::config::resolve_max_continuations(
                &prepared_execution.runtime_config,
            ),
        };

        prepare_session_execution(
            &session_state,
            &session_meta.session_id,
            &turn_id,
            session_cancel.clone(),
            turn_lease,
            None,
        )
        .map_err(ServiceError::from)?;

        let observability = runtime.observability.clone();
        let session_state_for_task = Arc::clone(&session_state);
        let accepted_turn_id = turn_id.clone();
        // 在 spawn 前克隆 agent_control，避免借用 `self` 逃逸到 'static 闭包
        let agent_control = self.control();
        let execution_service = self.clone();
        let drain_session_id = session_meta.session_id.clone();
        let launch = prepare_root_execution_launch(
            &session_meta.session_id,
            &turn_id,
            root_agent_id.clone(),
            profile.id.clone(),
            prepared_execution
                .execution_spec
                .resolved_context_snapshot
                .task_payload
                .clone(),
        );
        let handle = tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let task_result = run_session_turn(
                &session_state_for_task,
                &prepared_execution.loop_,
                &turn_id,
                session_cancel.clone(),
                RuntimeTurnInput::from_user_event(launch.user_event),
                launch.agent,
                launch.execution_owner,
                budget_settings,
                Some(observability.clone()),
            )
            .await;
            complete_session_execution(&session_state_for_task, task_result.phase, &agent_control)
                .await;
            if let Err(error) = execution_service
                .try_start_parent_delivery_turn(&drain_session_id)
                .await
            {
                log::warn!(
                    "failed to drain parent delivery queue after root turn '{}' completed: {}",
                    turn_id,
                    error
                );
            }

            let elapsed = turn_started_at.elapsed();
            observability.record_turn_execution(elapsed, task_result.succeeded);
        });
        // 保存 root execution 的 JoinHandle 以便 shutdown 时 abort。
        self.runtime.lifecycle().register_turn_task(handle);

        Ok(ExecutionAccepted {
            session_id: session_meta.session_id,
            turn_id: accepted_turn_id,
            agent_id: Some(root_agent_id),
            branched_from_session_id: None,
        })
    }
}

impl ToolExecutionServiceHandle {
    pub async fn list_tools(&self) -> Vec<ToolSummary> {
        let surface = self.runtime.surface.read().await;
        let mut tools = surface
            .capabilities
            .descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind.is_tool())
            .map(|descriptor| ToolSummary {
                name: descriptor.name,
                description: descriptor.description,
                profiles: descriptor.profiles,
                streaming: descriptor.streaming,
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools
    }
}
