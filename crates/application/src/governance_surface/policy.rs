//! 治理策略上下文与审批管线构建。
//!
//! 提供两个核心功能：
//! - 构建协作策略上下文（`collaboration_policy_context`），包含 depth/spawn 限制
//! - 构建审批管线（`default_approval_pipeline`），当 mode 要求审批时安装占位骨架

use astrcode_core::{
    AgentCollaborationPolicyContext, ApprovalPending, ApprovalRequest, CapabilityCall, ModeId,
    PolicyContext, ResolvedRuntimeConfig, ResolvedTurnEnvelope,
};
use serde_json::{Value, json};

use super::{GovernanceApprovalPipeline, GovernanceBusyPolicy};

pub const GOVERNANCE_POLICY_REVISION: &str = "governance-surface-v1";
pub const GOVERNANCE_APPROVAL_MODE_INHERIT: &str = "inherit";

#[derive(Debug, Clone)]
pub struct ToolCollaborationGovernanceContext {
    runtime: ResolvedRuntimeConfig,
    session_id: String,
    turn_id: String,
    parent_agent_id: Option<String>,
    source_tool_call_id: Option<String>,
    policy: AgentCollaborationPolicyContext,
    governance_revision: String,
    mode_id: ModeId,
}

#[derive(Debug, Clone)]
pub struct ToolCollaborationGovernanceContextInput {
    pub runtime: ResolvedRuntimeConfig,
    pub session_id: String,
    pub turn_id: String,
    pub parent_agent_id: Option<String>,
    pub source_tool_call_id: Option<String>,
    pub policy: AgentCollaborationPolicyContext,
    pub governance_revision: String,
    pub mode_id: ModeId,
}

impl ToolCollaborationGovernanceContext {
    pub fn new(input: ToolCollaborationGovernanceContextInput) -> Self {
        Self {
            runtime: input.runtime,
            session_id: input.session_id,
            turn_id: input.turn_id,
            parent_agent_id: input.parent_agent_id,
            source_tool_call_id: input.source_tool_call_id,
            policy: input.policy,
            governance_revision: input.governance_revision,
            mode_id: input.mode_id,
        }
    }

    pub fn runtime(&self) -> &ResolvedRuntimeConfig {
        &self.runtime
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn parent_agent_id(&self) -> Option<String> {
        self.parent_agent_id.clone()
    }

    pub fn source_tool_call_id(&self) -> Option<String> {
        self.source_tool_call_id.clone()
    }

    pub fn policy(&self) -> &AgentCollaborationPolicyContext {
        &self.policy
    }

    pub fn governance_revision(&self) -> &str {
        &self.governance_revision
    }

    pub fn mode_id(&self) -> &ModeId {
        &self.mode_id
    }
}

pub fn collaboration_policy_context(
    runtime: &ResolvedRuntimeConfig,
) -> AgentCollaborationPolicyContext {
    AgentCollaborationPolicyContext {
        policy_revision: GOVERNANCE_POLICY_REVISION.to_string(),
        max_subrun_depth: runtime.agent.max_subrun_depth,
        max_spawn_per_turn: runtime.agent.max_spawn_per_turn,
    }
}

pub(super) fn build_policy_context(
    session_id: &str,
    turn_id: &str,
    working_dir: &str,
    profile: &str,
    envelope: &ResolvedTurnEnvelope,
) -> PolicyContext {
    PolicyContext {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        step_index: 0,
        working_dir: working_dir.to_string(),
        profile: profile.to_string(),
        metadata: json!({
            "governanceRevision": GOVERNANCE_POLICY_REVISION,
            "modeId": envelope.mode_id,
            "modeDiagnostics": envelope.diagnostics,
        }),
    }
}

pub(super) fn default_approval_pipeline(
    session_id: &str,
    turn_id: &str,
    envelope: &ResolvedTurnEnvelope,
) -> GovernanceApprovalPipeline {
    if !envelope.action_policies.requires_approval() {
        return GovernanceApprovalPipeline { pending: None };
    }
    // 安装占位审批骨架：当前 disabled，后续会接入真实审批引擎
    GovernanceApprovalPipeline {
        pending: Some(ApprovalPending {
            request: ApprovalRequest {
                request_id: format!("approval-skeleton:{session_id}:{turn_id}"),
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                capability: astrcode_core::CapabilitySpec::builder(
                    "governance.approval.placeholder",
                    astrcode_core::CapabilityKind::Tool,
                )
                .description("placeholder approval skeleton")
                .schema(json!({"type": "object"}), json!({"type": "object"}))
                .build()
                .expect("placeholder capability should build"),
                payload: json!({
                    "modeId": envelope.mode_id,
                }),
                prompt: "Governance approval skeleton is installed but disabled by default."
                    .to_string(),
                default: astrcode_core::ApprovalDefault::Allow,
                metadata: json!({
                    "disabled": true,
                    "governanceRevision": GOVERNANCE_POLICY_REVISION,
                    "modeId": envelope.mode_id,
                }),
            },
            action: CapabilityCall {
                request_id: format!("approval-call:{session_id}:{turn_id}"),
                capability: astrcode_core::CapabilitySpec::builder(
                    "governance.approval.placeholder",
                    astrcode_core::CapabilityKind::Tool,
                )
                .description("placeholder approval skeleton")
                .schema(json!({"type": "object"}), json!({"type": "object"}))
                .build()
                .expect("placeholder capability should build"),
                payload: Value::Null,
                metadata: json!({
                    "disabled": true,
                    "modeId": envelope.mode_id,
                }),
            },
        }),
    }
}

/// 解析 busy policy：mode 级别 RejectOnBusy 强制覆盖，否则使用请求方指定的策略。
pub(super) fn resolve_busy_policy(
    submit_busy_policy: astrcode_core::SubmitBusyPolicy,
    requested_busy_policy: GovernanceBusyPolicy,
) -> GovernanceBusyPolicy {
    match submit_busy_policy {
        astrcode_core::SubmitBusyPolicy::BranchOnBusy => requested_busy_policy,
        astrcode_core::SubmitBusyPolicy::RejectOnBusy => GovernanceBusyPolicy::RejectOnBusy,
    }
}
