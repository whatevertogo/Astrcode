use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::approval_service::ApprovalBroker;
use astrcode_core::{
    ApprovalDefault, ApprovalRequest, ApprovalResolution, AstrError, CancelToken, CapabilityCall,
    CapabilityRouter as CoreCapabilityRouter, ContextPressureInput, ContextStrategyDecision,
    ModelRequest, Phase, PluginManifest, PluginType, PolicyContext, PolicyEngine, PolicyVerdict,
    Result, Tool, ToolCapabilityMetadata, ToolContext,
};
use astrcode_plugin::Supervisor;
use astrcode_protocol::plugin::{PeerDescriptor, PeerRole};
use async_trait::async_trait;
use serde_json::json;
use tokio::time::{sleep, Duration};

use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::prompt::{
    BlockKind, BlockSpec, PromptComposer, PromptComposerOptions, PromptContext, PromptContribution,
    PromptContributor,
};
use crate::provider_factory::ProviderFactory;
use crate::test_support::{capabilities_from_tools, empty_capabilities, TestEnvGuard};
use astrcode_core::AgentState;
use astrcode_core::StorageEvent;
use astrcode_core::ToolRegistry;
use astrcode_core::{LlmMessage, ToolCallRequest, ToolDefinition, ToolExecutionResult};

use super::{AgentLoop, TurnOutcome};

