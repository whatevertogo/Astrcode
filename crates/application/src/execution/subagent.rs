//! 子代理执行入口。
//!
//! 实现 `launch_subagent`：spawn 参数解析 → profile 校验 → child session 创建 → turn 启动。
//! 子代理执行结果通过 parent delivery 机制回流到父级。

use std::sync::Arc;

use astrcode_core::{
    AgentLifecycleStatus, AgentMode, AgentProfile, ExecutionAccepted, ModeId,
    ResolvedRuntimeConfig, RuntimeMetricsRecorder, SpawnCapabilityGrant,
};
use astrcode_kernel::AgentControlError;

use crate::{
    AgentKernelPort, AgentSessionPort,
    agent::{
        persist_delegation_for_handle, persist_resolved_limits_for_handle, subrun_event_context,
    },
    errors::ApplicationError,
    execution::{ensure_profile_mode, merge_task_with_context},
    governance_surface::{
        FreshChildGovernanceInput, GovernanceBusyPolicy, GovernanceSurfaceAssembler,
        build_delegation_metadata,
    },
};

/// 子代理执行请求。
pub struct SubagentExecutionRequest {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub parent_turn_id: String,
    pub working_dir: String,
    pub mode_id: ModeId,
    pub profile: AgentProfile,
    pub description: String,
    pub task: String,
    pub context: Option<String>,
    pub parent_allowed_tools: Vec<String>,
    pub capability_grant: Option<SpawnCapabilityGrant>,
    pub source_tool_call_id: Option<String>,
}

/// 启动子代理执行。
///
/// 完整流程：
/// 1. 参数校验
/// 2. 校验 profile mode
/// 3. 创建独立 child session
/// 4. 在控制树中注册子 agent
/// 5. 异步提交 prompt
pub async fn launch_subagent(
    kernel: &dyn AgentKernelPort,
    session_runtime: &dyn AgentSessionPort,
    governance: &GovernanceSurfaceAssembler,
    request: SubagentExecutionRequest,
    runtime_config: ResolvedRuntimeConfig,
    metrics: &Arc<dyn RuntimeMetricsRecorder>,
) -> Result<ExecutionAccepted, ApplicationError> {
    validate_subagent_request(&request)?;
    ensure_subagent_profile_mode(&request.profile)?;
    let surface = governance
        .fresh_child_surface(
            kernel,
            session_runtime,
            FreshChildGovernanceInput {
                session_id: request.parent_session_id.clone(),
                turn_id: request.parent_turn_id.clone(),
                working_dir: request.working_dir.clone(),
                mode_id: request.mode_id.clone(),
                runtime: runtime_config,
                parent_allowed_tools: request.parent_allowed_tools.clone(),
                capability_grant: request.capability_grant.clone(),
                description: request.description.clone(),
                task: request.task.clone(),
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )
        .await?;
    let delegation = build_delegation_metadata(
        request.description.as_str(),
        request.task.as_str(),
        &surface.resolved_limits,
        request.capability_grant.is_some(),
    );

    let child_session = session_runtime
        .create_child_session(&request.working_dir, &request.parent_session_id)
        .await
        .map_err(ApplicationError::from)?;

    let handle = kernel
        .spawn_independent_child(
            &request.profile,
            request.parent_session_id.clone(),
            child_session.session_id.clone(),
            request.parent_turn_id,
            request.parent_agent_id,
        )
        .await
        .map_err(map_spawn_error)?;
    if kernel
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await
        .is_none()
    {
        return Err(ApplicationError::Internal(format!(
            "failed to mark child agent '{}' as running because the control handle disappeared \
             immediately after spawn",
            handle.agent_id
        )));
    }
    let handle =
        persist_resolved_limits_for_handle(kernel, handle, surface.resolved_limits.clone())
            .await
            .map_err(ApplicationError::Internal)?;
    let handle = persist_delegation_for_handle(kernel, handle, delegation)
        .await
        .map_err(ApplicationError::Internal)?;
    let merged_task = merge_task_with_context(&request.task, request.context.as_deref());

    let mut accepted = session_runtime
        .submit_prompt_for_agent_with_submission(
            &child_session.session_id,
            merged_task,
            surface.runtime.clone(),
            surface.into_submission(
                subrun_event_context(&handle),
                request.source_tool_call_id.clone(),
            ),
        )
        .await
        .map_err(ApplicationError::from)?;
    metrics.record_child_spawned();
    accepted.agent_id = Some(handle.agent_id);
    Ok(accepted)
}

fn validate_subagent_request(request: &SubagentExecutionRequest) -> Result<(), ApplicationError> {
    if request.parent_session_id.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'parentSessionId' must not be empty".to_string(),
        ));
    }
    if request.parent_agent_id.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'parentAgentId' must not be empty".to_string(),
        ));
    }
    if request.working_dir.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'workingDir' must not be empty".to_string(),
        ));
    }
    if request.task.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'task' must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn ensure_subagent_profile_mode(profile: &AgentProfile) -> Result<(), ApplicationError> {
    ensure_profile_mode(profile, &[AgentMode::SubAgent, AgentMode::All], "subagent")
}

