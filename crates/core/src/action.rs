//! # LLM 消息与工具调用数据结构
//!
//! 定义了与 LLM 交互所需的消息格式、工具定义、调用请求和结果。
//!
//! ## 核心类型
//!
//! - [`LlmMessage`][]: 与 LLM 交互的消息枚举（User / Assistant / Tool）
//! - [`ToolCallRequest`][]: 工具调用请求（ID、名称、参数）
//! - [`ToolExecutionResult`][]: 工具执行结果（输出、错误、元数据）
//! - [`ToolOutputDelta`][]: 工具流式输出增量（stdout/stderr）

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ExecutionContinuation, ExecutionResultCommon};

/// LLM 推理/思考内容。
///
/// 用于支持扩展思考模型（如 Claude extended thinking），
/// `signature` 用于验证思考内容的完整性。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContent {
    /// 思考内容正文
    pub content: String,
    /// 完整性签名（可选，用于验证思考内容未被篡改）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// 工具定义，用于向 LLM 描述可用的工具。
///
/// 该结构会被序列化为 LLM API 的 `tools` 参数格式。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    /// 工具名称（在会话中唯一标识该工具）
    pub name: String,
    /// 工具描述（LLM 据此决定何时调用）
    pub description: String,
    /// JSON Schema 格式的参数定义
    pub parameters: Value,
}

/// 工具调用请求。
///
/// 由 LLM 响应中的 `tool_calls` 字段解析而来，
/// 包含调用哪个工具以及传入的参数。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRequest {
    /// 调用 ID（由 LLM 生成，用于将结果与调用关联）
    pub id: String,
    /// 工具名称
    pub name: String,
    /// 调用参数（已解析为 JSON Value）
    pub args: Value,
}

/// 工具执行结果。
///
/// 包含工具调用的完整执行信息，用于反馈给 LLM 或展示给前端。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionResult {
    /// 对应的工具调用 ID
    pub tool_call_id: String,
    /// 工具名称
    pub tool_name: String,
    /// 执行是否成功
    pub ok: bool,
    /// 工具输出内容
    pub output: String,
    /// 错误信息（仅在失败时设置）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 额外元数据（如 diff 信息、终端显示提示等）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// 工具结果产生的 typed 续接目标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ExecutionContinuation>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否因大小限制被截断
    #[serde(default)]
    pub truncated: bool,
}

/// 工具流式输出的通道类型。
///
/// 用于区分标准输出和标准错误流，前端据此渲染不同的终端视图。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolOutputStream {
    /// 标准输出
    Stdout,
    /// 标准错误
    Stderr,
}

/// 工具流式输出增量。
///
/// 长耗时工具（如 shell 命令）在执行过程中持续产生的输出片段，
/// 通过此结构持久化并广播到前端，实现实时终端视图更新。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputDelta {
    /// 对应的工具调用 ID
    pub tool_call_id: String,
    /// 工具名称
    pub tool_name: String,
    /// 输出通道（stdout 或 stderr）
    pub stream: ToolOutputStream,
    /// 本次增量文本
    pub delta: String,
}

impl ToolExecutionResult {
    /// 用公共执行结果字段一次性构造工具结果，避免先写占位值再二次覆盖。
    pub fn from_common(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        ok: bool,
        output: impl Into<String>,
        continuation: Option<ExecutionContinuation>,
        common: ExecutionResultCommon,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            ok,
            output: output.into(),
            error: common.error,
            metadata: common.metadata,
            continuation,
            duration_ms: common.duration_ms,
            truncated: common.truncated,
        }
    }

    /// 生成面向模型的工具结果内容。
    ///
    /// 成功时直接返回输出；失败时拼接错误信息和输出，
    /// 确保 LLM 能理解工具执行的结果。
    /// 如果产生了 child agent continuation，追加精确引用提示，
    /// 防止 LLM 自作主张改写 agentId。
    pub fn model_content(&self) -> String {
        let base = if self.ok {
            self.output.clone()
        } else {
            match self.error.as_deref() {
                Some(error) if self.output.is_empty() => format!("tool execution failed: {error}"),
                Some(error) => format!("tool execution failed: {error}\n{}", self.output),
                None => self.output.clone(),
            }
        };

        let Some(child_ref_hint) = self.child_agent_reference_hint() else {
            return base;
        };

        if base.trim().is_empty() {
            child_ref_hint
        } else {
            format!("{base}\n\n{child_ref_hint}")
        }
    }

    pub fn continuation(&self) -> Option<&ExecutionContinuation> {
        self.continuation.as_ref()
    }

    fn child_agent_reference_hint(&self) -> Option<String> {
        let child_ref = self.continuation()?.child_agent_ref()?;

        let mut lines = vec![
            "Child agent reference:".to_string(),
            format!("- agentId: {}", child_ref.agent_id()),
        ];

        lines.push(format!("- subRunId: {}", child_ref.sub_run_id()));
        lines.push(format!("- sessionId: {}", child_ref.session_id()));
        lines.push(format!("- openSessionId: {}", child_ref.open_session_id));
        lines.push(format!("- status: {:?}", child_ref.status).to_lowercase());

        // 这里显式强调“精确复用原值”，避免模型把 `agent-1` 自作主张改写成
        // `agent-01` 之类的展示型编号，导致后续协作工具命中不存在的 agent。
        lines.push("Use this exact `agentId` value in later send/observe/close calls.".to_string());
        Some(lines.join("\n"))
    }

    pub fn common(&self) -> ExecutionResultCommon {
        ExecutionResultCommon {
            error: self.error.clone(),
            metadata: self.metadata.clone(),
            duration_ms: self.duration_ms,
            truncated: self.truncated,
        }
    }
}

