//! 根代理执行入口。
//!
//! 实现 `execute_root_agent`：参数校验 → profile 解析 → session 创建 → agent 注册到
//! 控制树 → 异步提交 prompt。
//!
//! `application` 只做编排，不持有 session 真相或 turn 执行细节。

use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentMode, ExecutionAccepted, ModeId, ResolvedRuntimeConfig, SubagentContextOverrides,
};

use crate::{
    AppKernelPort, AppSessionPort,
    agent::root_execution_event_context,
    errors::ApplicationError,
    execution::{
        ExecutionControl, ProfileResolutionService, ensure_profile_mode, merge_task_with_context,
    },
    governance_surface::{GovernanceSurfaceAssembler, RootGovernanceInput},
};

/// 根代理执行请求。
pub struct RootExecutionRequest {
    pub agent_id: String,
    pub working_dir: String,
    pub task: String,
    pub context: Option<String>,
    pub control: Option<ExecutionControl>,
    pub context_overrides: Option<SubagentContextOverrides>,
}

/// 执行根代理。
///
/// 完整流程：
/// 1. 参数校验
/// 2. 解析 root profile 并校验 mode
/// 3. 创建 session
/// 4. 注册根 agent 到控制树
/// 5. 合并 task + context
/// 6. 异步提交 prompt
pub async fn execute_root_agent(
    kernel: &dyn AppKernelPort,
    session_runtime: &dyn AppSessionPort,
    profiles: &Arc<ProfileResolutionService>,
    governance: &GovernanceSurfaceAssembler,
    request: RootExecutionRequest,
    runtime_config: ResolvedRuntimeConfig,
) -> Result<ExecutionAccepted, ApplicationError> {
    validate_root_request(&request)?;
    validate_root_context_overrides_supported(request.context_overrides.as_ref())?;

    let profile = profiles.find_profile(Path::new(&request.working_dir), &request.agent_id)?;
    ensure_root_profile_mode(&profile)?;
    let profile_id = profile.id.clone();

    let session = session_runtime
        .create_session(request.working_dir.clone())
        .await
        .map_err(ApplicationError::from)?;

    let handle = kernel
        .register_root_agent(
            request.agent_id.clone(),
            session.session_id.clone(),
            profile_id.clone(),
        )
        .await
        .map_err(|e| ApplicationError::Internal(format!("failed to register root agent: {e}")))?;
    let surface = governance.root_surface(
        kernel,
        RootGovernanceInput {
            session_id: session.session_id.clone(),
            turn_id: astrcode_core::generate_turn_id(),
            working_dir: request.working_dir.clone(),
            profile: profile_id.clone(),
            mode_id: ModeId::default(),
            runtime: runtime_config,
            control: request.control.clone(),
        },
    )?;
    let resolved_limits = surface.resolved_limits.clone();
    if kernel
        .set_resolved_limits(&handle.agent_id, resolved_limits.clone())
        .await
        .is_none()
    {
        return Err(ApplicationError::Internal(format!(
            "failed to persist resolved limits for root agent '{}' because the control handle \
             disappeared before the limits snapshot was recorded",
            handle.agent_id
        )));
    }
    let mut handle = handle;
    handle.resolved_limits = resolved_limits;

    let merged_task = merge_task_with_context(&request.task, request.context.as_deref());

    let mut accepted = session_runtime
        .submit_prompt_for_agent(
            &session.session_id,
            merged_task,
            surface.runtime.clone(),
            surface.into_submission(
                root_execution_event_context(handle.agent_id.clone(), profile_id),
                None,
            ),
        )
        .await
        .map_err(ApplicationError::from)?;
    accepted.agent_id = Some(request.agent_id.into());
    Ok(accepted)
}

fn validate_root_request(request: &RootExecutionRequest) -> Result<(), ApplicationError> {
    if request.agent_id.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'agentId' must not be empty".to_string(),
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
    if let Some(control) = &request.control {
        control.validate()?;
        if control.manual_compact.is_some() {
            return Err(ApplicationError::InvalidArgument(
                "manualCompact is not valid for root execution".to_string(),
            ));
        }
    }
    Ok(())
}

/// 校验根执行请求不支持 context overrides。
///
/// 根执行没有"父上下文"可继承，任何显式 overrides 都不会真正改变执行输入。
/// 宁可明确拒绝，也不要伪装成"已接受但生效未知"。
fn validate_root_context_overrides_supported(
    overrides: Option<&SubagentContextOverrides>,
) -> Result<(), ApplicationError> {
    let Some(overrides) = overrides else {
        return Ok(());
    };

    // 根执行当前没有“父上下文”可继承，任何显式 overrides 都不会真正改变执行输入。
    // 这里宁可明确拒绝，也不要把请求伪装成“已接受但生效未知”。
    if overrides != &SubagentContextOverrides::default() {
        return Err(ApplicationError::InvalidArgument(
            "contextOverrides is not supported yet for root execution".to_string(),
        ));
    }

    Ok(())
}

fn ensure_root_profile_mode(profile: &astrcode_core::AgentProfile) -> Result<(), ApplicationError> {
    ensure_profile_mode(
        profile,
        &[AgentMode::Primary, AgentMode::All],
        "root execution",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> RootExecutionRequest {
        RootExecutionRequest {
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
    fn validate_rejects_empty_agent_id() {
        let mut req = valid_request();
        req.agent_id = "   ".to_string();
        let err = validate_root_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("agentId"),
            "should mention agentId: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_working_dir() {
        let mut req = valid_request();
        req.working_dir = String::new();
        let err = validate_root_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("workingDir"),
            "should mention workingDir: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_task() {
        let mut req = valid_request();
        req.task = "  ".to_string();
        let err = validate_root_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("task"),
            "should mention task: {err}"
        );
    }

    #[test]
    fn validate_accepts_context_but_uses_empty_as_none() {
        let req = RootExecutionRequest {
            agent_id: "agent".to_string(),
            working_dir: "/tmp".to_string(),
            task: "task".to_string(),
            context: Some("".to_string()),
            control: None,
            context_overrides: None,
        };
        assert!(validate_root_request(&req).is_ok());
    }

    #[test]
    fn validate_rejects_manual_compact_control() {
        let mut req = valid_request();
        req.control = Some(ExecutionControl {
            manual_compact: Some(true),
        });

        let err = validate_root_request(&req).unwrap_err();
        assert!(err.to_string().contains("manualCompact"));
    }

    #[test]
    fn merge_context_and_task() {
        let merged = merge_task_with_context("main task", Some("background info"));
        assert_eq!(merged, "background info\n\nmain task");
    }

    #[test]
    fn merge_skips_empty_context() {
        let merged = merge_task_with_context("main task", Some("  "));
        assert_eq!(merged, "main task");
    }

    #[test]
    fn validate_root_context_overrides_accepts_empty_overrides() {
        validate_root_context_overrides_supported(Some(&SubagentContextOverrides::default()))
            .expect("empty overrides should pass");
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
    fn root_execution_rejects_subagent_only_profile() {
        let err = ensure_root_profile_mode(&astrcode_core::AgentProfile {
            id: "explore".to_string(),
            name: "Explore".to_string(),
            description: "subagent".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            model_preference: None,
        })
        .expect_err("subagent-only profile should be rejected");

        assert!(err.to_string().contains("root execution"));
    }
}
