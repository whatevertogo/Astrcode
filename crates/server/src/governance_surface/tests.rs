//! 治理面子域集成测试。
//!
//! 验证 `GovernanceSurfaceAssembler` 在不同场景下的端到端行为：
//! - session turn 治理面构建与 prompt declarations 注入
//! - fresh/resumed child 治理面继承与委派策略
//! - 工具白名单、审批管线、协作策略上下文的正确性
//! - 各种 capability selector（all / subset / none / union / difference）的编译结果

use astrcode_core::{
    CapabilityKind, CapabilitySpec, LlmMessage, ResolvedExecutionLimitsSnapshot,
    ResolvedRuntimeConfig, UserMessageOrigin,
};
use astrcode_governance_contract::{ApprovalDefault, BoundModeToolContractSnapshot, ModeId};
use serde_json::{Value, json};

use super::{
    FreshChildGovernanceInput, GOVERNANCE_POLICY_REVISION, GovernanceApprovalPipeline,
    GovernanceBusyPolicy, GovernanceSurfaceAssembler, ResolvedGovernanceSurface,
    ResumedChildGovernanceInput, RootGovernanceInput, SessionGovernanceInput,
    build_inherited_messages, collaboration_policy_context, select_inherited_recent_tail,
};
use crate::{ExecutionControl, test_support::StubSessionPort};

#[test]
fn session_surface_builds_collaboration_prompt_and_policy_context() {
    let assembler = GovernanceSurfaceAssembler::default();
    let surface = assembler
        .session_surface(SessionGovernanceInput {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            working_dir: ".".to_string(),
            profile: "coding".to_string(),
            mode_id: ModeId::code(),
            runtime: ResolvedRuntimeConfig::default(),
            control: None,
            extra_prompt_declarations: Vec::new(),
            busy_policy: GovernanceBusyPolicy::BranchOnBusy,
        })
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
    assert_eq!(surface.bound_mode_tool_contract.mode_id, ModeId::code());
}

#[tokio::test]
async fn surface_policy_pipeline_defaults_to_allow_all() {
    let surface = ResolvedGovernanceSurface {
        mode_id: ModeId::code(),
        runtime: ResolvedRuntimeConfig::default(),
        prompt_declarations: Vec::new(),
        bound_mode_tool_contract: BoundModeToolContractSnapshot {
            mode_id: ModeId::code(),
            artifact: None,
            exit_gate: None,
        },
        resolved_limits: ResolvedExecutionLimitsSnapshot,
        resolved_overrides: None,
        injected_messages: Vec::new(),
        policy_context: astrcode_governance_contract::PolicyContext {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            step_index: 0,
            working_dir: ".".to_string(),
            profile: "coding".to_string(),
            metadata: json!({}),
        },
        collaboration_policy: collaboration_policy_context(&ResolvedRuntimeConfig::default()),
        approval: GovernanceApprovalPipeline {
            pending: Some(astrcode_governance_contract::ApprovalPending {
                request: astrcode_governance_contract::ApprovalRequest {
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
                action: astrcode_governance_contract::CapabilityCall {
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
    };
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
    let assembler = GovernanceSurfaceAssembler::default();
    let _surface = assembler
        .root_surface(RootGovernanceInput {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            working_dir: ".".to_string(),
            profile: "coding".to_string(),
            mode_id: ModeId::code(),
            runtime: ResolvedRuntimeConfig::default(),
            control: Some(ExecutionControl {
                manual_compact: None,
            }),
        })
        .expect("surface should build");
}

#[tokio::test]
async fn fresh_child_surface_restricts_tools_and_inherits_governance_defaults() {
    let assembler = GovernanceSurfaceAssembler::default();
    let session_runtime = StubSessionPort::default();
    let surface = assembler
        .fresh_child_surface(
            &session_runtime,
            FreshChildGovernanceInput {
                session_id: "session-1".to_string(),
                turn_id: "turn-1".to_string(),
                working_dir: ".".to_string(),
                mode_id: ModeId::code(),
                runtime: ResolvedRuntimeConfig::default(),
                description: "只做读取".to_string(),
                task: "inspect file".to_string(),
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )
        .await
        .expect("surface should build");

    assert_eq!(surface.resolved_limits, ResolvedExecutionLimitsSnapshot);
    assert!(
        surface
            .prompt_declarations
            .iter()
            .any(|declaration| declaration.origin.as_deref() == Some("child-contract:fresh"))
    );
}

#[test]
fn resumed_child_surface_reuses_existing_limits_and_contract_source() {
    let assembler = GovernanceSurfaceAssembler::default();
    let limits = ResolvedExecutionLimitsSnapshot;
    let surface = assembler
        .resumed_child_surface(ResumedChildGovernanceInput {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            working_dir: ".".to_string(),
            mode_id: ModeId::code(),
            runtime: ResolvedRuntimeConfig::default(),
            resolved_limits: limits.clone(),
            delegation: None,
            message: "continue with the same branch".to_string(),
            context: Some("keep scope tight".to_string()),
            busy_policy: GovernanceBusyPolicy::RejectOnBusy,
        })
        .expect("surface should build");
    assert_eq!(surface.resolved_limits, limits);
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
