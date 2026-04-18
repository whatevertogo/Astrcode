//! 会话管理相关 DTO
//!
//! 定义会话创建、列表、提示词提交、消息历史等接口的请求/响应结构。
//! 会话是 Astrcode 的核心概念，代表一次独立的 AI 辅助编程交互。

pub use astrcode_core::DeleteProjectResult as DeleteProjectResultDto;
use serde::{Deserialize, Serialize};

use super::{ExecutionControlDto, PhaseDto};

/// `POST /api/sessions` 请求体——创建新会话。
///
/// `working_dir` 是会话的工作目录，用于确定项目上下文和配置文件路径。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    /// 会话的工作目录绝对路径
    pub working_dir: String,
}

/// `POST /api/sessions/:id/fork` 请求体——从稳定前缀分叉新会话。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForkSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
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
pub struct PromptSkillInvocation {
    /// 用户显式选择的 skill id（kebab-case）。
    pub skill_id: String,
    /// slash 命令头之后剩余的用户提示词。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
}

/// `POST /api/sessions/:id/prompt` 请求体——向会话提交用户提示词。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    /// 用户输入的文本内容
    pub text: String,
    /// 用户通过一级 slash 命令显式点名的 skill。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_invocation: Option<PromptSkillInvocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ExecutionControlDto>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_control: Option<ExecutionControlDto>,
}

/// `POST /api/sessions/:id/compact` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ExecutionControlDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `POST /api/sessions/:id/compact` 响应体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactSessionResponse {
    pub accepted: bool,
    pub deferred: bool,
    pub message: String,
}