fn make_state(user_text: &str) -> AgentState {
    AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![LlmMessage::User {
            content: user_text.into(),
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<LlmOutput>>,
    delay: Duration,
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        if self.delay > Duration::from_millis(0) {
            tokio::select! {
                _ = crate::llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = sleep(self.delay) => {}
            }
        }
        let response = self
            .responses
            .lock()
            .expect("lock should work")
            .pop_front()
            .ok_or_else(|| AstrError::Internal("no scripted response".to_string()))?;

        if let Some(sink) = sink {
            for delta in response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(response)
    }
}

struct SlowTool;

struct QuickTool;
struct StreamingTool;
struct CountingTool {
    executions: Arc<AtomicUsize>,
}

#[derive(Default)]
struct ConcurrencyTracker {
    active: AtomicUsize,
    max_active: AtomicUsize,
    started: AtomicUsize,
    cancelled: AtomicUsize,
}

struct ConcurrencyTrackingTool {
    name: &'static str,
    concurrency_safe: bool,
    tracker: Arc<ConcurrencyTracker>,
}

struct StreamingProvider {
    response: LlmOutput,
    per_delta_delay: Duration,
}

struct RecordingProvider {
    responses: Mutex<VecDeque<LlmOutput>>,
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

struct CountingPromptContributor {
    calls: Arc<AtomicUsize>,
}
struct RewriteSystemPromptPolicy {
    suffix: String,
}
struct DenyCapabilityPolicy {
    capability_name: String,
    reason: String,
}
struct AskCapabilityPolicy {
    capability_name: String,
    prompt: String,
    default: ApprovalDefault,
}
struct RecordingApprovalBroker {
    requests: Arc<Mutex<Vec<ApprovalRequest>>>,
    resolutions: Mutex<VecDeque<ApprovalResolution>>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(&self, _working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>> {
        Ok(self.provider.clone())
    }
}

#[async_trait]
impl PromptContributor for CountingPromptContributor {
    fn contributor_id(&self) -> &'static str {
        "counting-prompt"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        self.calls.fetch_add(1, Ordering::SeqCst);
        PromptContribution {
            blocks: vec![BlockSpec::system_text(
                "cached-block",
                BlockKind::Identity,
                "Cached",
                "cached",
            )],
            ..PromptContribution::default()
        }
    }
}

#[async_trait]
impl PolicyEngine for RewriteSystemPromptPolicy {
    async fn check_model_request(
        &self,
        mut request: ModelRequest,
        _ctx: &PolicyContext,
    ) -> Result<ModelRequest> {
        let base = request.system_prompt.take().unwrap_or_default();
        request.system_prompt = Some(format!("{base}\n{}", self.suffix).trim().to_string());
        Ok(request)
    }

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        _ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>> {
        Ok(PolicyVerdict::Allow(call))
    }

    async fn decide_context_strategy(
        &self,
        _input: ContextPressureInput,
        _ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision> {
        Ok(ContextStrategyDecision::Ignore)
    }
}

#[async_trait]
impl PolicyEngine for DenyCapabilityPolicy {
    async fn check_model_request(
        &self,
        request: ModelRequest,
        _ctx: &PolicyContext,
    ) -> Result<ModelRequest> {
        Ok(request)
    }

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        _ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>> {
        if call.name() == self.capability_name {
            Ok(PolicyVerdict::deny(self.reason.clone()))
        } else {
            Ok(PolicyVerdict::Allow(call))
        }
    }

    async fn decide_context_strategy(
        &self,
        _input: ContextPressureInput,
        _ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision> {
        Ok(ContextStrategyDecision::Ignore)
    }
}

#[async_trait]
impl PolicyEngine for AskCapabilityPolicy {
    async fn check_model_request(
        &self,
        request: ModelRequest,
        _ctx: &PolicyContext,
    ) -> Result<ModelRequest> {
        Ok(request)
    }

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>> {
        if call.name() == self.capability_name {
            let prompt = self.prompt.clone();
            let request = ApprovalRequest {
                request_id: call.request_id.clone(),
                session_id: ctx.session_id.clone(),
                turn_id: ctx.turn_id.clone(),
                capability: call.descriptor.clone(),
                payload: call.payload.clone(),
                prompt,
                default: self.default.clone(),
                metadata: serde_json::Value::Null,
            };
            Ok(PolicyVerdict::ask(request, call))
        } else {
            Ok(PolicyVerdict::Allow(call))
        }
    }

    async fn decide_context_strategy(
        &self,
        _input: ContextPressureInput,
        _ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision> {
        Ok(ContextStrategyDecision::Ignore)
    }
}

#[async_trait]
impl ApprovalBroker for RecordingApprovalBroker {
    async fn request(
        &self,
        request: ApprovalRequest,
        cancel: CancelToken,
    ) -> Result<ApprovalResolution> {
        if cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }
        self.requests
            .lock()
            .expect("approval requests lock")
            .push(request);
        Ok(self
            .resolutions
            .lock()
            .expect("approval resolutions lock")
            .pop_front()
            .unwrap_or_else(ApprovalResolution::approved))
    }
}

#[async_trait]
impl LlmProvider for StreamingProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let Some(sink) = sink else {
            return Ok(self.response.clone());
        };

        for delta in self.response.content.chars() {
            sink(LlmEvent::TextDelta(delta.to_string()));

            tokio::select! {
                _ = crate::llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = sleep(self.per_delta_delay) => {}
            }
        }

        Ok(self.response.clone())
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        self.requests
            .lock()
            .expect("lock should work")
            .push(request.clone());

        let response = self
            .responses
            .lock()
            .expect("lock should work")
            .pop_front()
            .ok_or_else(|| AstrError::Internal("no scripted response".to_string()))?;

        if request.cancel.is_cancelled() {
            return Err(AstrError::LlmInterrupted);
        }

        if let Some(sink) = sink {
            for delta in response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(response)
    }
}

#[async_trait]
impl Tool for SlowTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "slowTool".to_string(),
            description: "slow test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        tokio::select! {
            _ = crate::llm::cancelled(ctx.cancel().clone()) => Err(AstrError::Cancelled),
            _ = sleep(Duration::from_millis(250)) => Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "slowTool".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 250,
                truncated: false,
            })
        }
    }
}

#[async_trait]
impl Tool for QuickTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "quickTool".to_string(),
            description: "quick test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "quickTool".to_string(),
            ok: true,
            output: "ok".to_string(),
            error: None,
            metadata: None,
            duration_ms: 1,
            truncated: false,
        })
    }
}

