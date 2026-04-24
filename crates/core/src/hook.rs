//! # Lifecycle Hook 契约
//!
//! 将"可拦截的生命周期点"和"纯事件广播"分开，避免再引入第二条事实来源。
//! Hook 只负责少数明确的执行节点，且输入输出必须是强类型的。
//!
//! ## PreCompact Hook 扩展能力
//!
//! PreCompact hook 支持三种控制方式：
//! - `Continue`: 允许压缩继续，不做任何修改
//! - `Block`: 阻止本次压缩
//! - `ModifyCompactContext`: 修改压缩参数，包括：
//!   - `additional_system_prompt`: 在默认 compact prompt 后追加指令
//!   - `override_keep_recent_turns`: 覆盖保留的最近 turn 数量
//!   - `custom_summary`: 直接提供摘要内容，跳过 LLM 调用
//!
//! 这个设计允许插件：
//! - 自定义压缩 prompt（注入特定指令）
//! - 调整保留策略（根据上下文动态决定保留多少）
//! - 提供自定义摘要（完全接管压缩逻辑）

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    CompactTrigger, LlmMessage,
    action::{ToolDefinition, ToolExecutionResult},
};

/// 可被外部扩展拦截的生命周期事件。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PreCompact,
    PostCompact,
}

/// 新 hooks catalog 的稳定事件键。
///
/// 这些键是跨 owner 共享的最小语义；具体 payload、effect 解释与调度报告
/// 归属 `agent-runtime`、`host-session` 或 `plugin-host`。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKey {
    Input,
    Context,
    BeforeAgentStart,
    BeforeProviderRequest,
    ToolCall,
    ToolResult,
    TurnStart,
    TurnEnd,
    SessionBeforeCompact,
    ResourcesDiscover,
    ModelSelect,
}

/// 工具调用的公共上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolHookContext {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: PathBuf,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
}

/// 工具调用完成后的上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolHookResultContext {
    pub tool: ToolHookContext,
    pub result: ToolExecutionResult,
}

/// 压缩前公共上下文。
///
/// 包含压缩决策所需的所有信息，允许 hook 根据上下文内容做出决策。
/// `messages` 和 `tools` 字段仅在 hook 需要检查上下文时才填充，
/// 避免不必要的序列化开销。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactionHookContext {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub reason: CompactTrigger,
    pub keep_recent_turns: usize,
    pub message_count: usize,
    /// 当前对话中的消息（序列化形式）。
    /// Hook 可以检查这些消息来决定是否需要修改压缩策略。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<LlmMessage>,
    /// 当前可用的工具定义。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    /// 当前正常对话请求使用的 runtime system prompt。
    ///
    /// 注意：这里暴露的是“运行时请求上下文”，不是最终发送给 compact LLM 的完整
    /// system prompt 模板。compact 流程会把这段内容嵌入专用摘要模板中，hook 应把
    /// 它理解为当前会话约束的参考材料，而不是可直接覆盖的最终提示词。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// 压缩完成后的上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactionHookResultContext {
    pub compaction: CompactionHookContext,
    pub summary: String,
    pub strategy_id: String,
    pub preserved_recent_turns: usize,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub messages_removed: usize,
    pub tokens_freed: usize,
}
