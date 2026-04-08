//! # Token 使用跟踪 (Token Usage Tracking)
//!
//! 提供 Token 估算和跟踪能力，用于：
//! - 构建 Prompt Token 快照（当前上下文大小、预算、窗口限制）
//! - 估算消息和文本的 Token 数量
//! - 判断是否需要触发压缩
//!
//! ## Token 估算启发式
//!
//! 当前使用简化的启发式估算：
//! - 每条消息基础开销: 6 tokens
//! - 每个工具调用基础开销: 12 tokens
//! - 文本内容: 按字符数估算（粗略近似）
//!
//! ## 为什么不用精确 Tokenizer
//!
//! 精确 Token 计数需要 Provider 原生的 Tokenizer，当前后端未暴露此能力。
//! 一旦后端暴露精确 Token 计算和上下文窗口元数据，应替换此启发式。
//!
//! ## 预算跟踪
//!
//! `TokenUsageTracker` 优先使用 Provider 报告的 usage 数据（最接近计费 Token），
//! 若 Provider 未报告则回退到估算值。

use astrcode_core::{LlmMessage, UserMessageOrigin};
use astrcode_runtime_llm::{LlmUsage, ModelLimits};

pub(crate) const SUMMARY_RESERVE_TOKENS: usize = 20_000;
const MESSAGE_BASE_TOKENS: usize = 6;
const TOOL_CALL_BASE_TOKENS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTokenSnapshot {
    pub context_tokens: usize,
    pub budget_tokens: usize,
    pub context_window: usize,
    pub effective_window: usize,
    pub threshold_tokens: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TokenUsageTracker {
    anchored_budget_tokens: usize,
}

impl TokenUsageTracker {
    pub(crate) fn record_usage(&mut self, usage: Option<LlmUsage>) {
        let Some(usage) = usage else {
            return;
        };
        // Provider-reported usage is the closest thing we have to billed tokens, so budget
        // accounting prefers it over prompt-size heuristics whenever the backend exposes it.
        self.anchored_budget_tokens = self
            .anchored_budget_tokens
            .saturating_add(usage.total_tokens());
    }

    pub(crate) fn budget_tokens(&self, estimated_context_tokens: usize) -> usize {
        if self.anchored_budget_tokens > 0 {
            self.anchored_budget_tokens
        } else {
            estimated_context_tokens
        }
    }
}

pub fn build_prompt_snapshot(
    tracker: &TokenUsageTracker,
    messages: &[LlmMessage],
    system_prompt: Option<&str>,
    limits: ModelLimits,
    threshold_percent: u8,
) -> PromptTokenSnapshot {
    let context_tokens = estimate_request_tokens(messages, system_prompt);
    PromptTokenSnapshot {
        context_tokens,
        budget_tokens: tracker.budget_tokens(context_tokens),
        context_window: limits.context_window,
        effective_window: effective_context_window(limits),
        threshold_tokens: compact_threshold_tokens(limits, threshold_percent),
    }
}

pub fn effective_context_window(limits: ModelLimits) -> usize {
    limits
        .context_window
        .saturating_sub(SUMMARY_RESERVE_TOKENS.min(limits.context_window))
}

pub(crate) fn compact_threshold_tokens(limits: ModelLimits, threshold_percent: u8) -> usize {
    effective_context_window(limits)
        .saturating_mul(threshold_percent as usize)
        .saturating_div(100)
}

pub fn should_compact(snapshot: PromptTokenSnapshot) -> bool {
    snapshot.context_tokens >= snapshot.threshold_tokens
}

pub fn estimate_request_tokens(messages: &[LlmMessage], system_prompt: Option<&str>) -> usize {
    // TODO(claude-auto-compact): replace this full-scan heuristic with a provider-native tokenizer
    // once the backends expose exact token accounting and context-window metadata.
    let system_tokens = system_prompt.map_or(0, estimate_text_tokens);
    system_tokens + messages.iter().map(estimate_message_tokens).sum::<usize>()
}

pub fn estimate_message_tokens(message: &LlmMessage) -> usize {
    match message {
        LlmMessage::User { content, origin } => {
            MESSAGE_BASE_TOKENS
                + estimate_text_tokens(content)
                + match origin {
                    UserMessageOrigin::User => 0,
                    UserMessageOrigin::AutoContinueNudge => 8,
                    UserMessageOrigin::ReactivationPrompt => 8,
                    UserMessageOrigin::CompactSummary => 16,
                }
        },
        LlmMessage::Assistant {
            content,
            tool_calls,
            reasoning,
        } => {
            MESSAGE_BASE_TOKENS
                + estimate_text_tokens(content)
                + reasoning
                    .as_ref()
                    .map_or(0, |value| estimate_text_tokens(&value.content))
                + tool_calls
                    .iter()
                    .map(|call| {
                        TOOL_CALL_BASE_TOKENS
                            + estimate_text_tokens(&call.id)
                            + estimate_text_tokens(&call.name)
                            + estimate_json_tokens(&call.args.to_string())
                    })
                    .sum::<usize>()
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => {
            MESSAGE_BASE_TOKENS + estimate_text_tokens(tool_call_id) + estimate_text_tokens(content)
        },
    }
}

pub fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    // 4 chars/token is a conservative cross-provider heuristic for ASCII-heavy code/chat text.
    chars.div_ceil(4).max(1)
}

fn estimate_json_tokens(json: &str) -> usize {
    estimate_text_tokens(json) + 4
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ReasoningContent, ToolCallRequest};
    use serde_json::json;

    use super::*;

    #[test]
    fn request_estimate_includes_system_and_message_content() {
        let messages = vec![
            LlmMessage::User {
                content: "inspect src/main.rs".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "I will inspect it.".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: json!({"path": "src/main.rs"}),
                }],
                reasoning: Some(ReasoningContent {
                    content: "Need file contents first.".to_string(),
                    signature: None,
                }),
            },
        ];

        let estimate = estimate_request_tokens(&messages, Some("system"));
        assert!(estimate > 0);
    }

    #[test]
    fn compact_threshold_uses_effective_window() {
        let limits = ModelLimits {
            context_window: 100_000,
            max_output_tokens: 8_000,
        };

        assert_eq!(effective_context_window(limits), 80_000);
        assert_eq!(compact_threshold_tokens(limits, 90), 72_000);
    }
}
