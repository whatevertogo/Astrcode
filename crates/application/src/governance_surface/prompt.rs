//! 治理 prompt 声明构建。
//!
//! 生成子代理委派相关的 prompt declarations：
//! - `build_delegation_metadata`：构建委派元数据（责任摘要、复用边界、能力限制）
//! - `build_fresh_child_contract`：新子代理的系统 prompt 契约
//! - `build_resumed_child_contract`：继续委派的增量指令 prompt
//! - `collaboration_prompt_declarations`：四工具协作指导 prompt

use astrcode_core::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, ResolvedExecutionLimitsSnapshot, SystemPromptLayer,
};

const AGENT_COLLABORATION_TOOLS: &[&str] = &["spawn", "send", "observe", "close"];

pub fn build_delegation_metadata(
    description: &str,
    prompt: &str,
    resolved_limits: &ResolvedExecutionLimitsSnapshot,
    restricted: bool,
) -> astrcode_core::DelegationMetadata {
    let responsibility_summary = compact_delegation_summary(description, prompt);
    let reuse_scope_summary = if restricted {
        "只有当下一步仍属于同一责任分支，且所需操作仍落在当前收缩后的 capability surface \
         内时，才应继续复用这个 child。"
            .to_string()
    } else {
        "只有当下一步仍属于同一责任分支时，才应继续复用这个 child；若责任边界已经改变，应 close \
         当前分支并重新选择更合适的执行主体。"
            .to_string()
    };

    astrcode_core::DelegationMetadata {
        responsibility_summary,
        reuse_scope_summary,
        restricted,
        capability_limit_summary: restricted
            .then(|| capability_limit_summary(&resolved_limits.allowed_tools))
            .flatten(),
    }
}

pub fn build_fresh_child_contract(
    metadata: &astrcode_core::DelegationMetadata,
) -> PromptDeclaration {
    let mut content = format!(
        "You are a delegated child responsible for one isolated branch.\n\nResponsibility \
         branch:\n- {}\n\nFresh-child rule:\n- Treat this as a new responsibility branch with its \
         own ownership boundary.\n- Do not expand into unrelated exploration or \
         implementation.\n\nUnified send contract:\n- Use downstream `send(agentId + message)` \
         only when you need a direct child to continue a more specific sub-branch.\n- When this \
         branch reaches progress, completion, failure, or a close request, use upstream \
         `send(kind + payload)` to report to your direct parent.\n- Do not wait for an extra \
         confirmation loop before reporting terminal state.\n\nReuse boundary:\n- {}",
        metadata.responsibility_summary, metadata.reuse_scope_summary
    );
    if let Some(limit_summary) = &metadata.capability_limit_summary {
        content.push_str(&format!(
            "\n\nCapability limit:\n- {limit_summary}\n- Do not take work that needs tools \
             outside this surface."
        ));
    }

    governance_prompt_declaration(
        "child.execution.contract",
        "Child Execution Contract",
        content,
        SystemPromptLayer::Inherited,
        Some(585),
        "child-contract:fresh",
    )
}

pub fn build_resumed_child_contract(
    metadata: &astrcode_core::DelegationMetadata,
    message: &str,
    context: Option<&str>,
) -> PromptDeclaration {
    let mut content = format!(
        "You are continuing an existing delegated child branch.\n\nResponsibility continuity:\n- \
         Keep ownership of the same branch: {}\n\nResumed-child rule:\n- Prioritize the latest \
         delta instruction from the parent.\n- Do not restate or reinterpret the whole original \
         brief unless the new delta requires it.\n\nDelta instruction:\n- {}",
        metadata.responsibility_summary,
        message.trim()
    );
    if let Some(context) = context.filter(|value| !value.trim().is_empty()) {
        content.push_str(&format!("\n- Supplementary context: {}", context.trim()));
    }
    content.push_str(&format!(
        "\n\nUnified send contract:\n- Keep using downstream `send(agentId + message)` only for \
         direct child delegation inside the same branch.\n- Use upstream `send(kind + payload)` \
         to report concrete progress, completion, failure, or a close request back to your direct \
         parent.\n- Do not restate the whole branch transcript when reporting upward.\n\nReuse \
         boundary:\n- {}",
        metadata.reuse_scope_summary
    ));
    if let Some(limit_summary) = &metadata.capability_limit_summary {
        content.push_str(&format!(
            "\n\nCapability limit:\n- {limit_summary}\n- If the delta now needs broader tools, \
             stop stretching this child and let the parent choose a different branch."
        ));
    }

    governance_prompt_declaration(
        "child.execution.contract",
        "Child Execution Contract",
        content,
        SystemPromptLayer::Inherited,
        Some(585),
        "child-contract:resumed",
    )
}

