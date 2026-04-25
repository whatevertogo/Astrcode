use std::{sync::Arc, time::Instant};

use astrcode_context_window::{
    ContextWindowSettings,
    compaction::{
        CompactResult, auto_compact, build_post_compact_events,
        build_post_compact_recovery_messages, compact_config_from_settings,
    },
    prune_pass::apply_prune_pass,
    token_usage::{
        PromptTokenSnapshot, build_prompt_snapshot, estimate_request_tokens, should_compact,
    },
    tool_result_budget::{
        ApplyToolResultBudgetRequest, ToolResultBudgetStats, apply_tool_result_budget,
    },
};
use astrcode_core::{
    AgentEventContext, AstrError, CompactAppliedMeta, CompactMode, CompactTrigger, HookEventKey,
    LlmMessage, PromptMetricsPayload, StorageEvent, StorageEventPayload, UserMessageOrigin,
    format_compact_summary,
};
use astrcode_llm_contract::{LlmProvider, LlmRequest, LlmUsage, PromptCacheDiagnostics};
use astrcode_runtime_contract::{RuntimeTurnEvent, TurnIdentity};

use crate::{
    hook_dispatch::{HookDispatchRequest, HookEffect, HookEventPayload},
    r#loop::{TurnExecutionContext, TurnExecutionResources},
};

struct SessionBeforeCompactDecision {
    trigger: CompactTrigger,
    messages: Vec<LlmMessage>,
    provided_summary: Option<String>,
    cancel_reason: Option<String>,
}

