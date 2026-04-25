//! server-owned root execute bridge。
//!
//! agent route / runtime state 只暴露 server-owned 根执行入口和治理装配类型。

use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentMode, AgentProfile, ExecutionControl, ResolvedExecutionLimitsSnapshot,
    ResolvedRuntimeConfig, SubagentContextOverrides, generate_turn_id,
};
use astrcode_runtime_contract::ExecutionSubmissionOutcome;

use crate::{
    agent::implicit_session_root_agent_id,
    agent_control_bridge::ServerAgentControlPort,
    application_error_bridge::ServerRouteError,
    config_service_bridge::ServerConfigService,
    ports::{AppAgentPromptSubmission, AppSessionPort},
    profile_service::ServerProfileService,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerRootExecutionRequest {
    pub agent_id: String,
    pub working_dir: String,
    pub task: String,
    pub context: Option<String>,
    pub control: Option<ExecutionControl>,
    pub context_overrides: Option<SubagentContextOverrides>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerSessionPromptRequest {
    pub session_id: String,
    pub working_dir: String,
    pub text: String,
    pub control: Option<ExecutionControl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerAgentExecuteSummary {
    pub accepted: bool,
    pub message: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub branched_from_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerRootGovernanceInput {
    pub agent_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub profile_id: String,
    pub runtime: ResolvedRuntimeConfig,
    pub control: Option<ExecutionControl>,
}

#[derive(Clone)]
pub(crate) struct ServerPreparedRootExecution {
    pub runtime: ResolvedRuntimeConfig,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub submission: AppAgentPromptSubmission,
}

pub(crate) trait ServerRootGovernancePort: Send + Sync {
    fn prepare_root_submission(
        &self,
        input: ServerRootGovernanceInput,
    ) -> Result<ServerPreparedRootExecution, ServerRouteError>;
}

#[derive(Clone)]
pub(crate) struct ServerRootExecuteService {
    agent_control: Arc<dyn ServerAgentControlPort>,
    sessions: Arc<dyn AppSessionPort>,
    profiles: Arc<ServerProfileService>,
    config_service: Arc<ServerConfigService>,
    governance: Arc<dyn ServerRootGovernancePort>,
}

impl ServerRootExecuteService {
    pub(crate) fn new(
        agent_control: Arc<dyn ServerAgentControlPort>,
        sessions: Arc<dyn AppSessionPort>,
        profiles: Arc<ServerProfileService>,
        config_service: Arc<ServerConfigService>,
        governance: Arc<dyn ServerRootGovernancePort>,
    ) -> Self {
        Self {
            agent_control,
            sessions,
            profiles,
            config_service,
            governance,
        }
    }

    pub(crate) async fn execute_summary(
        &self,
        request: ServerRootExecutionRequest,
    ) -> Result<ServerAgentExecuteSummary, ServerRouteError> {
        validate_root_request(&request)?;
        validate_root_context_overrides_supported(request.context_overrides.as_ref())?;

        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&request.working_dir)))?;
        let profile = self
            .profiles
            .find_profile(Path::new(&request.working_dir), &request.agent_id)?;
        ensure_root_profile_mode(&profile)?;
        let profile_id = profile.id.clone();

        let session = self
            .sessions
            .create_session(request.working_dir.clone())
            .await
            .map_err(ServerRouteError::from)?;
        let turn_id = generate_turn_id();
        let handle = self
            .agent_control
            .register_root_agent(
                request.agent_id.clone(),
                session.session_id.clone(),
                profile_id.clone(),
            )
            .await?;
        let prepared = self
            .governance
            .prepare_root_submission(ServerRootGovernanceInput {
                agent_id: request.agent_id.clone(),
                session_id: session.session_id.clone(),
                turn_id: turn_id.clone(),
                working_dir: request.working_dir.clone(),
                profile_id: profile_id.clone(),
                runtime,
                control: request.control.clone(),
            })?;
        let resolved_limits = prepared.resolved_limits.clone();
        if !self
            .agent_control
            .set_resolved_limits(&handle.agent_id, resolved_limits)
            .await
        {
            return Err(ServerRouteError::internal(format!(
                "failed to persist resolved limits for root agent '{}' because the control handle \
                 disappeared before the limits snapshot was recorded",
                handle.agent_id
            )));
        }

        let outcome = self
            .sessions
            .submit_prompt_for_agent(
                &session.session_id,
                merge_task_with_context(&request.task, request.context.as_deref()),
                prepared.runtime,
                prepared.submission,
            )
            .await
            .map_err(ServerRouteError::from)?;
        let agent_id = request.agent_id.clone();
        match outcome {
            ExecutionSubmissionOutcome::Accepted(accepted) => {
                let session_id = accepted.session_id.to_string();
                Ok(ServerAgentExecuteSummary {
                    accepted: true,
                    message: format!(
                        "agent '{}' execution accepted; subscribe to \
                         /api/v1/conversation/sessions/{}/stream for progress",
                        agent_id, session_id
                    ),
                    session_id: Some(session_id),
                    turn_id: Some(accepted.turn_id.to_string()),
                    agent_id: Some(agent_id),
                    branched_from_session_id: accepted.branched_from_session_id,
                })
            },
            ExecutionSubmissionOutcome::Handled {
                session_id,
                response,
            } => Ok(ServerAgentExecuteSummary {
                accepted: false,
                message: response,
                session_id: Some(session_id.to_string()),
                turn_id: None,
                agent_id: Some(agent_id),
                branched_from_session_id: None,
            }),
        }
    }

    pub(crate) async fn submit_existing_session_prompt(
        &self,
        request: ServerSessionPromptRequest,
    ) -> Result<ServerAgentExecuteSummary, ServerRouteError> {
        validate_session_prompt_request(&request)?;

        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&request.working_dir)))?;
        let root_status = self
            .agent_control
            .query_root_status(&request.session_id)
            .await;
        let (agent_id, profile_id) = root_status
            .as_ref()
            .map(|status| (status.agent_id.clone(), status.agent_profile.clone()))
            .unwrap_or_else(|| {
                (
                    implicit_session_root_agent_id(&request.session_id),
                    crate::agent::IMPLICIT_ROOT_PROFILE_ID.to_string(),
                )
            });
        let prepared = self
            .governance
            .prepare_root_submission(ServerRootGovernanceInput {
                agent_id: agent_id.clone(),
                session_id: request.session_id.clone(),
                turn_id: generate_turn_id(),
                working_dir: request.working_dir.clone(),
                profile_id: profile_id.clone(),
                runtime,
                control: request.control.clone(),
            })?;
        let resolved_limits = prepared.resolved_limits.clone();
        if root_status.is_none() {
            self.agent_control
                .register_root_agent(
                    agent_id.clone(),
                    request.session_id.clone(),
                    profile_id.clone(),
                )
                .await?;
        }
        if !self
            .agent_control
            .set_resolved_limits(&agent_id, resolved_limits)
            .await
        {
            return Err(ServerRouteError::internal(format!(
                "failed to persist resolved limits for root agent '{}' because the control handle \
                 disappeared before the limits snapshot was recorded",
                agent_id
            )));
        }

        let outcome = self
            .sessions
            .submit_prompt_for_agent(
                &request.session_id,
                request.text,
                prepared.runtime,
                prepared.submission,
            )
            .await
            .map_err(ServerRouteError::from)?;

        match outcome {
            ExecutionSubmissionOutcome::Accepted(accepted) => Ok(ServerAgentExecuteSummary {
                accepted: true,
                message: format!(
                    "session '{}' prompt accepted; subscribe to \
                     /api/v1/conversation/sessions/{}/stream for progress",
                    request.session_id, accepted.session_id
                ),
                session_id: Some(accepted.session_id.to_string()),
                turn_id: Some(accepted.turn_id.to_string()),
                agent_id: Some(agent_id),
                branched_from_session_id: accepted.branched_from_session_id,
            }),
            ExecutionSubmissionOutcome::Handled {
                session_id,
                response,
            } => Ok(ServerAgentExecuteSummary {
                accepted: false,
                message: response,
                session_id: Some(session_id.to_string()),
                turn_id: None,
                agent_id: Some(agent_id),
                branched_from_session_id: None,
            }),
        }
    }
}

