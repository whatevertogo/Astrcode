use std::{collections::HashMap, path::Path, time::Instant};

use astrcode_core::{AstrError, LlmMessage, LlmOutput, Result, StorageEvent, StorageEventPayload};

use super::{
    TurnExecutionContext, TurnExecutionResources,
    driver::StepDriver,
    streaming_tools::{
        StreamingToolFinalizeResult, StreamingToolPlannerHandle, StreamingToolStats,
    },
};
use crate::turn::tool_cycle::{ToolCycleOutcome, ToolCycleResult, ToolEventEmissionMode};

pub(super) enum ToolExecutionDisposition {
    Completed,
    Interrupted,
}

pub(super) async fn finalize_and_execute_tool_calls(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
    streaming_planner: &StreamingToolPlannerHandle,
    output: &LlmOutput,
    llm_finished_at: Instant,
) -> Result<ToolExecutionDisposition> {
    let finalized_streaming = streaming_planner
        .finalize(&output.tool_calls, llm_finished_at)
        .await;
    let StreamingToolFinalizeResult {
        matched_results,
        remaining_tool_calls,
        stats,
        used_streaming_path,
    } = finalized_streaming;
    apply_streaming_stats(execution, stats);

    // Why: durable truth 现在以 step 为提交边界，工具结构事件也必须与
    // PromptMetrics / AssistantFinal 同批落盘，避免 turn 中断时留下半个 step。
    let event_emission_mode = ToolEventEmissionMode::Buffered;
    let mut executed_remaining = if remaining_tool_calls.is_empty() {
        empty_tool_cycle_result()
    } else {
        driver
            .execute_tool_cycle(
                execution,
                resources,
                remaining_tool_calls,
                event_emission_mode,
            )
            .await?
    };

    if used_streaming_path {
        merge_buffered_and_remaining_tool_results(
            execution,
            output,
            &matched_results,
            &mut executed_remaining,
        )?;
    } else {
        execution
            .journal
            .extend(std::mem::take(&mut executed_remaining.events));
    }

    track_tool_results(execution, resources.working_dir, &executed_remaining);
    execution
        .messages
        .extend(std::mem::take(&mut executed_remaining.tool_messages));

    if matches!(executed_remaining.outcome, ToolCycleOutcome::Interrupted) {
        return Ok(ToolExecutionDisposition::Interrupted);
    }

    Ok(ToolExecutionDisposition::Completed)
}

fn apply_streaming_stats(execution: &mut TurnExecutionContext, stats: StreamingToolStats) {
    execution.streaming_tools.launch_count = execution
        .streaming_tools
        .launch_count
        .saturating_add(stats.launched_count);
    execution.streaming_tools.match_count = execution
        .streaming_tools
        .match_count
        .saturating_add(stats.matched_count);
    execution.streaming_tools.fallback_count = execution
        .streaming_tools
        .fallback_count
        .saturating_add(stats.fallback_count);
    execution.streaming_tools.discard_count = execution
        .streaming_tools
        .discard_count
        .saturating_add(stats.discard_count);
    execution.streaming_tools.overlap_ms = execution
        .streaming_tools
        .overlap_ms
        .saturating_add(stats.overlap_ms);
}

fn empty_tool_cycle_result() -> ToolCycleResult {
    ToolCycleResult {
        outcome: ToolCycleOutcome::Completed,
        tool_messages: Vec::new(),
        raw_results: Vec::new(),
        events: Vec::new(),
    }
}

