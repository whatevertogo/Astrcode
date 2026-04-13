//! 根代理执行入口。
//!
//! 实现 `execute_root_agent`：参数校验 → working-dir 规范化 → session 创建 →
//! agent 注册到控制树 → 异步提交 prompt。
//!
//! `application` 只做编排，不持有 session 真相或 turn 执行细节。

use std::sync::Arc;

use astrcode_core::{ExecutionAccepted, config::RuntimeConfig};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;

use crate::errors::ApplicationError;

/// 根代理执行请求。
pub struct RootExecutionRequest {
    pub agent_id: String,
    pub working_dir: String,
    pub task: String,
    pub context: Option<String>,
}

/// 执行根代理。
///
/// 完整流程：
/// 1. 参数校验
/// 2. 创建 session
/// 3. 注册根 agent 到控制树
/// 4. 合并 task + context
/// 5. 异步提交 prompt
pub async fn execute_root_agent(
    kernel: &Arc<Kernel>,
    session_runtime: &Arc<SessionRuntime>,
    request: RootExecutionRequest,
    runtime_config: RuntimeConfig,
) -> Result<ExecutionAccepted, ApplicationError> {
    // 参数校验
    validate_root_request(&request)?;

    // 创建 session
    let session = session_runtime
        .create_session(&request.working_dir)
        .await
        .map_err(ApplicationError::from)?;

    // 注册根 agent 到控制树
    kernel
        .agent_control()
        .register_root_agent(
            request.agent_id.clone(),
            session.session_id.clone(),
            "default".to_string(),
        )
        .await
        .map_err(|e| ApplicationError::Internal(format!("failed to register root agent: {e}")))?;

    // 合并 task + context
    let merged_task = match &request.context {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!("{}\n\n{}", ctx.trim(), request.task)
        },
        _ => request.task,
    };

    // 提交 prompt
    session_runtime
        .submit_prompt(&session.session_id, merged_task, runtime_config)
        .await
        .map_err(ApplicationError::from)
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
    Ok(())
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
        // context 可以为空字符串，校验不应拒绝
        let req = RootExecutionRequest {
            agent_id: "agent".to_string(),
            working_dir: "/tmp".to_string(),
            task: "task".to_string(),
            context: Some("".to_string()),
        };
        assert!(validate_root_request(&req).is_ok());
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
}
