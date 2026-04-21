use astrcode_core::{AgentEventContext, EventTranslator, Result, SessionId};
use chrono::Utc;

use crate::{
    SessionRuntime,
    state::append_and_broadcast,
    turn::{events::error_event, finalize::persist_pending_manual_compact_if_any},
};

impl SessionRuntime {
    pub async fn interrupt_session(&self, session_id: &str) -> Result<()> {
        let session_id = SessionId::from(crate::state::normalize_session_id(session_id));
        let actor = self.ensure_loaded_session(&session_id).await?;
        let Some(interrupted) = actor.turn_runtime().interrupt_if_running()? else {
            return Ok(());
        };
        let active_turn_id = interrupted.turn_id.clone();

        if let Some(active_turn_id) = active_turn_id.as_deref() {
            let cancelled = self
                .kernel
                .agent()
                .cancel_subruns_for_turn(active_turn_id)
                .await;
            if !cancelled.is_empty() {
                log::info!(
                    "cancelled {} subruns for interrupted turn '{}'",
                    cancelled.len(),
                    active_turn_id
                );
            }
        }

        let mut translator = EventTranslator::new(actor.state().current_phase()?);
        let event = error_event(
            active_turn_id.as_deref(),
            &AgentEventContext::default(),
            "interrupted".to_string(),
            Some(Utc::now()),
        );
        append_and_broadcast(actor.state(), &event, &mut translator).await?;
        persist_pending_manual_compact_if_any(
            self.kernel.gateway(),
            self.prompt_facts_provider.as_ref(),
            &self.event_store,
            actor.working_dir(),
            actor.turn_runtime(),
            actor.state(),
            session_id.as_str(),
            interrupted.pending_request,
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        LlmFinishReason, LlmOutput, LlmProvider, LlmRequest, ModelLimits, Phase, PromptBuildOutput,
        PromptBuildRequest, PromptFacts, PromptFactsProvider, PromptFactsRequest, PromptProvider,
        ResolvedRuntimeConfig, ResourceProvider, ResourceReadResult, ResourceRequestContext,
        Result, SessionTurnLease,
    };
    use astrcode_kernel::Kernel;
    use async_trait::async_trait;

    use crate::turn::test_support::{
        BranchingTestEventStore, append_root_turn_event_to_actor, assert_contains_compact_summary,
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
                prompt_cache_diagnostics: None,
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
    struct TestPromptProvider;

    #[async_trait]
    impl PromptProvider for TestPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                prompt_cache_hints: Default::default(),
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

    #[derive(Debug)]
    struct NoopPromptFactsProvider;

    struct StubTurnLease;

    impl SessionTurnLease for StubTurnLease {}

    #[async_trait]
    impl PromptFactsProvider for NoopPromptFactsProvider {
        async fn resolve_prompt_facts(&self, _request: &PromptFactsRequest) -> Result<PromptFacts> {
            Ok(PromptFacts::default())
        }
    }

    fn summary_runtime(event_store: Arc<dyn astrcode_core::EventStore>) -> crate::SessionRuntime {
        crate::SessionRuntime::new(
            Arc::new(
                Kernel::builder()
                    .with_capabilities(astrcode_kernel::CapabilityRouter::empty())
                    .with_llm_provider(Arc::new(SummaryLlmProvider))
                    .with_prompt_provider(Arc::new(TestPromptProvider))
                    .with_resource_provider(Arc::new(TestResourceProvider))
                    .build()
                    .expect("kernel should build"),
            ),
            Arc::new(NoopPromptFactsProvider),
            event_store,
            Arc::new(crate::turn::test_support::NoopMetrics),
        )
    }

    #[tokio::test]
    async fn interrupt_session_persists_pending_manual_compact() {
        let runtime = summary_runtime(Arc::new(BranchingTestEventStore::default()));
        let session = runtime
            .create_session(".")
            .await
            .expect("test session should be created");
        let session_id = session.session_id.clone();
        let actor = runtime
            .ensure_loaded_session(&astrcode_core::SessionId::from(session_id.clone()))
            .await
            .expect("session should load");
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_user_message_event("turn-0", "hello"),
        )
        .await;
        append_root_turn_event_to_actor(
            &actor,
            crate::turn::test_support::root_assistant_final_event("turn-0", "latest answer"),
        )
        .await;
        actor
            .turn_runtime()
            .request_manual_compact(crate::turn::PendingManualCompactRequest {
                runtime: ResolvedRuntimeConfig::default(),
                instructions: None,
            })
            .expect("manual compact flag should set");
        actor
            .turn_runtime()
            .prepare(
                session_id.as_str(),
                "turn-1",
                astrcode_core::CancelToken::new(),
                Box::new(StubTurnLease),
            )
            .expect("turn runtime should enter running state");

        runtime
            .interrupt_session(&session_id)
            .await
            .expect("interrupt should succeed");

        assert_eq!(
            actor
                .state()
                .current_phase()
                .expect("phase should be readable"),
            Phase::Interrupted
        );
        let stored = actor
            .state()
            .snapshot_recent_stored_events()
            .expect("stored events should be available");
        assert_contains_compact_summary(&stored, "manual compact summary");
    }
}
