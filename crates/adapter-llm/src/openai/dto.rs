//! # OpenAI 共享 DTO 与 SSE 处理基础设施
//!
//! 本模块提取 Chat Completions 和 Responses 两条路径共享的：
//! - 请求/响应 DTO（`OpenAiRequestMessage`、`OpenAiUsage`、`OpenAiToolDef` 等）
//! - 消息/工具转换函数（`to_openai_message`、`to_openai_tool_def`）
//! - `SseProcessor` trait（统一 SSE 流式处理骨架）
//!
//! ## 设计原则
//!
//! - Chat Completions 专有类型（`OpenAiChatRequest`、`OpenAiStreamChunk` 等）留在 `super`
//! - Responses 专有类型继续使用 `serde_json::Value`（在 `responses.rs`）
//! - 本模块只存放"两个路径都会用到"的类型和函数

use astrcode_core::{LlmMessage, LlmUsage, ToolDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{EventSink, LlmAccumulator, LlmOutput, Result};

// ===========================================================================
// 共享 DTO
// ===========================================================================

/// OpenAI 请求消息（user / assistant / system / tool）。
///
/// 用于 Chat Completions 请求体中的 `messages` 数组，
/// Responses 路径通过 `build_input_items` 使用 `Value`。
#[derive(Debug, Serialize)]
pub(super) struct OpenAiRequestMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiRequestFunctionCall>>,
}

/// 请求体中的函数调用（assistant 消息的 `tool_calls` 字段）。
///
/// 注意：这是请求侧结构（序列化），与响应侧的 `OpenAiResponseFunctionCall` 不同。
#[derive(Debug, Serialize)]
pub(super) struct OpenAiRequestFunctionCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAiRequestFunction,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiRequestFunction {
    pub name: String,
    pub arguments: String,
}

/// 工具定义（用于请求体中的 `tools` 字段）。
///
/// OpenAI 工具定义需要 `type: "function"` 包装层。
#[derive(Debug, Serialize)]
pub(super) struct OpenAiToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAiToolFunctionDef,
}

#[derive(Debug, Serialize)]
pub(super) struct OpenAiToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// OpenAI 响应中的 token 用量统计。
///
/// 两个字段均为 `Option` 且带 `#[serde(default)]`，
/// 因为某些兼容 API 可能不返回用量信息。
#[derive(Debug, Deserialize, Clone)]
pub(super) struct OpenAiUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct OpenAiPromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
}

impl OpenAiUsage {
    pub fn cached_tokens(&self) -> u64 {
        self.prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .unwrap_or_default()
    }
}

// ===========================================================================
// 共享转换函数
// ===========================================================================

/// 将 `LlmMessage` 转换为 OpenAI 请求消息格式。
pub(super) fn to_openai_message(message: &LlmMessage) -> OpenAiRequestMessage {
    match message {
        LlmMessage::User { content, .. } => OpenAiRequestMessage {
            role: "user".to_string(),
            content: Some(content.clone()),
            tool_call_id: None,
            tool_calls: None,
        },
        LlmMessage::Assistant {
            content,
            tool_calls,
            reasoning: _,
        } => OpenAiRequestMessage {
            role: "assistant".to_string(),
            content: if content.is_empty() {
                None
            } else {
                Some(content.clone())
            },
            tool_call_id: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .iter()
                        .map(|call| OpenAiRequestFunctionCall {
                            id: call.id.clone(),
                            tool_type: "function".to_string(),
                            function: OpenAiRequestFunction {
                                name: call.name.clone(),
                                arguments: call.args.to_string(),
                            },
                        })
                        .collect(),
                )
            },
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => OpenAiRequestMessage {
            role: "tool".to_string(),
            content: Some(content.clone()),
            tool_call_id: Some(tool_call_id.clone()),
            tool_calls: None,
        },
    }
}

/// 将 `ToolDefinition` 转换为 OpenAI 工具定义格式。
pub(super) fn to_openai_tool_def(def: &ToolDefinition) -> OpenAiToolDef {
    OpenAiToolDef {
        tool_type: "function".to_string(),
        function: OpenAiToolFunctionDef {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        },
    }
}

/// 将 OpenAI 用量统计转换为内部 `LlmUsage`。
pub(super) fn openai_usage_to_llm_usage(usage: OpenAiUsage) -> LlmUsage {
    LlmUsage {
        input_tokens: usage.prompt_tokens.unwrap_or_default() as usize,
        output_tokens: usage.completion_tokens.unwrap_or_default() as usize,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: usage.cached_tokens() as usize,
    }
}

// ===========================================================================
// SSE 处理器 trait
// ===========================================================================

/// SSE 协议处理器：不同 API 模式实现此 trait 来处理各自的 SSE 行/块协议。
///
/// 每个处理器拥有自己的 `sse_buffer`，负责管理行/块缓冲和协议解析。
pub(super) trait SseProcessor {
    /// 处理一块 SSE 文本。
    ///
    /// 返回 `(is_done, finish_reason, usage)`：
    /// - `is_done`: 遇到流结束标记
    /// - `finish_reason`: 本次 chunk 中提取到的 finish_reason（非流结束标记时通常为 None）
    /// - `usage`: 本次 chunk 中提取到的 token 用量
    fn process_chunk(
        &mut self,
        chunk_text: &str,
        accumulator: &mut LlmAccumulator,
        sink: &EventSink,
    ) -> Result<(bool, Option<String>, Option<LlmUsage>)>;

    /// 流结束后刷新缓冲区中剩余的不完整内容。
    ///
    /// 返回 `(finish_reason, usage)`。
    fn flush(
        &mut self,
        accumulator: &mut LlmAccumulator,
        sink: &EventSink,
    ) -> Result<(Option<String>, Option<LlmUsage>)>;

    /// 流结束后，如果处理器有完整的已完成输出（如 Responses API 的 `response.completed`），
    /// 返回它。默认实现返回 `None`。
    fn take_completed_output(&mut self) -> Option<LlmOutput> {
        None
    }
}
