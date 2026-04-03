//! Agent 事件流 DTO
//!
//! 定义 Agent 运行期间产生的各类事件的协议格式，用于 SSE 实时推送和会话回放。
//! 事件采用 `tagged enum` 序列化（`#[serde(tag = "event", content = "data")]`），
//! 前端通过 `event` 字段区分事件类型，`data` 字段获取具体数据。
//!
//! ## 事件生命周期
//!
//! 一个完整的 turn 通常产生以下事件序列：
//! `SessionStarted` → `UserMessage` → `PhaseChanged(Thinking)` → `ModelDelta`* →
//! `ToolCallStart` → `ToolCallDelta`* → `ToolCallResult` → `PhaseChanged(Done)` → `TurnDone`

use serde::{Deserialize, Serialize};

/// 协议版本号，用于事件格式的版本控制。
///
/// 每个 `AgentEventEnvelope` 都携带此版本号，前端可根据版本号决定如何解析事件。
pub const PROTOCOL_VERSION: u32 = 1;

/// Agent 当前执行阶段。
///
/// 前端根据阶段切换 UI 状态（如加载动画、终端视图等）。
/// 阶段转换通过 `PhaseChanged` 事件通知。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PhaseDto {
    /// 空闲状态，无活跃 turn
    Idle,
    /// 模型正在思考（生成 reasoning content）
    Thinking,
    /// 正在执行工具调用
    CallingTool,
    /// 正在流式输出文本内容
    Streaming,
    /// 用户中断了当前 turn
    Interrupted,
    /// 当前 turn 已完成
    Done,
}

/// 工具输出流类型，区分 stdout 和 stderr。
///
/// 用于 `ToolCallDelta` 事件中指示增量输出来自哪个流。
/// 前端根据此字段将输出渲染到终端视图的不同区域。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolOutputStreamDto {
    /// 标准输出
    Stdout,
    /// 标准错误
    Stderr,
}

/// 工具调用的最终结果。
///
/// 包含工具执行的完整输出、耗时、是否被截断等信息。
/// `metadata` 字段携带展示相关的额外信息（如 diff 数据、终端展示提示等）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultDto {
    /// 工具调用的唯一标识，与 `ToolCallStart` 中的 `tool_call_id` 对应
    pub tool_call_id: String,
    /// 工具名称
    pub tool_name: String,
    /// 工具调用是否成功
    pub ok: bool,
    /// 工具的输出内容（成功时为正常输出，失败时为错误摘要）
    pub output: String,
    /// 失败时的详细错误信息
    pub error: Option<String>,
    /// 展示相关的元数据（如 diff 信息、终端展示提示等）
    pub metadata: Option<serde_json::Value>,
    /// 工具调用耗时（毫秒）
    ///
    /// 使用 `u64` 而非 `u128`，因为 `u64` 已可表示约 5.8 亿年的毫秒数，
    /// 足够覆盖任何合理的工具执行时间。
    pub duration_ms: u64,
    /// 输出是否被截断（超出最大长度限制）
    pub truncated: bool,
}

/// Agent 事件载荷的 tagged enum。
///
/// 采用 `#[serde(tag = "event", content = "data")]` 序列化策略，
/// 每个变体对应一种事件类型。前端通过 `event` 字段路由到不同的处理器。
///
/// ## 事件分类
///
/// - **会话级**: `SessionStarted`
/// - **用户交互**: `UserMessage`
/// - **阶段变更**: `PhaseChanged`
/// - **模型输出**: `ModelDelta`, `ThinkingDelta`, `AssistantMessage`
/// - **工具调用**: `ToolCallStart`, `ToolCallDelta`, `ToolCallResult`
/// - **生命周期**: `TurnDone`
/// - **错误**: `Error`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum AgentEventPayload {
    /// 会话开始事件，携带新会话的 ID。
    SessionStarted { session_id: String },
    /// 用户发送消息事件，携带 turn ID 和用户输入内容。
    UserMessage { turn_id: String, content: String },
    /// Agent 执行阶段变更事件。
    ///
    /// `turn_id` 在会话初始阶段可能为 None（如全局阶段切换）。
    PhaseChanged {
        turn_id: Option<String>,
        phase: PhaseDto,
    },
    /// 模型正常输出的增量文本片段。
    ///
    /// 前端需将多个 `ModelDelta` 的 `delta` 拼接成完整回复。
    ModelDelta { turn_id: String, delta: String },
    /// 模型推理过程（thinking/reasoning）的增量输出。
    ///
    /// 此内容通常不直接展示给用户，但可用于调试或特殊 UI。
    ThinkingDelta { turn_id: String, delta: String },
    /// 助手完整消息事件，在模型输出完成后发送。
    ///
    /// 包含完整的回复内容和可选的 reasoning content。
    AssistantMessage {
        turn_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    /// 工具调用开始事件。
    ///
    /// 标记一个工具调用的开始，携带工具名称和完整输入参数。
    /// 输入参数序列化为 `args` 以匹配前端事件格式约定。
    ToolCallStart {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(rename = "args")]
        input: serde_json::Value,
    },
    /// 工具调用的增量输出事件。
    ///
    /// 用于长耗时工具（如 shell 命令）的流式输出。
    /// `stream` 字段区分 stdout/stderr，`delta` 为本次增量内容。
    ToolCallDelta {
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        stream: ToolOutputStreamDto,
        delta: String,
    },
    /// 工具调用完成事件，携带完整的执行结果。
    ToolCallResult {
        turn_id: String,
        result: ToolCallResultDto,
    },
    /// 当前 turn 完成事件。
    TurnDone { turn_id: String },
    /// 错误事件。
    ///
    /// `turn_id` 为 None 时表示会话级错误（如连接断开）。
    Error {
        turn_id: Option<String>,
        code: String,
        message: String,
    },
}

/// Agent 事件信封，为事件载荷添加协议版本等元数据。
///
/// 信封结构确保前端可以验证协议版本兼容性。
/// `#[serde(flatten)]` 使内部 `AgentEventPayload` 的 tagged 字段直接暴露在 JSON 顶层，
/// 即序列化后 `protocol_version`、`event`、`data` 处于同一层级。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventEnvelope {
    /// 协议版本号，用于向前/向后兼容判断
    pub protocol_version: u32,
    /// 事件载荷，序列化后其 tag/content 字段会扁平化到信封层级
    #[serde(flatten)]
    pub event: AgentEventPayload,
}

impl AgentEventEnvelope {
    /// 创建新的事件信封，自动设置协议版本。
    pub fn new(event: AgentEventPayload) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            event,
        }
    }
}