/// 组装下一次 provider 请求。
///
/// 按 5 个阶段依次处理消息：
/// 1. **tool result budget** — 超预算的工具输出持久化到磁盘，替换为引用
/// 2. **micro compact** — 空闲间隙期间过期且可清除的工具结果被压缩
/// 3. **prune pass** — 超过大小限制的工具结果被截断，旧的可清除结果被清空
/// 4. **auto compact** — 若 token 估算超过阈值，调用 LLM 生成摘要压缩历史
/// 5. **prompt metrics** — 记录当前 token 快照供后续使用量回填
pub(crate) async fn assemble_runtime_request(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
) -> astrcode_core::Result<LlmRequest> {
    // 阶段 1：工具结果预算控制
    let budget_outcome = apply_tool_result_budget(ApplyToolResultBudgetRequest {
        messages: &execution.messages,
        session_id: &resources.session_id,
        working_dir: &resources.working_dir,
        replacement_state: &mut execution.tool_result_replacement_state,
        aggregate_budget_bytes: resources.settings.aggregate_result_bytes_budget,
        turn_id: &resources.turn_id,
        agent: &resources.agent,
    })?;
    execution
        .pending_events
        .extend(
            budget_outcome
                .events
                .into_iter()
                .map(|event| RuntimeTurnEvent::StorageEvent {
                    event: Box::new(event),
                }),
        );
    accumulate_tool_result_budget_stats(
        &mut execution.tool_result_budget_stats,
        budget_outcome.stats,
    );

    // 阶段 2：micro compact（基于空闲间隔的轻量压缩）
    let micro_outcome = execution.micro_compact_state.apply_if_idle(
        &budget_outcome.messages,
        &resources.clearable_tools,
        resources.settings.micro_compact_config(),
        Instant::now(),
    );

    // 阶段 3：prune pass（按大小截断 + 按年龄清空旧结果）
    let prune_outcome = apply_prune_pass(
        &micro_outcome.messages,
        &resources.clearable_tools,
        resources.settings.tool_result_max_bytes,
        resources.settings.compact_keep_recent_turns,
    );
    let mut messages = prune_outcome.messages;

    let Some(provider) = &resources.provider else {
        execution.messages = messages.clone();
        return Ok(LlmRequest::new(
            messages,
            Arc::clone(&resources.tools),
            resources.cancel.clone(),
        ));
    };

    // 阶段 4：auto compact（若 token 估算超过阈值，调用 LLM 生成摘要）
    let mut snapshot = build_prompt_snapshot(
        &execution.token_tracker,
        &messages,
        None,
        provider.model_limits(),
        resources.settings.compact_threshold_percent,
        resources.settings.summary_reserve_tokens,
        resources.settings.reserved_context_size,
    );

    if should_compact(snapshot) {
        if resources.settings.auto_compact_enabled {
            let compact_decision = dispatch_session_before_compact_hook(
                execution,
                resources,
                &messages,
                CompactTrigger::Auto,
            )
            .await?;
            if let Some(reason) = compact_decision.cancel_reason {
                log::info!(
                    "turn {} step {}: auto compact cancelled by hook: {}",
                    resources.turn_id,
                    execution.step_index,
                    reason
                );
            } else {
                let (compact_trigger, compaction) = compact_from_hook_decision(
                    provider.as_ref(),
                    compact_decision,
                    &resources.settings,
                    resources.events_history_path.clone(),
                    resources.cancel.clone(),
                )
                .await?;
                if let Some(compaction) = compaction {
                    messages = compaction.messages.clone();
                    // compact 后重新注入最近访问过的文件内容，恢复被压缩的文件上下文
                    messages.extend(build_post_compact_recovery_messages(
                        &execution.file_access_tracker,
                        &resources.settings,
                    ));
                    execution.pending_events.extend(build_post_compact_events(
                        Some(&resources.turn_id),
                        &resources.agent,
                        compact_trigger,
                        &compaction,
                    ));
                    execution.auto_compaction_count =
                        execution.auto_compaction_count.saturating_add(1);
                    snapshot = build_prompt_snapshot(
                        &execution.token_tracker,
                        &messages,
                        None,
                        provider.model_limits(),
                        resources.settings.compact_threshold_percent,
                        resources.settings.summary_reserve_tokens,
                        resources.settings.reserved_context_size,
                    );
                }
            }
        } else {
            log::warn!(
                "turn {} step {}: context tokens ({}) exceed threshold ({}) but auto compact is \
                 disabled",
                resources.turn_id,
                execution.step_index,
                snapshot.context_tokens,
                snapshot.threshold_tokens,
            );
        }
    }

    execution.pending_events.push(prompt_metrics_runtime_event(
        &resources.turn_id,
        &resources.agent,
        execution.step_index,
        snapshot,
        prune_outcome.stats.truncated_tool_results,
        provider.supports_cache_metrics(),
    ));
    execution.messages = messages.clone();

    Ok(LlmRequest::new(
        messages,
        Arc::clone(&resources.tools),
        resources.cancel.clone(),
    ))
}

/// "prompt too long" 错误后的响应式 compact。
/// 在 `run_single_step` 中 provider 返回上下文溢出错误时调用，
/// 尝试通过 compact 缩小消息后重试当前 step。
pub(crate) async fn recover_from_prompt_too_long(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    provider: &dyn LlmProvider,
) -> astrcode_core::Result<bool> {
    execution.reactive_compact_attempts = execution.reactive_compact_attempts.saturating_add(1);
    let messages = execution.messages.clone();
    let compact_decision =
        dispatch_session_before_compact_hook(execution, resources, &messages, CompactTrigger::Auto)
            .await?;
    if compact_decision.cancel_reason.is_some() {
        return Ok(false);
    }
    let (compact_trigger, compaction) = compact_from_hook_decision(
        provider,
        compact_decision,
        &resources.settings,
        resources.events_history_path.clone(),
        resources.cancel.clone(),
    )
    .await?;
    let Some(compaction) = compaction else {
        return Ok(false);
    };

    let mut messages = compaction.messages.clone();
    messages.extend(build_post_compact_recovery_messages(
        &execution.file_access_tracker,
        &resources.settings,
    ));
    execution.messages = messages;
    execution.pending_events.extend(build_post_compact_events(
        Some(&resources.turn_id),
        &resources.agent,
        compact_trigger,
        &compaction,
    ));
    Ok(true)
}

