//! # 领域事件类型
//!
//! 定义了 Agent 运行时产生的所有领域事件，用于 SSE 推送和状态投影。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ToolExecutionResult, ToolOutputStream};

/// 会话阶段
///
/// 表示 Agent 当前所处的执行阶段，用于 UI 展示和状态管理。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// 空闲状态，等待用户输入
    Idle,
    /// 思考中（正在调用 LLM）
    Thinking,
    /// 正在调用工具
    CallingTool,
    /// 正在流式输出 LLM 响应
    Streaming,
    /// 被用户中断
    Interrupted,
    /// 已完成（Turn 结束）
    Done,
}

/// Agent 领域事件
///
/// 这些事件通过 SSE 推送到前端，用于实时更新 UI。
/// 与 `StorageEvent` 不同，`AgentEvent` 是面向展示的，不直接持久化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEvent {
    /// 会话开始
    SessionStarted { session_id: String },
    /// 用户消息
    UserMessage {
        /// 所属 Turn ID
        turn_id: String,
        /// 用户输入内容
        content: String,
    },
    /// 阶段变更（用于 UI 状态指示器）
    PhaseChanged {
        /// 所属 Turn ID（可能为空，如会话刚启动）
        turn_id: Option<String>,
        /// 新阶段
        phase: Phase,
    },
    /// LLM 输出增量（流式响应）
    ModelDelta { turn_id: String, delta: String },
    /// 思考内容增量（Claude thinking）
    ThinkingDelta { turn_id: String, delta: String },
    /// 助手消息（完整内容）
    AssistantMessage {
        turn_id: String,
        content: String,
        /// 推理内容（Claude extended thinking）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    /// 工具调用开始
    ToolCallStart {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        /// 工具参数
        #[serde(rename = "args")]
        input: Value,
    },
    /// 工具输出增量
    ToolCallDelta {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        stream: ToolOutputStream,
        delta: String,
    },
    /// 工具调用结果
    ToolCallResult {
        turn_id: String,
        result: ToolExecutionResult,
    },
    /// Turn 完成
    TurnDone { turn_id: String },
    /// 错误事件
    Error {
        /// 发生错误的 Turn ID（可能为空，如会话级别错误）
        turn_id: Option<String>,
        /// 错误码（如 "interrupted"、"agent_error"）
        code: String,
        /// 错误信息
        message: String,
    },
}