/// 用户消息的来源。
///
/// 用于区分用户直接输入、内部唤醒提示和压缩摘要，
/// 影响事件翻译和前端展示逻辑。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserMessageOrigin {
    /// 用户直接输入
    #[default]
    User,
    /// 从 durable 输入队列恢复并注入的内部输入。
    QueuedInput,
    /// assistant 输出被截断后，为同一 turn 续写而注入的内部提示。
    ContinuationPrompt,
    /// 子会话交付后用于唤醒父会话继续决策的内部提示。
    ReactivationPrompt,
    /// compact 后为最近真实用户消息生成的极短目的摘要。
    RecentUserContextDigest,
    /// compact 后重新注入的最近真实用户消息原文。
    RecentUserContext,
    /// 压缩摘要（上下文压缩后插入的摘要消息）
    CompactSummary,
}

/// 与 LLM 交互的消息。
///
/// 对应 OpenAI 兼容 API 的三种角色消息：
/// - `User`: 用户输入或内部上下文消息（含来源标记）
/// - `Assistant`: 助手回复（含文本、工具调用、推理内容）
/// - `Tool`: 工具执行结果
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LlmMessage {
    /// 用户消息
    User {
        /// 消息内容
        content: String,
        /// 消息来源（用户输入/内部唤醒/压缩摘要）
        origin: UserMessageOrigin,
    },
    /// 助手消息
    Assistant {
        /// 可见文本内容
        content: String,
        /// 工具调用列表（由 LLM 决定调用哪些工具）
        tool_calls: Vec<ToolCallRequest>,
        /// 推理/思考内容（可选，如 Claude extended thinking）
        reasoning: Option<ReasoningContent>,
    },
    /// 工具结果消息
    Tool {
        /// 对应的工具调用 ID
        tool_call_id: String,
        /// 工具执行结果内容（供 LLM 参考）
        content: String,
    },
}

/// 助手消息的内容拆分结果。
///
/// 将 LLM 原始输出分离为「用户可见文本」和「推理内容」，
/// 用于前端分别渲染正文和思考过程。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantContentParts {
    /// 用户可见的文本内容（已移除内联推理标签）
    pub visible_content: String,
    /// 推理/思考内容（合并显式和内联来源，可能为空）
    pub reasoning_content: Option<String>,
}

/// 将 LLM 原始输出文本拆分为「可见内容」和「推理内容」两部分。
///
/// ## 为什么需要这个函数
///
/// 某些 LLM（如 Anthropic Claude）使用 `<think>...</think>` 标签包裹推理过程。
/// 但 LLM 可能在不同位置以不同方式输出这些标签：
/// - 作为独立的 reasoning_content 字段（由 LLM API 返回）
/// - 内联在文本内容中（某些模型/提供商的输出风格）
///
/// 此函数统一处理这两种情况，提取出推理内容并清理可见文本。
///
/// ## 算法要点
///
/// 1. 用游标扫描全文，查找大小写不敏感的 `<think＞...＜/think＞` 标签对
/// 2. 空的 think 块（`<think＞＜/think＞`）保留原样不动——避免破坏无推理内容时的输出
/// 3. 非空 think 块的内容被提取到 `inline_blocks`，标签从可见文本中移除
/// 4. 移除标签后，连续超过两个空行的位置会被折叠为两个空行（`collapse_extra_blank_lines`），
///    因为标签移除可能留下大片空白
/// 5. 如果同时存在显式 reasoning（API 返回的）和内联 reasoning（从标签提取的），
///    合并两者；内容相同时去重
pub fn split_assistant_content(
    content: &str,
    explicit_reasoning: Option<&str>,
) -> AssistantContentParts {
    let mut visible_content = String::new();
    let mut inline_blocks = Vec::new();
    let lower = content.to_ascii_lowercase();
    let mut cursor = 0usize;
    let mut removed_tags = false;

    while let Some(start_rel) = lower[cursor..].find("<think>") {
        let start = cursor + start_rel;
        let block_start = start + "<think>".len();
        let Some(end_rel) = lower[block_start..].find("</think>") else {
            break;
        };
        let end = block_start + end_rel;
        let raw_inner = &content[block_start..end];
        let normalized = raw_inner.trim();

        if normalized.is_empty() {
            visible_content.push_str(&content[cursor..end + "</think>".len()]);
            cursor = end + "</think>".len();
            continue;
        }

        visible_content.push_str(&content[cursor..start]);
        inline_blocks.push(normalized.to_string());
        cursor = end + "</think>".len();
        removed_tags = true;
    }

    visible_content.push_str(&content[cursor..]);

    let explicit_reasoning = explicit_reasoning
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let inline_reasoning = (!inline_blocks.is_empty()).then(|| inline_blocks.join("\n\n"));
    let reasoning_content = match (explicit_reasoning, inline_reasoning) {
        (Some(explicit), Some(inline)) if explicit == inline => Some(explicit),
        (Some(explicit), Some(inline)) => Some(format!("{explicit}\n\n{inline}")),
        (Some(explicit), None) => Some(explicit),
        (None, Some(inline)) => Some(inline),
        (None, None) => None,
    };

    let visible_content = if removed_tags {
        collapse_extra_blank_lines(visible_content.trim())
            .trim()
            .to_string()
    } else {
        content.to_string()
    };

    AssistantContentParts {
        visible_content,
        reasoning_content,
    }
}

