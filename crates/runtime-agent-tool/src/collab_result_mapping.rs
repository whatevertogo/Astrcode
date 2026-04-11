//! 协作工具（send/wait/close/resume/deliver）共享的结果映射逻辑。
//!
//! 与 `result_mapping` 拆开是因为协作工具的返回类型是 `CollaborationResult`，
//! 其结构与 spawnAgent 的 `SubRunResult` 完全不同：
//! - CollaborationResult 侧重 accepted/failure/summary 三元组
//! - SubRunResult 侧重 status/handoff/artifacts 组合
//!
//! 映射策略：
//! - `accepted` → ok（表示操作被 runtime 接受）
//! - `failure` → error（描述拒绝或运行时错误的原因）
//! - `summary` → output（LLM 可见的文本摘要）
//! - 整个 CollaborationResult 序列化为 metadata（供前端消费）

use astrcode_core::{CollaborationResult, ToolExecutionResult};

/// 协作工具的错误结果（参数校验失败等）。
///
/// duration_ms = 0 因为错误在到达 executor 之前就被拦截了。
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
///
/// metadata 中序列化了完整的 CollaborationResult，前端据此渲染子 agent 状态。
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
