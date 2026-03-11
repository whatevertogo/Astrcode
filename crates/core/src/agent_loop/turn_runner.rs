use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::action::{
    build_history, rebuild_reasoning_cache_from_history, HistoryEntry, LlmMessage,
    MessageMetadata,
};
use crate::events::StorageEvent;
use crate::projection::AgentState;
use crate::prompt::{append_unique_tools, PromptComposer, PromptContext};

use super::{finish_interrupted, finish_turn, finish_with_error, llm_cycle, tool_cycle, AgentLoop};

pub(crate) async fn run_turn(
    agent_loop: &AgentLoop,
    state: &AgentState,
    on_event: &mut impl FnMut(StorageEvent),
    cancel: CancellationToken,
) -> Result<()> {
    let provider = llm_cycle::build_provider(agent_loop.factory.clone()).await?;
    let mut messages = state.messages.clone();
    let mut local_reasoning_cache = agent_loop.reasoning_cache_snapshot();
    let mut history = build_history(&messages, &local_reasoning_cache);
    let mut step_index = 0usize;

    loop {
        if reached_max_steps(agent_loop.max_steps, step_index) {
            finish_turn(on_event);
            return Ok(());
        }

        if cancel.is_cancelled() {
            finish_interrupted(on_event);
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
        let mut request_messages: Vec<HistoryEntry> = plan
            .prepend_messages
            .into_iter()
            .map(HistoryEntry::plain)
            .collect();
        request_messages.extend(history.iter().cloned());
        request_messages.extend(plan.append_messages.into_iter().map(HistoryEntry::plain));
        let mut tool_definitions = agent_loop.tools.definitions();
        append_unique_tools(&mut tool_definitions, plan.extra_tools);

        let output = match llm_cycle::generate_response(
            &provider,
            &request_messages,
            tool_definitions,
            system_prompt,
            cancel.child_token(),
            on_event,
            agent_loop.transient_llm_sink(),
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                if cancel.is_cancelled() {
                    finish_interrupted(on_event);
                } else {
                    finish_with_error(error.to_string(), on_event);
                }
                return Ok(());
            }
        };

        if !output.content.is_empty() || !output.tool_calls.is_empty() {
            on_event(StorageEvent::AssistantFinal {
                content: output.content.clone(),
            });
        }

        let tool_calls = output.tool_calls.clone();
        let assistant_message = LlmMessage::Assistant {
            content: output.content,
            tool_calls: output.tool_calls,
        };
        messages.push(assistant_message.clone());
        history.push(HistoryEntry {
            message: assistant_message,
            metadata: MessageMetadata {
                reasoning_content: output.reasoning_content,
            },
        });
        local_reasoning_cache = rebuild_reasoning_cache_from_history(&history);
        agent_loop.replace_reasoning_cache(local_reasoning_cache.clone());

        if tool_calls.is_empty() {
            finish_turn(on_event);
            return Ok(());
        }

        if matches!(
            tool_cycle::execute_tool_calls(
                &agent_loop.tools,
                tool_calls,
                &mut messages,
                &mut history,
                on_event,
                &cancel,
            )
            .await,
            tool_cycle::ToolCycleOutcome::Interrupted
        ) {
            finish_interrupted(on_event);
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
