//! 测试夹具：共享的 Provider、Tool、Policy、Broker 等测试基础设施。

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{
    AgentState, ApprovalDefault, ApprovalRequest, ApprovalResolution, AstrError, CancelToken,
    CapabilityCall, ChildAgentRef, ChildSessionNode, CompactionHookResultContext, HookEvent,
    HookHandler, HookInput, HookOutcome, LlmMessage, ModelRequest, Phase, PolicyContext,
    PolicyEngine, PolicyVerdict, Result, StorageEvent, Tool, ToolCapabilityMetadata, ToolContext,
    ToolDefinition, ToolExecutionResult, ToolHookContext, ToolHookResultContext, UserMessageOrigin,
};
use astrcode_protocol::capability::SideEffectLevel;
use astrcode_runtime_llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest, ModelLimits};
use astrcode_runtime_prompt::{
    BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor,
};
use serde_json::json;
use tokio::time::{Duration, sleep};

use crate::{ApprovalBroker, provider_factory::ProviderFactory};

// ---------------------------------------------------------------------------
// AgentState 工厂
// ---------------------------------------------------------------------------

pub fn make_state(user_text: &str) -> AgentState {
    AgentState {
        session_id: "test".into(),
        working_dir: std::env::temp_dir(),
        messages: vec![LlmMessage::User {
            content: user_text.into(),
            origin: UserMessageOrigin::User,
        }],
        phase: Phase::Thinking,
        turn_count: 0,
    }
}

#[allow(dead_code)]
pub fn child_session_fixture(seed: &str) -> ChildSessionNode {
    astrcode_core::test_support::child_session_node_fixture(seed)
}

#[allow(dead_code)]
pub fn child_agent_ref_fixture(seed: &str) -> ChildAgentRef {
    child_session_fixture(seed).child_ref()
}

// ---------------------------------------------------------------------------
// Provider 工厂
// ---------------------------------------------------------------------------

pub struct StaticProviderFactory {
    pub provider: Arc<dyn LlmProvider>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(
        &self,
        _working_dir: Option<std::path::PathBuf>,
    ) -> Result<Arc<dyn LlmProvider>> {
        Ok(self.provider.clone())
    }
}

// ---------------------------------------------------------------------------
// ScriptedProvider — 返回预设响应序列
// ---------------------------------------------------------------------------

pub struct ScriptedProvider {
    pub responses: Mutex<VecDeque<LlmOutput>>,
    pub delay: Duration,
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        if self.delay > Duration::from_millis(0) {
            tokio::select! {
                _ = astrcode_runtime_llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
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

// ---------------------------------------------------------------------------
// FailingProvider — 支持返回错误的脚本 Provider
// ---------------------------------------------------------------------------

pub struct FailingProvider {
    pub results: Mutex<VecDeque<Result<LlmOutput>>>,
    pub delay: Duration,
}

#[async_trait::async_trait]
impl LlmProvider for FailingProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(&self, _request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        if self.delay > Duration::from_millis(0) {
            sleep(self.delay).await;
        }
        let result = self
            .results
            .lock()
            .expect("lock should work")
            .pop_front()
            .ok_or_else(|| AstrError::Internal("no scripted result".to_string()))?;

        if let (Ok(response), Some(sink)) = (&result, sink) {
            for delta in response.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// StreamingProvider — 模拟流式响应
// ---------------------------------------------------------------------------

pub struct StreamingProvider {
    pub response: LlmOutput,
    pub per_delta_delay: Duration,
}

#[async_trait::async_trait]
impl LlmProvider for StreamingProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let Some(sink) = sink else {
            return Ok(self.response.clone());
        };

        for delta in self.response.content.chars() {
            sink(LlmEvent::TextDelta(delta.to_string()));

            tokio::select! {
                _ = astrcode_runtime_llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = sleep(self.per_delta_delay) => {}
            }
        }

        Ok(self.response.clone())
    }
}

// ---------------------------------------------------------------------------
// RecordingProvider — 记录所有请求
// ---------------------------------------------------------------------------

pub struct RecordingProvider {
    pub responses: Mutex<VecDeque<LlmOutput>>,
    pub requests: Arc<Mutex<Vec<LlmRequest>>>,
}

#[async_trait::async_trait]
impl LlmProvider for RecordingProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

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

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

pub struct SlowTool;

#[async_trait::async_trait]
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
            _ = astrcode_runtime_llm::cancelled(ctx.cancel().clone()) => Err(AstrError::Cancelled),
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

pub struct QuickTool;

#[async_trait::async_trait]
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

pub struct StreamingTool;

#[async_trait::async_trait]
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

pub struct CountingTool {
    pub executions: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
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

pub struct EchoArgsTool;

#[async_trait::async_trait]
impl Tool for EchoArgsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "echoArgsTool".to_string(),
            description: "echoes JSON args".to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "echoArgsTool".to_string(),
            ok: true,
            output: args.to_string(),
            error: None,
            metadata: None,
            duration_ms: 1,
            truncated: false,
        })
    }
}

pub struct FailingExecutionTool;

#[async_trait::async_trait]
impl Tool for FailingExecutionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "failingExecutionTool".to_string(),
            description: "returns a structured tool failure".to_string(),
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
            tool_name: "failingExecutionTool".to_string(),
            ok: false,
            output: String::new(),
            error: Some("tool failed".to_string()),
            metadata: None,
            duration_ms: 1,
            truncated: false,
        })
    }
}

pub struct ConcurrencyTrackingTool {
    pub name: &'static str,
    pub concurrency_safe: bool,
    pub tracker: Arc<ConcurrencyTracker>,
}

