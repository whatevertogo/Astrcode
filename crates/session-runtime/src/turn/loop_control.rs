//! turn loop 的显式过渡/停止语义。
//!
//! Why: `request -> llm -> tool` 的编排已经模块化，但“为什么继续/停止”
//! 仍需要一个稳定骨架，否则后续 auto-continue、输出截断恢复和流式工具调度
//! 都会退化成新的局部布尔值。


use astrcode_core::{LlmFinishReason, LlmOutput, ModelLimits, ResolvedRuntimeConfig};

use crate::context_window::token_usage::estimate_text_tokens;

/// 自动续写提示的稳定文本。
pub const AUTO_CONTINUE_NUDGE: &str =
    "继续推进当前任务。仅在仍有未完成内容时继续，不要重复已经给出的结论。";

/// 内部 loop 的“继续下一轮”原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnLoopTransition {
    ToolCycleCompleted,
    ReactiveCompactRecovered,
    BudgetAllowsContinuation,
    OutputContinuationRequested,
}

/// turn 停止的细粒度原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStopCause {
    Completed,
    Cancelled,
    Error,
    StepLimitExceeded,
    BudgetStoppedContinuation,
    ContinuationLimitReached,
    MaxOutputContinuationLimitReached,
}

/// budget 驱动 auto-continue 的判断结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetContinuationDecision {
    Continue,
    Stop(TurnStopCause),
    NotNeeded,
}

impl TurnStopCause {
    pub fn turn_done_reason(self) -> Option<&'static str> {
        match self {
            Self::Completed => Some("completed"),
            Self::BudgetStoppedContinuation => Some("budget_stopped"),
            Self::ContinuationLimitReached => Some("continuation_limit_reached"),
            Self::MaxOutputContinuationLimitReached => Some("token_exceeded"),
            Self::Cancelled | Self::Error | Self::StepLimitExceeded => None,
        }
    }
}

/// Why: 当前仓库还没有正式的显式 `tokenBudget` 输入合同，
/// 第一阶段使用 provider `max_output_tokens * (max_continuations + 1)` 作为稳定默认预算，
/// 先把 loop 语义和恢复路径接稳，后续再把显式 budget 接进来替换这层默认值。
pub fn decide_budget_continuation(
    output: &LlmOutput,
    step_index: usize,
    continuation_count: usize,
    runtime: &ResolvedRuntimeConfig,
    limits: ModelLimits,
    used_budget_tokens: usize,
) -> BudgetContinuationDecision {
    if !output.tool_calls.is_empty() || !matches!(output.finish_reason, LlmFinishReason::Stop) {
        return BudgetContinuationDecision::NotNeeded;
    }

    let output_tokens = output
        .usage
        .map(|usage| usage.output_tokens)
        .unwrap_or_else(|| estimate_text_tokens(output.content.trim()));
    if output_tokens == 0 {
        return BudgetContinuationDecision::NotNeeded;
    }
    if step_index == 0 {
        return BudgetContinuationDecision::NotNeeded;
    }

    if continuation_count >= runtime.max_continuations as usize {
        return BudgetContinuationDecision::Stop(TurnStopCause::ContinuationLimitReached);
    }

    let total_budget = limits
        .max_output_tokens
        .saturating_mul(runtime.max_continuations as usize + 1);
    let remaining_budget = total_budget.saturating_sub(used_budget_tokens);

    // Why: auto-continue 只针对“输出明显偏短、且预算还有富余”的场景。
    // 这里故意保守：短输出阈值固定为 96 tokens，且剩余预算至少还能再撑两轮同规模回复。
    // TODO: 待评估
    let output_is_short = output_tokens <= 96;
    let budget_is_healthy = remaining_budget >= output_tokens.saturating_mul(2).max(96);

    if output_is_short && budget_is_healthy {
        BudgetContinuationDecision::Continue
    } else if output_is_short {
        BudgetContinuationDecision::Stop(TurnStopCause::BudgetStoppedContinuation)
    } else {
        BudgetContinuationDecision::NotNeeded
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmUsage, ReasoningContent, ToolCallRequest};
    use serde_json::json;

    use super::*;

    fn output(content: &str, finish_reason: LlmFinishReason, output_tokens: u32) -> LlmOutput {
        LlmOutput {
            content: content.to_string(),
            tool_calls: Vec::new(),
            reasoning: Some(ReasoningContent {
                content: "thinking".to_string(),
                signature: None,
            }),
            usage: Some(LlmUsage {
                input_tokens: 20,
                output_tokens: output_tokens as usize,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }),
            finish_reason,
        }
    }

    #[test]
    fn budget_continuation_continues_when_output_is_short_and_budget_is_healthy() {
        let decision = decide_budget_continuation(
            &output("brief", LlmFinishReason::Stop, 24),
            1,
            0,
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
            50,
        );

        assert_eq!(decision, BudgetContinuationDecision::Continue);
    }

    #[test]
    fn budget_continuation_stops_when_limit_is_reached() {
        let runtime = ResolvedRuntimeConfig {
            max_continuations: 1,
            ..ResolvedRuntimeConfig::default()
        };

        let decision = decide_budget_continuation(
            &output("brief", LlmFinishReason::Stop, 24),
            1,
            1,
            &runtime,
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
            50,
        );

        assert_eq!(
            decision,
            BudgetContinuationDecision::Stop(TurnStopCause::ContinuationLimitReached)
        );
    }

    #[test]
    fn budget_continuation_ignores_long_or_tool_call_outputs() {
        let tool_output = LlmOutput {
            content: String::new(),
            tool_calls: vec![ToolCallRequest {
                id: "call-1".to_string(),
                name: "readFile".to_string(),
                args: json!({"path":"src/lib.rs"}),
            }],
            reasoning: None,
            usage: None,
            finish_reason: LlmFinishReason::ToolCalls,
        };
        let long_output = output(&"x".repeat(800), LlmFinishReason::Stop, 128);

        assert_eq!(
            decide_budget_continuation(
                &tool_output,
                1,
                0,
                &ResolvedRuntimeConfig::default(),
                ModelLimits {
                    context_window: 128_000,
                    max_output_tokens: 8_000,
                },
                50,
            ),
            BudgetContinuationDecision::NotNeeded
        );
        assert_eq!(
            decide_budget_continuation(
                &long_output,
                1,
                0,
                &ResolvedRuntimeConfig::default(),
                ModelLimits {
                    context_window: 128_000,
                    max_output_tokens: 8_000,
                },
                50,
            ),
            BudgetContinuationDecision::NotNeeded
        );
    }
}
