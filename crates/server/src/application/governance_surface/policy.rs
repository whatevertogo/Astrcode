//! 治理策略上下文构建。

use astrcode_core::{AgentCollaborationPolicyContext, ResolvedRuntimeConfig};

pub const GOVERNANCE_POLICY_REVISION: &str = "governance-surface-v1";

pub fn collaboration_policy_context(
    runtime: &ResolvedRuntimeConfig,
) -> AgentCollaborationPolicyContext {
    AgentCollaborationPolicyContext {
        policy_revision: GOVERNANCE_POLICY_REVISION.to_string(),
        max_subrun_depth: runtime.agent.max_subrun_depth,
        max_spawn_per_turn: runtime.agent.max_spawn_per_turn,
    }
}