#[async_trait]
impl Tool for StreamingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "streamingTool".to_string(),
            description: "streaming test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let _ = ctx.emit_stdout(tool_call_id.clone(), "streamingTool", "stdout line\n");
        sleep(Duration::from_millis(20)).await;
        let _ = ctx.emit_stderr(tool_call_id.clone(), "streamingTool", "stderr line\n");
        sleep(Duration::from_millis(20)).await;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "streamingTool".to_string(),
            ok: true,
            output: "[stdout]\nstdout line\n\n[stderr]\nstderr line\n".to_string(),
            error: None,
            metadata: Some(json!({
                "display": {
                    "kind": "terminal",
                    "command": "streaming-tool",
                    "exitCode": 0,
                }
            })),
            duration_ms: 1,
            truncated: false,
        })
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "policyTool".to_string(),
            description: "policy-aware test tool".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "policyTool".to_string(),
            ok: true,
            output: "counted".to_string(),
            error: None,
            metadata: None,
            duration_ms: 1,
            truncated: false,
        })
    }
}

#[async_trait]
impl Tool for ConcurrencyTrackingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: "tracks concurrent executions".to_string(),
            parameters: json!({
                "type":"object",
                "properties": {
                    "delayMs": { "type": "integer", "minimum": 0 }
                },
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .side_effect(if self.concurrency_safe {
                astrcode_core::SideEffectLevel::None
            } else {
                astrcode_core::SideEffectLevel::Workspace
            })
            .concurrency_safe(self.concurrency_safe)
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let delay_ms = args
            .get("delayMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(100);

        self.tracker.started.fetch_add(1, Ordering::SeqCst);
        let active_now = self.tracker.active.fetch_add(1, Ordering::SeqCst) + 1;
        update_max_active(&self.tracker.max_active, active_now);

        let run_result = tokio::select! {
            _ = crate::llm::cancelled(ctx.cancel().clone()) => {
                self.tracker.cancelled.fetch_add(1, Ordering::SeqCst);
                Err(AstrError::Cancelled)
            }
            _ = sleep(Duration::from_millis(delay_ms)) => {
                Ok(ToolExecutionResult {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: self.name.to_string(),
                    ok: true,
                    output: tool_call_id,
                    error: None,
                    metadata: None,
                    duration_ms: delay_ms,
                    truncated: false,
                })
            }
        };

        self.tracker.active.fetch_sub(1, Ordering::SeqCst);
        run_result
    }
}

fn update_max_active(max_active: &AtomicUsize, candidate: usize) {
    let mut observed = max_active.load(Ordering::SeqCst);
    while candidate > observed {
        match max_active.compare_exchange(observed, candidate, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(actual) => observed = actual,
        }
    }
}

#[tokio::test]
async fn tool_events_are_ordered_and_turn_finishes() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call1".to_string(),
                    name: "quickTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("list files");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };

    let outcome = loop_runner
        .run_turn(&state, "turn-1", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let start_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::ToolCall { .. }))
        .expect("ToolCall event expected");
    let result_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::ToolResult { .. }))
        .expect("ToolResult event expected");
    let done_idx = events
        .iter()
        .position(|e| matches!(e, StorageEvent::TurnDone { .. }))
        .expect("TurnDone event expected");

    assert!(start_idx < result_idx);
    assert!(result_idx < done_idx);
    assert!(matches!(
        &events[done_idx],
        StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
    ));
}

#[tokio::test]
async fn streaming_tool_emits_deltas_before_tool_result() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: "".to_string(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-stream".to_string(),
                    name: "streamingTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(StreamingTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("stream tool");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |event: StorageEvent| {
        events_clone.lock().expect("lock").push(event);
        Ok(())
    };

    let outcome = loop_runner
        .run_turn(&state, "turn-stream", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let call_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolCall { .. }))
        .expect("tool call event expected");
    let first_delta_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolCallDelta { .. }))
        .expect("tool call delta event expected");
    let result_idx = events
        .iter()
        .position(|event| matches!(event, StorageEvent::ToolResult { .. }))
        .expect("tool result event expected");

    assert!(call_idx < first_delta_idx);
    assert!(first_delta_idx < result_idx);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StorageEvent::ToolCallDelta { .. }))
            .count(),
        2,
        "streaming tool should emit both stdout and stderr deltas"
    );
    assert!(matches!(
        &events[first_delta_idx],
        StorageEvent::ToolCallDelta {
            tool_name,
            delta,
            ..
        } if tool_name == "streamingTool" && delta == "stdout line\n"
    ));
}

