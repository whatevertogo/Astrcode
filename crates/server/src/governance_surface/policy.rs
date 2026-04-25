//! 治理策略上下文与审批管线构建。
//!
//! 提供两个核心功能：
//! - 构建协作策略上下文（`collaboration_policy_context`），包含 depth/spawn 限制
//! - 解析 busy policy

use astrcode_core::{AgentCollaborationPolicyContext, ResolvedRuntimeConfig};
use astrcode_governance_contract::{PolicyContext, ResolvedTurnEnvelope, SubmitBusyPolicy};
use serde_json::json;

pub const GOVERNANCE_POLICY_REVISION: &str = "governance-surface-v1";
pub const GOVERNANCE_APPROVAL_MODE_INHERIT: &str = "inherit";

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

/// 解析 busy policy：mode 级别 RejectOnBusy 强制覆盖，否则使用请求方指定的策略。
pub(super) fn resolve_busy_policy(
    submit_busy_policy: SubmitBusyPolicy,
    requested_busy_policy: GovernanceBusyPolicy,
) -> GovernanceBusyPolicy {
    match submit_busy_policy {
        SubmitBusyPolicy::BranchOnBusy => requested_busy_policy,
        SubmitBusyPolicy::RejectOnBusy => GovernanceBusyPolicy::RejectOnBusy,
    }
}

use super::GovernanceBusyPolicy;
