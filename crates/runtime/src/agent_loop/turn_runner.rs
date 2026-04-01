use astrcode_core::{CancelToken, Result};
use std::collections::HashMap;

use crate::prompt::{append_unique_tools, DiagnosticLevel, PromptContext, PromptDiagnostics};
use astrcode_core::AgentState;
use astrcode_core::LlmMessage;
use astrcode_core::ModelRequest;
use astrcode_core::StorageEvent;

use super::{
    finish_interrupted, finish_turn, finish_with_error, internal_error, llm_cycle, tool_cycle,
    AgentLoop,
};

/// 执行一个完整的 agent turn（从用户提示到最终响应）。
///
/// ## Turn 内部的 step 循环
///
/// 一个 turn 可能包含多个 step（LLM 调用 → 工具执行 → 再调用 LLM → ...），
/// 直到 LLM 不再请求工具调用为止。每个 step 的流程：
///
/// ```text
/// 1. compose prompt  →  组装系统提示词 + 历史消息
/// 2. call LLM        →  发送到 provider，流式接收 delta
/// 3. process result   →  如果有 tool_calls → 执行工具 → 回到步骤 1
///                       如果没有 tool_calls → turn 结束
/// ```
///
/// ## 终止条件
///
/// - `max_steps` 达到上限（防止无限循环）
/// - LLM 返回纯文本（无工具调用）
/// - 取消信号触发
/// - 任何步骤返回错误
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

        let mut vars = HashMap::new();
        if let Some(latest_user_message) = latest_user_message(&messages) {
            vars.insert(
                "turn.user_message".to_string(),
                latest_user_message.to_string(),
            );
        }
        let ctx = PromptContext {
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            tool_names: agent_loop.capabilities.tool_names().to_vec(),
            capability_descriptors: agent_loop.prompt_capability_descriptors.clone(),
            prompt_declarations: agent_loop.prompt_declarations.clone(),
            skills: agent_loop.prompt_skills.clone(),
            step_index,
            turn_index: state.turn_count,
            vars,
        };
        let build_output = match agent_loop.prompt_composer.build(&ctx).await {
            Ok(output) => output,
            Err(error) => {
                finish_with_error(turn_id, error.to_string(), on_event)?;
                return Ok(());
            }
        };
        log_prompt_diagnostics(&build_output.diagnostics);
        let plan = build_output.plan;
        let system_prompt = plan.render_system();
        let mut request_messages = plan.prepend_messages;
        request_messages.extend(messages.iter().cloned());
        request_messages.extend(plan.append_messages);
        let mut tool_definitions = agent_loop.capabilities.tool_definitions();
        append_unique_tools(&mut tool_definitions, plan.extra_tools);
        let policy_ctx = agent_loop.policy_context(state, turn_id, step_index);
        let request = ModelRequest {
            messages: request_messages,
            tools: tool_definitions,
            system_prompt,
        };
        let request = match agent_loop
            .policy
            .check_model_request(request, &policy_ctx)
            .await
        {
            Ok(request) => request,
            Err(error) => {
                finish_with_error(turn_id, error.to_string(), on_event)?;
                return Ok(());
            }
        };

        let output = match llm_cycle::generate_response(
            &provider,
            request,
            turn_id,
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

        if !output.content.is_empty() || !output.tool_calls.is_empty() || output.reasoning.is_some()
        {
            on_event(StorageEvent::AssistantFinal {
                turn_id: Some(turn_id.to_string()),
                content: output.content.clone(),
                reasoning_content: output.reasoning.as_ref().map(|value| value.content.clone()),
                reasoning_signature: output
                    .reasoning
                    .as_ref()
                    .and_then(|value| value.signature.clone()),
                timestamp: Some(chrono::Utc::now()),
            })?;
        }

        let tool_calls = output.tool_calls.clone();
        messages.push(LlmMessage::Assistant {
            content: output.content,
            tool_calls: output.tool_calls,
            reasoning: output.reasoning,
        });

        if tool_calls.is_empty() {
            finish_turn(turn_id, on_event)?;
            return Ok(());
        }

        if matches!(
            tool_cycle::execute_tool_calls(
                agent_loop,
                &agent_loop.capabilities,
                tool_calls,
                turn_id,
                state,
                step_index,
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

fn latest_user_message(messages: &[LlmMessage]) -> Option<&str> {
    messages.iter().rev().find_map(|message| match message {
        LlmMessage::User { content } => Some(content.as_str()),
        LlmMessage::Assistant { .. } | LlmMessage::Tool { .. } => None,
    })
}

fn reached_max_steps(max_steps: Option<usize>, step_index: usize) -> bool {
    let Some(max_steps) = max_steps else {
        return false;
    };

    if step_index >= max_steps {
        log::warn!(
            "[agent_loop] reached max tool iteration steps ({}), finishing turn gracefully",
            max_steps
        );
        true
    } else {
        false
    }
}

fn log_prompt_diagnostics(diagnostics: &PromptDiagnostics) {
    for diagnostic in &diagnostics.items {
        let block_id = diagnostic.block_id.as_deref().unwrap_or("-");
        let contributor_id = diagnostic.contributor_id.as_deref().unwrap_or("-");
        let message = format!(
            "prompt diagnostic contributor={contributor_id} block={block_id} reason={:?} suggestion={}",
            diagnostic.reason,
            diagnostic.suggestion.as_deref().unwrap_or("-")
        );

        match diagnostic.level {
            DiagnosticLevel::Info => log::debug!("{message}"),
            DiagnosticLevel::Warning => log::warn!("{message}"),
            DiagnosticLevel::Error => log::error!("{message}"),
        }
    }
}
