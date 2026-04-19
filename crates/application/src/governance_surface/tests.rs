use std::sync::Arc;

use astrcode_core::{
    AllowAllPolicyEngine, ApprovalDefault, AstrError, CapabilityKind, CapabilitySpec, LlmMessage,
    LlmOutput, LlmProvider, LlmRequest, ModeId, ModelLimits, ModelRequest, PromptBuildOutput,
    PromptBuildRequest, PromptProvider, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig,
    ResourceProvider, ResourceReadResult, ResourceRequestContext, SideEffect, Stability,
    UserMessageOrigin,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    FreshChildGovernanceInput, GOVERNANCE_POLICY_REVISION, GovernanceApprovalPipeline,
    GovernanceBusyPolicy, GovernanceSurfaceAssembler, ResolvedGovernanceSurface,
    ResumedChildGovernanceInput, RootGovernanceInput, SessionGovernanceInput,
    build_inherited_messages, collaboration_policy_context, select_inherited_recent_tail,
};
use crate::{ExecutionControl, test_support::StubSessionPort};

#[derive(Debug)]
struct TestTool {
    name: &'static str,
}

#[async_trait]
impl astrcode_core::Tool for TestTool {
    fn definition(&self) -> astrcode_core::ToolDefinition {
        astrcode_core::ToolDefinition {
            name: self.name.to_string(),
            description: self.name.to_string(),
            parameters: json!({"type":"object"}),
        }
    }

    fn capability_spec(
        &self,
    ) -> std::result::Result<CapabilitySpec, astrcode_core::CapabilitySpecBuildError> {
        CapabilitySpec::builder(self.name, CapabilityKind::Tool)
            .description(self.name)
            .schema(json!({"type":"object"}), json!({"type":"object"}))
            .side_effect(SideEffect::Workspace)
            .stability(Stability::Stable)
            .build()
    }

    async fn execute(
        &self,
        tool_call_id: String,
        _input: Value,
        _ctx: &astrcode_core::ToolContext,
    ) -> astrcode_core::Result<astrcode_core::ToolExecutionResult> {
        Ok(astrcode_core::ToolExecutionResult {
            tool_call_id,
            tool_name: self.name.to_string(),
            ok: true,
            output: String::new(),
            child_ref: None,
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        })
    }
}

fn router() -> astrcode_kernel::CapabilityRouter {
    astrcode_kernel::CapabilityRouter::builder()
        .register_invoker(Arc::new(
            astrcode_kernel::ToolCapabilityInvoker::new(Arc::new(TestTool { name: "spawn" }))
                .expect("tool should build"),
        ))
        .register_invoker(Arc::new(
            astrcode_kernel::ToolCapabilityInvoker::new(Arc::new(TestTool { name: "readFile" }))
                .expect("tool should build"),
        ))
        .build()
        .expect("router should build")
}

#[derive(Debug)]
struct NoopLlmProvider;

#[async_trait]
impl LlmProvider for NoopLlmProvider {
    async fn generate(
        &self,
        _request: LlmRequest,
        _sink: Option<astrcode_core::LlmEventSink>,
    ) -> astrcode_core::Result<LlmOutput> {
        Err(AstrError::Validation("noop".to_string()))
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 32_000,
            max_output_tokens: 4_096,
        }
    }
}

#[derive(Debug)]
struct NoopPromptProvider;

#[async_trait]
impl PromptProvider for NoopPromptProvider {
    async fn build_prompt(
        &self,
        _request: PromptBuildRequest,
    ) -> astrcode_core::Result<PromptBuildOutput> {
        Ok(PromptBuildOutput {
            system_prompt: "noop".to_string(),
            system_prompt_blocks: Vec::new(),
            cache_metrics: Default::default(),
            metadata: json!({}),
        })
    }
}

#[derive(Debug)]
struct NoopResourceProvider;

#[async_trait]
impl ResourceProvider for NoopResourceProvider {
    async fn read_resource(
        &self,
        uri: &str,
        _context: &ResourceRequestContext,
    ) -> astrcode_core::Result<ResourceReadResult> {
        Ok(ResourceReadResult {
            uri: uri.to_string(),
            content: json!({}),
            metadata: json!({}),
        })
    }
}

