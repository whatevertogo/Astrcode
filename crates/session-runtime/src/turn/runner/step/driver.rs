use std::{path::Path, sync::Arc};

use astrcode_core::{LlmOutput, LlmRequest, Result, ToolCallRequest};
use async_trait::async_trait;

use super::{TurnExecutionContext, TurnExecutionResources};
use crate::turn::{
    compaction_cycle::{self, ReactiveCompactContext},
    llm_cycle::ToolCallDeltaSink,
    request::{AssemblePromptRequest, AssemblePromptResult, assemble_prompt_request},
    tool_cycle::{self, ToolCycleContext, ToolCycleResult, ToolEventEmissionMode},
};

pub(super) struct RuntimeStepDriver;

#[async_trait]
pub(super) trait StepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult>;

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
        tool_delta_sink: Option<ToolCallDeltaSink>,
    ) -> Result<LlmOutput>;

    async fn try_reactive_compact(
        &self,
        execution: &TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<Option<compaction_cycle::RecoveryResult>>;

    async fn execute_tool_cycle(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
        tool_calls: Vec<ToolCallRequest>,
        event_emission_mode: ToolEventEmissionMode,
    ) -> Result<ToolCycleResult>;
}

#[async_trait]
impl StepDriver for RuntimeStepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult> {
        let mut assembled = assemble_prompt_request(AssemblePromptRequest {
            gateway: resources.gateway,
            prompt_facts_provider: resources.prompt_facts_provider,
            session_id: resources.session_id,
            turn_id: resources.turn_id,
            working_dir: Path::new(resources.working_dir),
            messages: std::mem::take(&mut execution.messages),
            cancel: resources.cancel.clone(),
            agent: resources.agent,
            step_index: execution.step_index,
            token_tracker: &execution.token_tracker,
            tools: Arc::clone(&resources.tools),
            settings: &resources.settings,
            clearable_tools: &resources.clearable_tools,
            micro_compact_state: &mut execution.micro_compact_state,
            file_access_tracker: &execution.file_access_tracker,
            session_state: resources.session_state,
            tool_result_replacement_state: &mut execution.tool_result_replacement_state,
            prompt_declarations: resources.prompt_declarations,
            prompt_governance: resources.prompt_governance,
        })
        .await?;
        execution.messages = std::mem::take(&mut assembled.messages);
        if assembled.auto_compacted {
            execution.auto_compaction_count = execution.auto_compaction_count.saturating_add(1);
        }
        execution.tool_result_replacement_count = execution
            .tool_result_replacement_count
            .saturating_add(assembled.tool_result_budget_stats.replacement_count);
        execution.tool_result_reapply_count = execution
            .tool_result_reapply_count
            .saturating_add(assembled.tool_result_budget_stats.reapply_count);
        execution.tool_result_bytes_saved = execution
            .tool_result_bytes_saved
            .saturating_add(assembled.tool_result_budget_stats.bytes_saved);
        execution.tool_result_over_budget_message_count = execution
            .tool_result_over_budget_message_count
            .saturating_add(assembled.tool_result_budget_stats.over_budget_message_count);
        execution.events.extend(assembled.events.iter().cloned());
        Ok(assembled)
    }

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
        tool_delta_sink: Option<ToolCallDeltaSink>,
    ) -> Result<LlmOutput> {
        crate::turn::llm_cycle::call_llm_streaming(
            resources.gateway,
            llm_request,
            resources.turn_id,
            resources.agent,
            resources.session_state,
            resources.cancel,
            tool_delta_sink,
        )
        .await
    }

    async fn try_reactive_compact(
        &self,
        execution: &TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<Option<compaction_cycle::RecoveryResult>> {
        compaction_cycle::try_reactive_compact(&ReactiveCompactContext {
            gateway: resources.gateway,
            prompt_facts_provider: resources.prompt_facts_provider,
            messages: &execution.messages,
            session_id: resources.session_id,
            working_dir: resources.working_dir,
            turn_id: resources.turn_id,
            step_index: execution.step_index,
            agent: resources.agent,
            cancel: resources.cancel.clone(),
            settings: &resources.settings,
            file_access_tracker: &execution.file_access_tracker,
        })
        .await
    }

    async fn execute_tool_cycle(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
        tool_calls: Vec<ToolCallRequest>,
        event_emission_mode: ToolEventEmissionMode,
    ) -> Result<ToolCycleResult> {
        tool_cycle::execute_tool_calls(
            &mut ToolCycleContext {
                gateway: resources.gateway,
                session_state: resources.session_state,
                session_id: resources.session_id,
                working_dir: resources.working_dir,
                turn_id: resources.turn_id,
                agent: resources.agent,
                cancel: resources.cancel,
                events: &mut execution.events,
                max_concurrency: resources.runtime.max_tool_concurrency,
                tool_result_inline_limit: resources.runtime.tool_result_inline_limit,
                event_emission_mode,
            },
            tool_calls,
        )
        .await
    }
}
