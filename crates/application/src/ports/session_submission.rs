//! 应用层定义的 agent prompt 提交载荷。
//!
//! Why: application 可以表达“要提交什么治理上下文”，
//! 但不应该直接依赖 session-runtime 的具体提交结构。

use astrcode_core::{
    AgentEventContext, BoundModeToolContractSnapshot, CapabilityCall, LlmMessage, ModeId,
    PolicyContext, PromptDeclaration, PromptGovernanceContext, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides,
};
use astrcode_kernel::CapabilityRouter;

/// 应用层提交给 session 端口的稳定载荷。
#[derive(Clone, Default)]
pub struct AppAgentPromptSubmission {
    pub agent: AgentEventContext,
    pub capability_router: Option<CapabilityRouter>,
    pub current_mode_id: ModeId,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub injected_messages: Vec<LlmMessage>,
    pub source_tool_call_id: Option<String>,
    pub policy_context: Option<PolicyContext>,
    pub governance_revision: Option<String>,
    pub approval: Option<astrcode_core::ApprovalPending<CapabilityCall>>,
    pub prompt_governance: Option<PromptGovernanceContext>,
}

impl From<AppAgentPromptSubmission> for astrcode_session_runtime::AgentPromptSubmission {
    fn from(value: AppAgentPromptSubmission) -> Self {
        Self {
            agent: value.agent,
            capability_router: value.capability_router,
            current_mode_id: value.current_mode_id,
            prompt_declarations: value.prompt_declarations,
            bound_mode_tool_contract: value.bound_mode_tool_contract,
            resolved_limits: value.resolved_limits,
            resolved_overrides: value.resolved_overrides,
            injected_messages: value.injected_messages,
            source_tool_call_id: value.source_tool_call_id,
            policy_context: value.policy_context,
            governance_revision: value.governance_revision,
            approval: value.approval.map(Box::new),
            prompt_governance: value.prompt_governance,
        }
    }
}
