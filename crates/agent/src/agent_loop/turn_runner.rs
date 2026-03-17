use astrcode_core::{CancelToken, Result};

use crate::events::StorageEvent;
use crate::projection::AgentState;
use crate::prompt::{append_unique_tools, PromptComposer, PromptContext};
use astrcode_core::LlmMessage;

use super::{
    finish_interrupted, finish_turn, finish_with_error, internal_error, llm_cycle, tool_cycle,
    AgentLoop,
};

pub(crate) async fn run_turn(
    agent_loop: &AgentLoop,
    state: &AgentState,
    turn_id: &str,
    on_event: &mut impl FnMut(StorageEvent) -> Result<()>,
    cancel: CancelToken,
) -> Result<()> {
    let provider = llm_cycle::build_provider(agent_loop.factory.clone())
        .await
        .map_err(internal_error)?;
    let mut messages = state.messages.clone();
    let mut step_index = 0usize;

    loop {
        if reached_max_steps(agent_loop.max_steps, step_index) {
            finish_turn(turn_id, on_event)?;
            return Ok(());
        }

        if cancel.is_cancelled() {
            finish_interrupted(turn_id, on_event)?;
            return Ok(());
        }

        let ctx = PromptContext {
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            tool_names: agent_loop.tools.names(),
            step_index,
            turn_index: state.turn_count,
        };
        let plan = PromptComposer::with_defaults().build(&ctx);
        let system_prompt = plan.render_system();
        let mut request_messages = plan.prepend_messages;
        request_messages.extend(messages.iter().cloned());
        request_messages.extend(plan.append_messages);
        let mut tool_definitions = agent_loop.tools.definitions();
        append_unique_tools(&mut tool_definitions, plan.extra_tools);

        let output = match llm_cycle::generate_response(
            &provider,
            &request_messages,
            tool_definitions,
            turn_id,
            system_prompt,
            cancel.clone(),
            on_event,
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                if cancel.is_cancelled() {
                    finish_interrupted(turn_id, on_event)?;
                } else {
                    finish_with_error(turn_id, error.to_string(), on_event)?;
                }
                return Ok(());
            }
        };

        if !output.content.is_empty() || !output.tool_calls.is_empty() {
            on_event(StorageEvent::AssistantFinal {
                turn_id: Some(turn_id.to_string()),
                content: output.content.clone(),
                timestamp: Some(chrono::Utc::now()),
            })?;
        }

        let tool_calls = output.tool_calls.clone();
        messages.push(LlmMessage::Assistant {
            content: output.content,
            tool_calls: output.tool_calls,
        });

        if tool_calls.is_empty() {
            finish_turn(turn_id, on_event)?;
            return Ok(());
        }

        if matches!(
            tool_cycle::execute_tool_calls(
                agent_loop,
                &agent_loop.tools,
                tool_calls,
                turn_id,
                state,
                &mut messages,
                on_event,
                &cancel,
            )
            .await
            .map_err(internal_error)?,
            tool_cycle::ToolCycleOutcome::Interrupted
        ) {
            finish_interrupted(turn_id, on_event)?;
            return Ok(());
        }

        step_index += 1;
    }
}

fn reached_max_steps(max_steps: Option<usize>, step_index: usize) -> bool {
    let Some(max_steps) = max_steps else {
        return false;
    };

    if step_index >= max_steps {
        eprintln!(
            "[agent_loop] reached max tool iteration steps ({}), finishing turn gracefully",
            max_steps
        );
        true
    } else {
        false
    }
}
