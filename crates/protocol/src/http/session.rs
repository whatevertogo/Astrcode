//! 会话管理相关 DTO
//!
//! 定义会话创建、列表、提示词提交、消息历史等接口的请求/响应结构。
//! 会话是 Astrcode 的核心概念，代表一次独立的 AI 辅助编程交互。

use serde::{Deserialize, Serialize};

use super::{AgentEventEnvelope, CompactTriggerDto, PhaseDto};

/// `POST /api/sessions` 请求体——创建新会话。
///
/// `working_dir` 是会话的工作目录，用于确定项目上下文和配置文件路径。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    /// 会话的工作目录绝对路径
    pub working_dir: String,
}

/// 会话列表中的单个会话摘要。
///
/// 用于 `GET /api/sessions` 响应，返回所有会话的概览信息。
/// `parent_session_id` 和 `parent_storage_seq` 在会话是从其他会话分支出来时存在。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionListItem {
    /// 会话唯一标识
    pub session_id: String,
    /// 工作目录
    pub working_dir: String,
    /// 用于 UI 展示的会话名称（通常基于工作目录生成）
    pub display_name: String,
    /// 用户自定义的会话标题
    pub title: String,
    /// 创建时间戳（ISO 8601）
    pub created_at: String,
    /// 最后更新时间戳（ISO 8601）
    pub updated_at: String,
    /// 如果此会话是从其他会话分支出来的，指向源会话 ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// 分支点在源会话中的 storage_seq，用于事件回放定位
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_storage_seq: Option<u64>,
    /// 当前执行阶段
    pub phase: PhaseDto,
}

/// `POST /api/sessions/:id/prompt` 请求体——向会话提交用户提示词。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    /// 用户输入的文本内容
    pub text: String,
}

/// `POST /api/sessions/:id/prompt` 响应体——提示词已被接受。
///
/// 返回新创建的 turn ID 和会话 ID。
/// 如果是从其他会话分支出来的新会话，`branched_from_session_id` 会指向源会话。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptAcceptedResponse {
    /// 新 turn 的唯一标识
    pub turn_id: String,
    /// 会话 ID（分支场景下可能是新会话的 ID）
    pub session_id: String,
    /// 如果是分支会话，指向源会话 ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branched_from_session_id: Option<String>,
}

/// 会话历史中的单条消息。
///
/// 采用 `#[serde(tag = "kind")]` 序列化策略，通过 `kind` 字段区分消息类型。
/// 用于 `GET /api/sessions/:id/messages` 响应中返回会话历史。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionMessageDto {
    /// 用户消息。
    User {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        timestamp: String,
    },
    /// 助手回复消息。
    ///
    /// `reasoning_content` 在模型支持 thinking 时存在。
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        content: String,
        timestamp: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    /// 工具调用消息。
    ///
    /// 包含工具名称、参数、输出、执行状态等完整信息。
    ToolCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    /// 上下文压缩消息。
    Compact {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        trigger: CompactTriggerDto,
        summary: String,
        preserved_recent_turns: u32,
        timestamp: String,
    },
}

/// `GET /api/sessions/:id/history` 响应体。
///
/// 初始 hydration 返回历史 `AgentEvent` 序列和当前 phase/cursor，
/// 让前端用和 SSE 增量相同的事件协议重建消息状态，避免再维护
/// 一套专用 `SessionMessage` 快照协议。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryResponseDto {
    pub events: Vec<AgentEventEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub phase: PhaseDto,
}

/// `DELETE /api/projects/:working_dir` 响应体——项目删除结果。
///
/// 由于项目下可能有多个会话，删除是批量操作。
/// `failed_session_ids` 列出删除失败的会话 ID（如文件被锁定）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectResultDto {
    /// 成功删除的会话数量
    pub success_count: usize,
    /// 删除失败的会话 ID 列表
    pub failed_session_ids: Vec<String>,
}
