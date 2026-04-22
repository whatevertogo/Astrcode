//! step 级 LLM 后置决策策略。
//!
//! Why: 把“无工具输出后是否继续、何时停止”的判断收敛到单一决策层，
//! 避免 `continuation_cycle`、`step` 与后续扩展通过执行顺序隐式耦合。

use astrcode_core::{LlmOutput, ModelLimits, ResolvedRuntimeConfig, UserMessageOrigin};

use crate::turn::{
    continuation_cycle::{
        OUTPUT_CONTINUATION_PROMPT, OutputContinuationDecision, continuation_transition,
        decide_output_continuation,
    },
    loop_control::{TurnLoopTransition, TurnStopCause},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostLlmDecision {
    ContinueWithPrompt {
        nudge: &'static str,
        origin: UserMessageOrigin,
        transition: TurnLoopTransition,
    },
    Stop(TurnStopCause),
    ExecuteTools,
}

#[derive(Debug, Clone)]
pub(crate) struct PostLlmDecisionPolicy {
    runtime: ResolvedRuntimeConfig,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PostLlmDecisionInput<'a> {
    pub(crate) output: &'a LlmOutput,
    pub(crate) max_output_continuation_count: usize,
}

impl PostLlmDecisionPolicy {
    pub(crate) fn new(runtime: &ResolvedRuntimeConfig, _limits: ModelLimits) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }

    pub(crate) fn decide(&self, input: PostLlmDecisionInput<'_>) -> PostLlmDecision {
        if !input.output.tool_calls.is_empty() {
            return PostLlmDecision::ExecuteTools;
        }

        match decide_output_continuation(
            input.output,
            input.max_output_continuation_count,
            &self.runtime,
        ) {
            OutputContinuationDecision::Continue => PostLlmDecision::ContinueWithPrompt {
                nudge: OUTPUT_CONTINUATION_PROMPT,
                origin: UserMessageOrigin::ContinuationPrompt,
                transition: continuation_transition(),
            },
            OutputContinuationDecision::NotNeeded => {
                PostLlmDecision::Stop(TurnStopCause::Completed)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmFinishReason, LlmUsage, ReasoningContent};

    use super::*;

    fn output(
        content: &str,
        finish_reason: LlmFinishReason,
        output_tokens: usize,
        tool_calls: Vec<astrcode_core::ToolCallRequest>,
    ) -> LlmOutput {
        LlmOutput {
            content: content.to_string(),
            tool_calls,
            reasoning: Some(ReasoningContent {
                content: "thinking".to_string(),
                signature: None,
            }),
            usage: Some(LlmUsage {
                input_tokens: 20,
                output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }),
            finish_reason,
            prompt_cache_diagnostics: None,
        }
    }

    #[test]
    fn policy_prefers_execute_tools_when_tool_calls_exist() {
        let policy = PostLlmDecisionPolicy::new(
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        );

        let decision = policy.decide(PostLlmDecisionInput {
            output: &output(
                "",
                LlmFinishReason::ToolCalls,
                0,
                vec![astrcode_core::ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "readFile".to_string(),
                    args: serde_json::json!({"path":"src/lib.rs"}),
                }],
            ),
            max_output_continuation_count: 0,
        });

        assert_eq!(decision, PostLlmDecision::ExecuteTools);
    }

    #[test]
    fn policy_requests_output_continuation_before_completion() {
        let policy = PostLlmDecisionPolicy::new(
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        );

        let decision = policy.decide(PostLlmDecisionInput {
            output: &output("partial", LlmFinishReason::MaxTokens, 24, Vec::new()),
            max_output_continuation_count: 0,
        });

        assert_eq!(
            decision,
            PostLlmDecision::ContinueWithPrompt {
                nudge: OUTPUT_CONTINUATION_PROMPT,
                origin: UserMessageOrigin::ContinuationPrompt,
                transition: TurnLoopTransition::OutputContinuationRequested,
            }
        );
    }

    #[test]
    fn policy_falls_back_to_completed_when_no_continuation_is_needed() {
        let policy = PostLlmDecisionPolicy::new(
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        );

        let decision = policy.decide(PostLlmDecisionInput {
            output: &output("done", LlmFinishReason::Stop, 128, Vec::new()),
            max_output_continuation_count: 0,
        });

        assert_eq!(decision, PostLlmDecision::Stop(TurnStopCause::Completed));
    }
}
