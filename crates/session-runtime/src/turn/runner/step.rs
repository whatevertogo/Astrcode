use std::{path::Path, time::Instant};

use astrcode_core::{LlmFinishReason, LlmMessage, LlmOutput, LlmRequest, Result, ToolCallRequest};
use async_trait::async_trait;
use chrono::Utc;

use super::{TurnExecutionContext, TurnExecutionResources};
use crate::turn::{
    compaction_cycle::{self, ReactiveCompactContext},
    events::{assistant_final_event, turn_done_event},
    llm_cycle,
    request::{AssemblePromptRequest, AssemblePromptResult, assemble_prompt_request},
    tool_cycle::{self, ToolCycleContext, ToolCycleOutcome, ToolCycleResult},
};

struct RuntimeStepDriver;

/// 单步执行的结果，决定 turn 主循环的后续走向。
pub(super) enum StepOutcome {
    /// 有工具调用，继续下一个 step。
    Continue,
    /// LLM 无工具调用，turn 自然结束。
    Completed,
    /// 取消信号或工具中断。
    Cancelled,
}

/// 抽象单步执行的各个阶段，方便测试时注入 mock 替代真实 LLM / 工具调用。
#[async_trait]
trait StepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult>;

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
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
    ) -> Result<ToolCycleResult>;
}

/// 使用真实运行时 driver 执行一个 step。
pub(super) async fn run_single_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
) -> Result<StepOutcome> {
    run_single_step_with(execution, resources, &RuntimeStepDriver).await
}

/// 单步编排：assemble → LLM → 工具 → 决定下一步走向。
/// 可注入 driver 以便测试时替换真实 LLM / 工具调用。
async fn run_single_step_with(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
) -> Result<StepOutcome> {
    let assembled = driver.assemble_prompt(execution, resources).await?;
    let Some(output) =
        call_llm_for_step(execution, resources, driver, assembled.llm_request).await?
    else {
        return Ok(StepOutcome::Continue);
    };

    record_llm_usage(execution, &output);
    let has_tool_calls = append_assistant_output(execution, resources, &output);
    warn_if_output_truncated(resources, execution, &output);

    if !has_tool_calls {
        append_turn_done_event(execution, resources);
        return Ok(StepOutcome::Completed);
    }

    let tool_result = driver
        .execute_tool_cycle(execution, resources, output.tool_calls)
        .await?;
    track_tool_results(execution, resources.working_dir, &tool_result);
    execution.messages.extend(tool_result.tool_messages);

    if matches!(tool_result.outcome, ToolCycleOutcome::Interrupted) {
        return Ok(StepOutcome::Cancelled);
    }

    execution.step_index += 1;
    Ok(StepOutcome::Continue)
}

/// 调用 LLM，遇到 prompt too long 时尝试 reactive compact 恢复。
/// 恢复成功返回 `Ok(None)`（消息已更新，主循环应 continue 重新组装请求）。
async fn call_llm_for_step(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    driver: &impl StepDriver,
    llm_request: LlmRequest,
) -> Result<Option<LlmOutput>> {
    match driver.call_llm(resources, llm_request).await {
        Ok(output) => Ok(Some(output)),
        Err(error) => {
            if error.is_cancelled() {
                return Err(error);
            }
            if error.is_prompt_too_long()
                && execution.reactive_compact_attempts
                    < compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS
            {
                execution.reactive_compact_attempts += 1;
                log::warn!(
                    "turn {} step {}: prompt too long, reactive compact ({}/{})",
                    resources.turn_id,
                    execution.step_index,
                    execution.reactive_compact_attempts,
                    compaction_cycle::MAX_REACTIVE_COMPACT_ATTEMPTS,
                );

                let recovery = driver.try_reactive_compact(execution, resources).await?;

                if let Some(result) = recovery {
                    execution.events.extend(result.events);
                    execution.messages = result.messages;
                    return Ok(None);
                }
            }
            Err(error)
        },
    }
}