#[tokio::test]
async fn interrupt_emits_error_and_turn_done() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "".to_string(),
            tool_calls: vec![ToolCallRequest {
                id: "call-slow".to_string(),
                name: "slowTool".to_string(),
                args: json!({}),
            }],
            reasoning: None,
        }])),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder().register(Box::new(SlowTool)).build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("run slow");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();
    let events_clone = events.clone();

    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
    });

    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };
    let outcome = loop_runner
        .run_turn(&state, "turn-2", &mut on_event, cancel)
        .await
        .expect("turn should end cleanly");
    assert_eq!(outcome, TurnOutcome::Cancelled);
    cancel_task.await.expect("cancel task should join");

    let events = events.lock().expect("lock").clone();
    let has_error = events
        .iter()
        .any(|e| matches!(e, StorageEvent::Error { message, .. } if message == "interrupted"));
    let has_done = events.iter().any(|e| {
        matches!(
            e,
            StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("cancelled")
        )
    });

    assert!(has_error, "should have Error(interrupted)");
    assert!(has_done, "should have TurnDone");
}

#[tokio::test]
async fn concurrency_safe_tools_run_in_parallel() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-safe-1".to_string(),
                        name: "parallelSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                    ToolCallRequest {
                        id: "call-safe-2".to_string(),
                        name: "parallelSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                ],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "parallelSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("run parallel safe tools"),
            "turn-parallel-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(tracker.started.load(Ordering::SeqCst), 2);
    assert!(
        tracker.max_active.load(Ordering::SeqCst) >= 2,
        "safe tools should overlap in execution"
    );
}

#[tokio::test]
async fn unsafe_tools_remain_sequential() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-unsafe-1".to_string(),
                        name: "sequentialUnsafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                    ToolCallRequest {
                        id: "call-unsafe-2".to_string(),
                        name: "sequentialUnsafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                ],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "sequentialUnsafeTool",
            concurrency_safe: false,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("run unsafe tools"),
            "turn-sequential-unsafe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(tracker.started.load(Ordering::SeqCst), 2);
    assert_eq!(
        tracker.max_active.load(Ordering::SeqCst),
        1,
        "unsafe tools must never overlap"
    );
}

#[tokio::test]
async fn max_tool_concurrency_limits_safe_parallelism() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-limit-1".to_string(),
                        name: "limitedSafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                    ToolCallRequest {
                        id: "call-limit-2".to_string(),
                        name: "limitedSafeTool".to_string(),
                        args: json!({ "delayMs": 80 }),
                    },
                ],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "limitedSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    )
    .with_max_tool_concurrency(1);

    loop_runner
        .run_turn(
            &make_state("limit safe concurrency"),
            "turn-limit-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(
        tracker.max_active.load(Ordering::SeqCst),
        1,
        "configured concurrency limit should cap safe tool overlap"
    );
}