fn validate_root_request(request: &ServerRootExecutionRequest) -> Result<(), ServerRouteError> {
    if request.agent_id.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'agentId' must not be empty",
        ));
    }
    if request.working_dir.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'workingDir' must not be empty",
        ));
    }
    if request.task.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'task' must not be empty",
        ));
    }
    if let Some(control) = &request.control {
        control.validate().map_err(ServerRouteError::from)?;
        if control.manual_compact.is_some() {
            return Err(ServerRouteError::invalid_argument(
                "manualCompact is not valid for root execution",
            ));
        }
    }
    Ok(())
}

fn validate_session_prompt_request(
    request: &ServerSessionPromptRequest,
) -> Result<(), ServerRouteError> {
    if request.session_id.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'sessionId' must not be empty",
        ));
    }
    if request.working_dir.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'workingDir' must not be empty",
        ));
    }
    if request.text.trim().is_empty() {
        return Err(ServerRouteError::invalid_argument(
            "field 'text' must not be empty",
        ));
    }
    if let Some(control) = &request.control {
        control.validate().map_err(ServerRouteError::from)?;
        if control.manual_compact.is_some() {
            return Err(ServerRouteError::invalid_argument(
                "manualCompact is not valid for prompt submission",
            ));
        }
    }
    Ok(())
}