/// 将 kernel spawn 错误映射为用户友好的应用层错误。
///
/// - MaxDepthExceeded → InvalidArgument（提示复用已有 child）
/// - MaxConcurrentExceeded → Conflict（提示等待或关闭已有 child）
/// - ParentAgentNotFound → NotFound
fn map_spawn_error(error: AgentControlError) -> ApplicationError {
    match error {
        AgentControlError::MaxDepthExceeded { current, max } => {
            ApplicationError::InvalidArgument(format!(
                "subagent depth limit reached at depth {current} (configured max: {max}); reuse \
                 an existing child with send/observe/close, or finish the work in the current \
                 agent"
            ))
        },
        AgentControlError::MaxConcurrentExceeded { current, max } => {
            ApplicationError::Conflict(format!(
                "too many active agents ({current}/{max}); wait for an existing child to go idle \
                 or close one before spawning more"
            ))
        },
        AgentControlError::ParentAgentNotFound { agent_id } => {
            ApplicationError::NotFound(format!("parent agent '{agent_id}' not found"))
        },
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentMode, AgentProfile};

    use super::*;
    use crate::governance_surface::GovernanceSurfaceAssembler;

    fn test_profile() -> AgentProfile {
        AgentProfile {
            id: "explore".to_string(),
            name: "Explore".to_string(),
            description: "探索代码".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            model_preference: None,
        }
    }

    fn valid_request() -> SubagentExecutionRequest {
        SubagentExecutionRequest {
            parent_session_id: "session-1".to_string(),
            parent_agent_id: "root-agent".to_string(),
            parent_turn_id: "turn-1".to_string(),
            working_dir: "/tmp/project".to_string(),
            mode_id: ModeId::default(),
            profile: test_profile(),
            description: "探索代码".to_string(),
            task: "explore the code".to_string(),
            context: None,
            parent_allowed_tools: vec!["read_file".to_string(), "grep".to_string()],
            capability_grant: None,
            source_tool_call_id: None,
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_subagent_request(&valid_request()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_parent_session_id() {
        let mut req = valid_request();
        req.parent_session_id = "  ".to_string();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("parentSessionId"),
            "should mention parentSessionId: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_parent_agent_id() {
        let mut req = valid_request();
        req.parent_agent_id = String::new();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("parentAgentId"),
            "should mention parentAgentId: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_working_dir() {
        let mut req = valid_request();
        req.working_dir.clear();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(err.to_string().contains("workingDir"));
    }

    #[test]
    fn validate_rejects_empty_task() {
        let mut req = valid_request();
        req.task = "   ".to_string();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("task"),
            "should mention task: {err}"
        );
    }

    #[test]
    fn validate_does_not_check_profile_fields() {
        let mut req = valid_request();
        req.profile.name = String::new();
        assert!(validate_subagent_request(&req).is_ok());
    }

    #[test]
    fn ensure_subagent_profile_mode_rejects_primary_only_profile() {
        let err = ensure_subagent_profile_mode(&AgentProfile {
            mode: AgentMode::Primary,
            ..test_profile()
        })
        .expect_err("primary-only profile should be rejected");

        assert!(err.to_string().contains("subagent"));
    }

    #[test]
    fn map_spawn_error_turns_depth_limit_into_actionable_invalid_argument() {
        let err = map_spawn_error(AgentControlError::MaxDepthExceeded { current: 4, max: 3 });

        assert!(matches!(err, ApplicationError::InvalidArgument(_)));
        assert!(err.to_string().contains("configured max: 3"));
        assert!(err.to_string().contains("send/observe/close"));
    }

    #[test]
    fn map_spawn_error_turns_concurrency_limit_into_conflict() {
        let err = map_spawn_error(AgentControlError::MaxConcurrentExceeded { current: 8, max: 4 });

        assert!(matches!(err, ApplicationError::Conflict(_)));
        assert!(err.to_string().contains("too many active agents"));
    }

    #[test]
    fn governance_surface_builder_exists_for_subagent_paths() {
        let _assembler = GovernanceSurfaceAssembler::default();
    }
}
