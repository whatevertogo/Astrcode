mod driver;
mod llm_step;
mod streaming_tools;
mod tool_execution;

#[cfg(test)]
mod tests;

use std::time::Instant;

use astrcode_core::{LlmMessage, LlmOutput, Result, UserMessageOrigin};
use chrono::Utc;
use driver::{RuntimeStepDriver, StepDriver};
use llm_step::{StepLlmResult, call_llm_for_step, record_llm_usage, warn_if_output_truncated};
use streaming_tools::StreamingToolPlannerHandle;
use tool_execution::{ToolExecutionDisposition, finalize_and_execute_tool_calls};

use super::{TurnExecutionContext, TurnExecutionResources};
use crate::turn::{
    continuation_cycle::{
        OUTPUT_CONTINUATION_PROMPT, OutputContinuationDecision, continuation_transition,
        decide_output_continuation,
    },
    events::{assistant_final_event, turn_done_event, user_message_event},
    loop_control::{
        AUTO_CONTINUE_NUDGE, BudgetContinuationDecision, TurnLoopTransition, TurnStopCause,
        decide_budget_continuation,
    },
};

pub(super) enum StepOutcome {
    Continue(TurnLoopTransition),
    Completed(TurnStopCause),
    Cancelled(TurnStopCause),
}

pub(super) async fn run_single_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
) -> Result<StepOutcome> {
    run_single_step_with(execution, resources, &RuntimeStepDriver).await
}

async fn run_single_step_with(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
) -> Result<StepOutcome> {
    let assembled = driver.assemble_prompt(execution, resources).await?;
    let streaming_planner = StreamingToolPlannerHandle::new(resources);
    let llm_result = call_llm_for_step(
        execution,
        resources,
        driver,
        assembled.llm_request,
        Some(streaming_planner.tool_delta_sink()),
    )
    .await;

    let output = match llm_result {
        Ok(StepLlmResult::Output(output)) => output,
        Ok(StepLlmResult::RecoveredByReactiveCompact) => {
            streaming_planner.abort_all();
            return Ok(StepOutcome::Continue(
                TurnLoopTransition::ReactiveCompactRecovered,
            ));
        },
        Err(error) => {
            streaming_planner.abort_all();
            return Err(error);
        },
    };

    let llm_finished_at = Instant::now();
    record_llm_usage(execution, &output);
    let has_tool_calls = append_assistant_output(execution, resources, &output);
    warn_if_output_truncated(resources, execution, &output);

    if !has_tool_calls {
        streaming_planner.abort_all();
        return Ok(handle_assistant_without_tool_calls(
            execution, resources, &output,
        ));
    }

    match finalize_and_execute_tool_calls(
        execution,
        resources,
        driver,
        &streaming_planner,
        &output,
        llm_finished_at,
    )
    .await?
    {
        ToolExecutionDisposition::Completed => {
            execution.step_index += 1;
            Ok(StepOutcome::Continue(
                TurnLoopTransition::ToolCycleCompleted,
            ))
        },
        ToolExecutionDisposition::Interrupted => {
            Ok(StepOutcome::Cancelled(TurnStopCause::Cancelled))
        },
    }
}

fn handle_assistant_without_tool_calls(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> StepOutcome {
    match decide_output_continuation(
        output,
        execution.max_output_continuation_count,
        resources.runtime,
    ) {
        OutputContinuationDecision::Continue => {
            execution.max_output_continuation_count =
                execution.max_output_continuation_count.saturating_add(1);
            append_internal_user_message(
                execution,
                resources,
                OUTPUT_CONTINUATION_PROMPT,
                UserMessageOrigin::ContinuationPrompt,
            );
            execution.step_index += 1;
            return StepOutcome::Continue(continuation_transition());
        },
        OutputContinuationDecision::Stop(stop_cause) => {
            append_turn_done_event(execution, resources, stop_cause);
            return StepOutcome::Completed(stop_cause);
        },
        OutputContinuationDecision::NotNeeded => {},
    }

    match decide_budget_continuation(
        output,
        execution.step_index,
        execution.continuation_count,
        resources.runtime,
        resources.gateway.model_limits(),
        execution.token_tracker.budget_tokens(0),
    ) {
        BudgetContinuationDecision::Continue => {
            append_internal_user_message(
                execution,
                resources,
                AUTO_CONTINUE_NUDGE,
                UserMessageOrigin::AutoContinueNudge,
            );
            execution.step_index += 1;
            StepOutcome::Continue(TurnLoopTransition::BudgetAllowsContinuation)
        },
        BudgetContinuationDecision::Stop(stop_cause) => {
            append_turn_done_event(execution, resources, stop_cause);
            StepOutcome::Completed(stop_cause)
        },
        BudgetContinuationDecision::NotNeeded => {
            append_turn_done_event(execution, resources, TurnStopCause::Completed);
            StepOutcome::Completed(TurnStopCause::Completed)
        },
    }
}

fn append_assistant_output(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> bool {
    let content = output.content.trim().to_string();
    let has_tool_calls = !output.tool_calls.is_empty();
    let reasoning_content = output
        .reasoning
        .as_ref()
        .map(|reasoning| reasoning.content.clone());
    let reasoning_signature = output
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.signature.clone());
    let has_persistable_assistant_output = !content.is_empty()
        || reasoning_content
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    execution.messages.push(LlmMessage::Assistant {
        content: content.clone(),
        tool_calls: output.tool_calls.clone(),
        reasoning: output.reasoning.clone(),
    });
    execution
        .micro_compact_state
        .record_assistant_activity(Instant::now());
    if has_persistable_assistant_output {
        execution.events.push(assistant_final_event(
            resources.turn_id,
            resources.agent,
            content,
            reasoning_content,
            reasoning_signature,
            Some(Utc::now()),
        ));
    }
    has_tool_calls
}

fn append_turn_done_event(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    stop_cause: TurnStopCause,
) {
    execution.events.push(turn_done_event(
        resources.turn_id,
        resources.agent,
        stop_cause.turn_done_reason().map(ToString::to_string),
        Utc::now(),
    ));
}

fn append_internal_user_message(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    content: &str,
    origin: UserMessageOrigin,
) {
    execution.messages.push(LlmMessage::User {
        content: content.to_string(),
        origin,
    });
    execution.events.push(user_message_event(
        resources.turn_id,
        resources.agent,
        content.to_string(),
        origin,
        Utc::now(),
    ));
}