async fn dispatch_session_before_compact_hook(
    execution: &mut TurnExecutionContext,
    resources: &TurnExecutionResources,
    messages: &[LlmMessage],
    trigger: CompactTrigger,
) -> astrcode_core::Result<SessionBeforeCompactDecision> {
    let mut decision = SessionBeforeCompactDecision {
        trigger,
        messages: messages.to_vec(),
        provided_summary: None,
        cancel_reason: None,
    };
    let Some(dispatcher) = &resources.hook_dispatcher else {
        return Ok(decision);
    };

    let event = HookEventKey::SessionBeforeCompact;
    let outcome = dispatcher
        .dispatch_hook(HookDispatchRequest {
            snapshot_id: resources.hook_snapshot_id.clone(),
            event,
            session_id: resources.session_id.clone(),
            turn_id: resources.turn_id.clone(),
            agent_id: resources.agent_id.clone(),
            payload: HookEventPayload::SessionBeforeCompact {
                session_id: resources.session_id.clone(),
                reason: trigger,
                messages: messages.to_vec(),
                settings: compact_settings_payload(&resources.settings),
                current_mode: resources.current_mode.clone(),
            },
        })
        .await?;

    execution
        .pending_events
        .push(RuntimeTurnEvent::HookDispatched {
            identity: TurnIdentity::new(
                resources.session_id.clone(),
                resources.turn_id.clone(),
                resources.agent_id.clone(),
            ),
            event,
            effect_count: outcome.effects.len(),
        });

    for effect in outcome.effects {
        match effect {
            HookEffect::Continue | HookEffect::Diagnostic { .. } => {},
            HookEffect::CancelCompact { reason } => {
                decision.cancel_reason = Some(reason);
                return Ok(decision);
            },
            HookEffect::OverrideCompactInput { reason, messages } => {
                decision.trigger = reason;
                decision.messages = messages;
            },
            HookEffect::ProvideCompactSummary { summary } => {
                decision.provided_summary = Some(summary);
                return Ok(decision);
            },
            other => {
                return Err(AstrError::Validation(format!(
                    "session_before_compact hook returned unsupported effect '{}'",
                    hook_effect_name(&other)
                )));
            },
        }
    }

    Ok(decision)
}

async fn compact_from_hook_decision(
    provider: &dyn LlmProvider,
    decision: SessionBeforeCompactDecision,
    settings: &ContextWindowSettings,
    history_path: Option<String>,
    cancel: astrcode_core::CancelToken,
) -> astrcode_core::Result<(CompactTrigger, Option<CompactResult>)> {
    let trigger = decision.trigger;
    if let Some(summary) = decision.provided_summary {
        return Ok((
            trigger,
            Some(compact_result_from_provided_summary(
                summary,
                &decision.messages,
                None,
            )),
        ));
    }

    let compaction = auto_compact(
        provider,
        &decision.messages,
        None,
        compact_config_from_settings(settings, trigger, history_path, None),
        cancel,
    )
    .await?;
    Ok((trigger, compaction))
}

fn compact_result_from_provided_summary(
    summary: String,
    source_messages: &[LlmMessage],
    compact_prompt_context: Option<&str>,
) -> CompactResult {
    let summary = summary.trim().to_string();
    let messages = vec![LlmMessage::User {
        content: format_compact_summary(&summary),
        origin: UserMessageOrigin::CompactSummary,
    }];
    let pre_tokens = estimate_request_tokens(source_messages, compact_prompt_context);
    let post_tokens_estimate = estimate_request_tokens(&messages, compact_prompt_context);
    let output_summary_chars = summary.chars().count().min(u32::MAX as usize) as u32;
    CompactResult {
        messages,
        summary,
        recent_user_context_digest: None,
        recent_user_context_messages: Vec::new(),
        preserved_recent_turns: 0,
        pre_tokens,
        post_tokens_estimate,
        messages_removed: source_messages.len(),
        tokens_freed: pre_tokens.saturating_sub(post_tokens_estimate),
        timestamp: chrono::Utc::now(),
        meta: CompactAppliedMeta {
            mode: CompactMode::Full,
            instructions_present: false,
            fallback_used: false,
            retry_count: 0,
            input_units: source_messages.len().min(u32::MAX as usize) as u32,
            output_summary_chars,
        },
    }
}