fn collapse_extra_blank_lines(input: &str) -> String {
    let mut collapsed = String::with_capacity(input.len());
    let mut newline_run = 0usize;

    for ch in input.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                collapsed.push(ch);
            }
            continue;
        }

        newline_run = 0;
        collapsed.push(ch);
    }

    collapsed
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ToolExecutionResult, split_assistant_content};
    use crate::{AgentId, ExecutionResultCommon, SessionId, SubRunId};

    #[test]
    fn split_assistant_content_extracts_inline_thinking_blocks() {
        let parts = split_assistant_content(
            "Answer before\n<think> first step </think>\n<think>second step</think>\nAnswer after",
            None,
        );

        assert_eq!(parts.visible_content, "Answer before\n\nAnswer after");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("first step\n\nsecond step")
        );
    }

    #[test]
    fn split_assistant_content_prefers_explicit_reasoning_and_strips_inline_think_tags() {
        let parts = split_assistant_content(
            "<think>hidden</think>\nvisible",
            Some("persisted reasoning"),
        );

        assert_eq!(parts.visible_content, "visible");
        assert_eq!(
            parts.reasoning_content.as_deref(),
            Some("persisted reasoning\n\nhidden")
        );
    }

    #[test]
    fn split_assistant_content_keeps_empty_think_blocks_literal() {
        let parts = split_assistant_content("<think>   </think>\n\nvisible", None);

        assert_eq!(parts.visible_content, "<think>   </think>\n\nvisible");
        assert_eq!(parts.reasoning_content, None);
    }

    #[test]
    fn model_content_appends_exact_child_agent_reference_from_continuation() {
        let result = ToolExecutionResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "spawn".to_string(),
            ok: true,
            output: "spawn 已在后台启动。".to_string(),
            error: None,
            metadata: Some(json!({ "schema": "subRunResult" })),
            continuation: Some(crate::ExecutionContinuation::child_agent(
                crate::ChildAgentRef {
                    identity: crate::ChildExecutionIdentity {
                        agent_id: AgentId::from("agent-1"),
                        session_id: SessionId::from("session-parent"),
                        sub_run_id: SubRunId::from("subrun-1"),
                    },
                    parent: crate::ParentExecutionRef::default(),
                    lineage_kind: crate::ChildSessionLineageKind::Spawn,
                    status: crate::AgentLifecycleStatus::Running,
                    open_session_id: SessionId::from("session-parent"),
                },
            )),
            duration_ms: 0,
            truncated: false,
        };

        let content = result.model_content();
        assert!(content.contains("spawn 已在后台启动。"));
        assert!(content.contains("- agentId: agent-1"));
        assert!(content.contains("- subRunId: subrun-1"));
        assert!(content.contains("Use this exact `agentId` value"));
    }

    #[test]
    fn from_common_preserves_failure_fields_without_placeholder_override() {
        let result = ToolExecutionResult::from_common(
            "call-1",
            "spawn",
            false,
            "",
            None,
            ExecutionResultCommon::failure(
                "spawn failed",
                Some(json!({ "schema": "subRunResult" })),
                17,
                true,
            ),
        );

        assert!(!result.ok);
        assert_eq!(result.error.as_deref(), Some("spawn failed"));
        assert_eq!(result.metadata, Some(json!({ "schema": "subRunResult" })));
        assert_eq!(result.duration_ms, 17);
        assert!(result.truncated);
    }
}
