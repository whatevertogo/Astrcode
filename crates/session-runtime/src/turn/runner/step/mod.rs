mod driver;
mod llm_step;
mod streaming_tools;
mod tool_execution;

#[cfg(test)]
mod tests;

use std::time::Instant;

use astrcode_core::{LlmMessage, LlmOutput, Result, StorageEventPayload, UserMessageOrigin};
use chrono::Utc;
use driver::{RuntimeStepDriver, StepDriver};
use llm_step::{StepLlmResult, call_llm_for_step, record_llm_usage, warn_if_output_truncated};
use streaming_tools::StreamingToolPlannerHandle;
use tool_execution::{ToolExecutionDisposition, finalize_and_execute_tool_calls};

use super::{TurnExecutionContext, TurnExecutionResources};
use crate::turn::{
    events::{apply_prompt_metrics_usage, assistant_final_event, user_message_event},
    loop_control::{TurnLoopTransition, TurnStopCause},
    post_llm_policy::{PostLlmDecision, PostLlmDecisionInput, PostLlmDecisionPolicy},
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
        Ok(StepLlmResult::Output(output)) => *output,
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
    apply_prompt_metrics_usage(
        execution.journal.events_mut(),
        execution.lifecycle.step_index,
        output.usage,
        output.prompt_cache_diagnostics.clone(),
    );
    append_assistant_output(execution, resources, &output);
    warn_if_output_truncated(resources, execution, &output);

    match decide_post_llm_action(execution, resources, &output) {
        PostLlmDecision::ExecuteTools => match finalize_and_execute_tool_calls(
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
                execution.lifecycle.step_index += 1;
                Ok(StepOutcome::Continue(
                    TurnLoopTransition::ToolCycleCompleted,
                ))
            },
            ToolExecutionDisposition::Interrupted => {
                Ok(StepOutcome::Cancelled(TurnStopCause::Cancelled))
            },
        },
        PostLlmDecision::ContinueWithPrompt {
            nudge,
            origin,
            transition,
        } => {
            streaming_planner.abort_all();
            if matches!(origin, UserMessageOrigin::ContinuationPrompt) {
                execution.lifecycle.max_output_continuation_count = execution
                    .lifecycle
                    .max_output_continuation_count
                    .saturating_add(1);
            }
            append_internal_user_message(execution, resources, nudge, origin);
            execution.lifecycle.step_index += 1;
            Ok(StepOutcome::Continue(transition))
        },
        PostLlmDecision::Stop(stop_cause) => {
            streaming_planner.abort_all();
            Ok(StepOutcome::Completed(stop_cause))
        },
    }
}

fn decide_post_llm_action(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> PostLlmDecision {
    let policy = PostLlmDecisionPolicy::new(resources.runtime, resources.gateway.model_limits());

    policy.decide(PostLlmDecisionInput {
        output,
        max_output_continuation_count: execution.lifecycle.max_output_continuation_count,
    })
}

fn append_assistant_output(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) {
    let content = output.content.trim().to_string();
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
    let suppress_assistant_follow_up = execution.draft_plan_approval_guard_active
        || should_suppress_exit_plan_follow_up(execution);
    execution.messages.push(LlmMessage::Assistant {
        content: content.clone(),
        tool_calls: output.tool_calls.clone(),
        reasoning: output.reasoning.clone(),
    });
    if suppress_assistant_follow_up {
        execution.messages.pop();
    }
    execution
        .budget
        .micro_compact_state
        .record_assistant_activity(Instant::now());
    if has_persistable_assistant_output && !suppress_assistant_follow_up {
        execution.journal.push(assistant_final_event(
            resources.turn_id,
            resources.agent,
            content,
            reasoning_content,
            reasoning_signature,
            execution.lifecycle.step_index,
            Some(Utc::now()),
        ));
    }
}

fn should_suppress_exit_plan_follow_up(execution: &TurnExecutionContext) -> bool {
    execution
        .journal
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
            StorageEventPayload::ToolResult {
                tool_name,
                metadata,
                ..
            } if tool_name == "exitPlanMode" => metadata
                .as_ref()
                .and_then(|metadata| metadata.get("schema"))
                .and_then(|value| value.as_str()),
            _ => None,
        })
        .is_some_and(|schema| matches!(schema, "sessionPlanExitReviewPending" | "sessionPlanExit"))
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
    execution.journal.push(user_message_event(
        resources.turn_id,
        resources.agent,
        content.to_string(),
        origin,
        Utc::now(),
    ));
}
