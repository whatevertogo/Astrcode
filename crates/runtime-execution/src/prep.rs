use std::{collections::HashSet, sync::Arc};

use astrcode_core::{
    AgentMode, AgentProfile, AgentState, ArtifactRef, AstrError, ExecutionOwner, HookHandler,
    InvocationKind, LlmMessage, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SubagentContextOverrides, UserMessageOrigin,
};
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;

use crate::{
    ResolvedContextSnapshot, policy::resolve_subagent_overrides, resolve_context_snapshot,
};

#[derive(Debug, Clone)]
pub struct AgentExecutionSpec {
    pub invocation_kind: InvocationKind,
    pub resolved_overrides: ResolvedSubagentContextOverrides,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub resolved_context_snapshot: ResolvedContextSnapshot,
}

#[derive(Debug, Clone)]
pub struct AgentExecutionRequest {
    pub task: String,
    pub context: Option<String>,
    pub max_steps: Option<u32>,
    pub context_overrides: Option<SubagentContextOverrides>,
}

#[derive(Clone)]
pub struct PreparedAgentExecution<TLoop> {
    pub execution_spec: AgentExecutionSpec,
    pub runtime_config: astrcode_runtime_config::RuntimeConfig,
    pub loop_: TLoop,
}

#[derive(Clone)]
pub struct ScopedExecutionSurface<TSkillCatalog> {
    pub capabilities: CapabilityRouter,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub skill_catalog: TSkillCatalog,
    pub hook_handlers: Vec<Arc<dyn HookHandler>>,
    pub runtime_config: astrcode_runtime_config::RuntimeConfig,
}

fn build_execution_spec(
    invocation_kind: InvocationKind,
    profile: &AgentProfile,
    params: &AgentExecutionRequest,
    allowed_tools: &[String],
    runtime_config: &astrcode_runtime_config::RuntimeConfig,
    parent_state: Option<&AgentState>,
) -> Result<AgentExecutionSpec, AstrError> {
    let resolved_overrides =
        resolve_subagent_overrides(params.context_overrides.as_ref(), runtime_config)?;
    let resolved_context_snapshot =
        resolve_context_snapshot(params, parent_state, &resolved_overrides);

    Ok(AgentExecutionSpec {
        invocation_kind,
        resolved_overrides,
        resolved_limits: ResolvedExecutionLimitsSnapshot {
            max_steps: profile.max_steps.or(params.max_steps),
            token_budget: profile.token_budget,
            allowed_tools: allowed_tools.to_vec(),
        },
        resolved_context_snapshot,
    })
}

// 这里统一 root/sub-agent 的 profile 裁剪与 loop 装配，避免 façade 同时维护两套
// 几乎一致的 surface -> execution spec -> prompt -> loop 组装路径。
pub fn prepare_scoped_agent_execution<F, TSkillCatalog, TLoop>(
    invocation_kind: InvocationKind,
    profile: &AgentProfile,
    params: &AgentExecutionRequest,
    surface: ScopedExecutionSurface<TSkillCatalog>,
    parent_state: Option<&AgentState>,
    build_loop: F,
) -> Result<PreparedAgentExecution<TLoop>, AstrError>
where
    TSkillCatalog: Clone,
    F: FnOnce(
        CapabilityRouter,
        Vec<PromptDeclaration>,
        TSkillCatalog,
        Vec<Arc<dyn HookHandler>>,
        &astrcode_runtime_config::RuntimeConfig,
    ) -> TLoop,
{
    let final_tool_names = resolve_profile_tool_names(&surface.capabilities, profile)?;
    if final_tool_names.is_empty() {
        return Err(AstrError::Validation(format!(
            "agent profile '{}' does not expose any available tools in the current runtime surface",
            profile.id
        )));
    }

    let execution_spec = build_execution_spec(
        invocation_kind,
        profile,
        params,
        &final_tool_names,
        &surface.runtime_config,
        parent_state,
    )?;
    let prompt_declarations = build_child_prompt_declarations(
        &surface.prompt_declarations,
        profile,
        &execution_spec.resolved_overrides,
    );
    let scoped_capabilities = surface.capabilities.subset_for_tools(&final_tool_names)?;
    let loop_ = build_loop(
        scoped_capabilities,
        prompt_declarations.clone(),
        surface.skill_catalog.clone(),
        surface.hook_handlers.clone(),
        &surface.runtime_config,
    );

    Ok(PreparedAgentExecution {
        execution_spec,
        runtime_config: surface.runtime_config,
        loop_,
    })
}


fn build_child_prompt_declarations(
    parent: &[PromptDeclaration],
    profile: &AgentProfile,
    overrides: &ResolvedSubagentContextOverrides,
) -> Vec<PromptDeclaration> {
    let mut declarations =
        if overrides.inherit_system_instructions || overrides.inherit_project_instructions {
            parent.to_vec()
        } else {
            Vec::new()
        };
    if let Some(system_prompt) = profile.system_prompt.as_ref() {
        declarations.push(PromptDeclaration {
            block_id: format!("subagent.profile.{}", profile.id),
            title: format!("Sub-Agent Profile: {}", profile.name),
            content: system_prompt.clone(),
            render_target: astrcode_runtime_prompt::PromptDeclarationRenderTarget::System,
            kind: astrcode_runtime_prompt::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(100),
            always_include: true,
            source: astrcode_runtime_prompt::PromptDeclarationSource::Builtin,
            capability_name: Some("runAgent".to_string()),
            origin: Some(format!("agent-profile:{}", profile.id)),
        });
    }
    declarations
}

