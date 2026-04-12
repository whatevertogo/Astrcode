//! # Request Assembler
//!
//! 将 step 前的上下文治理收拢到单一入口：
//! 1. micro compact
//! 2. prune pass
//! 3. prompt 构建
//! 4. token 快照与 auto compact
//! 5. compaction 后文件内容恢复

use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

use astrcode_core::{
    AgentEventContext, CompactTrigger, LlmMessage, LlmRequest, PromptBuildRequest,
    PromptMetricsPayload, Result, StorageEvent, StorageEventPayload, config::RuntimeConfig,
};
use astrcode_kernel::KernelGateway;

use super::{
    compaction::{CompactConfig, auto_compact},
    file_access::{FileAccessTracker, FileRecoveryConfig},
    micro_compact::{MicroCompactConfig, MicroCompactState},
    prune_pass::apply_prune_pass,
    token_usage::{PromptTokenSnapshot, TokenUsageTracker, build_prompt_snapshot, should_compact},
};

const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;
const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;
const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;
const DEFAULT_COMPACT_KEEP_RECENT_TURNS: usize = 2;
const DEFAULT_MAX_TRACKED_FILES: usize = 12;
const DEFAULT_MAX_RECOVERED_FILES: usize = 3;
const DEFAULT_RECOVERY_TOKEN_BUDGET: usize = 6_000;
const DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS: u64 = 45;
const DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowSettings {
    pub auto_compact_enabled: bool,
    pub compact_threshold_percent: u8,
    pub tool_result_max_bytes: usize,
    pub compact_keep_recent_turns: usize,
    pub max_tracked_files: usize,
    pub max_recovered_files: usize,
    pub recovery_token_budget: usize,
    pub micro_compact_gap_threshold: Duration,
    pub micro_compact_keep_recent_results: usize,
}

impl ContextWindowSettings {
    pub fn micro_compact_config(&self) -> MicroCompactConfig {
        MicroCompactConfig {
            gap_threshold: self.micro_compact_gap_threshold,
            keep_recent_results: self.micro_compact_keep_recent_results,
        }
    }

    pub fn file_recovery_config(&self) -> FileRecoveryConfig {
        FileRecoveryConfig {
            max_tracked_files: self.max_tracked_files,
            max_recovered_files: self.max_recovered_files,
            recovery_token_budget: self.recovery_token_budget,
        }
    }
}

impl From<&RuntimeConfig> for ContextWindowSettings {
    fn from(config: &RuntimeConfig) -> Self {
        Self {
            auto_compact_enabled: config
                .auto_compact_enabled
                .unwrap_or(DEFAULT_AUTO_COMPACT_ENABLED),
            compact_threshold_percent: config
                .compact_threshold_percent
                .unwrap_or(DEFAULT_COMPACT_THRESHOLD_PERCENT),
            tool_result_max_bytes: config
                .tool_result_max_bytes
                .unwrap_or(DEFAULT_TOOL_RESULT_MAX_BYTES),
            compact_keep_recent_turns: config
                .compact_keep_recent_turns
                .map(usize::from)
                .unwrap_or(DEFAULT_COMPACT_KEEP_RECENT_TURNS)
                .max(1),
            max_tracked_files: config
                .max_tracked_files
                .unwrap_or(DEFAULT_MAX_TRACKED_FILES)
                .max(1),
            max_recovered_files: config
                .max_recovered_files
                .unwrap_or(DEFAULT_MAX_RECOVERED_FILES)
                .max(1),
            recovery_token_budget: config
                .recovery_token_budget
                .unwrap_or(DEFAULT_RECOVERY_TOKEN_BUDGET)
                .max(1),
            micro_compact_gap_threshold: Duration::from_secs(
                config
                    .micro_compact_gap_threshold_secs
                    .unwrap_or(DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS)
                    .max(1),
            ),
            micro_compact_keep_recent_results: config
                .micro_compact_keep_recent_results
                .unwrap_or(DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS)
                .max(1),
        }
    }
}

pub struct AssemblePromptRequest<'a> {
    pub gateway: &'a KernelGateway,
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub working_dir: &'a Path,
    pub messages: Vec<LlmMessage>,
    pub cancel: astrcode_core::CancelToken,
    pub agent: &'a AgentEventContext,
    pub step_index: usize,
    pub token_tracker: &'a TokenUsageTracker,
    pub tools: Vec<astrcode_core::ToolDefinition>,
    pub settings: &'a ContextWindowSettings,
    pub clearable_tools: &'a HashSet<String>,
    pub micro_compact_state: &'a mut MicroCompactState,
    pub file_access_tracker: &'a FileAccessTracker,
}

pub struct AssemblePromptResult {
    pub llm_request: LlmRequest,
    pub messages: Vec<LlmMessage>,
    pub events: Vec<StorageEvent>,
}