fn validate_root_context_overrides_supported(
    overrides: Option<&SubagentContextOverrides>,
) -> Result<(), ServerRouteError> {
    let Some(overrides) = overrides else {
        return Ok(());
    };
    if overrides != &SubagentContextOverrides::default() {
        return Err(ServerRouteError::invalid_argument(
            "contextOverrides is not supported yet for root execution",
        ));
    }
    Ok(())
}

fn ensure_root_profile_mode(profile: &AgentProfile) -> Result<(), ServerRouteError> {
    if matches!(profile.mode, AgentMode::Primary | AgentMode::All) {
        return Ok(());
    }

    Err(ServerRouteError::invalid_argument(format!(
        "agent profile '{}' cannot be used for root execution",
        profile.id
    )))
}

fn merge_task_with_context(task: &str, context: Option<&str>) -> String {
    match context {
        Some(context) if !context.trim().is_empty() => {
            format!("{}\n\n{}", context.trim(), task)
        },
        _ => task.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::SubagentContextOverrides;

    use super::{
        ServerRootExecutionRequest, merge_task_with_context,
        validate_root_context_overrides_supported, validate_root_request,
    };

    fn valid_request() -> ServerRootExecutionRequest {
        ServerRootExecutionRequest {
            agent_id: "root-agent".to_string(),
            working_dir: "/tmp/project".to_string(),
            task: "do something".to_string(),
            context: None,
            control: None,
            context_overrides: None,
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_root_request(&valid_request()).is_ok());
    }

    #[test]
    fn validate_rejects_manual_compact_control() {
        let mut request = valid_request();
        request.control = Some(astrcode_core::ExecutionControl {
            manual_compact: Some(true),
        });

        let error = validate_root_request(&request).expect_err("manual compact should fail");

        assert!(error.to_string().contains("manualCompact"));
    }

    #[test]
    fn validate_root_context_overrides_rejects_non_empty_override() {
        let error = validate_root_context_overrides_supported(Some(&SubagentContextOverrides {
            include_compact_summary: Some(true),
            ..SubagentContextOverrides::default()
        }))
        .expect_err("non-empty overrides should fail");

        assert!(error.to_string().contains("contextOverrides"));
    }

    #[test]
    fn merge_context_and_task() {
        assert_eq!(
            merge_task_with_context("main task", Some("background info")),
            "background info\n\nmain task"
        );
    }
}