pub fn build_child_agent_state(
    session_id: &str,
    working_dir: std::path::PathBuf,
    task: &str,
) -> AgentState {
    AgentState {
        session_id: session_id.to_string(),
        working_dir,
        messages: vec![LlmMessage::User {
            content: task.to_string(),
            origin: UserMessageOrigin::User,
        }],
        phase: astrcode_core::Phase::Thinking,
        turn_count: 0,
    }
}

pub fn ensure_subagent_mode(profile: &AgentProfile) -> Result<(), AstrError> {
    if matches!(profile.mode, AgentMode::SubAgent | AgentMode::All) {
        return Ok(());
    }
    Err(AstrError::Validation(format!(
        "agent profile '{}' is not allowed to run as a sub-agent",
        profile.id
    )))
}

pub fn ensure_root_execution_mode(profile: &AgentProfile) -> Result<(), AstrError> {
    if matches!(
        profile.mode,
        AgentMode::Primary | AgentMode::SubAgent | AgentMode::All
    ) {
        return Ok(());
    }
    Err(AstrError::Validation(format!(
        "agent profile '{}' is not allowed to run as a root execution",
        profile.id
    )))
}

pub fn resolve_profile_tool_names(
    capabilities: &CapabilityRouter,
    profile: &AgentProfile,
) -> Result<Vec<String>, AstrError> {
    let available = capabilities
        .tool_names()
        .into_iter()
        .collect::<HashSet<_>>();
    let requested = if profile.allowed_tools.is_empty() {
        available.clone()
    } else {
        resolve_profile_tool_set(
            &profile.id,
            "allowed_tools",
            &profile.allowed_tools,
            &available,
        )?
    };
    let denied = resolve_profile_tool_set(
        &profile.id,
        "disallowed_tools",
        &profile.disallowed_tools,
        &available,
    )?;

    let mut final_tools = requested
        .into_iter()
        .filter(|tool| !denied.contains(tool))
        .collect::<Vec<_>>();
    final_tools.sort();
    Ok(final_tools)
}

fn resolve_profile_tool_set(
    profile_id: &str,
    field_name: &str,
    configured_tools: &[String],
    available: &HashSet<String>,
) -> Result<HashSet<String>, AstrError> {
    let mut unknown_tools = configured_tools
        .iter()
        .filter(|tool| !available.contains(tool.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown_tools.is_empty() {
        unknown_tools.sort();
        return Err(AstrError::Validation(format!(
            "agent profile '{}' references unknown {}: {}",
            profile_id,
            field_name,
            unknown_tools.join(", ")
        )));
    }

    Ok(configured_tools.iter().cloned().collect())
}

pub fn derive_child_execution_owner(
    ctx: &astrcode_core::ToolContext,
    parent_turn_id: &str,
    child: &astrcode_core::SubRunHandle,
) -> ExecutionOwner {
    ctx.execution_owner().cloned().map_or_else(
        || {
            ExecutionOwner::root(
                ctx.session_id().to_string(),
                parent_turn_id.to_string(),
                ctx.agent_context()
                    .invocation_kind
                    .unwrap_or(InvocationKind::RootExecution),
            )
            .for_sub_run(child.sub_run_id.clone())
        },
        |owner| owner.for_sub_run(child.sub_run_id.clone()),
    )
}

pub fn build_result_findings(spec: &AgentExecutionSpec) -> Vec<String> {
    let mut findings = vec![
        format!("invocationKind={:?}", spec.invocation_kind),
        format!("storageMode={:?}", spec.resolved_overrides.storage_mode),
    ];
    if let Some(summary) = spec
        .resolved_context_snapshot
        .inherited_compact_summary
        .as_ref()
    {
        findings.push(format!("inheritedCompactSummary={} chars", summary.len()));
    }
    if !spec
        .resolved_context_snapshot
        .inherited_recent_tail
        .is_empty()
    {
        findings.push(format!(
            "inheritedRecentTailLines={}",
            spec.resolved_context_snapshot.inherited_recent_tail.len()
        ));
    }
    findings
}

pub fn build_result_artifacts(child: &astrcode_core::SubRunHandle) -> Vec<ArtifactRef> {
    child
        .child_session_id
        .as_ref()
        .map_or_else(Vec::new, |session_id| {
            vec![ArtifactRef {
                kind: "session".to_string(),
                id: session_id.clone(),
                label: "Independent child session".to_string(),
                session_id: Some(session_id.clone()),
                storage_seq: None,
                uri: None,
            }]
        })
}

pub fn summarize_child_result(
    last_summary: Option<&str>,
    token_limit_hit: bool,
    step_limit_hit: bool,
    duration_ms: u64,
    fallback: &str,
) -> String {
    let base = last_summary
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string();
    if token_limit_hit || step_limit_hit {
        return format!(
            "{base}\n\n[stopped after {duration_ms}ms because a sub-agent budget limit was \
             reached]"
        );
    }
    base
}

#[cfg(test)]
mod tests {
    use super::summarize_child_result;

    #[test]
    fn summarize_child_result_appends_budget_limit_note() {
        let summary = summarize_child_result(Some("done"), true, false, 1200, "fallback summary");

        assert!(summary.contains("done"));
        assert!(summary.contains("stopped after 1200ms"));
    }

    #[test]
    fn summarize_child_result_uses_fallback_when_summary_missing() {
        let summary = summarize_child_result(None, false, false, 100, "fallback summary");

        assert_eq!(summary, "fallback summary");
    }
}
