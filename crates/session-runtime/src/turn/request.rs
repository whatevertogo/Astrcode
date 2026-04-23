//! Turn prompt request assembly.
//!
//! This module keeps the boundary between context-window sizing and final prompt
//! request construction. It composes prompt metadata, runs prune/micro-compact,
//! emits metrics, and finally builds `LlmRequest`.

use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use astrcode_core::{
    AgentEventContext, CompactTrigger, LlmMessage, LlmRequest, PromptBuildOutput,
    PromptBuildRequest, PromptDeclaration, PromptFacts, PromptFactsProvider, PromptFactsRequest,
    PromptGovernanceContext, Result, StorageEvent, UserMessageOrigin,
};
use astrcode_kernel::KernelGateway;

use crate::{
    context_window::{
        ContextWindowSettings,
        compaction::{CompactConfig, auto_compact},
        file_access::{FileAccessTracker, FileRecoveryConfig},
        micro_compact::MicroCompactState,
        token_usage::{TokenUsageTracker, build_prompt_snapshot, should_compact},
    },
    state::compact_history_event_log_path,
    turn::{
        compact_events::{build_post_compact_events, build_post_compact_recovery_messages},
        events::prompt_metrics_event,
        tool_result_budget::{
            ApplyToolResultBudgetRequest, ToolResultBudgetOutcome, ToolResultBudgetStats,
            ToolResultReplacementState, apply_tool_result_budget,
        },
    },
};
// TODO: 需要重构
pub struct AssemblePromptRequest<'a> {
    pub gateway: &'a KernelGateway,
    pub prompt_facts_provider: &'a dyn PromptFactsProvider,
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub working_dir: &'a Path,
    pub messages: Vec<LlmMessage>,
    pub cancel: astrcode_core::CancelToken,
    pub agent: &'a AgentEventContext,
    pub step_index: usize,
    pub token_tracker: &'a TokenUsageTracker,
    pub tools: Arc<[astrcode_core::ToolDefinition]>,
    pub settings: &'a ContextWindowSettings,
    pub clearable_tools: &'a HashSet<String>,
    pub micro_compact_state: &'a mut MicroCompactState,
    pub file_access_tracker: &'a FileAccessTracker,
    pub session_state: &'a crate::SessionState,
    pub tool_result_replacement_state: &'a mut ToolResultReplacementState,
    pub prompt_declarations: &'a [PromptDeclaration],
    pub prompt_governance: Option<&'a PromptGovernanceContext>,
}

pub struct AssemblePromptResult {
    pub llm_request: LlmRequest,
    pub messages: Vec<LlmMessage>,
    pub events: Vec<StorageEvent>,
    pub auto_compacted: bool,
    pub tool_result_budget_stats: ToolResultBudgetStats,
}

pub(crate) struct PromptOutputRequest<'a> {
    pub gateway: &'a KernelGateway,
    pub prompt_facts_provider: &'a dyn PromptFactsProvider,
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub working_dir: &'a Path,
    pub step_index: usize,
    pub messages: &'a [LlmMessage],
    pub session_state: Option<&'a crate::SessionState>,
    pub current_agent_id: Option<&'a str>,
    pub submission_prompt_declarations: &'a [PromptDeclaration],
    pub prompt_governance: Option<&'a PromptGovernanceContext>,
}