fn compact_settings_payload(settings: &ContextWindowSettings) -> serde_json::Value {
    serde_json::json!({
        "autoCompactEnabled": settings.auto_compact_enabled,
        "compactThresholdPercent": settings.compact_threshold_percent,
        "reservedContextSize": settings.reserved_context_size,
        "summaryReserveTokens": settings.summary_reserve_tokens,
        "compactMaxOutputTokens": settings.compact_max_output_tokens,
        "compactMaxRetryAttempts": settings.compact_max_retry_attempts,
        "compactKeepRecentTurns": settings.compact_keep_recent_turns,
        "compactKeepRecentUserMessages": settings.compact_keep_recent_user_messages,
    })
}

fn hook_effect_name(effect: &HookEffect) -> &'static str {
    match effect {
        HookEffect::Continue => "Continue",
        HookEffect::Diagnostic { .. } => "Diagnostic",
        HookEffect::TransformInput { .. } => "TransformInput",
        HookEffect::HandledInput { .. } => "HandledInput",
        HookEffect::SwitchMode { .. } => "SwitchMode",
        HookEffect::ModifyProviderRequest { .. } => "ModifyProviderRequest",
        HookEffect::DenyProviderRequest { .. } => "DenyProviderRequest",
        HookEffect::MutateToolArgs { .. } => "MutateToolArgs",
        HookEffect::BlockToolResult { .. } => "BlockToolResult",
        HookEffect::RequireApproval { .. } => "RequireApproval",
        HookEffect::OverrideToolResult { .. } => "OverrideToolResult",
        HookEffect::CancelTurn { .. } => "CancelTurn",
        HookEffect::CancelCompact { .. } => "CancelCompact",
        HookEffect::OverrideCompactInput { .. } => "OverrideCompactInput",
        HookEffect::ProvideCompactSummary { .. } => "ProvideCompactSummary",
        HookEffect::ResourcePath { .. } => "ResourcePath",
        HookEffect::ModelHint { .. } => "ModelHint",
        HookEffect::DenyModelSelect { .. } => "DenyModelSelect",
    }
}

/// 将 provider 返回的实际 token 使用量回填到之前发出的 PromptMetrics 事件中。
///
/// 从 events 尾部向前搜索匹配当前 step_index 的 PromptMetrics 事件，因为
/// metrics 事件在 provider 调用之前就已创建（只含估算值），需要用真实值覆盖。
pub(crate) fn apply_prompt_metrics_usage(
    events: &mut [RuntimeTurnEvent],
    step_index: usize,
    usage: Option<LlmUsage>,
    diagnostics: Option<PromptCacheDiagnostics>,
) {
    if usage.is_none() && diagnostics.is_none() {
        return;
    }

    let step_index = saturating_u32(step_index);
    let Some(metrics) = events.iter_mut().rev().find_map(|event| {
        let RuntimeTurnEvent::StorageEvent { event } = event else {
            return None;
        };
        let StorageEventPayload::PromptMetrics { metrics } = &mut event.payload else {
            return None;
        };
        (metrics.step_index == step_index).then_some(metrics)
    }) else {
        return;
    };

    if let Some(usage) = usage {
        metrics.provider_input_tokens = Some(saturating_u32(usage.input_tokens));
        metrics.provider_output_tokens = Some(saturating_u32(usage.output_tokens));
        metrics.cache_creation_input_tokens =
            Some(saturating_u32(usage.cache_creation_input_tokens));
        metrics.cache_read_input_tokens = Some(saturating_u32(usage.cache_read_input_tokens));
    }
    if let Some(diagnostics) = diagnostics {
        metrics.prompt_cache_diagnostics = Some(core_prompt_cache_diagnostics(diagnostics));
    }
}

