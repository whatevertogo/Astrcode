//! Agent 执行子域。
//!
//! 承接根代理执行 (`execute_root_agent`) 和子代理执行 (`launch_subagent`)。
//! `App` 通过薄 façade 委托到此子域，避免把执行逻辑堆进根文件。

mod control;
mod profiles;
mod root;
mod subagent;

use astrcode_core::{AgentMode, AgentProfile};
pub use control::ExecutionControl;
pub use profiles::{ProfileProvider, ProfileResolutionService};
pub use root::{RootExecutionRequest, execute_root_agent};
pub use subagent::{SubagentExecutionRequest, launch_subagent};

use crate::ApplicationError;

/// 将 context 信息拼接到 task 前面，形成完整的执行指令。
///
/// context 非空时格式为 `{context}\n\n{task}`，context 为空时直接返回 task。
pub(super) fn merge_task_with_context(task: &str, context: Option<&str>) -> String {
    match context {
        Some(context) if !context.trim().is_empty() => {
            format!("{}\n\n{}", context.trim(), task)
        },
        _ => task.to_string(),
    }
}

/// 校验 profile 的 mode 是否在允许列表内。
///
/// 根执行只允许 Primary / All，子代理执行只允许 SubAgent / All。
/// 不匹配时返回带上下文的错误信息。
pub(super) fn ensure_profile_mode(
    profile: &AgentProfile,
    allowed_modes: &[AgentMode],
    execution_name: &str,
) -> Result<(), ApplicationError> {
    if allowed_modes.iter().any(|mode| mode == &profile.mode) {
        return Ok(());
    }

    Err(ApplicationError::InvalidArgument(format!(
        "agent profile '{}' cannot be used for {execution_name}",
        profile.id
    )))
}
