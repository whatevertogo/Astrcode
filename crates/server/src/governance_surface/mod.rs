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
    LlmMessage, ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig,
    ResolvedSubagentContextOverrides,
};
use astrcode_governance_contract::{BoundModeToolContractSnapshot, ModeId};
use astrcode_prompt_contract::PromptDeclaration;
pub(crate) use inherited::resolve_inherited_parent_messages;
#[cfg(test)]
pub(crate) use inherited::{build_inherited_messages, select_inherited_recent_tail};
pub use policy::{GOVERNANCE_POLICY_REVISION, collaboration_policy_context};
pub use prompt::{
    build_delegation_metadata, build_fresh_child_contract, build_resumed_child_contract,
};

use crate::{CompiledModeEnvelope, ExecutionControl, ports::AppAgentPromptSubmission};

/// bind 完成的治理面，一次性消费的 turn 级上下文快照。
///
/// 包含 prompt declarations、注入消息、协作策略等全部治理决策。
/// 通过 `into_submission()` 转换为应用层提交载荷，再交给 session 端口适配到底层 runtime。
#[derive(Clone)]
#[allow(dead_code)]
pub struct ResolvedGovernanceSurface {
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub bound_mode_tool_contract: BoundModeToolContractSnapshot,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub injected_messages: Vec<LlmMessage>,
}

impl ResolvedGovernanceSurface {
    pub fn into_submission(
        self,
        agent: astrcode_core::AgentEventContext,
        source_tool_call_id: Option<String>,
    ) -> AppAgentPromptSubmission {
        AppAgentPromptSubmission {
            agent,
            current_mode_id: self.mode_id,
            bound_mode_tool_contract: Some(self.bound_mode_tool_contract),
            resolved_limits: Some(self.resolved_limits),
            resolved_overrides: self.resolved_overrides,
            injected_messages: self.injected_messages,
            source_tool_call_id,
        }
    }
}

struct BuildSurfaceInput {
    compiled: CompiledModeEnvelope,
    runtime: ResolvedRuntimeConfig,
    resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    injected_messages: Vec<LlmMessage>,
    leading_prompt_declaration: Option<PromptDeclaration>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SessionGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub profile: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub control: Option<ExecutionControl>,
    pub extra_prompt_declarations: Vec<PromptDeclaration>,
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FreshChildGovernanceInput {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: String,
    pub mode_id: ModeId,
    pub runtime: ResolvedRuntimeConfig,
    pub description: String,
    pub task: String,
}

#[allow(dead_code)]
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
}
