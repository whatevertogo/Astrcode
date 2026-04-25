//! 应用层定义的 agent prompt 提交载荷。
//!
//! Why: application 可以表达"要提交什么治理上下文"，
//! 但不应该直接依赖 session-runtime 的具体提交结构。

use astrcode_core::{
    AgentEventContext, LlmMessage, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides,
};
use astrcode_governance_contract::{BoundModeToolContractSnapshot, ModeId, PolicyContext};
use astrcode_host_session::PromptGovernanceContext;
use astrcode_prompt_contract::PromptDeclaration;

/// 应用层提交给 session 端口的稳定载荷。
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct AppAgentPromptSubmission {
    pub agent: AgentEventContext,
    pub current_mode_id: ModeId,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub injected_messages: Vec<LlmMessage>,
    pub source_tool_call_id: Option<String>,
    pub policy_context: Option<PolicyContext>,
    pub governance_revision: Option<String>,
    pub requires_approval: bool,
    pub prompt_governance: Option<PromptGovernanceContext>,
}