#[tokio::test]
async fn parallel_safe_tool_results_preserve_original_request_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-ordered-slow".to_string(),
                        name: "orderedSafeTool".to_string(),
                        args: json!({ "delayMs": 120 }),
                    },
                    ToolCallRequest {
                        id: "call-ordered-fast".to_string(),
                        name: "orderedSafeTool".to_string(),
                        args: json!({ "delayMs": 20 }),
                    },
                ],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        requests: Arc::clone(&requests),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "orderedSafeTool",
            concurrency_safe: true,
            tracker,
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );

    loop_runner
        .run_turn(
            &make_state("preserve tool order"),
            "turn-ordered-safe",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("recorded requests lock");
    let tool_messages = requests[1]
        .messages
        .iter()
        .filter_map(|message| match message {
            LlmMessage::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        tool_messages,
        vec![
            "call-ordered-slow".to_string(),
            "call-ordered-fast".to_string()
        ]
    );
}

#[tokio::test]
async fn cancellation_propagates_to_parallel_safe_tools() {
    let tracker = Arc::new(ConcurrencyTracker::default());
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: String::new(),
            tool_calls: vec![
                ToolCallRequest {
                    id: "call-cancel-1".to_string(),
                    name: "cancelSafeTool".to_string(),
                    args: json!({ "delayMs": 250 }),
                },
                ToolCallRequest {
                    id: "call-cancel-2".to_string(),
                    name: "cancelSafeTool".to_string(),
                    args: json!({ "delayMs": 250 }),
                },
            ],
            reasoning: None,
        }])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(ConcurrencyTrackingTool {
            name: "cancelSafeTool",
            concurrency_safe: true,
            tracker: Arc::clone(&tracker),
        }))
        .build();
    let loop_runner = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        capabilities_from_tools(tools),
    );
    let cancel = CancelToken::new();
    let cancel_clone = cancel.clone();

    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(40)).await;
        cancel_clone.cancel();
    });

    let outcome = loop_runner
        .run_turn(
            &make_state("cancel safe tools"),
            "turn-cancel-safe",
            &mut |_event| Ok(()),
            cancel,
        )
        .await
        .expect("turn should end cleanly");
    cancel_task.await.expect("cancel task should join");

    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(tracker.started.load(Ordering::SeqCst), 2);
    assert_eq!(
        tracker.cancelled.load(Ordering::SeqCst),
        2,
        "all running safe tools should observe cancellation"
    );
}

