//! # Token 使用跟踪
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
//! - 文本内容: 按字符数估算（4 chars/token）
//!
//! ## 为什么不用精确 Tokenizer
//!
//! 精确 Token 计数需要 Provider 原生的 Tokenizer，当前后端未暴露此能力。
//! 一旦后端暴露精确 Token 计算和上下文窗口元数据，应替换此启发式。

use astrcode_core::{LlmMessage, LlmUsage, ModelLimits, UserMessageOrigin};

use crate::heuristics::{MESSAGE_BASE_TOKENS, TOOL_CALL_BASE_TOKENS};

const REQUEST_ESTIMATE_PADDING_NUMERATOR: usize = 4;
const REQUEST_ESTIMATE_PADDING_DENOMINATOR: usize = 3;

/// Prompt token 使用快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptTokenSnapshot {
    /// 估算的上下文 token 数。
    pub context_tokens: usize,
    /// 已确认的预算 token 数（优先使用 Provider 报告值）。
    pub budget_tokens: usize,
    /// 模型上下文窗口大小。
    pub context_window: usize,
    /// 有效上下文窗口（扣除压缩预留）。
    pub effective_window: usize,
    /// 触发压缩的阈值 token 数。
    pub threshold_tokens: usize,
    /// 剩余可用 token 数（已经扣除 compact 输出预留）。
    pub remaining_context_tokens: usize,
    /// 当剩余空间低于该值时应触发 compact。
    pub reserved_context_size: usize,
}

/// Token 使用跟踪器。
///
/// 优先使用 Provider 报告的 usage 数据（最接近计费 Token），
/// 若 Provider 未报告则回退到估算值。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TokenUsageTracker {
    anchored_budget_tokens: usize,
}

impl TokenUsageTracker {
    /// 记录 Provider 报告的 token 使用量。
    pub fn record_usage(&mut self, usage: Option<LlmUsage>) {
        let Some(usage) = usage else {
            return;
        };
        self.anchored_budget_tokens = self
            .anchored_budget_tokens
            .saturating_add(usage.total_tokens());
    }

    /// 返回当前预算 token 数，优先使用 Provider 报告值。
    pub fn budget_tokens(&self, estimated_context_tokens: usize) -> usize {
        if self.anchored_budget_tokens > 0 {
            self.anchored_budget_tokens
        } else {
            estimated_context_tokens
        }
    }
}

/// 构建 Prompt Token 快照。
pub fn build_prompt_snapshot(
    tracker: &TokenUsageTracker,
    messages: &[LlmMessage],
    system_prompt: Option<&str>,
    limits: ModelLimits,
    threshold_percent: u8,
    summary_reserve_tokens: usize,
    reserved_context_size: usize,
) -> PromptTokenSnapshot {
    let context_tokens = estimate_request_tokens(messages, system_prompt);
    let effective_window = effective_context_window(limits, summary_reserve_tokens);
    PromptTokenSnapshot {
        context_tokens,
        budget_tokens: tracker.budget_tokens(context_tokens),
        context_window: limits.context_window,
        effective_window,
        threshold_tokens: compact_threshold_tokens(effective_window, threshold_percent),
        remaining_context_tokens: effective_window.saturating_sub(context_tokens),
        reserved_context_size,
    }
}

/// 计算有效上下文窗口（扣除压缩预留）。
pub fn effective_context_window(limits: ModelLimits, summary_reserve_tokens: usize) -> usize {
    limits
        .context_window
        .saturating_sub(summary_reserve_tokens.min(limits.context_window))
}

/// 计算压缩阈值 token 数。
pub fn compact_threshold_tokens(effective_window: usize, threshold_percent: u8) -> usize {
    effective_window
        .saturating_mul(threshold_percent as usize)
        .saturating_div(100)
}

/// 判断是否需要触发压缩。
pub fn should_compact(snapshot: PromptTokenSnapshot) -> bool {
    snapshot.context_tokens >= snapshot.threshold_tokens
        || snapshot.remaining_context_tokens <= snapshot.reserved_context_size
}

/// 估算完整 LLM 请求的 token 数（messages + system prompt）。
pub fn estimate_request_tokens(messages: &[LlmMessage], system_prompt: Option<&str>) -> usize {
    let system_tokens = system_prompt.map_or(0, estimate_text_tokens);
    let raw_total = system_tokens + messages.iter().map(estimate_message_tokens).sum::<usize>();
    raw_total
        .saturating_mul(REQUEST_ESTIMATE_PADDING_NUMERATOR)
        .div_ceil(REQUEST_ESTIMATE_PADDING_DENOMINATOR)
}

/// 估算单条消息的 token 数。
pub fn estimate_message_tokens(message: &LlmMessage) -> usize {
    match message {
        LlmMessage::User { content, origin } => {
            MESSAGE_BASE_TOKENS
                + estimate_text_tokens(content)
                + match origin {
                    UserMessageOrigin::User => 0,
                    UserMessageOrigin::QueuedInput => 8,
                    UserMessageOrigin::AutoContinueNudge => 6,
                    UserMessageOrigin::ContinuationPrompt => 10,
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
                    .map_or(0, |r| estimate_text_tokens(&r.content))
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

/// 文本 token 估算（4 chars/token）。
pub fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
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

        assert_eq!(effective_context_window(limits, 20_000), 80_000);
        assert_eq!(compact_threshold_tokens(80_000, 90), 72_000);
    }

    #[test]
    fn should_compact_when_remaining_context_is_below_reserved_size() {
        assert!(should_compact(PromptTokenSnapshot {
            context_tokens: 40_000,
            budget_tokens: 40_000,
            context_window: 100_000,
            effective_window: 80_000,
            threshold_tokens: 72_000,
            remaining_context_tokens: 10_000,
            reserved_context_size: 20_000,
        }));
    }

    #[test]
    fn tracker_prefers_provider_usage_over_estimate() {
        let mut tracker = TokenUsageTracker::default();
        let usage = LlmUsage {
            input_tokens: 1000,
            output_tokens: 200,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        tracker.record_usage(Some(usage));

        // 即使估算值不同，也应使用 Provider 报告值（total = input + output = 1200）
        assert_eq!(tracker.budget_tokens(5000), 1200);
    }

    #[test]
    fn tracker_falls_back_to_estimate_when_no_provider_usage() {
        let tracker = TokenUsageTracker::default();
        assert_eq!(tracker.budget_tokens(5000), 5000);
    }
}