pub async fn assemble_prompt_request(
    request: AssemblePromptRequest<'_>,
) -> Result<AssemblePromptResult> {
    let now = Instant::now();
    let mut events = Vec::new();

    let micro_outcome = request.micro_compact_state.apply_if_idle(
        &request.messages,
        request.clearable_tools,
        request.settings.micro_compact_config(),
        now,
    );
    let mut messages = micro_outcome.messages;

    let prune_outcome = apply_prune_pass(
        &messages,
        request.clearable_tools,
        request.settings.tool_result_max_bytes,
        request.settings.compact_keep_recent_turns,
    );
    messages = prune_outcome.messages;

    let mut prompt_output = build_prompt_output(
        request.gateway,
        request.session_id,
        request.turn_id,
        request.working_dir,
    )
    .await?;
    let mut snapshot = build_prompt_snapshot(
        request.token_tracker,
        &messages,
        Some(&prompt_output.system_prompt),
        request.gateway.model_limits(),
        request.settings.compact_threshold_percent,
    );

    if should_compact(snapshot) {
        if request.settings.auto_compact_enabled {
            if let Some(compaction) = auto_compact(
                request.gateway,
                &messages,
                Some(&prompt_output.system_prompt),
                CompactConfig {
                    keep_recent_turns: request.settings.compact_keep_recent_turns,
                    trigger: CompactTrigger::Auto,
                },
                request.cancel.clone(),
            )
            .await?
            {
                messages = compaction.messages;
                messages.extend(
                    request
                        .file_access_tracker
                        .build_recovery_messages(request.settings.file_recovery_config()),
                );

                events.push(StorageEvent {
                    turn_id: Some(request.turn_id.to_string()),
                    agent: request.agent.clone(),
                    payload: StorageEventPayload::CompactApplied {
                        trigger: CompactTrigger::Auto,
                        summary: compaction.summary,
                        preserved_recent_turns: saturating_u32(compaction.preserved_recent_turns),
                        pre_tokens: saturating_u32(compaction.pre_tokens),
                        post_tokens_estimate: saturating_u32(compaction.post_tokens_estimate),
                        messages_removed: saturating_u32(compaction.messages_removed),
                        tokens_freed: saturating_u32(compaction.tokens_freed),
                        timestamp: compaction.timestamp,
                    },
                });

                prompt_output = build_prompt_output(
                    request.gateway,
                    request.session_id,
                    request.turn_id,
                    request.working_dir,
                )
                .await?;
                snapshot = build_prompt_snapshot(
                    request.token_tracker,
                    &messages,
                    Some(&prompt_output.system_prompt),
                    request.gateway.model_limits(),
                    request.settings.compact_threshold_percent,
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
    ));

    let mut llm_request = LlmRequest::new(messages.clone(), request.tools, request.cancel.clone())
        .with_system(prompt_output.system_prompt);
    llm_request.system_prompt_blocks = prompt_output.system_prompt_blocks;

    Ok(AssemblePromptResult {
        llm_request,
        messages,
        events,
    })
}

async fn build_prompt_output(
    gateway: &KernelGateway,
    session_id: &str,
    turn_id: &str,
    working_dir: &Path,
) -> Result<astrcode_core::PromptBuildOutput> {
    gateway
        .build_prompt(PromptBuildRequest {
            session_id: Some(session_id.to_string().into()),
            turn_id: Some(turn_id.to_string().into()),
            working_dir: working_dir.to_path_buf(),
            profile: "coding".to_string(),
            profile_context: serde_json::Value::Null,
            capabilities: gateway.capabilities().capability_specs(),
            metadata: serde_json::Value::Null,
        })
        .await
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}

fn prompt_metrics_event(
    turn_id: &str,
    agent: &AgentEventContext,
    step_index: usize,
    snapshot: PromptTokenSnapshot,
    truncated_tool_results: usize,
) -> StorageEvent {
    StorageEvent {
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
                provider_cache_metrics_supported: false,
                prompt_cache_reuse_hits: 0,
                prompt_cache_reuse_misses: 0,
            },
        },
    }
}

fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentEventContext, CancelToken, LlmProvider, LlmRequest, PromptBuildOutput,
        PromptBuildRequest, PromptProvider, ResourceProvider, ResourceReadResult,
        ResourceRequestContext, Result, ToolDefinition,
    };
    use astrcode_kernel::CapabilityRouter;
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;

    struct TestPromptProvider;

    #[async_trait]
    impl PromptProvider for TestPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "system".to_string(),
                system_prompt_blocks: Vec::new(),
                metadata: json!(null),
            })
        }
    }

    struct TestResourceProvider;

    #[async_trait]
    impl ResourceProvider for TestResourceProvider {
        async fn read_resource(
            &self,
            uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: uri.to_string(),
                content: json!(null),
                metadata: json!(null),
            })
        }
    }

    struct TestLlmProvider {
        limits: astrcode_core::ModelLimits,
    }

    #[async_trait]
    impl LlmProvider for TestLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> Result<astrcode_core::LlmOutput> {
            Ok(astrcode_core::LlmOutput::default())
        }

        fn model_limits(&self) -> astrcode_core::ModelLimits {
            self.limits
        }
    }

    fn test_gateway(context_window: usize) -> KernelGateway {
        KernelGateway::new(
            CapabilityRouter::empty(),
            Arc::new(TestLlmProvider {
                limits: astrcode_core::ModelLimits {
                    context_window,
                    max_output_tokens: 4096,
                },
            }),
            Arc::new(TestPromptProvider),
            Arc::new(TestResourceProvider),
        )
    }

    #[tokio::test]
    async fn assembler_emits_prompt_metrics_for_final_prompt() {
        let gateway = test_gateway(64_000);
        let mut micro_state = MicroCompactState::default();
        let tracker = FileAccessTracker::new(4);
        let settings = ContextWindowSettings::from(&RuntimeConfig::default());

        let result = assemble_prompt_request(AssemblePromptRequest {
            gateway: &gateway,
            session_id: "session-1",
            turn_id: "turn-1",
            working_dir: Path::new("."),
            messages: vec![LlmMessage::User {
                content: "hello".to_string(),
                origin: astrcode_core::UserMessageOrigin::User,
            }],
            cancel: CancelToken::new(),
            agent: &AgentEventContext::default(),
            step_index: 0,
            token_tracker: &TokenUsageTracker::default(),
            tools: vec![ToolDefinition {
                name: "readFile".to_string(),
                description: "read".to_string(),
                parameters: json!({"type":"object"}),
            }],
            settings: &settings,
            clearable_tools: &HashSet::new(),
            micro_compact_state: &mut micro_state,
            file_access_tracker: &tracker,
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
}