#[tokio::test]
async fn deltas_emit_before_stream_completion() {
    let provider = Arc::new(StreamingProvider {
        response: LlmOutput {
            content: "streamed".to_string(),
            tool_calls: vec![],
            reasoning: None,
        },
        per_delta_delay: Duration::from_millis(20),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities());
    let state = make_state("stream please");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_task = events.clone();

    let run_task = tokio::spawn(async move {
        let mut on_event = move |event: StorageEvent| {
            events_for_task.lock().expect("lock").push(event);
            Ok(())
        };

        loop_runner
            .run_turn(&state, "turn-3", &mut on_event, CancelToken::new())
            .await
            .expect("turn should complete");
    });

    tokio::time::timeout(Duration::from_millis(50), async {
        loop {
            if events
                .lock()
                .expect("lock")
                .iter()
                .any(|event| matches!(event, StorageEvent::AssistantDelta { .. }))
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("delta should be emitted before streaming completes");

    let snapshot = events.lock().expect("lock").clone();
    assert!(snapshot
        .iter()
        .any(|event| matches!(event, StorageEvent::AssistantDelta { .. })));
    assert!(
        !snapshot
            .iter()
            .any(|event| matches!(event, StorageEvent::TurnDone { .. })),
        "turn should still be in progress when first delta arrives"
    );

    run_task.await.expect("run task should join");
}

#[tokio::test]
async fn long_tool_chains_complete_without_a_step_cap() {
    let mut scripted = (0..8)
        .map(|idx| LlmOutput {
            content: format!("step-{idx}"),
            tool_calls: vec![ToolCallRequest {
                id: format!("call-{idx}"),
                name: "quickTool".to_string(),
                args: json!({}),
            }],
            reasoning: None,
        })
        .collect::<Vec<_>>();
    scripted.push(LlmOutput {
        content: "done".to_string(),
        tool_calls: vec![],
        reasoning: None,
    });

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from(scripted)),
        delay: Duration::from_millis(0),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = make_state("loop test");

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };

    let outcome = loop_runner
        .run_turn(&state, "turn-4", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");
    assert_eq!(outcome, TurnOutcome::Completed);

    let events = events.lock().expect("lock").clone();
    let tool_results = events
        .iter()
        .filter(|event| matches!(event, StorageEvent::ToolResult { .. }))
        .count();
    let has_turn_done = events.iter().any(|event| {
        matches!(
            event,
            StorageEvent::TurnDone { reason, .. } if reason.as_deref() == Some("completed")
        )
    });

    assert_eq!(tool_results, 8, "every scripted tool call should complete");
    assert!(
        has_turn_done,
        "completed turns should carry the completed reason"
    );
}

#[tokio::test]
async fn rebuilds_system_prompt_for_every_step_and_keeps_agents_rules_active() {
    let guard = TestEnvGuard::new();
    let project = tempfile::tempdir().expect("tempdir should be created");
    let user_agents_path = guard.home_dir().join(".astrcode").join("AGENTS.md");
    fs::create_dir_all(user_agents_path.parent().expect("parent should exist"))
        .expect("user agents dir should be created");
    fs::write(&user_agents_path, "Follow user rule").expect("user agents file should be written");
    fs::write(project.path().join("AGENTS.md"), "Follow project rule")
        .expect("project agents file should be written");

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "quickTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        requests: requests.clone(),
    });

    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools));
    let state = AgentState {
        session_id: "test".into(),
        working_dir: project.path().to_path_buf(),
        messages: vec![LlmMessage::User {
            content: "run quick tool".into(),
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    loop_runner
        .run_turn(&state, "turn-5", &mut |_event| Ok(()), CancelToken::new())
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("lock should work").clone();
    assert_eq!(requests.len(), 2, "expected one request per llm step");

    for request in &requests {
        let prompt = request
            .system_prompt
            .as_deref()
            .expect("system prompt should be present for every step");
        assert!(prompt.contains("[Identity]"));
        assert!(prompt.contains("[Environment]"));
        assert!(prompt.contains(&format!(
            "User-wide instructions from {}:\nFollow user rule",
            user_agents_path.display()
        )));
        assert!(prompt.contains(&format!(
            "Project-specific instructions from {}:\nFollow project rule",
            project.path().join("AGENTS.md").display()
        )));
        assert!(prompt.contains(&format!(
            "Working directory: {}",
            project.path().to_string_lossy()
        )));
        assert!(request.tools.iter().any(|tool| tool.name == "quickTool"));
    }

    assert_eq!(requests[0].messages.len(), 3);
    assert_eq!(requests[1].messages.len(), 3);
    assert!(matches!(
        &requests[0].messages[0],
        LlmMessage::User { content } if content == "Before changing code, inspect the relevant files and gather context first."
    ));
    assert!(matches!(
        &requests[0].messages[1],
        LlmMessage::Assistant { content, tool_calls, .. } if content == "I will inspect the relevant files and gather context before making changes." && tool_calls.is_empty()
    ));
    assert!(matches!(
        &requests[0].messages[2],
        LlmMessage::User { content } if content == "run quick tool"
    ));
    assert!(matches!(
        &requests[1].messages[1],
        LlmMessage::Assistant { tool_calls, .. } if tool_calls.len() == 1 && tool_calls[0].name == "quickTool"
    ));
    assert!(matches!(
        &requests[1].messages[2],
        LlmMessage::Tool { tool_call_id, content } if tool_call_id == "call-1" && content == "ok"
    ));
}

#[tokio::test]
async fn reuses_prompt_contributor_cache_across_llm_steps() {
    let _guard = TestEnvGuard::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let composer = PromptComposer::with_options(PromptComposerOptions {
        cache_ttl: Duration::from_secs(60),
        ..PromptComposerOptions::default()
    })
    .with_contributor(Arc::new(CountingPromptContributor {
        calls: calls.clone(),
    }));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-cache".to_string(),
                    name: "quickTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_prompt_composer(composer);
    let state = make_state("cache prompt");

    loop_runner
        .run_turn(
            &state,
            "turn-cache",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn event_sink_failures_abort_the_turn() {
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "done".to_string(),
            tool_calls: vec![],
            reasoning: None,
        }])),
        delay: Duration::from_millis(0),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities());
    let state = make_state("fail event sink");

    let result = loop_runner
        .run_turn(
            &state,
            "turn-6",
            &mut |_event| Err(AstrError::Internal("event sink failed".to_string())),
            CancelToken::new(),
        )
        .await;

    assert!(result.is_err());
    assert!(result
        .expect_err("result should be error")
        .to_string()
        .contains("event sink failed"));
}