fn core_prompt_cache_diagnostics(
    diagnostics: PromptCacheDiagnostics,
) -> astrcode_core::PromptCacheDiagnostics {
    astrcode_core::PromptCacheDiagnostics {
        reasons: diagnostics
            .reasons
            .into_iter()
            .map(|reason| match reason {
                astrcode_llm_contract::PromptCacheBreakReason::SystemPromptChanged => {
                    astrcode_core::PromptCacheBreakReason::SystemPromptChanged
                },
                astrcode_llm_contract::PromptCacheBreakReason::ToolSchemasChanged => {
                    astrcode_core::PromptCacheBreakReason::ToolSchemasChanged
                },
                astrcode_llm_contract::PromptCacheBreakReason::ModelChanged => {
                    astrcode_core::PromptCacheBreakReason::ModelChanged
                },
                astrcode_llm_contract::PromptCacheBreakReason::GlobalCacheStrategyChanged => {
                    astrcode_core::PromptCacheBreakReason::GlobalCacheStrategyChanged
                },
                astrcode_llm_contract::PromptCacheBreakReason::CompactedPrompt => {
                    astrcode_core::PromptCacheBreakReason::CompactedPrompt
                },
                astrcode_llm_contract::PromptCacheBreakReason::ToolResultRebudgeted => {
                    astrcode_core::PromptCacheBreakReason::ToolResultRebudgeted
                },
            })
            .collect(),
        previous_cache_read_input_tokens: diagnostics.previous_cache_read_input_tokens,
        current_cache_read_input_tokens: diagnostics.current_cache_read_input_tokens,
        expected_drop: diagnostics.expected_drop,
        cache_break_detected: diagnostics.cache_break_detected,
    }
}

fn accumulate_tool_result_budget_stats(
    total: &mut ToolResultBudgetStats,
    next: ToolResultBudgetStats,
) {
    total.replacement_count = total
        .replacement_count
        .saturating_add(next.replacement_count);
    total.reapply_count = total.reapply_count.saturating_add(next.reapply_count);
    total.bytes_saved = total.bytes_saved.saturating_add(next.bytes_saved);
    total.over_budget_message_count = total
        .over_budget_message_count
        .saturating_add(next.over_budget_message_count);
}

fn prompt_metrics_runtime_event(
    turn_id: &str,
    agent: &AgentEventContext,
    step_index: usize,
    snapshot: PromptTokenSnapshot,
    truncated_tool_results: usize,
    provider_cache_metrics_supported: bool,
) -> RuntimeTurnEvent {
    RuntimeTurnEvent::StorageEvent {
        event: Box::new(StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::PromptMetrics {
                metrics: PromptMetricsPayload {
                    step_index: saturating_u32(step_index),
                    estimated_tokens: saturating_u32(snapshot.context_tokens),
                    context_window: saturating_u32(snapshot.context_window),
                    effective_window: saturating_u32(snapshot.effective_window),
                    threshold_tokens: saturating_u32(snapshot.threshold_tokens),
                    truncated_tool_results: saturating_u32(truncated_tool_results),
                    provider_input_tokens: None,
                    provider_output_tokens: None,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                    provider_cache_metrics_supported,
                    prompt_cache_reuse_hits: 0,
                    prompt_cache_reuse_misses: 0,
                    prompt_cache_unchanged_layers: Vec::new(),
                    prompt_cache_diagnostics: None,
                },
            },
        }),
    }
}

fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmMessage, UserMessageOrigin};

    use super::compact_result_from_provided_summary;

    #[test]
    fn provided_compact_summary_builds_compact_result_without_provider_call() {
        let source_messages = vec![
            LlmMessage::User {
                content: "first".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "answer".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ];

        let result = compact_result_from_provided_summary(
            "  condensed facts  ".to_string(),
            &source_messages,
            None,
        );

        assert_eq!(result.summary, "condensed facts");
        assert_eq!(result.messages_removed, source_messages.len());
        assert_eq!(result.meta.input_units, source_messages.len() as u32);
        assert!(matches!(
            &result.messages[0],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::CompactSummary,
            } if content.contains("condensed facts")
        ));
    }
}
