mod assembler;
mod inherited;
mod policy;
mod prompt;
#[cfg(test)]
mod tests;

pub use assembler::GovernanceSurfaceAssembler;
use astrcode_core::{
    AgentCollaborationPolicyContext, CapabilityCall, LlmMessage, ModeId, PolicyContext,
    ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
    SpawnCapabilityGrant,
};
use astrcode_kernel::CapabilityRouter;
use astrcode_session_runtime::AgentPromptSubmission;
pub(crate) use inherited::resolve_inherited_parent_messages;
#[cfg(test)]
pub(crate) use inherited::{build_inherited_messages, select_inherited_recent_tail};
pub use policy::{
    GOVERNANCE_APPROVAL_MODE_INHERIT, GOVERNANCE_POLICY_REVISION,
    ToolCollaborationGovernanceContext, ToolCollaborationGovernanceContextInput,
    collaboration_policy_context, effective_allowed_tools_for_limits,
};
pub use prompt::{
    build_delegation_metadata, build_fresh_child_contract, build_resumed_child_contract,
};

use crate::{ApplicationError, CompiledModeEnvelope, ExecutionControl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernanceBusyPolicy {
    BranchOnBusy,
    RejectOnBusy,
}

#[derive(Clone, PartialEq, Default)]
pub struct GovernanceApprovalPipeline {
    pub pending: Option<astrcode_core::ApprovalPending<CapabilityCall>>,
}

#[derive(Clone)]
pub struct ResolvedGovernanceSurface {
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub capability_router: Option<CapabilityRouter>,
    pub prompt_declarations: Vec<astrcode_core::PromptDeclaration>,
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

    pub fn allowed_capability_names(&self) -> Vec<String> {
        self.resolved_limits.allowed_tools.clone()
    }

    pub fn prompt_facts_context(&self) -> astrcode_core::PromptGovernanceContext {
        astrcode_core::PromptGovernanceContext {
            allowed_capability_names: self.allowed_capability_names(),
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
    ) -> AgentPromptSubmission {
        let prompt_governance = self.prompt_facts_context();
        AgentPromptSubmission {
            agent,
            capability_router: self.capability_router,
            prompt_declarations: self.prompt_declarations,
            resolved_limits: Some(self.resolved_limits),
            resolved_overrides: self.resolved_overrides,
            injected_messages: self.injected_messages,
            source_tool_call_id,
            policy_context: Some(self.policy_context),
            governance_revision: Some(self.governance_revision),
            approval: self.approval.pending.map(Box::new),
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
    pub parent_allowed_tools: Vec<String>,
    pub capability_grant: Option<SpawnCapabilityGrant>,
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
    pub allowed_tools: Vec<String>,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub delegation: Option<astrcode_core::DelegationMetadata>,
    pub message: String,
    pub context: Option<String>,
    pub busy_policy: GovernanceBusyPolicy,
}