/// Why: request assembly 要回答“最终如何形成一次 LLM 请求”，
/// 因此它属于 `turn/request`，而不属于 `context_window`。
pub async fn assemble_prompt_request(
    request: AssemblePromptRequest<'_>,
) -> Result<AssemblePromptResult> {
    let now = Instant::now();
    let mut events = Vec::new();
    let mut auto_compacted = false;

    let ToolResultBudgetOutcome {
        messages: budgeted_messages,
        events: budget_events,
        stats: tool_result_budget_stats,
    } = apply_tool_result_budget(ApplyToolResultBudgetRequest {
        messages: &request.messages,
        session_id: request.session_id,
        working_dir: request.working_dir,
        session_state: request.session_state,
        replacement_state: request.tool_result_replacement_state,
        aggregate_budget_bytes: request.settings.aggregate_result_bytes_budget,
        turn_id: request.turn_id,
        agent: request.agent,
    })?;
    events.extend(budget_events);

    let micro_outcome = request.micro_compact_state.apply_if_idle(
        &budgeted_messages,
        request.clearable_tools,
        request.settings.micro_compact_config(),
        now,
    );
    let mut messages = micro_outcome.messages;

    let prune_outcome = crate::context_window::prune_pass::apply_prune_pass(
        &messages,
        request.clearable_tools,
        request.settings.tool_result_max_bytes,
        request.settings.compact_keep_recent_turns,
    );
    messages = prune_outcome.messages;

    let mut prompt_output = build_prompt_output(PromptOutputRequest {
        gateway: request.gateway,
        prompt_facts_provider: request.prompt_facts_provider,
        session_id: request.session_id,
        turn_id: request.turn_id,
        working_dir: request.working_dir,
        step_index: request.step_index,
        messages: &messages,
        session_state: Some(request.session_state),
        current_agent_id: request.agent.agent_id.as_ref().map(|id| id.as_str()),
        submission_prompt_declarations: request.prompt_declarations,
        prompt_governance: request.prompt_governance,
    })
    .await?;
    let mut snapshot = build_prompt_snapshot(
        request.token_tracker,
        &messages,
        Some(&prompt_output.system_prompt),
        request.gateway.model_limits(),
        request.settings.compact_threshold_percent,
        request.settings.summary_reserve_tokens,
        request.settings.reserved_context_size,
    );

    if should_compact(snapshot) {
        if request.settings.auto_compact_enabled {
            if let Some(compaction) = auto_compact(
                request.gateway,
                &messages,
                Some(&prompt_output.system_prompt),
                CompactConfig {
                    keep_recent_turns: request.settings.compact_keep_recent_turns,
                    keep_recent_user_messages: request.settings.compact_keep_recent_user_messages,
                    trigger: CompactTrigger::Auto,
                    summary_reserve_tokens: request.settings.summary_reserve_tokens,
                    max_output_tokens: request.settings.compact_max_output_tokens,
                    max_retry_attempts: request.settings.compact_max_retry_attempts,
                    history_path: Some(compact_history_event_log_path(
                        request.session_id,
                        request.working_dir,
                    )?),
                    custom_instructions: None,
                },
                request.cancel.clone(),
            )
            .await?
            {
                let compact_events = build_post_compact_events(
                    Some(request.turn_id),
                    request.agent,
                    CompactTrigger::Auto,
                    &compaction,
                );
                messages = compaction.messages;
                auto_compacted = true;
                messages.extend(build_post_compact_recovery_messages(
                    request.file_access_tracker,
                    FileRecoveryConfig {
                        max_tracked_files: request.settings.max_tracked_files,
                        max_recovered_files: request.settings.max_recovered_files,
                        recovery_token_budget: request.settings.recovery_token_budget,
                    },
                ));
                events.extend(compact_events);

                prompt_output = build_prompt_output(PromptOutputRequest {
                    gateway: request.gateway,
                    prompt_facts_provider: request.prompt_facts_provider,
                    session_id: request.session_id,
                    turn_id: request.turn_id,
                    working_dir: request.working_dir,
                    step_index: request.step_index,
                    messages: &messages,
                    session_state: Some(request.session_state),
                    current_agent_id: request.agent.agent_id.as_ref().map(|id| id.as_str()),
                    submission_prompt_declarations: request.prompt_declarations,
                    prompt_governance: request.prompt_governance,
                })
                .await?;
                snapshot = build_prompt_snapshot(
                    request.token_tracker,
                    &messages,
                    Some(&prompt_output.system_prompt),
                    request.gateway.model_limits(),
                    request.settings.compact_threshold_percent,
                    request.settings.summary_reserve_tokens,
                    request.settings.reserved_context_size,
                );
            }
        } else {
            log::warn!(
                "turn {} step {}: context tokens ({}) exceed threshold ({}) but auto compact is \
                 disabled",
                request.turn_id,
                request.step_index,
                snapshot.context_tokens,
                snapshot.threshold_tokens,
            );
        }
    }

    events.push(prompt_metrics_event(
        request.turn_id,
        request.agent,
        request.step_index,
        snapshot,
        prune_outcome.stats.truncated_tool_results,
        prompt_output.cache_metrics,
        request.gateway.supports_cache_metrics(),
    ));

    let mut prompt_cache_hints = prompt_output.prompt_cache_hints.clone();
    prompt_cache_hints.compacted = auto_compacted;
    prompt_cache_hints.tool_result_rebudgeted = tool_result_budgeted(&tool_result_budget_stats);

    let mut llm_request = LlmRequest::new(messages.clone(), request.tools, request.cancel.clone())
        .with_system(prompt_output.system_prompt);
    llm_request.system_prompt_blocks = prompt_output.system_prompt_blocks;
    llm_request.prompt_cache_hints = Some(prompt_cache_hints);

    Ok(AssemblePromptResult {
        llm_request,
        messages,
        events,
        auto_compacted,
        tool_result_budget_stats,
    })
}