pub(super) fn collaboration_prompt_declarations(
    allowed_tools: &[String],
    max_depth: usize,
    max_spawn_per_turn: usize,
) -> Vec<PromptDeclaration> {
    if !allowed_tools.iter().any(|tool_name| {
        AGENT_COLLABORATION_TOOLS
            .iter()
            .any(|candidate| tool_name == candidate)
    }) {
        return Vec::new();
    }

    vec![governance_prompt_declaration(
        "governance.collaboration.guide",
        "Child Agent Collaboration Guide",
        format!(
            "Use the child-agent tools as one decision protocol.\n\nKeep `agentId` exact. Copy it \
             byte-for-byte in later `send`, `observe`, and `close` calls. Never renumber it, \
             never zero-pad it, and never invent `agent-01` when the tool result says \
             `agent-1`.\n\nDefault protocol:\n1. `spawn` only for a new isolated responsibility \
             with real parallel or context-isolation value.\n2. `send` when the same child should \
             take one concrete next step on the same responsibility branch.\n3. `observe` only \
             when the next decision depends on current child state.\n4. `close` when the branch \
             is done or no longer useful.\n\nDelegation modes:\n- Fresh child: use `spawn` for a \
             new responsibility branch. Give the child a full briefing: task scope, boundaries, \
             expected deliverable, and any important focus or exclusion. Do not treat a short \
             nudge like 'take a look' as a sufficient fresh-child brief.\n- Resumed child: use \
             `send` when the same child should continue the same responsibility branch. Send one \
             concrete delta instruction or clarification, not a full re-briefing of the original \
             task.\n- Restricted child: when you narrow a child with `capabilityGrant`, assign \
             only work that fits that reduced capability surface. If the next step needs tools \
             the restricted child does not have, choose a different child or do the work locally \
             instead of forcing a mismatch.\n\n`Idle` is normal and reusable. Do not respawn just \
             because a child finished one turn. Reuse an idle child with `send(agentId, message)` \
             when the responsibility stays the same. If you are unsure whether the child is still \
             running, idle, or terminated, call `observe(agentId)` once and act on the \
             result.\n\nSpawn sparingly. The runtime enforces a maximum child depth of \
             {max_depth} and at most {max_spawn_per_turn} new children per turn. Start with one \
             child unless there are clearly separate workstreams. Do not blanket-spawn agents \
             just to explore a repo broadly.\n\nAvoid waste:\n- Do not loop on `observe` with no \
             decision attached.\n- If a child is still running and you are simply waiting, prefer \
             a brief shell sleep over spending another tool call on `observe`.\n- Pick one wait \
             mode per pause: either `observe` now because you need a snapshot for the next \
             decision, or sleep briefly because you are only waiting. Do not alternate `shell` \
             and `observe` in a polling loop.\n- After a wait, call `observe` only when the next \
             decision depends on the child's current state.\n- Do not immediately re-`observe` \
             the same child after a fresh delivery unless the state is genuinely ambiguous.\n- Do \
             not stack speculative `send` calls.\n- Do not spawn a new child when an existing \
             idle child already owns the responsibility.\n\nIf a delivery satisfies the request, \
             `close` the branch. If the same child should continue, `send` one precise follow-up. \
             If you see the same `deliveryId` again after recovery, treat it as the same \
             delivery, not a new task.\n\nWhen you are the child on a delegated task, use \
             upstream `send(kind + payload)` to deliver a formal message to your direct parent. \
             Report `progress`, `completed`, `failed`, or `close_request` explicitly. Do not wait \
             for the parent to infer state from raw intermediate steps, and do not end with an \
             open loop like '继续观察中' unless you are also sending a non-terminal `progress` \
             delivery that keeps the branch alive.\n\nWhen you are the parent and receive a child \
             delivery, treat it as a decision point. Do not leave it hanging and do not \
             immediately re-observe the same child unless the state is unclear. Decide \
             immediately whether the result is complete enough to `close` the branch, or whether \
             the same child should continue with one concrete `send` follow-up that names the \
             exact next step."
        ),
        SystemPromptLayer::Dynamic,
        Some(600),
        "governance:collaboration-guide",
    )]
}

fn governance_prompt_declaration(
    block_id: impl Into<String>,
    title: impl Into<String>,
    content: String,
    layer: SystemPromptLayer,
    priority_hint: Option<i32>,
    origin: impl Into<String>,
) -> PromptDeclaration {
    PromptDeclaration {
        block_id: block_id.into(),
        title: title.into(),
        content,
        render_target: PromptDeclarationRenderTarget::System,
        layer,
        kind: PromptDeclarationKind::ExtensionInstruction,
        priority_hint,
        always_include: true,
        source: PromptDeclarationSource::Builtin,
        capability_name: None,
        origin: Some(origin.into()),
    }
}

fn compact_delegation_summary(description: &str, prompt: &str) -> String {
    let candidate = if !description.trim().is_empty() {
        description.trim()
    } else {
        prompt.trim()
    };
    let normalized = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let truncated = chars.by_ref().take(160).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn capability_limit_summary(allowed_tools: &[String]) -> Option<String> {
    if allowed_tools.is_empty() {
        return None;
    }
    Some(format!(
        "本分支当前只允许使用这些工具：{}。",
        allowed_tools.join(", ")
    ))
}
