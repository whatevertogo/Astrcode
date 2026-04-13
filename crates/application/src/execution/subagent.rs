//! 子代理执行入口。
//!
//! 实现 `launch_subagent`：spawn 参数解析 → control 协调 → turn 启动。
//! 子代理执行结果通过 parent delivery 机制回流到父级。

use std::sync::Arc;

use astrcode_core::{AgentProfile, ExecutionAccepted, config::RuntimeConfig};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use chrono::Utc;

use crate::errors::ApplicationError;

/// 子代理执行请求。
pub struct SubagentExecutionRequest {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub parent_turn_id: String,
    pub profile: AgentProfile,
    pub task: String,
    pub context: Option<String>,
}

/// 启动子代理执行。
///
/// 完整流程：
/// 1. 参数校验
/// 2. 创建独立 child session
/// 3. 在控制树中注册子 agent
/// 4. 异步提交 prompt
pub async fn launch_subagent(
    kernel: &Arc<Kernel>,
    session_runtime: &Arc<SessionRuntime>,
    request: SubagentExecutionRequest,
    runtime_config: RuntimeConfig,
) -> Result<ExecutionAccepted, ApplicationError> {
    validate_subagent_request(&request)?;

    // 创建独立 child session
    let child_session = session_runtime
        .create_session(format!("subagent-{}", Utc::now().timestamp_millis()))
        .await
        .map_err(ApplicationError::from)?;

    // 在控制树中注册子 agent
    let handle = kernel
        .agent_control()
        .spawn(
            &request.profile,
            child_session.session_id.clone(),
            request.parent_turn_id,
            Some(request.parent_agent_id),
        )
        .await
        .map_err(|e| ApplicationError::Internal(format!("failed to spawn subagent: {e}")))?;

    // 合并 task + context
    let merged_task = match &request.context {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!("{}\n\n{}", ctx.trim(), request.task)
        },
        _ => request.task,
    };

    // 异步提交 prompt
    let mut accepted = session_runtime
        .submit_prompt(&child_session.session_id, merged_task, runtime_config)
        .await
        .map_err(ApplicationError::from)?;
    accepted.agent_id = Some(handle.agent_id.into());
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
    if request.task.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'task' must not be empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentMode, AgentProfile};

    use super::*;

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
            profile: test_profile(),
            task: "explore the code".to_string(),
            context: None,
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
        // profile 存在性由业务逻辑在校验后判断，validate 只管必填字段
        let mut req = valid_request();
        req.profile.name = String::new();
        assert!(validate_subagent_request(&req).is_ok());
    }
}