fn tool_result_budgeted(stats: &ToolResultBudgetStats) -> bool {
    stats.replacement_count > 0 || stats.reapply_count > 0 || stats.over_budget_message_count > 0
}

pub(crate) async fn build_prompt_output(
    request: PromptOutputRequest<'_>,
) -> Result<PromptBuildOutput> {
    let PromptOutputRequest {
        gateway,
        prompt_facts_provider,
        session_id,
        turn_id,
        working_dir,
        step_index,
        messages,
        session_state,
        current_agent_id,
        submission_prompt_declarations,
        prompt_governance,
    } = request;
    let facts = prompt_facts_provider
        .resolve_prompt_facts(&PromptFactsRequest {
            session_id: Some(session_id.to_string().into()),
            turn_id: Some(turn_id.to_string().into()),
            working_dir: working_dir.to_path_buf(),
            allowed_capability_names: gateway
                .capabilities()
                .capability_specs()
                .into_iter()
                .map(|spec| spec.name.to_string())
                .collect(),
            governance: prompt_governance.cloned(),
        })
        .await?;
    let turn_index = count_user_turns(messages);
    let metadata = build_prompt_metadata(
        session_id, turn_id, step_index, turn_index, messages, &facts,
    );
    let PromptFacts {
        profile,
        profile_context,
        metadata: _,
        skills,
        agent_profiles,
        mut prompt_declarations,
    } = facts;
    if let Some(direct_child_snapshot) =
        live_direct_child_snapshot_declaration(session_state, current_agent_id)?
    {
        prompt_declarations.push(direct_child_snapshot);
    }
    if let Some(task_snapshot) =
        live_task_snapshot_declaration(session_state, session_id, current_agent_id)?
    {
        prompt_declarations.push(task_snapshot);
    }
    prompt_declarations.extend_from_slice(submission_prompt_declarations);
    gateway
        .build_prompt(PromptBuildRequest {
            session_id: Some(session_id.to_string().into()),
            turn_id: Some(turn_id.to_string().into()),
            working_dir: working_dir.to_path_buf(),
            profile,
            step_index,
            turn_index,
            profile_context,
            capabilities: gateway.capabilities().capability_specs(),
            skills,
            agent_profiles,
            prompt_declarations,
            metadata,
        })
        .await
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}

