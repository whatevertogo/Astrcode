//! 根代理执行入口。
//!
//! 实现 `execute_root_agent`：参数校验 → profile 解析 → session 创建 → agent 注册到
//! 控制树 → 异步提交 prompt。
//!
//! `application` 只做编排，不持有 session 真相或 turn 执行细节。

use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentMode, ExecutionAccepted, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
    SubagentContextOverrides,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;

use crate::{
    agent::root_execution_event_context,
    errors::ApplicationError,
    execution::{ExecutionControl, ProfileResolutionService},
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
    kernel: &Arc<Kernel>,
    session_runtime: &Arc<SessionRuntime>,
    profiles: &Arc<ProfileResolutionService>,
    request: RootExecutionRequest,
    mut runtime_config: ResolvedRuntimeConfig,
) -> Result<ExecutionAccepted, ApplicationError> {
    validate_root_request(&request)?;
    apply_execution_control(&mut runtime_config, request.control.as_ref());
    let _resolved_context_overrides =
        resolve_root_context_overrides(request.context_overrides.as_ref())?;

    let profile = profiles.find_profile(Path::new(&request.working_dir), &request.agent_id)?;
    ensure_root_profile_mode(&profile)?;
    let profile_id = profile.id.clone();

    let session = session_runtime
        .create_session(&request.working_dir)
        .await
        .map_err(ApplicationError::from)?;

    kernel
        .agent_control()
        .register_root_agent(
            request.agent_id.clone(),
            session.session_id.clone(),
            profile_id.clone(),
        )
        .await
        .map_err(|e| ApplicationError::Internal(format!("failed to register root agent: {e}")))?;

    let merged_task = match &request.context {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!("{}\n\n{}", ctx.trim(), request.task)
        },
        _ => request.task,
    };

    let mut accepted = session_runtime
        .submit_prompt_for_agent(
            &session.session_id,
            merged_task,
            runtime_config,
            root_execution_event_context(request.agent_id.clone(), profile_id),
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

fn resolve_root_context_overrides(
    overrides: Option<&SubagentContextOverrides>,
) -> Result<ResolvedSubagentContextOverrides, ApplicationError> {
    let resolved = ResolvedSubagentContextOverrides::default();
    let Some(overrides) = overrides else {
        return Ok(resolved);
    };

    // 根执行当前没有“父上下文”可继承，任何显式 overrides 都不会真正改变执行输入。
    // 这里宁可明确拒绝，也不要把请求伪装成“已接受但生效未知”。
    if overrides != &SubagentContextOverrides::default() {
        return Err(ApplicationError::InvalidArgument(
            "contextOverrides is not supported yet for root execution".to_string(),
        ));
    }

    Ok(resolved)
}

fn ensure_root_profile_mode(profile: &astrcode_core::AgentProfile) -> Result<(), ApplicationError> {
    if matches!(profile.mode, AgentMode::Primary | AgentMode::All) {
        return Ok(());
    }
    Err(ApplicationError::InvalidArgument(format!(
        "agent profile '{}' cannot be used for root execution",
        profile.id
    )))
}

fn apply_execution_control(
    runtime_config: &mut ResolvedRuntimeConfig,
    control: Option<&ExecutionControl>,
) {
    let Some(control) = control else {
        return;
    };
    if let Some(max_steps) = control.max_steps {
        runtime_config.max_steps = max_steps as usize;
    }
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
            ..ExecutionControl::default()
        });

        let err = validate_root_request(&req).unwrap_err();
        assert!(err.to_string().contains("manualCompact"));
    }

    #[test]
    fn merge_context_and_task() {
        let merged = match &Some("background info".to_string()) {
            Some(ctx) if !ctx.trim().is_empty() => {
                format!("{}\n\n{}", ctx.trim(), "main task")
            },
            _ => "main task".to_string(),
        };
        assert_eq!(merged, "background info\n\nmain task");
    }

    #[test]
    fn merge_skips_empty_context() {
        let merged = match &Some("  ".to_string()) {
            Some(ctx) if !ctx.trim().is_empty() => {
                format!("{}\n\n{}", ctx.trim(), "main task")
            },
            _ => "main task".to_string(),
        };
        assert_eq!(merged, "main task");
    }

    #[test]
    fn apply_execution_control_overrides_runtime_config() {
        let mut runtime = ResolvedRuntimeConfig::default();
        apply_execution_control(
            &mut runtime,
            Some(&ExecutionControl {
                max_steps: Some(5),
                manual_compact: None,
            }),
        );

        assert_eq!(runtime.max_steps, 5);
    }

    #[test]
    fn resolve_root_context_overrides_accepts_empty_overrides() {
        let resolved = resolve_root_context_overrides(Some(&SubagentContextOverrides::default()))
            .expect("empty overrides should pass");

        assert_eq!(resolved, ResolvedSubagentContextOverrides::default());
    }

    #[test]
    fn resolve_root_context_overrides_rejects_non_empty_override() {
        let error = resolve_root_context_overrides(Some(&SubagentContextOverrides {
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
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model_preference: None,
        })
        .expect_err("subagent-only profile should be rejected");

        assert!(err.to_string().contains("root execution"));
    }
}