fn record_llm_usage(execution: &mut TurnExecutionContext, output: &LlmOutput) {
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

/// 将 LLM 输出追加到 messages 和 events，返回是否包含工具调用。
fn append_assistant_output(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
    output: &LlmOutput,
) -> bool {
    let content = output.content.trim().to_string();
    let has_tool_calls = !output.tool_calls.is_empty();
    execution.messages.push(LlmMessage::Assistant {
        content: content.clone(),
        tool_calls: output.tool_calls.clone(),
        reasoning: output.reasoning.clone(),
    });
    execution.events.push(assistant_final_event(
        resources.turn_id,
        resources.agent,
        content,
        output
            .reasoning
            .as_ref()
            .map(|reasoning| reasoning.content.clone()),
        output
            .reasoning
            .as_ref()
            .and_then(|reasoning| reasoning.signature.clone()),
        Some(Utc::now()),
    ));
    has_tool_calls
}

/// 追加 TurnDone 事件（仅在 LLM 无工具调用、turn 自然结束时）。
fn append_turn_done_event(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources<'_>,
) {
    execution.events.push(turn_done_event(
        resources.turn_id,
        resources.agent,
        None,
        Utc::now(),
    ));
}

/// max_tokens 截断只记 warning，不改变流程（下一轮 prompt 预算仍会正确估算）。
fn warn_if_output_truncated(
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

/// 双重追踪：file_access_tracker（prune pass 用）+ micro_compact_state（idle 清理用）。
fn track_tool_results(
    execution: &mut TurnExecutionContext,
    working_dir: &str,
    tool_result: &ToolCycleResult,
) {
    for (call, result) in &tool_result.raw_results {
        execution
            .file_access_tracker
            .record_tool_result(call, result, Path::new(working_dir));
        execution
            .micro_compact_state
            .record_tool_result(result.tool_call_id.clone(), Instant::now());
    }
}

#[async_trait]
impl StepDriver for RuntimeStepDriver {
    async fn assemble_prompt(
        &self,
        execution: &mut TurnExecutionContext,
        resources: &TurnExecutionResources<'_>,
    ) -> Result<AssemblePromptResult> {
        let assembled = assemble_prompt_request(AssemblePromptRequest {
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
            tools: resources.tools.clone(),
            settings: &resources.settings,
            clearable_tools: &resources.clearable_tools,
            micro_compact_state: &mut execution.micro_compact_state,
            file_access_tracker: &execution.file_access_tracker,
        })
        .await?;
        execution.messages = assembled.messages.clone();
        execution.events.extend(assembled.events.iter().cloned());
        Ok(assembled)
    }

    async fn call_llm(
        &self,
        resources: &TurnExecutionResources<'_>,
        llm_request: LlmRequest,
    ) -> Result<LlmOutput> {
        llm_cycle::call_llm_streaming(
            resources.gateway,
            llm_request,
            resources.turn_id,
            resources.agent,
            resources.session_state,
            resources.cancel,
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
            },
            tool_calls,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use astrcode_core::{
        AgentEventContext, AstrError, CancelToken, LlmMessage, LlmUsage, PromptFactsProvider,
        ResolvedRuntimeConfig, StorageEventPayload, ToolDefinition, UserMessageOrigin,
    };
    use astrcode_kernel::KernelGateway;
    use serde_json::json;

    use super::*;
    use crate::{
        SessionState,
        context_window::token_usage::PromptTokenSnapshot,
        turn::{
            events::prompt_metrics_event,
            runner::TurnExecutionRequestView,
            test_support::{
                NoopPromptFactsProvider, assert_contains_compact_summary,
                assert_has_assistant_final, assert_has_turn_done, root_compact_applied_event,
                test_gateway, test_session_state,
            },
        },
    };

    #[derive(Default)]
    struct DriverCallCounts {
        assemble: AtomicUsize,
        llm: AtomicUsize,
        reactive_compact: AtomicUsize,
        tool_cycle: AtomicUsize,
    }

    struct ScriptedStepDriver {
        counts: DriverCallCounts,
        assemble_result: Mutex<Option<Result<AssemblePromptResult>>>,
        llm_result: Mutex<Option<Result<LlmOutput>>>,
        reactive_compact_result: Mutex<Option<Result<Option<compaction_cycle::RecoveryResult>>>>,
        tool_cycle_result: Mutex<Option<Result<ToolCycleResult>>>,
    }

    #[async_trait]
    impl StepDriver for ScriptedStepDriver {
        async fn assemble_prompt(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> Result<AssemblePromptResult> {
            self.counts.assemble.fetch_add(1, Ordering::SeqCst);
            self.assemble_result
                .lock()
                .expect("assemble result lock should work")
                .take()
                .expect("assemble result should be scripted")
        }

        async fn call_llm(
            &self,
            _resources: &TurnExecutionResources<'_>,
            _llm_request: LlmRequest,
        ) -> Result<LlmOutput> {
            self.counts.llm.fetch_add(1, Ordering::SeqCst);
            self.llm_result
                .lock()
                .expect("llm result lock should work")
                .take()
                .expect("llm result should be scripted")
        }

        async fn try_reactive_compact(
            &self,
            _execution: &TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
        ) -> Result<Option<compaction_cycle::RecoveryResult>> {
            self.counts.reactive_compact.fetch_add(1, Ordering::SeqCst);
            self.reactive_compact_result
                .lock()
                .expect("reactive compact result lock should work")
                .take()
                .expect("reactive compact result should be scripted")
        }

        async fn execute_tool_cycle(
            &self,
            _execution: &mut TurnExecutionContext,
            _resources: &TurnExecutionResources<'_>,
            _tool_calls: Vec<ToolCallRequest>,
        ) -> Result<ToolCycleResult> {
            self.counts.tool_cycle.fetch_add(1, Ordering::SeqCst);
            self.tool_cycle_result
                .lock()
                .expect("tool cycle result lock should work")
                .take()
                .expect("tool cycle result should be scripted")
        }
    }

    fn user_message(content: &str) -> LlmMessage {
        LlmMessage::User {
            content: content.to_string(),
            origin: UserMessageOrigin::User,
        }
    }

    fn assembled_prompt(messages: Vec<LlmMessage>) -> AssemblePromptResult {
        AssemblePromptResult {
            llm_request: LlmRequest::new(
                messages.clone(),
                vec![ToolDefinition {
                    name: "dummy_tool".to_string(),
                    description: "dummy".to_string(),
                    parameters: json!({"type": "object"}),
                }],
                CancelToken::new(),
            )
            .with_system("system"),
            messages,
            events: vec![prompt_metrics_event(
                "turn-1",
                &AgentEventContext::default(),
                0,
                PromptTokenSnapshot {
                    context_tokens: 10,
                    budget_tokens: 10,
                    context_window: 100,
                    effective_window: 90,
                    threshold_tokens: 80,
                },
                0,
            )],
        }
    }

    fn test_resources<'a>(
        gateway: &'a KernelGateway,
        session_state: &'a Arc<SessionState>,
        runtime: &'a ResolvedRuntimeConfig,
        cancel: &'a CancelToken,
        agent: &'a AgentEventContext,
        prompt_facts_provider: &'a dyn PromptFactsProvider,
    ) -> TurnExecutionResources<'a> {
        TurnExecutionResources::new(
            gateway,
            TurnExecutionRequestView {
                prompt_facts_provider,
                session_id: "session-1",
                working_dir: ".",
                turn_id: "turn-1",
                session_state,
                runtime,
                cancel,
                agent,
            },
        )
    }

    #[tokio::test]
    async fn run_single_step_returns_completed_when_llm_has_no_tool_calls() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")]);
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: "assistant reply".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: Some(LlmUsage {
                    input_tokens: 11,
                    output_tokens: 7,
                    cache_creation_input_tokens: 3,
                    cache_read_input_tokens: 2,
                }),
                finish_reason: LlmFinishReason::Stop,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should succeed");

        assert!(matches!(outcome, StepOutcome::Completed));
        assert_eq!(execution.step_index, 0);
        assert_eq!(execution.total_cache_read_tokens, 2);
        assert_eq!(execution.total_cache_creation_tokens, 3);
        assert_eq!(driver.counts.assemble.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.llm.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 0);
        assert!(matches!(
            execution.messages.last(),
            Some(LlmMessage::Assistant { content, .. }) if content == "assistant reply"
        ));
        assert_has_turn_done(&execution.events);
    }

    #[tokio::test]
    async fn run_single_step_returns_cancelled_when_tool_cycle_interrupts() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let mut execution =
            TurnExecutionContext::new(&resources, vec![user_message("hello from user")]);
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message("hello")])))),
            llm_result: Mutex::new(Some(Ok(LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "dummy_tool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::ToolCalls,
            }))),
            reactive_compact_result: Mutex::new(None),
            tool_cycle_result: Mutex::new(Some(Ok(ToolCycleResult {
                outcome: ToolCycleOutcome::Interrupted,
                tool_messages: Vec::new(),
                raw_results: Vec::new(),
            }))),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should succeed");

        assert!(matches!(outcome, StepOutcome::Cancelled));
        assert_eq!(execution.step_index, 0);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 1);
        assert_has_assistant_final(&execution.events);
    }

    #[tokio::test]
    async fn run_single_step_returns_continue_after_reactive_compact_recovery() {
        let gateway = test_gateway(8192);
        let session_state = test_session_state();
        let runtime = ResolvedRuntimeConfig::default();
        let cancel = CancelToken::new();
        let agent = AgentEventContext::default();
        let prompt_facts_provider = NoopPromptFactsProvider;
        let resources = test_resources(
            &gateway,
            &session_state,
            &runtime,
            &cancel,
            &agent,
            &prompt_facts_provider,
        );
        let original_messages = vec![user_message("message before compact")];
        let mut execution = TurnExecutionContext::new(&resources, original_messages);
        let recovered_messages = vec![
            user_message("compacted summary"),
            LlmMessage::Assistant {
                content: "recovered context".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];
        let driver = ScriptedStepDriver {
            counts: DriverCallCounts::default(),
            assemble_result: Mutex::new(Some(Ok(assembled_prompt(vec![user_message(
                "message before compact",
            )])))),
            llm_result: Mutex::new(Some(Err(AstrError::LlmRequestFailed {
                status: 400,
                body: "prompt too long for provider".to_string(),
            }))),
            reactive_compact_result: Mutex::new(Some(Ok(Some(compaction_cycle::RecoveryResult {
                messages: recovered_messages.clone(),
                events: vec![root_compact_applied_event(
                    "turn-1",
                    "compacted",
                    1,
                    100,
                    60,
                    2,
                    40,
                )],
            })))),
            tool_cycle_result: Mutex::new(None),
        };

        let outcome = run_single_step_with(&mut execution, &resources, &driver)
            .await
            .expect("step should recover via reactive compact");

        assert!(matches!(outcome, StepOutcome::Continue));
        assert_eq!(execution.step_index, 0);
        assert_eq!(execution.reactive_compact_attempts, 1);
        assert_eq!(driver.counts.llm.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.reactive_compact.load(Ordering::SeqCst), 1);
        assert_eq!(driver.counts.tool_cycle.load(Ordering::SeqCst), 0);
        assert_eq!(execution.messages, recovered_messages);
        let stored_like = execution
            .events
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, event)| astrcode_core::StoredEvent {
                storage_seq: index as u64 + 1,
                event,
            })
            .collect::<Vec<_>>();
        assert_contains_compact_summary(&stored_like, "compacted");
        assert!(
            execution
                .events
                .iter()
                .all(|event| !matches!(&event.payload, StorageEventPayload::AssistantFinal { .. })),
            "recovery path should continue without persisting a failed assistant reply"
        );
    }
}