#[test]
fn session_surface_builds_collaboration_prompt_and_policy_context() {
    let kernel = astrcode_kernel::Kernel::builder()
        .with_capabilities(router())
        .with_llm_provider(Arc::new(NoopLlmProvider))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build");
    let assembler = GovernanceSurfaceAssembler::default();
    let surface = assembler
        .session_surface(
            &kernel,
            SessionGovernanceInput {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                working_dir: ".".to_string(),
                profile: "coding".to_string(),
                mode_id: ModeId::code(),
                runtime: ResolvedRuntimeConfig::default(),
                control: None,
                extra_prompt_declarations: Vec::new(),
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )
        .expect("surface should build");

    assert_eq!(surface.governance_revision, GOVERNANCE_POLICY_REVISION);
    assert!(
        surface
            .prompt_declarations
            .iter()
            .any(|declaration| declaration.origin.as_deref()
                == Some("governance:collaboration-guide"))
    );
    assert_eq!(surface.prompt_facts_context().approval_mode, "inherit");
}

#[tokio::test]
async fn surface_policy_pipeline_defaults_to_allow_all() {
    let surface = ResolvedGovernanceSurface {
        mode_id: ModeId::code(),
        runtime: ResolvedRuntimeConfig::default(),
        capability_router: None,
        prompt_declarations: Vec::new(),
        resolved_limits: ResolvedExecutionLimitsSnapshot {
            allowed_tools: vec!["readFile".to_string()],
            max_steps: Some(4),
        },
        resolved_overrides: None,
        injected_messages: Vec::new(),
        policy_context: astrcode_core::PolicyContext {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            step_index: 0,
            working_dir: ".".to_string(),
            profile: "coding".to_string(),
            metadata: json!({}),
        },
        collaboration_policy: collaboration_policy_context(&ResolvedRuntimeConfig::default()),
        approval: GovernanceApprovalPipeline {
            pending: Some(astrcode_core::ApprovalPending {
                request: astrcode_core::ApprovalRequest {
                    request_id: "approval".to_string(),
                    session_id: "session-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    capability: CapabilitySpec::builder("placeholder", CapabilityKind::Tool)
                        .description("placeholder")
                        .schema(json!({"type":"object"}), json!({"type":"object"}))
                        .build()
                        .expect("placeholder should build"),
                    payload: Value::Null,
                    prompt: "disabled".to_string(),
                    default: ApprovalDefault::Allow,
                    metadata: json!({}),
                },
                action: astrcode_core::CapabilityCall {
                    request_id: "approval-call".to_string(),
                    capability: CapabilitySpec::builder("placeholder", CapabilityKind::Tool)
                        .description("placeholder")
                        .schema(json!({"type":"object"}), json!({"type":"object"}))
                        .build()
                        .expect("placeholder should build"),
                    payload: Value::Null,
                    metadata: json!({}),
                },
            }),
        },
        governance_revision: GOVERNANCE_POLICY_REVISION.to_string(),
        busy_policy: GovernanceBusyPolicy::BranchOnBusy,
        diagnostics: Vec::new(),
    };
    let request = ModelRequest {
        messages: vec![LlmMessage::User {
            content: "hello".to_string(),
            origin: UserMessageOrigin::User,
        }],
        tools: Vec::new(),
        system_prompt: Some("system".to_string()),
        system_prompt_blocks: Vec::new(),
    };
    let checked = surface
        .check_model_request(&AllowAllPolicyEngine, request)
        .await
        .expect("request should pass");
    assert_eq!(checked.system_prompt.as_deref(), Some("system"));
    assert!(surface.approval.pending.is_some());
}

#[test]
fn inherited_messages_follow_compact_and_tail_policy() {
    let inherited = build_inherited_messages(
        &[
            LlmMessage::User {
                content: "<summary>summary</summary>".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "turn-1".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "answer-1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::User {
                content: "turn-2".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        &astrcode_core::ResolvedSubagentContextOverrides {
            include_compact_summary: true,
            include_recent_tail: true,
            fork_mode: Some(astrcode_core::ForkMode::LastNTurns(1)),
            ..astrcode_core::ResolvedSubagentContextOverrides::default()
        },
    );
    assert_eq!(inherited.len(), 2);
}

#[test]
fn root_surface_applies_execution_control_without_special_case_logic() {
    let kernel = astrcode_kernel::Kernel::builder()
        .with_capabilities(router())
        .with_llm_provider(Arc::new(NoopLlmProvider))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build");
    let assembler = GovernanceSurfaceAssembler::default();
    let surface = assembler
        .root_surface(
            &kernel,
            RootGovernanceInput {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                working_dir: ".".to_string(),
                profile: "coding".to_string(),
                mode_id: ModeId::code(),
                runtime: ResolvedRuntimeConfig::default(),
                control: Some(ExecutionControl {
                    max_steps: Some(7),
                    manual_compact: None,
                }),
            },
        )
        .expect("surface should build");

    assert!(surface.capability_router.is_none());
    assert_eq!(surface.resolved_limits.max_steps, Some(7));
    assert_eq!(surface.busy_policy, GovernanceBusyPolicy::BranchOnBusy);
}

#[tokio::test]
async fn fresh_child_surface_restricts_tools_and_inherits_governance_defaults() {
    let kernel = astrcode_kernel::Kernel::builder()
        .with_capabilities(router())
        .with_llm_provider(Arc::new(NoopLlmProvider))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build");
    let assembler = GovernanceSurfaceAssembler::default();
    let session_runtime = StubSessionPort::default();
    let surface = assembler
        .fresh_child_surface(
            &kernel,
            &session_runtime,
            FreshChildGovernanceInput {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                working_dir: ".".to_string(),
                mode_id: ModeId::code(),
                runtime: ResolvedRuntimeConfig::default(),
                parent_allowed_tools: vec!["spawn".to_string(), "readFile".to_string()],
                capability_grant: Some(astrcode_core::SpawnCapabilityGrant {
                    allowed_tools: vec!["readFile".to_string()],
                }),
                description: "只做读取".to_string(),
                task: "inspect file".to_string(),
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )
        .await
        .expect("surface should build");

    assert_eq!(
        surface.resolved_limits.allowed_tools,
        vec!["readFile".to_string()]
    );
    assert!(surface.capability_router.is_some());
    assert!(
        surface
            .prompt_declarations
            .iter()
            .any(|declaration| declaration.origin.as_deref() == Some("child-contract:fresh"))
    );
}

#[test]
fn resumed_child_surface_reuses_existing_limits_and_contract_source() {
    let kernel = astrcode_kernel::Kernel::builder()
        .with_capabilities(router())
        .with_llm_provider(Arc::new(NoopLlmProvider))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .expect("kernel should build");
    let assembler = GovernanceSurfaceAssembler::default();
    let limits = ResolvedExecutionLimitsSnapshot {
        allowed_tools: vec!["readFile".to_string()],
        max_steps: Some(5),
    };
    let surface = assembler
        .resumed_child_surface(
            &kernel,
            ResumedChildGovernanceInput {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                working_dir: ".".to_string(),
                mode_id: ModeId::code(),
                runtime: ResolvedRuntimeConfig::default(),
                allowed_tools: Vec::new(),
                resolved_limits: limits.clone(),
                delegation: None,
                message: "continue with the same branch".to_string(),
                context: Some("keep scope tight".to_string()),
                busy_policy: GovernanceBusyPolicy::RejectOnBusy,
            },
        )
        .expect("surface should build");

    assert_eq!(surface.resolved_limits.allowed_tools, limits.allowed_tools);
    assert_eq!(surface.resolved_limits.max_steps, limits.max_steps);
    assert_eq!(surface.busy_policy, GovernanceBusyPolicy::RejectOnBusy);
    assert!(
        surface
            .prompt_declarations
            .iter()
            .any(|declaration| declaration.origin.as_deref() == Some("child-contract:resumed"))
    );
}

#[test]
fn select_inherited_recent_tail_keeps_full_history_without_fork_mode() {
    let messages = vec![
        LlmMessage::User {
            content: "hello".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "world".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
    ];

    assert_eq!(select_inherited_recent_tail(&messages, None), messages);
}
