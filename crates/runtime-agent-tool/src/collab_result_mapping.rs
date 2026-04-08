//! 协作工具共享的结果映射函数。

use astrcode_core::{CollaborationResult, ToolExecutionResult};

/// 协作工具的错误结果（参数校验失败等）。
pub(crate) fn collaboration_error_result(
    tool_call_id: String,
    tool_name: &str,
    message: String,
) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id,
        tool_name: tool_name.to_string(),
        ok: false,
        output: String::new(),
        error: Some(message),
        metadata: None,
        duration_ms: 0,
        truncated: false,
    }
}

/// 将 CollaborationResult 映射为 ToolExecutionResult。
pub(crate) fn map_collaboration_result(
    tool_call_id: String,
    tool_name: &str,
    result: CollaborationResult,
) -> ToolExecutionResult {
    let error = result.failure.clone();
    let output = result.summary.clone().unwrap_or_default();
    let metadata = serde_json::to_value(&result).ok();

    ToolExecutionResult {
        tool_call_id,
        tool_name: tool_name.to_string(),
        ok: result.accepted,
        output,
        error,
        metadata,
        duration_ms: 0,
        truncated: false,
    }
}
