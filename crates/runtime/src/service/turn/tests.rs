use std::{collections::VecDeque, sync::Arc, time::Duration};

use astrcode_core::{
    AgentEvent, AgentEventContext, AgentMode, CancelToken, EventTranslator, ExecutionOwner,
    InvocationKind, Phase, StorageEvent, StoredEvent, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{AgentLoop, ProviderFactory, TurnOutcome, estimate_text_tokens};
use astrcode_runtime_session::{
    SessionState, SessionTokenBudgetState, SessionWriter, append_and_broadcast,
    append_and_broadcast_from_turn_callback, recent_turn_event_tail,
};
use astrcode_storage::session::EventLog;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use super::{
    BudgetSettings,
    branch::{ensure_branch_depth_within_limit, stable_events_before_active_turn},
    orchestration::{complete_session_execution, execute_turn_chain},
};
use crate::{
    llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits},
    service::{RuntimeService, ServiceError, blocking_bridge::lock_anyhow},
    test_support::TestEnvGuard,
};

#[derive(Debug, Default, Clone, Copy)]
struct TestTurnExecutionStats {
    estimated_tokens_used: u64,
    pending_prompt_tokens: Option<u64>,
}

impl TestTurnExecutionStats {
    fn record_prompt_metrics(&mut self, estimated_tokens: u32) {
        self.pending_prompt_tokens = Some(estimated_tokens as u64);
    }

    fn record_assistant_output(&mut self, content: &str, reasoning_content: Option<&str>) {
        if let Some(prompt_tokens) = self.pending_prompt_tokens.take() {
            self.estimated_tokens_used = self.estimated_tokens_used.saturating_add(prompt_tokens);
        }
        let output_tokens =
            estimate_text_tokens(content) + reasoning_content.map_or(0, estimate_text_tokens);
        self.estimated_tokens_used = self
            .estimated_tokens_used
            .saturating_add(output_tokens as u64);
    }
}

fn observe_test_turn_event(stats: &mut TestTurnExecutionStats, event: &StorageEvent) {
    match event {
        StorageEvent::PromptMetrics {
            estimated_tokens,
            provider_input_tokens: None,
            ..
        } => stats.record_prompt_metrics(*estimated_tokens),
        StorageEvent::AssistantFinal {
            content,
            reasoning_content,
            ..
        } => stats.record_assistant_output(content, reasoning_content.as_deref()),
        _ => {},
    }
}

struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(
        &self,
        _working_dir: Option<std::path::PathBuf>,
    ) -> astrcode_core::Result<Arc<dyn LlmProvider>> {
        Ok(Arc::clone(&self.provider))
    }
}

struct ScriptedProvider {
    responses: std::sync::Mutex<VecDeque<LlmOutput>>,
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 128_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(
        &self,
        _request: LlmRequest,
        _sink: Option<EventSink>,
    ) -> astrcode_core::Result<LlmOutput> {
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| {
                astrcode_core::AstrError::Internal("missing scripted response".to_string())
            })
    }
}

struct DelayedProvider {
    delay: Duration,
}

#[async_trait]
impl LlmProvider for DelayedProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 128_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(
        &self,
        request: LlmRequest,
        _sink: Option<EventSink>,
    ) -> astrcode_core::Result<LlmOutput> {
        tokio::select! {
            _ = crate::llm::cancelled(request.cancel.clone()) => Err(astrcode_core::AstrError::LlmInterrupted),
            _ = tokio::time::sleep(self.delay) => Ok(LlmOutput {
                content: "done".to_string(),
                ..LlmOutput::default()
            }),
        }
    }
}

fn build_test_state() -> (tempfile::TempDir, SessionState, EventTranslator) {
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let log =
        EventLog::create("test-session", temp_dir.path()).expect("event log should be created");
    let state = SessionState::new(
        Phase::Idle,
        Arc::new(SessionWriter::new(Box::new(log))),
        Default::default(),
        Vec::new(),
        Vec::new(),
    );
    (temp_dir, state, EventTranslator::new(Phase::Idle))
}

