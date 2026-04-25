//! server-owned agent runtime bridge builder。
//!
//! 把 application orchestration / governance-surface 组装细节收敛到单独
//! bridge 文件，避免组合根直接消费 app service 类型。

use std::{path::Path, sync::Arc};

use astrcode_governance_contract::ModeId;
use astrcode_host_session::{CollaborationExecutor, SubAgentExecutor};

use crate::{
    AgentOrchestrationService, ApplicationError, GovernanceSurfaceAssembler, ProfileProvider,
    ProfileResolutionService, ResolvedGovernanceSurface, RootGovernanceInput,
    agent_api::ServerAgentApi,
    agent_control_bridge::ServerAgentControlPort,
    application_error_bridge::ServerRouteError,
    config_service_bridge::ServerConfigService,
    mode_catalog_service::ServerModeCatalog,
    ports::{AgentKernelPort, AgentSessionPort, AppSessionPort},
    profile_service::ServerProfileService,
    root_execute_service::ServerRootExecuteService,
    runtime_owner_bridge::{ServerRuntimeObservability, ServerTaskRegistry},
};

pub(crate) struct ServerAgentRuntimeBundle {
    pub agent_api: Arc<ServerAgentApi>,
    pub agent_control: Arc<dyn ServerAgentControlPort>,
    pub subagent_executor: Arc<dyn SubAgentExecutor>,
    pub collaboration_executor: Arc<dyn CollaborationExecutor>,
}

pub(crate) struct ServerAgentRuntimeBuildInput {
    pub agent_kernel: Arc<dyn AgentKernelPort>,
    pub agent_sessions: Arc<dyn AgentSessionPort>,
    pub app_sessions: Arc<dyn AppSessionPort>,
    pub agent_control: Arc<dyn ServerAgentControlPort>,
    pub config_service: Arc<ServerConfigService>,
    pub profiles: Arc<ServerProfileService>,
    pub mode_catalog: Arc<ServerModeCatalog>,
    pub task_registry: Arc<ServerTaskRegistry>,
    pub observability: Arc<ServerRuntimeObservability>,
}

pub(crate) fn build_server_agent_runtime_bundle(
    input: ServerAgentRuntimeBuildInput,
) -> ServerAgentRuntimeBundle {
    let ServerAgentRuntimeBuildInput {
        agent_kernel,
        agent_sessions,
        app_sessions,
        agent_control,
        config_service,
        profiles,
        mode_catalog,
        task_registry,
        observability,
    } = input;
    let profile_resolution = build_profile_resolution_service(profiles.clone());
    let governance_surface = Arc::new(GovernanceSurfaceAssembler::new((*mode_catalog).clone()));
    let agent_service = Arc::new(AgentOrchestrationService::new(
        agent_kernel,
        agent_sessions.clone(),
        Arc::clone(config_service.inner()),
        profile_resolution,
        Arc::clone(&governance_surface),
        task_registry.inner(),
        observability,
    ));
    let agent_api = Arc::new(ServerAgentApi::new(
        agent_control.clone(),
        app_sessions.clone(),
        profiles.clone(),
        Arc::new(ServerRootExecuteService::new(
            Arc::clone(&agent_control),
            app_sessions,
            profiles,
            config_service,
            Arc::new(ApplicationRootGovernancePort::new(governance_surface)),
        )),
    ));
    let subagent_executor: Arc<dyn SubAgentExecutor> = agent_service.clone();
    let collaboration_executor: Arc<dyn CollaborationExecutor> = agent_service;

    ServerAgentRuntimeBundle {
        agent_api,
        agent_control,
        subagent_executor,
        collaboration_executor,
    }
}

fn build_profile_resolution_service(
    profiles: Arc<ServerProfileService>,
) -> Arc<ProfileResolutionService> {
    Arc::new(ProfileResolutionService::new(Arc::new(
        ServerProfileProviderAdapter { profiles },
    )))
}

struct ApplicationRootGovernancePort {
    assembler: Arc<GovernanceSurfaceAssembler>,
}

struct ServerProfileProviderAdapter {
    profiles: Arc<ServerProfileService>,
}

impl ApplicationRootGovernancePort {
    fn new(assembler: Arc<GovernanceSurfaceAssembler>) -> Self {
        Self { assembler }
    }
}

impl crate::root_execute_service::ServerRootGovernancePort for ApplicationRootGovernancePort {
    fn prepare_root_submission(
        &self,
        input: crate::root_execute_service::ServerRootGovernanceInput,
    ) -> Result<crate::root_execute_service::ServerPreparedRootExecution, ServerRouteError> {
        let surface = self
            .assembler
            .root_surface(RootGovernanceInput {
                session_id: input.session_id,
                turn_id: input.turn_id,
                working_dir: input.working_dir,
                profile: input.profile_id.clone(),
                mode_id: ModeId::default(),
                runtime: input.runtime,
                control: input.control,
            })
            .map_err(application_error_to_server)?;
        prepared_root_execution_from_surface(input.agent_id, input.profile_id, surface)
    }
}

impl ProfileProvider for ServerProfileProviderAdapter {
    fn load_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> Result<Vec<astrcode_core::AgentProfile>, ApplicationError> {
        self.profiles
            .resolve(working_dir)
            .map(|profiles| profiles.as_ref().clone())
            .map_err(server_route_error_to_application_error)
    }

    fn load_global(&self) -> Result<Vec<astrcode_core::AgentProfile>, ApplicationError> {
        self.profiles
            .resolve_global()
            .map(|profiles| profiles.as_ref().clone())
            .map_err(server_route_error_to_application_error)
    }
}

fn server_route_error_to_application_error(error: ServerRouteError) -> ApplicationError {
    match error {
        ServerRouteError::NotFound(message) => ApplicationError::NotFound(message),
        ServerRouteError::Conflict(message) => ApplicationError::Conflict(message),
        ServerRouteError::InvalidArgument(message) => ApplicationError::InvalidArgument(message),
        ServerRouteError::PermissionDenied(message) => ApplicationError::PermissionDenied(message),
        ServerRouteError::Internal(message) => ApplicationError::Internal(message),
    }
}

fn application_error_to_server(error: ApplicationError) -> ServerRouteError {
    match error {
        ApplicationError::NotFound(message) => ServerRouteError::NotFound(message),
        ApplicationError::Conflict(message) => ServerRouteError::Conflict(message),
        ApplicationError::InvalidArgument(message) => ServerRouteError::InvalidArgument(message),
        ApplicationError::PermissionDenied(message) => ServerRouteError::PermissionDenied(message),
        ApplicationError::Internal(message) => ServerRouteError::Internal(message),
    }
}

fn prepared_root_execution_from_surface(
    agent_id: String,
    profile_id: String,
    surface: ResolvedGovernanceSurface,
) -> Result<crate::root_execute_service::ServerPreparedRootExecution, ServerRouteError> {
    let runtime = surface.runtime.clone();
    let resolved_limits = surface.resolved_limits.clone();
    let submission = surface.into_submission(
        astrcode_core::AgentEventContext::root_execution(agent_id, profile_id),
        None,
    );

    Ok(crate::root_execute_service::ServerPreparedRootExecution {
        runtime,
        resolved_limits,
        submission,
    })
}