#[tokio::test]
async fn policy_can_rewrite_model_request_before_provider_execution() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingProvider {
        responses: Mutex::new(VecDeque::from([LlmOutput {
            content: "done".to_string(),
            tool_calls: vec![],
            reasoning: None,
        }])),
        requests: Arc::clone(&requests),
    });
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, empty_capabilities())
        .with_policy_engine(Arc::new(RewriteSystemPromptPolicy {
            suffix: "[Policy Guardrail]".to_string(),
        }));

    loop_runner
        .run_turn(
            &make_state("rewrite prompt"),
            "turn-policy-rewrite",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    let requests = requests.lock().expect("recorded requests lock");
    let prompt = requests[0]
        .system_prompt
        .as_deref()
        .expect("system prompt should exist");
    assert!(prompt.contains("[Policy Guardrail]"));
}

#[tokio::test]
async fn denied_tool_calls_emit_failure_without_executing_tool() {
    let executions = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-deny".to_string(),
                    name: "policyTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(CountingTool {
            executions: Arc::clone(&executions),
        }))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(DenyCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            reason: "policy blocked tool".to_string(),
        }));
    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let mut on_event = move |event: StorageEvent| {
        events_clone.lock().expect("events lock").push(event);
        Ok(())
    };

    loop_runner
        .run_turn(
            &make_state("deny tool"),
            "turn-policy-deny",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(events.lock().expect("events lock").iter().any(|event| {
        matches!(
            event,
            StorageEvent::ToolResult {
                tool_name,
                success,
                error,
                ..
            } if tool_name == "policyTool"
                && !success
                && error.as_deref() == Some("policy blocked tool")
        )
    }));
}

#[tokio::test]
async fn ask_policy_uses_approval_broker_before_tool_execution() {
    let executions = Arc::new(AtomicUsize::new(0));
    let approval_requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-ask".to_string(),
                    name: "policyTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let broker = Arc::new(RecordingApprovalBroker {
        requests: Arc::clone(&approval_requests),
        resolutions: Mutex::new(VecDeque::from([ApprovalResolution::approved()])),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(CountingTool {
            executions: Arc::clone(&executions),
        }))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(AskCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            prompt: "Allow policyTool?".to_string(),
            default: ApprovalDefault::Deny,
        }))
        .with_approval_broker(broker);

    loop_runner
        .run_turn(
            &make_state("ask tool"),
            "turn-policy-ask",
            &mut |_event| Ok(()),
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let requests = approval_requests.lock().expect("approval requests lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].capability_name(), "policyTool");
    assert_eq!(requests[0].prompt, "Allow policyTool?");
}

#[tokio::test]
async fn denied_approval_returns_failed_tool_result_without_execution() {
    let executions = Arc::new(AtomicUsize::new(0));
    let approval_requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    id: "call-policy-ask-denied".to_string(),
                    name: "policyTool".to_string(),
                    args: json!({}),
                }],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });
    let broker = Arc::new(RecordingApprovalBroker {
        requests: Arc::clone(&approval_requests),
        resolutions: Mutex::new(VecDeque::from([ApprovalResolution::denied(
            "approval rejected in test",
        )])),
    });
    let tools = ToolRegistry::builder()
        .register(Box::new(CountingTool {
            executions: Arc::clone(&executions),
        }))
        .build();
    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities_from_tools(tools))
        .with_policy_engine(Arc::new(AskCapabilityPolicy {
            capability_name: "policyTool".to_string(),
            prompt: "Allow policyTool?".to_string(),
            default: ApprovalDefault::Allow,
        }))
        .with_approval_broker(broker);
    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let mut on_event = move |event: StorageEvent| {
        events_clone.lock().expect("events lock").push(event);
        Ok(())
    };

    loop_runner
        .run_turn(
            &make_state("deny approval"),
            "turn-policy-approval-deny",
            &mut on_event,
            CancelToken::new(),
        )
        .await
        .expect("turn should complete");

    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(
        approval_requests
            .lock()
            .expect("approval requests lock")
            .len(),
        1
    );
    assert!(events.lock().expect("events lock").iter().any(|event| {
        matches!(
            event,
            StorageEvent::ToolResult {
                tool_name,
                success,
                error,
                ..
            } if tool_name == "policyTool"
                && !success
                && error.as_deref() == Some("approval rejected in test")
        )
    }));
}

