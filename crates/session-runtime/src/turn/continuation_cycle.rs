//! 输出截断后的 continuation 恢复语义。
//!
//! Why: `max_tokens` 截断不是普通完成，也不该只停留在 warning。
//! 这里把“是否继续、何时停止”做成稳定的内部决策层，供 turn loop 复用。

use astrcode_core::{LlmFinishReason, LlmOutput, ResolvedRuntimeConfig};

use super::{TurnLoopTransition, TurnStopCause};

/// 输出截断 continuation 的稳定提示文本。
pub const OUTPUT_CONTINUATION_PROMPT: &str = "Continue from the exact point where the previous \
                                              response was cut off. Do not restart, recap, or \
                                              apologize.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputContinuationDecision {
    Continue,
    Stop(TurnStopCause),
    NotNeeded,
}

pub fn decide_output_continuation(
    output: &LlmOutput,
    continuation_attempts: usize,
    runtime: &ResolvedRuntimeConfig,
) -> OutputContinuationDecision {
    if !matches!(output.finish_reason, LlmFinishReason::MaxTokens) {
        return OutputContinuationDecision::NotNeeded;
    }
    if !output.tool_calls.is_empty() {
        return OutputContinuationDecision::NotNeeded;
    }
    if continuation_attempts >= runtime.max_output_continuation_attempts as usize {
        return OutputContinuationDecision::Stop(TurnStopCause::MaxOutputContinuationLimitReached);
    }
    OutputContinuationDecision::Continue
}

pub fn continuation_transition() -> TurnLoopTransition {
    TurnLoopTransition::OutputContinuationRequested
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmUsage, ReasoningContent};

    use super::*;

    fn output(finish_reason: LlmFinishReason) -> LlmOutput {
        LlmOutput {
            content: "partial".to_string(),
            tool_calls: Vec::new(),
            reasoning: Some(ReasoningContent {
                content: "thinking".to_string(),
                signature: None,
            }),
            usage: Some(LlmUsage {
                input_tokens: 12,
                output_tokens: 24,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }),
            finish_reason,
        }
    }

    #[test]
    fn output_continuation_continues_when_attempts_remain() {
        assert_eq!(
            decide_output_continuation(
                &output(LlmFinishReason::MaxTokens),
                0,
                &ResolvedRuntimeConfig::default()
            ),
            OutputContinuationDecision::Continue
        );
    }

    #[test]
    fn output_continuation_stops_when_limit_is_reached() {
        let runtime = ResolvedRuntimeConfig {
            max_output_continuation_attempts: 1,
            ..ResolvedRuntimeConfig::default()
        };

        assert_eq!(
            decide_output_continuation(&output(LlmFinishReason::MaxTokens), 1, &runtime),
            OutputContinuationDecision::Stop(TurnStopCause::MaxOutputContinuationLimitReached)
        );
    }
}