#[derive(Default)]
pub struct ConcurrencyTracker {
    pub active: AtomicUsize,
    pub max_active: AtomicUsize,
    pub started: AtomicUsize,
    pub cancelled: AtomicUsize,
}

#[async_trait::async_trait]
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
                SideEffectLevel::None
            } else {
                SideEffectLevel::Workspace
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
            _ = astrcode_runtime_llm::cancelled(ctx.cancel().clone()) => {
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

// ---------------------------------------------------------------------------
// Policy 实现
// ---------------------------------------------------------------------------

pub struct RewriteSystemPromptPolicy {
    pub suffix: String,
}

#[async_trait::async_trait]
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
        input: &astrcode_core::ContextDecisionInput,
        _ctx: &PolicyContext,
    ) -> Result<astrcode_core::ContextStrategy> {
        Ok(input.suggested_strategy)
    }
}

pub struct DenyCapabilityPolicy {
    pub capability_name: String,
    pub reason: String,
}

#[async_trait::async_trait]
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
        input: &astrcode_core::ContextDecisionInput,
        _ctx: &PolicyContext,
    ) -> Result<astrcode_core::ContextStrategy> {
        Ok(input.suggested_strategy)
    }
}

pub struct AskCapabilityPolicy {
    pub capability_name: String,
    pub prompt: String,
    pub default: ApprovalDefault,
}

#[async_trait::async_trait]
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
        input: &astrcode_core::ContextDecisionInput,
        _ctx: &PolicyContext,
    ) -> Result<astrcode_core::ContextStrategy> {
        Ok(input.suggested_strategy)
    }
}

// ---------------------------------------------------------------------------
// ApprovalBroker
// ---------------------------------------------------------------------------

pub struct RecordingApprovalBroker {
    pub requests: Arc<Mutex<Vec<ApprovalRequest>>>,
    pub resolutions: Mutex<VecDeque<ApprovalResolution>>,
}

pub struct ReplaceArgsHook {
    pub tool_name: &'static str,
    pub replacement: serde_json::Value,
}

#[async_trait::async_trait]
impl HookHandler for ReplaceArgsHook {
    fn name(&self) -> &str {
        "replace-args-hook"
    }

    fn event(&self) -> HookEvent {
        HookEvent::PreToolUse
    }

    fn matches(&self, input: &HookInput) -> bool {
        matches!(
            input,
            HookInput::PreToolUse(ToolHookContext { tool_name, .. }) if tool_name == self.tool_name
        )
    }

    async fn run(&self, _input: &HookInput) -> Result<HookOutcome> {
        Ok(HookOutcome::ReplaceToolArgs {
            args: self.replacement.clone(),
        })
    }
}

pub struct BlockingToolHook {
    pub tool_name: &'static str,
    pub reason: &'static str,
}

#[async_trait::async_trait]
impl HookHandler for BlockingToolHook {
    fn name(&self) -> &str {
        "blocking-tool-hook"
    }

    fn event(&self) -> HookEvent {
        HookEvent::PreToolUse
    }

    fn matches(&self, input: &HookInput) -> bool {
        matches!(
            input,
            HookInput::PreToolUse(ToolHookContext { tool_name, .. }) if tool_name == self.tool_name
        )
    }

    async fn run(&self, _input: &HookInput) -> Result<HookOutcome> {
        Ok(HookOutcome::Block {
            reason: self.reason.to_string(),
        })
    }
}

pub struct RecordingToolHook {
    pub event: HookEvent,
    pub hits: Arc<Mutex<Vec<ToolHookResultContext>>>,
}

#[async_trait::async_trait]
impl HookHandler for RecordingToolHook {
    fn name(&self) -> &str {
        "recording-tool-hook"
    }

    fn event(&self) -> HookEvent {
        self.event
    }

    async fn run(&self, input: &HookInput) -> Result<HookOutcome> {
        match input {
            HookInput::PostToolUse(context) | HookInput::PostToolUseFailure(context) => {
                self.hits
                    .lock()
                    .expect("tool hook hits lock")
                    .push(context.clone());
            },
            _ => {},
        }
        Ok(HookOutcome::Continue)
    }
}

pub struct RecordingCompactHook {
    pub event: HookEvent,
    pub pre_hits: Arc<Mutex<Vec<astrcode_core::CompactionHookContext>>>,
    pub post_hits: Arc<Mutex<Vec<CompactionHookResultContext>>>,
}

#[async_trait::async_trait]
impl HookHandler for RecordingCompactHook {
    fn name(&self) -> &str {
        "recording-compact-hook"
    }

    fn event(&self) -> HookEvent {
        self.event
    }

    async fn run(&self, input: &HookInput) -> Result<HookOutcome> {
        match input {
            HookInput::PreCompact(context) => {
                self.pre_hits
                    .lock()
                    .expect("pre compact hits lock")
                    .push(context.clone());
            },
            HookInput::PostCompact(context) => {
                self.post_hits
                    .lock()
                    .expect("post compact hits lock")
                    .push(context.clone());
            },
            _ => {},
        }
        Ok(HookOutcome::Continue)
    }
}

#[async_trait::async_trait]
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

// ---------------------------------------------------------------------------
// PromptContributor
// ---------------------------------------------------------------------------

pub struct CountingPromptContributor {
    pub calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
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

// ---------------------------------------------------------------------------
// 事件收集辅助函数
// ---------------------------------------------------------------------------

pub fn collect_events() -> (
    Arc<Mutex<Vec<StorageEvent>>>,
    impl FnMut(StorageEvent) -> Result<()>,
) {
    let events: Arc<Mutex<Vec<StorageEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let on_event = move |event: StorageEvent| {
        events_clone.lock().expect("events lock").push(event);
        Ok(())
    };
    (events, on_event)
}
