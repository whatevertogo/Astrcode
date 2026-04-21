//! step 级 LLM 后置决策策略。
//!
//! Why: 把“无工具输出后是否继续、何时停止”的判断收敛到单一决策层，
//! 避免 `continuation_cycle`、`loop_control` 与 `step` 通过执行顺序隐式耦合。

use astrcode_core::{LlmOutput, ModelLimits, ResolvedRuntimeConfig, UserMessageOrigin};

use crate::{
    context_window::token_usage::estimate_text_tokens,
    turn::{
        continuation_cycle::{
            OUTPUT_CONTINUATION_PROMPT, OutputContinuationDecision, continuation_transition,
            decide_output_continuation,
        },
        loop_control::{
            AUTO_CONTINUE_NUDGE, BudgetContinuationDecision, TurnLoopTransition, TurnStopCause,
            decide_budget_continuation,
        },
    },
};

const DIMINISHING_RETURNS_MIN_CONTINUATIONS: usize = 2;
const DIMINISHING_RETURNS_LOW_OUTPUT_TOKENS: usize = 48;
const DIMINISHING_RETURNS_WINDOW: usize = 3;

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
    limits: ModelLimits,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PostLlmDecisionInput<'a> {
    pub(crate) output: &'a LlmOutput,
    pub(crate) step_index: usize,
    pub(crate) continuation_count: usize,
    pub(crate) max_output_continuation_count: usize,
    pub(crate) used_budget_tokens: usize,
    pub(crate) recent_output_tokens: &'a [usize],
}

impl PostLlmDecisionPolicy {
    pub(crate) fn new(runtime: &ResolvedRuntimeConfig, limits: ModelLimits) -> Self {
        Self {
            runtime: runtime.clone(),
            limits,
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
            OutputContinuationDecision::Continue => {
                return PostLlmDecision::ContinueWithPrompt {
                    nudge: OUTPUT_CONTINUATION_PROMPT,
                    origin: UserMessageOrigin::ContinuationPrompt,
                    transition: continuation_transition(),
                };
            },
            OutputContinuationDecision::Stop(stop_cause) => {
                return PostLlmDecision::Stop(stop_cause);
            },
            OutputContinuationDecision::NotNeeded => {},
        }

        if has_diminishing_returns(input.continuation_count, input.recent_output_tokens) {
            return PostLlmDecision::Stop(TurnStopCause::BudgetStoppedContinuation);
        }

        match decide_budget_continuation(
            input.output,
            input.step_index,
            input.continuation_count,
            &self.runtime,
            self.limits,
            input.used_budget_tokens,
        ) {
            BudgetContinuationDecision::Continue => PostLlmDecision::ContinueWithPrompt {
                nudge: AUTO_CONTINUE_NUDGE,
                origin: UserMessageOrigin::AutoContinueNudge,
                transition: TurnLoopTransition::BudgetAllowsContinuation,
            },
            BudgetContinuationDecision::Stop(stop_cause) => PostLlmDecision::Stop(stop_cause),
            BudgetContinuationDecision::NotNeeded => {
                PostLlmDecision::Stop(TurnStopCause::Completed)
            },
        }
    }
}

pub(crate) fn output_token_count(output: &LlmOutput) -> usize {
    output
        .usage
        .map(|usage| usage.output_tokens)
        .unwrap_or_else(|| estimate_text_tokens(output.content.trim()))
}

fn has_diminishing_returns(continuation_count: usize, recent_output_tokens: &[usize]) -> bool {
    continuation_count >= DIMINISHING_RETURNS_MIN_CONTINUATIONS
        && recent_output_tokens.len() >= DIMINISHING_RETURNS_WINDOW
        && recent_output_tokens
            .iter()
            .rev()
            .take(DIMINISHING_RETURNS_WINDOW)
            .all(|tokens| *tokens <= DIMINISHING_RETURNS_LOW_OUTPUT_TOKENS)
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
            step_index: 1,
            continuation_count: 0,
            max_output_continuation_count: 0,
            used_budget_tokens: 0,
            recent_output_tokens: &[],
        });

        assert_eq!(decision, PostLlmDecision::ExecuteTools);
    }

    #[test]
    fn policy_requests_output_continuation_before_budget_logic() {
        let policy = PostLlmDecisionPolicy::new(
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        );

        let decision = policy.decide(PostLlmDecisionInput {
            output: &output("partial", LlmFinishReason::MaxTokens, 24, Vec::new()),
            step_index: 1,
            continuation_count: 0,
            max_output_continuation_count: 0,
            used_budget_tokens: 0,
            recent_output_tokens: &[24],
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
    fn policy_stops_on_diminishing_returns_before_budget_continue() {
        let policy = PostLlmDecisionPolicy::new(
            &ResolvedRuntimeConfig::default(),
            ModelLimits {
                context_window: 128_000,
                max_output_tokens: 8_000,
            },
        );

        let decision = policy.decide(PostLlmDecisionInput {
            output: &output("brief", LlmFinishReason::Stop, 20, Vec::new()),
            step_index: 3,
            continuation_count: 2,
            max_output_continuation_count: 0,
            used_budget_tokens: 50,
            recent_output_tokens: &[24, 20, 18],
        });

        assert_eq!(
            decision,
            PostLlmDecision::Stop(TurnStopCause::BudgetStoppedContinuation)
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
            step_index: 1,
            continuation_count: 0,
            max_output_continuation_count: 0,
            used_budget_tokens: 50,
            recent_output_tokens: &[128],
        });

        assert_eq!(decision, PostLlmDecision::Stop(TurnStopCause::Completed));
    }
}