#[tokio::test(flavor = "current_thread")]
async fn append_and_broadcast_from_turn_callback_works_on_current_thread_runtime() {
    let _guard = TestEnvGuard::new();
    let (temp_dir, state, mut translator) = build_test_state();
    let mut receiver = state.broadcaster.subscribe();

    append_and_broadcast_from_turn_callback(
        &state,
        &StorageEvent::SessionStart {
            session_id: "test-session".to_string(),
            timestamp: Utc::now(),
            working_dir: temp_dir.path().to_string_lossy().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        &mut translator,
    )
    .expect("append should succeed");

    let record = receiver.recv().await.expect("record should be broadcast");
    assert_eq!(record.event_id, "1.0");
    assert!(matches!(
        record.event,
        AgentEvent::SessionStarted { ref session_id } if session_id == "test-session"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn append_and_broadcast_from_turn_callback_works_on_multi_thread_runtime() {
    let _guard = TestEnvGuard::new();
    let (temp_dir, state, mut translator) = build_test_state();
    let mut receiver = state.broadcaster.subscribe();

    append_and_broadcast_from_turn_callback(
        &state,
        &StorageEvent::SessionStart {
            session_id: "test-session".to_string(),
            timestamp: Utc::now(),
            working_dir: temp_dir.path().to_string_lossy().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        &mut translator,
    )
    .expect("append should succeed on multi-thread runtimes too");

    let record = receiver.recv().await.expect("record should be broadcast");
    assert_eq!(record.event_id, "1.0");
}

#[test]
fn stable_events_before_active_turn_stops_at_the_active_turn_boundary() {
    let timestamp = Utc::now();
    let events = vec![
        StoredEvent {
            storage_seq: 1,
            event: StorageEvent::SessionStart {
                session_id: "session-1".to_string(),
                timestamp,
                working_dir: "D:/workspace".to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StoredEvent {
            storage_seq: 2,
            event: StorageEvent::UserMessage {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                content: "first".to_string(),
                origin: UserMessageOrigin::User,
                timestamp,
            },
        },
        StoredEvent {
            storage_seq: 3,
            event: StorageEvent::TurnDone {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                timestamp,
                reason: Some("completed".to_string()),
            },
        },
        StoredEvent {
            storage_seq: 4,
            event: StorageEvent::UserMessage {
                turn_id: Some("turn-2".to_string()),
                agent: AgentEventContext::default(),
                content: "second".to_string(),
                origin: UserMessageOrigin::User,
                timestamp,
            },
        },
        StoredEvent {
            storage_seq: 5,
            event: StorageEvent::ToolCall {
                turn_id: None,
                agent: AgentEventContext::default(),
                tool_call_id: "call-1".to_string(),
                tool_name: "echo".to_string(),
                args: json!({"message": "legacy event without turn id"}),
            },
        },
    ];

    let stable = stable_events_before_active_turn(&events, "turn-2");
    let stable_seq = stable
        .iter()
        .map(|event| event.storage_seq)
        .collect::<Vec<_>>();
    assert_eq!(stable_seq, vec![1, 2, 3]);
}

#[test]
fn recent_turn_event_tail_keeps_real_stored_tail_for_latest_turns() {
    let timestamp = Utc::now();
    let events = vec![
        StoredEvent {
            storage_seq: 1,
            event: StorageEvent::SessionStart {
                session_id: "session-1".to_string(),
                timestamp,
                working_dir: "D:/workspace".to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
        StoredEvent {
            storage_seq: 2,
            event: StorageEvent::UserMessage {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                content: "first".to_string(),
                origin: UserMessageOrigin::User,
                timestamp,
            },
        },
        StoredEvent {
            storage_seq: 3,
            event: StorageEvent::AssistantFinal {
                turn_id: Some("turn-1".to_string()),
                agent: AgentEventContext::default(),
                content: "done".to_string(),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: Some(timestamp),
            },
        },
        StoredEvent {
            storage_seq: 4,
            event: StorageEvent::UserMessage {
                turn_id: Some("turn-2".to_string()),
                agent: AgentEventContext::default(),
                content: "second".to_string(),
                origin: UserMessageOrigin::User,
                timestamp,
            },
        },
        StoredEvent {
            storage_seq: 5,
            event: StorageEvent::ToolResult {
                turn_id: Some("turn-2".to_string()),
                agent: AgentEventContext::default(),
                tool_call_id: "call-1".to_string(),
                tool_name: "echo".to_string(),
                output: "result".to_string(),
                success: true,
                error: None,
                metadata: None,
                duration_ms: 12,
            },
        },
        StoredEvent {
            storage_seq: 6,
            event: StorageEvent::PromptMetrics {
                turn_id: Some("turn-2".to_string()),
                agent: AgentEventContext::default(),
                step_index: 0,
                estimated_tokens: 128,
                context_window: 4096,
                effective_window: 4096,
                threshold_tokens: 3584,
                truncated_tool_results: 0,
                provider_input_tokens: None,
                provider_output_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        },
    ];

    let tail = recent_turn_event_tail(&events, 1);
    let tail_seq = tail
        .into_iter()
        .map(|stored| stored.storage_seq)
        .collect::<Vec<_>>();
    assert_eq!(tail_seq, vec![4, 5]);
}

#[test]
fn branch_depth_guard_rejects_unbounded_branch_chains() {
    let error = ensure_branch_depth_within_limit(3)
        .expect_err("depth at the configured limit should be rejected");

    assert!(matches!(error, ServiceError::Conflict(_)));
    assert!(
        error
            .to_string()
            .contains("too many concurrent branch attempts")
    );
}

#[test]
fn prompt_metrics_only_charge_budget_after_a_real_model_response() {
    let mut stats = TestTurnExecutionStats::default();

    observe_test_turn_event(
        &mut stats,
        &StorageEvent::PromptMetrics {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            step_index: 0,
            estimated_tokens: 800,
            context_window: 100_000,
            effective_window: 80_000,
            threshold_tokens: 72_000,
            truncated_tool_results: 0,
            provider_input_tokens: None,
            provider_output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        },
    );
    assert_eq!(stats.estimated_tokens_used, 0);

    observe_test_turn_event(
        &mut stats,
        &StorageEvent::AssistantFinal {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            content: "done".to_string(),
            reasoning_content: None,
            reasoning_signature: None,
            timestamp: None,
        },
    );

    assert!(stats.estimated_tokens_used >= 800);
}

#[tokio::test(flavor = "current_thread")]
async fn execute_turn_chain_appends_a_single_auto_continue_nudge_before_stopping() {
    let _guard = TestEnvGuard::new();
    let (temp_dir, state, mut translator) = build_test_state();
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        responses: std::sync::Mutex::new(VecDeque::from([
            LlmOutput {
                content: "a".repeat(240),
                ..LlmOutput::default()
            },
            LlmOutput {
                content: "done".to_string(),
                ..LlmOutput::default()
            },
        ])),
    });
    let loop_ = astrcode_runtime_agent_loop::AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        crate::test_support::empty_capabilities(),
    );

    append_and_broadcast(
        &state,
        &StorageEvent::SessionStart {
            session_id: "test-session".to_string(),
            timestamp: Utc::now(),
            working_dir: temp_dir.path().to_string_lossy().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        &mut translator,
    )
    .await
    .expect("session start should persist");
    append_and_broadcast(
        &state,
        &StorageEvent::UserMessage {
            turn_id: Some("turn-auto".to_string()),
            agent: AgentEventContext::default(),
            content: "work ".repeat(200),
            origin: UserMessageOrigin::User,
            timestamp: Utc::now(),
        },
        &mut translator,
    )
    .await
    .expect("user message should persist");

    *lock_anyhow(&state.token_budget, "session token budget").expect("budget lock") =
        Some(SessionTokenBudgetState {
            total_budget: 1_000,
            used_tokens: 850,
            continuation_count: 0,
        });

    let outcome = execute_turn_chain(
        &state,
        &loop_,
        "turn-auto",
        CancelToken::new(),
        &mut translator,
        AgentEventContext::root_execution("test-agent", "test-profile"),
        ExecutionOwner::root("session-test", "turn-auto", InvocationKind::RootExecution),
        BudgetSettings {
            continuation_min_delta_tokens: 1,
            max_continuations: 1,
        },
    )
    .await
    .expect("turn chain should complete");

    assert!(matches!(outcome, TurnOutcome::Completed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_cascades_to_registered_child_agents() {
    let _guard = TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let service = Arc::new(
        RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
            .expect("service should build"),
    );
    let loop_ = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory {
            provider: Arc::new(DelayedProvider {
                delay: Duration::from_secs(30),
            }),
        }),
        crate::test_support::empty_capabilities(),
    );
    *service.loop_.write().await = Arc::new(loop_);

    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");
    let accepted = service
        .execution()
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should be accepted");

    let control = service.agent_control();
    let child = control
        .spawn(
            &astrcode_core::AgentProfile {
                id: "review".to_string(),
                name: "Review".to_string(),
                description: "review".to_string(),
                mode: AgentMode::SubAgent,
                system_prompt: None,
                allowed_tools: vec!["readFile".to_string()],
                disallowed_tools: Vec::new(),
                model_preference: None,
            },
            &session.session_id,
            Some(accepted.turn_id.clone()),
            None,
        )
        .await
        .expect("child spawn should succeed");
    let _ = control.mark_running(&child.agent_id).await;

    service
        .execution()
        .interrupt_session(&session.session_id)
        .await
        .expect("interrupt should succeed");

    let child_handle = control
        .wait(&child.agent_id)
        .await
        .expect("child should still exist");
    assert_eq!(child_handle.status, astrcode_core::AgentStatus::Cancelled);
}

#[tokio::test(flavor = "current_thread")]
async fn complete_session_execution_keeps_background_child_agents_alive() {
    let _guard = TestEnvGuard::new();
    let (temp_dir, state, mut translator) = build_test_state();
    let control = astrcode_runtime_agent_control::AgentControl::new();

    append_and_broadcast(
        &state,
        &StorageEvent::SessionStart {
            session_id: "test-session".to_string(),
            timestamp: Utc::now(),
            working_dir: temp_dir.path().to_string_lossy().to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
        },
        &mut translator,
    )
    .await
    .expect("session start should persist");

    let child = control
        .spawn(
            &astrcode_core::AgentProfile {
                id: "explore".to_string(),
                name: "Explore".to_string(),
                description: "explore".to_string(),
                mode: AgentMode::SubAgent,
                system_prompt: None,
                allowed_tools: vec!["readFile".to_string()],
                disallowed_tools: Vec::new(),
                model_preference: None,
            },
            "test-session",
            Some("turn-parent".to_string()),
            None,
        )
        .await
        .expect("child spawn should succeed");
    let _ = control.mark_running(&child.agent_id).await;

    complete_session_execution(&state, Phase::Idle).await;

    let child_handle = control
        .get(&child.agent_id)
        .await
        .expect("child should remain registered");
    assert_eq!(child_handle.status, astrcode_core::AgentStatus::Running);
}

#[tokio::test(flavor = "current_thread")]
async fn compact_session_rejects_busy_sessions() {
    let _guard = TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let service = Arc::new(
        RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
            .expect("service should build"),
    );
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let state = service
        .sessions
        .get(&session.session_id)
        .expect("session state should be loaded");
    state
        .running
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let error = service
        .sessions()
        .compact(&session.session_id)
        .await
        .expect_err("busy session should reject manual compact");

    assert!(matches!(error, ServiceError::Conflict(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn compact_session_rejects_sessions_without_compressible_history() {
    let _guard = TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let service = Arc::new(
        RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
            .expect("service should build"),
    );
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let error = service
        .sessions()
        .compact(&session.session_id)
        .await
        .expect_err("empty session should not have compressible history");

    assert!(matches!(error, ServiceError::InvalidInput(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_prompt_surface_returns_accepted_shape() {
    let _guard = TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let service = Arc::new(
        RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
            .expect("service should build"),
    );
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    let accepted = service
        .execution()
        .submit_prompt(&session.session_id, "hello".to_string())
        .await
        .expect("prompt should be accepted");

    assert_eq!(accepted.session_id, session.session_id);
    assert!(!accepted.turn_id.is_empty());
    assert!(accepted.branched_from_session_id.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn interrupt_surface_is_idempotent_for_idle_session() {
    let _guard = TestEnvGuard::new();
    let temp_dir = tempfile::tempdir().expect("tempdir should be created");
    let service = Arc::new(
        RuntimeService::from_capabilities(crate::test_support::empty_capabilities())
            .expect("service should build"),
    );
    let session = service
        .sessions()
        .create(temp_dir.path())
        .await
        .expect("session should be created");

    service
        .execution()
        .interrupt_session(&session.session_id)
        .await
        .expect("interrupt should be a no-op for idle sessions");
}