fn live_direct_child_snapshot_declaration(
    session_state: Option<&crate::SessionState>,
    current_agent_id: Option<&str>,
) -> Result<Option<PromptDeclaration>> {
    let Some(session_state) = session_state else {
        return Ok(None);
    };
    let Some(current_agent_id) = current_agent_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    let direct_children = session_state.child_nodes_for_parent(current_agent_id)?;
    let children_block = if direct_children.is_empty() {
        "- (none)\n- If work needs a new branch, use `spawn` instead of guessing an older \
         `agentId`."
            .to_string()
    } else {
        direct_children
            .iter()
            .map(|node| {
                format!(
                    "- agentId=`{}` status=`{:?}` subRunId=`{}` childSessionId=`{}` lineage=`{:?}`",
                    node.agent_id(),
                    node.status,
                    node.sub_run_id(),
                    node.child_session_id,
                    node.lineage_kind
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(Some(PromptDeclaration {
        block_id: "agent.live.direct_children".to_string(),
        title: "Live Direct Child Snapshot".to_string(),
        content: format!(
            "Authoritative direct-child snapshot for the current agent.\n\nRouting rules:\n- Only \
             use `agentId` values from this snapshot or from a newer live tool result / child \
             notification in the current prompt tail.\n- Never reuse `agentId`, `subRunId`, or \
             `sessionId` from compact summaries, stale errors, or historical notes.\n- If a child \
             is absent from this snapshot, treat it as unavailable for `send`, `observe`, or \
             `close` at prompt-build time.\n\nDirect children:\n{children_block}"
        ),
        render_target: astrcode_core::PromptDeclarationRenderTarget::System,
        layer: astrcode_core::SystemPromptLayer::Dynamic,
        kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(592),
        always_include: true,
        source: astrcode_core::PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some(format!("live-direct-children:{current_agent_id}")),
    }))
}

fn live_task_snapshot_declaration(
    session_state: Option<&crate::SessionState>,
    session_id: &str,
    current_agent_id: Option<&str>,
) -> Result<Option<PromptDeclaration>> {
    let Some(session_state) = session_state else {
        return Ok(None);
    };
    let owner = current_agent_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(session_id);
    let Some(snapshot) = session_state.active_tasks_for(owner)? else {
        return Ok(None);
    };
    let active_items = snapshot.active_items();
    if active_items.is_empty() {
        return Ok(None);
    }

    let items_block = active_items
        .iter()
        .map(|item| match item.status {
            astrcode_core::ExecutionTaskStatus::InProgress => format!(
                "- in_progress: {}{}",
                item.content,
                item.active_form
                    .as_deref()
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default()
            ),
            astrcode_core::ExecutionTaskStatus::Pending => format!("- pending: {}", item.content),
            astrcode_core::ExecutionTaskStatus::Completed => String::new(),
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(Some(PromptDeclaration {
        block_id: "task.active_snapshot".to_string(),
        title: "Live Task Snapshot".to_string(),
        content: format!(
            "Authoritative execution-task snapshot for the current owner.\n\nRules:\n- Treat this \
             snapshot as the current execution checklist for this branch of work.\n- Only \
             `in_progress` and `pending` tasks appear here; completed items are intentionally \
             omitted.\n- Update this snapshot with `taskWrite` before changing focus or after \
             completing a task.\n\nActive tasks:\n{items_block}"
        ),
        render_target: astrcode_core::PromptDeclarationRenderTarget::System,
        layer: astrcode_core::SystemPromptLayer::Dynamic,
        kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
        priority_hint: Some(593),
        always_include: true,
        source: astrcode_core::PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some(format!("live-task-snapshot:{owner}")),
    }))
}

pub(crate) fn build_prompt_metadata(
    session_id: &str,
    turn_id: &str,
    step_index: usize,
    turn_index: usize,
    messages: &[LlmMessage],
    facts: &PromptFacts,
) -> serde_json::Value {
    let latest_user_message = messages.iter().rev().find_map(|message| match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User,
            ..
        } => Some(content.clone()),
        _ => None,
    });
    let mut metadata = match &facts.metadata {
        serde_json::Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    metadata.insert(
        "sessionId".to_string(),
        serde_json::Value::String(session_id.to_string()),
    );
    metadata.insert(
        "turnId".to_string(),
        serde_json::Value::String(turn_id.to_string()),
    );
    metadata.insert(
        "stepIndex".to_string(),
        serde_json::Value::Number((step_index as u64).into()),
    );
    metadata.insert(
        "turnIndex".to_string(),
        serde_json::Value::Number((turn_index as u64).into()),
    );
    metadata.insert(
        "latestUserMessage".to_string(),
        latest_user_message
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    serde_json::Value::Object(metadata)
}

pub(crate) fn count_user_turns(messages: &[LlmMessage]) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                }
            )
        })
        .count()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{
        AgentLifecycleStatus, AstrError, ChildExecutionIdentity, ChildSessionLineageKind,
        ChildSessionNode, ChildSessionStatusSource, ExecutionTaskItem, ExecutionTaskStatus,
        LlmOutput, LlmProvider, LlmRequest, ModelLimits, ParentExecutionRef, PromptBuildOutput,
        PromptBuildRequest, PromptDeclaration, PromptDeclarationKind,
        PromptDeclarationRenderTarget, PromptDeclarationSource, PromptFacts, PromptFactsProvider,
        PromptFactsRequest, PromptProvider, ResolvedRuntimeConfig, ResourceProvider,
        ResourceReadResult, ResourceRequestContext, StorageEventPayload, SystemPromptLayer,
        ToolDefinition,
    };
    use astrcode_kernel::{CapabilityRouter, KernelGateway};
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{
        context_window::token_usage::TokenUsageTracker,
        turn::{
            test_support::{NoopPromptFactsProvider, test_gateway, test_session_state},
            tool_result_budget::ToolResultReplacementState,
        },
    };

    #[tokio::test]
    async fn assemble_prompt_request_emits_prompt_metrics_for_final_prompt() {
        let gateway = test_gateway(64_000);
        let mut micro_state = crate::context_window::micro_compact::MicroCompactState::default();
        let tracker = crate::context_window::file_access::FileAccessTracker::new(4);
        let session_state = test_session_state();
        let mut replacement_state = ToolResultReplacementState::default();
        let settings = ContextWindowSettings::from(&ResolvedRuntimeConfig::default());

        let result = assemble_prompt_request(AssemblePromptRequest {
            gateway: &gateway,
            prompt_facts_provider: &NoopPromptFactsProvider,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            messages: vec![LlmMessage::User {
                content: "hello".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            }],
            cancel: astrcode_core::CancelToken::new(),
            agent: &AgentEventContext::default(),
            step_index: 0,
            token_tracker: &TokenUsageTracker::default(),
            tools: vec![ToolDefinition {
                name: "readFile".to_string(),
                description: "read".to_string(),
                parameters: json!({"type":"object"}),
            }]
            .into(),
            settings: &settings,
            clearable_tools: &std::collections::HashSet::new(),
            micro_compact_state: &mut micro_state,
            file_access_tracker: &tracker,
            session_state: &session_state,
            tool_result_replacement_state: &mut replacement_state,
            prompt_declarations: &[],
            prompt_governance: None,
        })
        .await
        .expect("assembly should succeed");

        assert_eq!(result.events.len(), 1);
        assert!(matches!(
            &result.events[0].payload,
            StorageEventPayload::PromptMetrics { .. }
        ));
        assert_eq!(result.llm_request.messages.len(), 1);
    }

    #[tokio::test]
    async fn assemble_prompt_request_carries_prompt_cache_reuse_counts() {
        let base_gateway = test_gateway(64_000);
        let gateway = KernelGateway::new(
            base_gateway.capabilities().clone(),
            Arc::new(LocalNoopLlmProvider),
            Arc::new(RecordingPromptProvider {
                captured: Arc::new(Mutex::new(Vec::new())),
            }),
            Arc::new(LocalNoopResourceProvider),
        );
        let mut micro_state = crate::context_window::micro_compact::MicroCompactState::default();
        let tracker = crate::context_window::file_access::FileAccessTracker::new(4);
        let session_state = test_session_state();
        let mut replacement_state = ToolResultReplacementState::default();
        let settings = ContextWindowSettings::from(&ResolvedRuntimeConfig::default());

        let result = assemble_prompt_request(AssemblePromptRequest {
            gateway: &gateway,
            prompt_facts_provider: &NoopPromptFactsProvider,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            messages: vec![LlmMessage::User {
                content: "hello".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            }],
            cancel: astrcode_core::CancelToken::new(),
            agent: &AgentEventContext::default(),
            step_index: 0,
            token_tracker: &TokenUsageTracker::default(),
            tools: vec![ToolDefinition {
                name: "readFile".to_string(),
                description: "read".to_string(),
                parameters: json!({"type":"object"}),
            }]
            .into(),
            settings: &settings,
            clearable_tools: &std::collections::HashSet::new(),
            micro_compact_state: &mut micro_state,
            file_access_tracker: &tracker,
            session_state: &session_state,
            tool_result_replacement_state: &mut replacement_state,
            prompt_declarations: &[],
            prompt_governance: None,
        })
        .await
        .expect("assembly should succeed");

        assert!(matches!(
            &result.events[0].payload,
            StorageEventPayload::PromptMetrics { metrics }
                if metrics.prompt_cache_reuse_hits == 2
                    && metrics.prompt_cache_reuse_misses == 1
                    && !metrics.provider_cache_metrics_supported
        ));
    }

    #[derive(Debug)]
    struct RecordingPromptProvider {
        captured: Arc<Mutex<Vec<PromptDeclaration>>>,
    }

    #[async_trait]
    impl PromptProvider for RecordingPromptProvider {
        async fn build_prompt(
            &self,
            request: PromptBuildRequest,
        ) -> astrcode_core::Result<PromptBuildOutput> {
            *self.captured.lock().expect("capture lock should work") =
                request.prompt_declarations.clone();
            Ok(PromptBuildOutput {
                system_prompt: "recorded".to_string(),
                system_prompt_blocks: Vec::new(),
                prompt_cache_hints: Default::default(),
                cache_metrics: astrcode_core::PromptBuildCacheMetrics {
                    reuse_hits: 2,
                    reuse_misses: 1,
                    unchanged_layers: Vec::new(),
                },
                metadata: serde_json::Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct RecordingPromptFactsProvider;

    #[async_trait]
    impl PromptFactsProvider for RecordingPromptFactsProvider {
        async fn resolve_prompt_facts(
            &self,
            _request: &PromptFactsRequest,
        ) -> astrcode_core::Result<PromptFacts> {
            Ok(PromptFacts {
                prompt_declarations: vec![PromptDeclaration {
                    block_id: "facts.contract".to_string(),
                    title: "Facts Contract".to_string(),
                    content: "facts".to_string(),
                    render_target: PromptDeclarationRenderTarget::System,
                    layer: SystemPromptLayer::Inherited,
                    kind: PromptDeclarationKind::ExtensionInstruction,
                    priority_hint: None,
                    always_include: true,
                    source: PromptDeclarationSource::Builtin,
                    capability_name: None,
                    origin: Some("facts-origin".to_string()),
                }],
                ..PromptFacts::default()
            })
        }
    }

    #[derive(Debug)]
    struct LocalNoopLlmProvider;

    #[async_trait]
    impl LlmProvider for LocalNoopLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> astrcode_core::Result<LlmOutput> {
            Err(AstrError::Validation(
                "request test noop llm provider should not execute".to_string(),
            ))
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 32_000,
                max_output_tokens: 4096,
            }
        }
    }

    #[derive(Debug)]
    struct LocalNoopResourceProvider;

    #[async_trait]
    impl ResourceProvider for LocalNoopResourceProvider {
        async fn read_resource(
            &self,
            uri: &str,
            _context: &ResourceRequestContext,
        ) -> astrcode_core::Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: uri.to_string(),
                content: serde_json::Value::Null,
                metadata: serde_json::Value::Null,
            })
        }
    }

    #[tokio::test]
    async fn build_prompt_output_merges_submission_prompt_declarations() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let gateway = KernelGateway::new(
            CapabilityRouter::empty(),
            Arc::new(LocalNoopLlmProvider),
            Arc::new(RecordingPromptProvider {
                captured: captured.clone(),
            }),
            Arc::new(LocalNoopResourceProvider),
        );
        let submission_declarations = vec![PromptDeclaration {
            block_id: "child.execution.contract".to_string(),
            title: "Child Execution Contract".to_string(),
            content: "submission".to_string(),
            render_target: PromptDeclarationRenderTarget::System,
            layer: SystemPromptLayer::Inherited,
            kind: PromptDeclarationKind::ExtensionInstruction,
            priority_hint: None,
            always_include: true,
            source: PromptDeclarationSource::Builtin,
            capability_name: None,
            origin: Some("submission-origin".to_string()),
        }];

        let output = build_prompt_output(PromptOutputRequest {
            gateway: &gateway,
            prompt_facts_provider: &RecordingPromptFactsProvider,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            step_index: 0,
            messages: &[],
            session_state: None,
            current_agent_id: None,
            submission_prompt_declarations: &submission_declarations,
            prompt_governance: None,
        })
        .await
        .expect("prompt output should build");

        assert_eq!(output.system_prompt, "recorded");
        let captured = captured.lock().expect("capture lock should work");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].origin.as_deref(), Some("facts-origin"));
        assert_eq!(captured[1].origin.as_deref(), Some("submission-origin"));
    }

    #[tokio::test]
    async fn build_prompt_output_includes_live_task_snapshot_for_current_owner() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let gateway = KernelGateway::new(
            CapabilityRouter::empty(),
            Arc::new(LocalNoopLlmProvider),
            Arc::new(RecordingPromptProvider {
                captured: captured.clone(),
            }),
            Arc::new(LocalNoopResourceProvider),
        );
        let session_state = test_session_state();
        session_state
            .replace_active_task_snapshot(astrcode_core::TaskSnapshot {
                owner: "agent-root".to_string(),
                items: vec![
                    ExecutionTaskItem {
                        content: "实现 task prompt 注入".to_string(),
                        status: ExecutionTaskStatus::InProgress,
                        active_form: Some("正在实现 task prompt 注入".to_string()),
                    },
                    ExecutionTaskItem {
                        content: "补充 request 测试".to_string(),
                        status: ExecutionTaskStatus::Pending,
                        active_form: None,
                    },
                    ExecutionTaskItem {
                        content: "完成旧任务".to_string(),
                        status: ExecutionTaskStatus::Completed,
                        active_form: Some("已完成旧任务".to_string()),
                    },
                ],
            })
            .expect("task snapshot should store");

        build_prompt_output(PromptOutputRequest {
            gateway: &gateway,
            prompt_facts_provider: &RecordingPromptFactsProvider,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            step_index: 0,
            messages: &[],
            session_state: Some(session_state.as_ref()),
            current_agent_id: Some("agent-root"),
            submission_prompt_declarations: &[],
            prompt_governance: None,
        })
        .await
        .expect("prompt output should build");

        let captured = captured.lock().expect("capture lock should work");
        let declaration = captured
            .iter()
            .find(|declaration| declaration.block_id == "task.active_snapshot")
            .expect("task snapshot declaration should exist");
        assert!(
            declaration
                .content
                .contains("in_progress: 实现 task prompt 注入")
        );
        assert!(declaration.content.contains("pending: 补充 request 测试"));
        assert!(!declaration.content.contains("完成旧任务"));
    }

    #[tokio::test]
    async fn build_prompt_output_omits_live_task_snapshot_when_no_active_items_exist() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let gateway = KernelGateway::new(
            CapabilityRouter::empty(),
            Arc::new(LocalNoopLlmProvider),
            Arc::new(RecordingPromptProvider {
                captured: captured.clone(),
            }),
            Arc::new(LocalNoopResourceProvider),
        );
        let session_state = test_session_state();
        session_state
            .replace_active_task_snapshot(astrcode_core::TaskSnapshot {
                owner: "session-1".to_string(),
                items: vec![ExecutionTaskItem {
                    content: "已完成任务".to_string(),
                    status: ExecutionTaskStatus::Completed,
                    active_form: Some("已完成任务".to_string()),
                }],
            })
            .expect("task snapshot should store");

        build_prompt_output(PromptOutputRequest {
            gateway: &gateway,
            prompt_facts_provider: &RecordingPromptFactsProvider,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            step_index: 0,
            messages: &[],
            session_state: Some(session_state.as_ref()),
            current_agent_id: None,
            submission_prompt_declarations: &[],
            prompt_governance: None,
        })
        .await
        .expect("prompt output should build");

        let captured = captured.lock().expect("capture lock should work");
        assert!(
            captured
                .iter()
                .all(|declaration| declaration.block_id != "task.active_snapshot")
        );
    }

    #[test]
    fn live_direct_child_snapshot_declaration_only_uses_current_agents_children() {
        let session_state = test_session_state();
        session_state
            .upsert_child_session_node(ChildSessionNode {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-child-1".into(),
                    session_id: "session-parent".into(),
                    sub_run_id: "subrun-child-1".into(),
                },
                child_session_id: "session-child-1".into(),
                parent_session_id: "session-parent".into(),
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-root".into()),
                    parent_sub_run_id: Some("subrun-root".into()),
                },
                parent_turn_id: "turn-1".into(),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Idle,
                status_source: ChildSessionStatusSource::Durable,
                created_by_tool_call_id: None,
                lineage_snapshot: None,
            })
            .expect("direct child should insert");
        session_state
            .upsert_child_session_node(ChildSessionNode {
                identity: ChildExecutionIdentity {
                    agent_id: "agent-grandchild-1".into(),
                    session_id: "session-child-1".into(),
                    sub_run_id: "subrun-grandchild-1".into(),
                },
                child_session_id: "session-grandchild-1".into(),
                parent_session_id: "session-child-1".into(),
                parent: ParentExecutionRef {
                    parent_agent_id: Some("agent-child-1".into()),
                    parent_sub_run_id: Some("subrun-child-1".into()),
                },
                parent_turn_id: "turn-2".into(),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: AgentLifecycleStatus::Running,
                status_source: ChildSessionStatusSource::Durable,
                created_by_tool_call_id: None,
                lineage_snapshot: None,
            })
            .expect("grandchild should insert");

        let declaration =
            live_direct_child_snapshot_declaration(Some(&session_state), Some("agent-root"))
                .expect("declaration build should succeed")
                .expect("root declaration should exist");

        assert_eq!(declaration.block_id, "agent.live.direct_children");
        assert!(declaration.content.contains("agentId=`agent-child-1`"));
        assert!(declaration.content.contains("subRunId=`subrun-child-1`"));
        assert!(!declaration.content.contains("agent-grandchild-1"));
        assert!(
            declaration
                .content
                .contains("Never reuse `agentId`, `subRunId`, or `sessionId`")
        );
    }
}
