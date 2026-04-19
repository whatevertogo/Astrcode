use astrcode_core::{LlmFinishReason, LlmOutput, LlmRequest, Result};

use super::{TurnExecutionContext, TurnExecutionResources, driver::StepDriver};
use crate::turn::llm_cycle::ToolCallDeltaSink;

pub(super) enum StepLlmResult {
    Output(LlmOutput),
    RecoveredByReactiveCompact,
}

pub(super) async fn call_llm_for_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
    llm_request: LlmRequest,
    tool_delta_sink: Option<ToolCallDeltaSink>,
) -> Result<StepLlmResult> {
    match driver
        .call_llm(resources, llm_request, tool_delta_sink)
        .await
    {
        Ok(output) => Ok(StepLlmResult::Output(output)),
        Err(error) => {
            if error.is_cancelled() {
                return Err(error);
            }
            if error.is_prompt_too_long()
                && execution.reactive_compact_attempts
                    < resources.settings.compact_max_retry_attempts
            {
                execution.reactive_compact_attempts =
                    execution.reactive_compact_attempts.saturating_add(1);
                log::warn!(
                    "turn {} step {}: prompt too long, reactive compact ({}/{})",
                    resources.turn_id,
                    execution.step_index,
                    execution.reactive_compact_attempts,
                    resources.settings.compact_max_retry_attempts,
                );

                let recovery = driver.try_reactive_compact(execution, resources).await?;

                if let Some(result) = recovery {
                    execution.events.extend(result.events);
                    execution.messages = result.messages;
                    return Ok(StepLlmResult::RecoveredByReactiveCompact);
                }
            }
            Err(error)
        },
    }
}

pub(super) fn record_llm_usage(execution: &mut TurnExecutionContext, output: &LlmOutput) {
    execution.token_tracker.record_usage(output.usage);
    if let Some(usage) = &output.usage {
        execution.total_cache_read_tokens = execution
            .total_cache_read_tokens
            .saturating_add(usage.cache_read_input_tokens as u64);
        execution.total_cache_creation_tokens = execution
            .total_cache_creation_tokens
            .saturating_add(usage.cache_creation_input_tokens as u64);
    }
}

pub(super) fn warn_if_output_truncated(
    resources: &TurnExecutionResources<'_>,
    execution: &TurnExecutionContext,
    output: &LlmOutput,
) {
    if matches!(output.finish_reason, LlmFinishReason::MaxTokens) {
        log::warn!(
            "turn {} step {}: LLM output truncated by max_tokens",
            resources.turn_id,
            execution.step_index
        );
    }
}