fn merge_buffered_and_remaining_tool_results(
    execution: &mut TurnExecutionContext,
    output: &LlmOutput,
    matched_results: &HashMap<String, crate::turn::tool_cycle::BufferedToolExecution>,
    executed_remaining: &mut ToolCycleResult,
) -> Result<()> {
    let mut combined_events = Vec::new();
    let mut remaining_results = executed_remaining
        .raw_results
        .iter()
        .cloned()
        .map(|(call, result)| (call.id.clone(), (call, result)))
        .collect::<HashMap<_, _>>();
    let (mut remaining_event_groups, remaining_event_order, mut ungrouped_events) =
        group_events_by_tool_call_id(std::mem::take(&mut executed_remaining.events));
    let mut merged_raw_results = Vec::with_capacity(output.tool_calls.len());
    let mut merged_tool_messages = Vec::with_capacity(output.tool_calls.len());
    let mut dropped_tool_call_ids = Vec::new();

    for call in &output.tool_calls {
        if let Some(buffered) = matched_results.get(&call.id) {
            combined_events.extend(buffered.events.iter().cloned());
            merged_tool_messages.push(LlmMessage::Tool {
                tool_call_id: buffered.result.tool_call_id.clone(),
                content: buffered.result.model_content(),
            });
            merged_raw_results.push((call.clone(), buffered.result.clone()));
            continue;
        }
        if let Some((remaining_call, result)) = remaining_results.remove(&call.id) {
            if let Some(events) = remaining_event_groups.remove(&call.id) {
                combined_events.extend(events);
            }
            merged_tool_messages.push(LlmMessage::Tool {
                tool_call_id: result.tool_call_id.clone(),
                content: result.model_content(),
            });
            merged_raw_results.push((remaining_call, result));
            continue;
        }

        dropped_tool_call_ids.push(call.id.clone());
    }

    debug_assert_eq!(
        merged_tool_messages.len(),
        output.tool_calls.len(),
        "merge dropped tool calls: expected {} results, got {}",
        output.tool_calls.len(),
        merged_tool_messages.len()
    );
    if !dropped_tool_call_ids.is_empty() {
        return Err(AstrError::Internal(format!(
            "buffered tool merge dropped results for tool calls: {}",
            dropped_tool_call_ids.join(", ")
        )));
    }

    for call_id in remaining_event_order {
        if let Some(events) = remaining_event_groups.remove(&call_id) {
            combined_events.extend(events);
        }
    }
    combined_events.append(&mut ungrouped_events);
    execution.journal.extend(combined_events);
    executed_remaining.tool_messages = merged_tool_messages;
    executed_remaining.raw_results = merged_raw_results;
    Ok(())
}

fn group_events_by_tool_call_id(
    events: Vec<StorageEvent>,
) -> (
    HashMap<String, Vec<StorageEvent>>,
    Vec<String>,
    Vec<StorageEvent>,
) {
    let mut grouped = HashMap::<String, Vec<StorageEvent>>::new();
    let mut order = Vec::new();
    let mut ungrouped = Vec::new();

    for event in events {
        let Some(tool_call_id) = event_tool_call_id(&event) else {
            ungrouped.push(event);
            continue;
        };
        if !grouped.contains_key(tool_call_id) {
            order.push(tool_call_id.to_string());
        }
        grouped
            .entry(tool_call_id.to_string())
            .or_default()
            .push(event);
    }

    (grouped, order, ungrouped)
}

fn event_tool_call_id(event: &StorageEvent) -> Option<&str> {
    match &event.payload {
        StorageEventPayload::ToolCall { tool_call_id, .. }
        | StorageEventPayload::ToolResult { tool_call_id, .. }
        | StorageEventPayload::ToolResultReferenceApplied { tool_call_id, .. } => {
            Some(tool_call_id.as_str())
        },
        _ => None,
    }
}

fn track_tool_results(
    execution: &mut TurnExecutionContext,
    working_dir: &str,
    tool_result: &ToolCycleResult,
) {
    for (call, result) in &tool_result.raw_results {
        execution.budget.file_access_tracker.record_tool_result(
            call,
            result,
            Path::new(working_dir),
        );
        execution
            .budget
            .micro_compact_state
            .record_tool_result(result.tool_call_id.clone(), Instant::now());
    }
}
