use std::{collections::HashSet, sync::Arc};

use astrcode_core::{
    AgentMode, AgentProfile, AgentState, ArtifactRef, AstrError, ExecutionOwner, HookHandler,
    InvocationKind, LlmMessage, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SubRunStorageMode, SubagentContextOverrides, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::AgentLoop;
use astrcode_runtime_agent_tool::RunAgentParams;
use astrcode_runtime_config::resolve_agent_experimental_independent_session;
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_skill_loader::SkillCatalog;

use crate::{ResolvedContextSnapshot, resolve_context_snapshot};

#[derive(Debug, Clone)]
pub struct AgentExecutionSpec {
    pub invocation_kind: InvocationKind,
    pub resolved_overrides: ResolvedSubagentContextOverrides,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub resolved_context_snapshot: ResolvedContextSnapshot,
}

#[derive(Clone)]
pub struct PreparedAgentExecution {
    pub execution_spec: AgentExecutionSpec,
    pub runtime_config: astrcode_runtime_config::RuntimeConfig,
    pub loop_: Arc<AgentLoop>,
}

#[derive(Clone)]
pub struct ScopedExecutionSurface {
    pub capabilities: CapabilityRouter,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub skill_catalog: Arc<SkillCatalog>,
    pub hook_handlers: Vec<Arc<dyn HookHandler>>,
    pub runtime_config: astrcode_runtime_config::RuntimeConfig,
}

fn build_execution_spec(
    invocation_kind: InvocationKind,
    profile: &AgentProfile,
    params: &RunAgentParams,
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
pub fn prepare_scoped_agent_execution<F>(
    invocation_kind: InvocationKind,
    profile: &AgentProfile,
    params: &RunAgentParams,
    surface: ScopedExecutionSurface,
    parent_state: Option<&AgentState>,
    build_loop: F,
) -> Result<PreparedAgentExecution, AstrError>
where
    F: FnOnce(
        CapabilityRouter,
        Vec<PromptDeclaration>,
        Arc<SkillCatalog>,
        Vec<Arc<dyn HookHandler>>,
        &astrcode_runtime_config::RuntimeConfig,
    ) -> Arc<AgentLoop>,
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
        Arc::clone(&surface.skill_catalog),
        surface.hook_handlers.clone(),
        &surface.runtime_config,
    );

    Ok(PreparedAgentExecution {
        execution_spec,
        runtime_config: surface.runtime_config,
        loop_,
    })
}

pub fn resolve_subagent_overrides(
    overrides: Option<&SubagentContextOverrides>,
    runtime_config: &astrcode_runtime_config::RuntimeConfig,
) -> Result<ResolvedSubagentContextOverrides, AstrError> {
    let mut resolved = ResolvedSubagentContextOverrides::default();
    if let Some(overrides) = overrides {
        if let Some(storage_mode) = overrides.storage_mode {
            resolved.storage_mode = storage_mode;
        }
        if let Some(value) = overrides.inherit_system_instructions {
            resolved.inherit_system_instructions = value;
        }
        if let Some(value) = overrides.inherit_project_instructions {
            resolved.inherit_project_instructions = value;
        }
        if let Some(value) = overrides.inherit_working_dir {
            resolved.inherit_working_dir = value;
        }
        if let Some(value) = overrides.inherit_policy_upper_bound {
            resolved.inherit_policy_upper_bound = value;
        }
        if let Some(value) = overrides.inherit_cancel_token {
            resolved.inherit_cancel_token = value;
        }
        if let Some(value) = overrides.include_compact_summary {
            resolved.include_compact_summary = value;
        }
        if let Some(value) = overrides.include_recent_tail {
            resolved.include_recent_tail = value;
        }
        if let Some(value) = overrides.include_recovery_refs {
            resolved.include_recovery_refs = value;
        }
        if let Some(value) = overrides.include_parent_findings {
            resolved.include_parent_findings = value;
        }
    }

    if matches!(resolved.storage_mode, SubRunStorageMode::IndependentSession)
        && !resolve_agent_experimental_independent_session(runtime_config.agent.as_ref())
    {
        return Err(AstrError::Validation(
            "independent_session is experimental and currently disabled by \
             runtime.agent.experimentalIndependentSession"
                .to_string(),
        ));
    }
    if resolved.inherit_system_instructions != resolved.inherit_project_instructions {
        return Err(AstrError::Validation(
            "inheritSystemInstructions and inheritProjectInstructions must currently resolve to \
             the same value"
                .to_string(),
        ));
    }
    if !resolved.inherit_working_dir {
        return Err(AstrError::Validation(
            "inheritWorkingDir=false is not supported yet; child agents must stay in the parent \
             workspace"
                .to_string(),
        ));
    }
    if !resolved.inherit_cancel_token {
        return Err(AstrError::Validation(
            "inheritCancelToken=false is not supported yet; child agents must stay linked to the \
             parent cancellation chain"
                .to_string(),
        ));
    }
    if resolved.include_recovery_refs {
        return Err(AstrError::Validation(
            "includeRecoveryRefs=true is not supported yet; recovery refs are not exposed to \
             sub-agent context overrides in this release"
                .to_string(),
        ));
    }
    if resolved.include_parent_findings {
        return Err(AstrError::Validation(
            "includeParentFindings=true is not supported yet; parent findings are not exposed to \
             sub-agent context overrides in this release"
                .to_string(),
        ));
    }

    Ok(resolved)
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
    tracker: &astrcode_runtime_agent_loop::ChildExecutionTracker,
    duration_ms: u64,
    fallback: &str,
) -> String {
    let base = tracker
        .last_summary()
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string();
    if tracker.token_limit_hit() || tracker.step_limit_hit() {
        return format!(
            "{base}\n\n[stopped after {duration_ms}ms because a sub-agent budget limit was \
             reached]"
        );
    }
    base
}