#[tokio::test]
async fn unified_capability_router_executes_builtin_and_plugin_tools() {
    let workspace = tempfile::tempdir().expect("tempdir should be created");
    fs::write(
        workspace.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .expect("Cargo.toml should be written");
    fs::create_dir_all(workspace.path().join("src")).expect("src dir should be created");
    fs::write(
        workspace.path().join("src").join("lib.rs"),
        "pub fn demo() {}\n",
    )
    .expect("lib.rs should be written");

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(VecDeque::from([
            LlmOutput {
                content: String::new(),
                tool_calls: vec![
                    ToolCallRequest {
                        id: "call-builtin".to_string(),
                        name: "quickTool".to_string(),
                        args: json!({}),
                    },
                    ToolCallRequest {
                        id: "call-plugin".to_string(),
                        name: "workspace.summary".to_string(),
                        args: json!({}),
                    },
                ],
                reasoning: None,
            },
            LlmOutput {
                content: "done".to_string(),
                tool_calls: vec![],
                reasoning: None,
            },
        ])),
        delay: Duration::from_millis(0),
    });

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime crate should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf();
    let manifest = PluginManifest {
        name: "repo-inspector".to_string(),
        version: "0.1.0".to_string(),
        description: "example plugin".to_string(),
        plugin_type: vec![PluginType::Tool],
        capabilities: vec![],
        executable: Some("cargo".to_string()),
        args: vec![
            "run".to_string(),
            "-p".to_string(),
            "astrcode-example-plugin".to_string(),
            "--quiet".to_string(),
        ],
        working_dir: Some(repo_root.to_string_lossy().into_owned()),
        repository: None,
    };
    let supervisor = Supervisor::start(
        &manifest,
        PeerDescriptor {
            id: "runtime-test-supervisor".to_string(),
            name: "runtime-test-supervisor".to_string(),
            role: PeerRole::Supervisor,
            version: env!("CARGO_PKG_VERSION").to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: serde_json::Value::Null,
        },
    )
    .await
    .expect("supervisor should start");

    let mut capability_builder = CoreCapabilityRouter::builder();
    for invoker in ToolRegistry::builder()
        .register(Box::new(QuickTool))
        .build()
        .into_capability_invokers()
        .expect("tool descriptors should build")
    {
        capability_builder = capability_builder.register_invoker(invoker);
    }
    for invoker in supervisor.capability_invokers() {
        capability_builder = capability_builder.register_invoker(invoker);
    }
    let capabilities = capability_builder
        .build()
        .expect("capability router should build");

    let factory = Arc::new(StaticProviderFactory { provider });
    let loop_runner = AgentLoop::from_capabilities(factory, capabilities);
    let state = AgentState {
        session_id: "test".into(),
        working_dir: workspace.path().to_path_buf(),
        messages: vec![LlmMessage::User {
            content: "summarize workspace".into(),
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    };

    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut on_event = move |e: StorageEvent| {
        events_clone.lock().expect("lock").push(e);
        Ok(())
    };

    loop_runner
        .run_turn(&state, "turn-plugin", &mut on_event, CancelToken::new())
        .await
        .expect("turn should complete");

    // 在关闭 supervisor 前先把断言结果提取出来，避免同步锁跨 await 持有。
    let (saw_quick_tool, saw_workspace_summary) = {
        let events = events.lock().expect("lock");
        let saw_quick_tool = events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_name,
                    output,
                    ..
                } if tool_name == "quickTool" && output == "ok"
            )
        });
        let saw_workspace_summary = events.iter().any(|event| {
            matches!(
                event,
                StorageEvent::ToolResult {
                    tool_name,
                    output,
                    ..
                } if tool_name == "workspace.summary"
                    && output.contains("Cargo.toml")
                    && output.contains("\"workspaceRoot\"")
            )
        });
        (saw_quick_tool, saw_workspace_summary)
    };
    assert!(saw_quick_tool);
    assert!(saw_workspace_summary);

    supervisor
        .shutdown()
        .await
        .expect("supervisor should shut down");
}
