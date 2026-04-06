use std::{path::PathBuf, sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, AgentMode, CancelToken, ExecutionOwner, InvocationKind,
    SessionTurnAcquireResult, SubRunStorageMode, SubagentContextOverrides, UserMessageOrigin,
};
use astrcode_runtime_session::{
    complete_session_execution, prepare_session_execution, run_session_turn,
};
use chrono::Utc;
use uuid::Uuid;

use crate::service::{
    AgentExecutionAccepted, RuntimeService, ServiceError, ServiceResult,
    blocking_bridge::spawn_blocking_service, turn::BudgetSettings,
};

/// 面向 API / Tool 的 Agent Profile 摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub max_steps: Option<u32>,
    pub token_budget: Option<u64>,
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
    pub(super) runtime: Arc<RuntimeService>,
}

/// Tool 执行服务句柄。
#[derive(Clone)]
pub struct ToolExecutionServiceHandle {
    pub(super) runtime: Arc<RuntimeService>,
}

impl AgentExecutionServiceHandle {
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
                max_steps: profile.max_steps,
                token_budget: profile.token_budget,
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
        max_steps: Option<u32>,
        context_overrides: Option<SubagentContextOverrides>,
        working_dir: PathBuf,
    ) -> ServiceResult<AgentExecutionAccepted> {
        let params = astrcode_runtime_agent_tool::RunAgentParams {
            name: agent_id,
            task,
            context,
            max_steps,
            context_overrides,
        };
        let runtime = &self.runtime;
        let profiles = runtime.agent_profiles();
        let profile = profiles.get(&params.name).cloned().ok_or_else(|| {
            ServiceError::InvalidInput(format!("unknown agent profile '{}'", params.name))
        })?;
        astrcode_runtime_execution::ensure_root_execution_mode(&profile)?;
        let prepared_execution = self.prepare_scoped_execution(
            InvocationKind::RootExecution,
            &profile,
            &params,
            self.snapshot_execution_surface().await,
            None,
        )?;
        if matches!(
            prepared_execution
                .execution_spec
                .resolved_overrides
                .storage_mode,
            SubRunStorageMode::IndependentSession
        ) {
            return Err(ServiceError::InvalidInput(
                "root execution already runs in its own session; \
                 contextOverrides.storageMode=independentSession is not applicable"
                    .to_string(),
            ));
        }

        let session_meta = runtime.create_session(working_dir).await?;
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
        let root_agent =
            AgentEventContext::root_execution(root_agent_id.clone(), profile.id.clone());
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
        let agent_control = runtime.agent_control.clone();
        let session_state_for_task = Arc::clone(&session_state);
        let accepted_turn_id = turn_id.clone();
        let root_execution_owner = ExecutionOwner::root(
            session_meta.session_id.clone(),
            turn_id.clone(),
            InvocationKind::RootExecution,
        );
        tokio::spawn(async move {
            let turn_started_at = Instant::now();
            let user_event = astrcode_core::StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                agent: root_agent.clone(),
                content: prepared_execution
                    .execution_spec
                    .resolved_context_snapshot
                    .composed_task
                    .clone(),
                timestamp: Utc::now(),
                origin: UserMessageOrigin::User,
            };
            let task_result = run_session_turn(
                &session_state_for_task,
                &prepared_execution.loop_,
                &turn_id,
                session_cancel.clone(),
                user_event,
                root_agent.clone(),
                root_execution_owner.clone(),
                budget_settings,
            )
            .await;
            complete_session_execution(
                &session_state_for_task,
                &agent_control,
                &turn_id,
                task_result.phase,
            )
            .await;

            let elapsed = turn_started_at.elapsed();
            observability.record_turn_execution(elapsed, task_result.succeeded);
        });

        Ok(AgentExecutionAccepted {
            session_id: session_meta.session_id,
            turn_id: accepted_turn_id,
            agent_id: root_agent_id,
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
