//! # 治理面子域（Governance Surface）
//!
//! 统一管理每次 turn 的治理决策：审批策略、子代理委派策略、协作指导 prompt。
//!
//! 核心流程：`*GovernanceInput` → compile mode surface → bind runtime/session facts →
//! `ResolvedGovernanceSurface` → `AppAgentPromptSubmission`
//!
//! 入口场景：
//! - **Session turn**：`session_surface()` — 用户直接发起的 turn
//! - **Root execution**：`root_surface()` — 根代理首次执行（委托到 session_surface）
//! - **Fresh child**：`fresh_child_surface()` — spawn 新子代理，需要继承父级上下文
//! - **Resumed child**：`resumed_child_surface()` — 向已有子代理 send 消息，复用已有策略

mod assembler;
mod inherited;
mod policy;
mod prompt;
#[cfg(test)]
mod tests;

pub use assembler::GovernanceSurfaceAssembler;
use astrcode_core::{
    AgentCollaborationPolicyContext, BoundModeToolContractSnapshot, CapabilityCall, LlmMessage,
    ModeId, PolicyContext, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig,
    ResolvedSubagentContextOverrides,
};
use astrcode_kernel::CapabilityRouter;
pub(crate) use inherited::resolve_inherited_parent_messages;
#[cfg(test)]
pub(crate) use inherited::{build_inherited_messages, select_inherited_recent_tail};
pub use policy::{
    GOVERNANCE_APPROVAL_MODE_INHERIT, GOVERNANCE_POLICY_REVISION,
    ToolCollaborationGovernanceContext, ToolCollaborationGovernanceContextInput,
    collaboration_policy_context,
};
pub use prompt::{
    build_delegation_metadata, build_fresh_child_contract, build_resumed_child_contract,
};

use crate::{
    ApplicationError, CompiledModeEnvelope, ExecutionControl, ports::AppAgentPromptSubmission,
};

/// Session busy 时的行为策略。
///
/// - `BranchOnBusy`：自动创建分支 session 继续处理（默认）
/// - `RejectOnBusy`：拒绝请求并返回错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernanceBusyPolicy {
    BranchOnBusy,
    RejectOnBusy,
}

/// 审批管线状态。
///
/// 如果 mode 要求审批某些能力调用，`pending` 会携带一个 `ApprovalPending` 占位骨架，
/// 在实际执行前需要用户确认。
#[derive(Clone, PartialEq, Default)]
pub struct GovernanceApprovalPipeline {
    pub pending: Option<astrcode_core::ApprovalPending<CapabilityCall>>,
}

/// bind 完成的治理面，一次性消费的 turn 级上下文快照。
///
/// 包含审批管线、prompt declarations、注入消息、协作策略等全部治理决策。
/// 通过 `into_submission()` 转换为应用层提交载荷，再交给 session 端口适配到底层 runtime。
#[derive(Clone)]
pub struct ResolvedGovernanceSurface {
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub capability_router: Option<CapabilityRouter>,
    pub prompt_declarations: Vec<astrcode_core::PromptDeclaration>,
    pub bound_mode_tool_contract: BoundModeToolContractSnapshot,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub injected_messages: Vec<LlmMessage>,
    pub policy_context: PolicyContext,
    pub collaboration_policy: AgentCollaborationPolicyContext,
    pub approval: GovernanceApprovalPipeline,
    pub governance_revision: String,
    pub busy_policy: GovernanceBusyPolicy,
    pub diagnostics: Vec<String>,
}

impl ResolvedGovernanceSurface {
    pub fn validate(&self) -> Result<(), ApplicationError> {
        if self.governance_revision.trim().is_empty() {
            return Err(ApplicationError::Internal(
                "governance revision must not be empty".to_string(),
            ));
        }
        if self.collaboration_policy.policy_revision != self.governance_revision {
            return Err(ApplicationError::Internal(
                "collaboration policy revision must match governance surface revision".to_string(),
            ));
        }
        Ok(())
    }

    pub fn prompt_facts_context(&self) -> astrcode_core::PromptGovernanceContext {
        astrcode_core::PromptGovernanceContext {
            allowed_capability_names: Vec::new(),
            mode_id: Some(self.mode_id.clone()),
            approval_mode: if self.approval.pending.is_some() {
                "required".to_string()
            } else {
                GOVERNANCE_APPROVAL_MODE_INHERIT.to_string()
            },
            policy_revision: self.governance_revision.clone(),
            max_subrun_depth: Some(self.collaboration_policy.max_subrun_depth),
            max_spawn_per_turn: Some(self.collaboration_policy.max_spawn_per_turn),
        }
    }

    pub fn into_submission(
        self,
        agent: astrcode_core::AgentEventContext,
        source_tool_call_id: Option<String>,
    ) -> AppAgentPromptSubmission {
        let prompt_governance = self.prompt_facts_context();
        AppAgentPromptSubmission {
            agent,
            capability_router: self.capability_router,
            current_mode_id: self.mode_id,
            prompt_declarations: self.prompt_declarations,
            bound_mode_tool_contract: Some(self.bound_mode_tool_contract),
            resolved_limits: Some(self.resolved_limits),
            resolved_overrides: self.resolved_overrides,
            injected_messages: self.injected_messages,
            source_tool_call_id,
            policy_context: Some(self.policy_context),
            governance_revision: Some(self.governance_revision),
            approval: self.approval.pending,
            prompt_governance: Some(prompt_governance),
        }
    }

    pub async fn check_model_request(
        &self,
        engine: &dyn astrcode_core::PolicyEngine,
        request: astrcode_core::ModelRequest,
    ) -> astrcode_core::Result<astrcode_core::ModelRequest> {
        engine
            .check_model_request(request, &self.policy_context)
            .await
    }

    pub async fn check_capability_call(
        &self,
        engine: &dyn astrcode_core::PolicyEngine,
        call: CapabilityCall,
    ) -> astrcode_core::Result<astrcode_core::PolicyVerdict<CapabilityCall>> {
        engine
            .check_capability_call(call, &self.policy_context)
            .await
    }
}

struct BuildSurfaceInput {
    session_id: String,
    turn_id: String,
    working_dir: String,
    profile: String,
    compiled: CompiledModeEnvelope,
    runtime: ResolvedRuntimeConfig,
    requested_busy_policy: GovernanceBusyPolicy,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    injected_messages: Vec<LlmMessage>,
    leading_prompt_declaration: Option<astrcode_core::PromptDeclaration>,
}

#[derive(Debug, Clone)]
pub struct SessionGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub profile: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub control: Option<ExecutionControl>,
    pub extra_prompt_declarations: Vec<astrcode_core::PromptDeclaration>,
    pub busy_policy: GovernanceBusyPolicy,
}

#[derive(Debug, Clone)]
pub struct RootGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub profile: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub control: Option<ExecutionControl>,
}

#[derive(Debug, Clone)]
pub struct FreshChildGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub description: String,
    pub task: String,
    pub busy_policy: GovernanceBusyPolicy,
}

#[derive(Debug, Clone)]
pub struct ResumedChildGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub delegation: Option<astrcode_core::DelegationMetadata>,
    pub message: String,
    pub context: Option<String>,
    pub busy_policy: GovernanceBusyPolicy,
}
