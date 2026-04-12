//! Turn 执行核心类型。
//!
//! 从 `runtime-agent-loop` 迁入 turn 执行相关的类型定义。
//! 实际的 TurnRunner 执行引擎需要 adapter-llm、adapter-prompt 等依赖，
//! 留在旧 crate 中直到 Phase 10 组合根阶段统一接线。

use astrcode_core::{SessionId, TurnId};

/// Turn 运行请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRunRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
}

/// Turn 结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// LLM 返回纯文本（无 tool_calls），自然结束。
    Completed,
    /// 用户取消或 CancelToken 触发。
    Cancelled,
    /// 不可恢复错误。
    Error { message: String },
}
