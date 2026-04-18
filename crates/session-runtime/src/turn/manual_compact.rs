use std::path::Path;

use astrcode_core::{
    AgentEventContext, CancelToken, ResolvedRuntimeConfig, Result, StorageEvent,
    StorageEventPayload,
};
use astrcode_kernel::KernelGateway;
use chrono::Utc;

use crate::{
    SessionState,
    context_window::{
        ContextWindowSettings,
        compaction::{CompactConfig, auto_compact},
        file_access::FileAccessTracker,
    },
    turn::{
        events::{CompactAppliedStats, compact_applied_event},
        request::{PromptOutputRequest, build_prompt_output},
    },
};

pub(crate) struct ManualCompactRequest<'a> {
    pub gateway: &'a KernelGateway,
    pub prompt_facts_provider: &'a dyn astrcode_core::PromptFactsProvider,
    pub session_state: &'a SessionState,
    pub session_id: &'a str,
    pub working_dir: &'a Path,
    pub runtime: &'a ResolvedRuntimeConfig,
    pub trigger: astrcode_core::CompactTrigger,
    pub instructions: Option<&'a str>,
}

pub(crate) async fn build_manual_compact_events(
    request: ManualCompactRequest<'_>,
) -> Result<Option<Vec<StorageEvent>>> {
    let settings = ContextWindowSettings::from(request.runtime);
    let projected = request.session_state.snapshot_projected_state()?;
    let file_access_tracker = FileAccessTracker::seed_from_messages(
        &projected.messages,
        settings.max_tracked_files,
        request.working_dir,
    );
    let prompt_output = build_prompt_output(PromptOutputRequest {
        gateway: request.gateway,
        prompt_facts_provider: request.prompt_facts_provider,
        session_id: request.session_id,
        turn_id: "manual-compact",
        working_dir: request.working_dir,
        step_index: 0,
        messages: &projected.messages,
        session_state: Some(request.session_state),
        current_agent_id: None,
        submission_prompt_declarations: &[],
    })
    .await?;

    let Some(compaction) = auto_compact(
        request.gateway,
        &projected.messages,
        Some(&prompt_output.system_prompt),
        CompactConfig {
            keep_recent_turns: settings.compact_keep_recent_turns,
            trigger: request.trigger,
            summary_reserve_tokens: settings.summary_reserve_tokens,
            max_retry_attempts: settings.compact_max_retry_attempts,
            custom_instructions: request.instructions.map(str::to_string),
        },
        CancelToken::new(),
    )
    .await?
    else {
        return Ok(None);
    };

    let mut events = vec![compact_applied_event(
        None,
        &AgentEventContext::default(),
        request.trigger,
        compaction.summary,
        CompactAppliedStats {
            meta: compaction.meta,
            preserved_recent_turns: compaction.preserved_recent_turns,
            pre_tokens: compaction.pre_tokens,
            post_tokens_estimate: compaction.post_tokens_estimate,
            messages_removed: compaction.messages_removed,
            tokens_freed: compaction.tokens_freed,
        },
        compaction.timestamp,
    )];

    for message in file_access_tracker.build_recovery_messages(settings.file_recovery_config()) {
        let astrcode_core::LlmMessage::User { content, origin } = message else {
            continue;
        };
        events.push(StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::UserMessage {
                content,
                origin,
                timestamp: Utc::now(),
            },
        });
    }

    Ok(Some(events))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        CompactMode, EventTranslator, LlmFinishReason, LlmOutput, LlmProvider, LlmRequest,
        ModelLimits, Phase, PromptBuildOutput, PromptBuildRequest, PromptFactsProvider,
        PromptFactsRequest, PromptProvider, ResourceProvider, ResourceReadResult,
        ResourceRequestContext, Result, SessionId, StorageEventPayload, UserMessageOrigin,
    };
    use astrcode_kernel::Kernel;
    use async_trait::async_trait;

    use super::*;
    use crate::{
        actor::SessionActor, state::append_and_broadcast, turn::test_support::StubEventStore,
    };

    #[derive(Debug)]
    struct SummaryLlmProvider;

    #[async_trait]
    impl LlmProvider for SummaryLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<astrcode_core::LlmEventSink>,
        ) -> Result<LlmOutput> {
            Ok(LlmOutput {
                content: "<analysis>ok</analysis><summary>manual compact summary</summary>"
                    .to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
                usage: None,
                finish_reason: LlmFinishReason::Stop,
            })
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 64_000,
                max_output_tokens: 8_000,
            }
        }
    }

    #[derive(Debug)]
    struct ManualCompactPromptFactsProvider;

    #[async_trait]
    impl PromptFactsProvider for ManualCompactPromptFactsProvider {
        async fn resolve_prompt_facts(
            &self,
            _request: &PromptFactsRequest,
        ) -> Result<astrcode_core::PromptFacts> {
            Ok(astrcode_core::PromptFacts::default())
        }
    }

    #[derive(Debug)]
    struct TestPromptProvider;

    #[async_trait]
    impl PromptProvider for TestPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                cache_metrics: Default::default(),
                metadata: serde_json::Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct TestResourceProvider;

    #[async_trait]
    impl ResourceProvider for TestResourceProvider {
        async fn read_resource(
            &self,
            _uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: "noop://resource".to_string(),
                content: serde_json::Value::Null,
                metadata: serde_json::Value::Null,
            })
        }
    }

    fn summary_kernel() -> Arc<Kernel> {
        Arc::new(
            Kernel::builder()
                .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
                .with_llm_provider(Arc::new(SummaryLlmProvider))
                .with_prompt_provider(Arc::new(TestPromptProvider))
                .with_resource_provider(Arc::new(TestResourceProvider))
                .build()
                .expect("kernel should build"),
        )
    }

    #[tokio::test]
    async fn build_manual_compact_events_generates_real_summary_event() {
        let event_store = Arc::new(StubEventStore::default());
        let actor = SessionActor::new_persistent_with_lineage(
            SessionId::from("session-1".to_string()),
            ".".to_string(),
            "root-agent".into(),
            event_store,
            None,
            None,
        )
        .await
        .expect("actor should build");
        let mut translator = EventTranslator::new(Phase::Idle);

        append_and_broadcast(
            actor.state(),
            &StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::UserMessage {
                    content: "hello".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            },
            &mut translator,
        )
        .await
        .expect("user event should persist");
        append_and_broadcast(
            actor.state(),
            &StorageEvent {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::AssistantFinal {
                    content: "latest answer".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
            &mut translator,
        )
        .await
        .expect("assistant event should persist");

        let kernel = summary_kernel();
        let events = build_manual_compact_events(ManualCompactRequest {
            gateway: kernel.gateway(),
            prompt_facts_provider: &ManualCompactPromptFactsProvider,
            session_state: actor.state(),
            session_id: "session-1",
            working_dir: Path::new("."),
            runtime: &ResolvedRuntimeConfig::default(),
            trigger: astrcode_core::CompactTrigger::Manual,
            instructions: Some("保留错误和文件路径"),
        })
        .await
        .expect("manual compact should succeed")
        .expect("manual compact should produce events");

        assert!(matches!(
            &events[0].payload,
            StorageEventPayload::CompactApplied { summary, meta, .. }
                if summary == "manual compact summary"
                    && meta.mode == CompactMode::Full
                    && meta.instructions_present
        ));
    }
}
