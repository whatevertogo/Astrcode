//! Agent 执行装配模块。
//!
//! 负责执行前的准备工作，包括：
//! - Profile 工具集裁剪与验证
//! - 执行限制解析（步数、token、工具白名单）
//! - 子 Agent 状态构建
//! - 执行结果构建（handoff/failure/artifacts）
//!
//! 设计原则：纯函数无状态，让 runtime façade 专注于编排。

use std::{collections::HashSet, sync::Arc};

use astrcode_core::{
    AgentMode, AgentProfile, AgentState, ArtifactRef, AstrError, ExecutionOwner, HookHandler,
    InvocationKind, LlmMessage, ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides,
    SubRunFailure, SubRunFailureCode, SubRunHandoff, SubagentContextOverrides, UserMessageOrigin,
};
use astrcode_runtime_agent_tool::SpawnAgentParams;
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
    /// 子 Agent 类型标识。
    pub subagent_type: Option<String>,
    /// 任务描述摘要（用于可观测性）。
    pub description: String,
    /// 任务正文。子 Agent 收到的指令主体。
    pub prompt: String,
    /// 可选补充材料。
    pub context: Option<String>,
    /// 可选的步数上限覆盖。
    pub max_steps: Option<u32>,
    /// 内部使用：上下文继承控制。
    /// TODO: 未来 compact agent 将通过此字段实现 fork 上下文继承。
    pub context_overrides: Option<SubagentContextOverrides>,
}

impl AgentExecutionRequest {
    pub fn from_spawn_agent_params(
        params: &SpawnAgentParams,
        max_steps: Option<u32>,
        context_overrides: Option<SubagentContextOverrides>,
    ) -> Self {
        Self {
            subagent_type: params.r#type.clone(),
            description: params.description.clone(),
            prompt: params.prompt.clone(),
            context: params.context.clone(),
            max_steps,
            context_overrides,
        }
    }
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
            // 步数限制采用“更严格者优先”，避免外部请求绕过 profile 上限。
            max_steps: match (profile.max_steps, params.max_steps) {
                (Some(profile_limit), Some(request_limit)) => {
                    Some(profile_limit.min(request_limit))
                },
                (Some(profile_limit), None) => Some(profile_limit),
                (None, Some(request_limit)) => Some(request_limit),
                (None, None) => None,
            },
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
            capability_name: Some("spawnAgent".to_string()),
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

pub fn build_subrun_handoff(
    child: &astrcode_core::SubRunHandle,
    last_summary: Option<&str>,
    token_limit_hit: bool,
    step_limit_hit: bool,
    duration_ms: u64,
    fallback: &str,
) -> SubRunHandoff {
    let base = last_summary
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string();
    let summary = if token_limit_hit || step_limit_hit {
        format!(
            "{base}\n\n[stopped after {duration_ms}ms because a sub-agent budget limit was \
             reached]"
        )
    } else {
        base
    };

    SubRunHandoff {
        summary,
        // `findings` 应只承载对子任务有业务价值的发现，不能再混入内部执行诊断。
        findings: Vec::new(),
        artifacts: build_result_artifacts(child),
    }
}

pub fn build_background_subrun_handoff(child: &astrcode_core::SubRunHandle) -> SubRunHandoff {
    let mut artifacts = vec![ArtifactRef {
        // `subRun` artifact 把后台句柄结构化暴露给上层，避免再把 sub_run_id 塞进 summary/findings。
        kind: "subRun".to_string(),
        id: child.sub_run_id.clone(),
        label: "Background sub-run".to_string(),
        session_id: None,
        storage_seq: None,
        uri: None,
    }];
    artifacts.extend(build_result_artifacts(child));

    SubRunHandoff {
        summary: "spawnAgent 已在后台启动。".to_string(),
        findings: Vec::new(),
        artifacts,
    }
}

pub fn build_subrun_failure(error: &AstrError) -> SubRunFailure {
    let code = classify_subrun_failure(error);
    let display_message = match code {
        SubRunFailureCode::Transport => "子 Agent 调用模型时网络连接中断，未完成任务。",
        SubRunFailureCode::ProviderHttp => "子 Agent 调用模型服务失败，未完成任务。",
        SubRunFailureCode::StreamParse => "子 Agent 解析模型流式响应失败，未完成任务。",
        SubRunFailureCode::Interrupted => "子 Agent 执行被中断，未完成任务。",
        SubRunFailureCode::Internal => "子 Agent 执行失败，未完成任务。",
    }
    .to_string();

    SubRunFailure {
        code,
        display_message,
        technical_message: error.to_string(),
        retryable: error.is_retryable(),
    }
}

fn classify_subrun_failure(error: &AstrError) -> SubRunFailureCode {
    match error {
        AstrError::LlmRequestFailed { .. } => SubRunFailureCode::ProviderHttp,
        AstrError::LlmStreamError(_) => SubRunFailureCode::StreamParse,
        AstrError::Cancelled | AstrError::LlmInterrupted => SubRunFailureCode::Interrupted,
        AstrError::Network(_) | AstrError::HttpRequest { .. } => SubRunFailureCode::Transport,
        _ => SubRunFailureCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentMode, AgentProfile, AgentStatus, AstrError, InvocationKind, SubRunFailureCode,
        SubRunStorageMode, SubagentContextOverrides,
    };
    use astrcode_runtime_agent_tool::SpawnAgentParams;

    use super::{
        AgentExecutionRequest, build_background_subrun_handoff, build_execution_spec,
        build_subrun_failure, build_subrun_handoff,
    };

    #[test]
    fn build_subrun_handoff_appends_budget_limit_note() {
        let handoff = build_subrun_handoff(
            &astrcode_core::SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-1".to_string(),
                session_id: "session-1".to_string(),
                child_session_id: None,
                depth: 1,
                parent_turn_id: Some("turn-1".to_string()),
                parent_agent_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::SharedSession,
                status: AgentStatus::Completed,
            },
            Some("done"),
            true,
            false,
            1200,
            "fallback summary",
        );

        assert!(handoff.summary.contains("done"));
        assert!(handoff.summary.contains("stopped after 1200ms"));
    }

    #[test]
    fn build_subrun_handoff_uses_fallback_when_summary_missing() {
        let handoff = build_subrun_handoff(
            &astrcode_core::SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-1".to_string(),
                session_id: "session-1".to_string(),
                child_session_id: None,
                depth: 1,
                parent_turn_id: Some("turn-1".to_string()),
                parent_agent_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::SharedSession,
                status: AgentStatus::Completed,
            },
            None,
            false,
            false,
            100,
            "fallback summary",
        );

        assert_eq!(handoff.summary, "fallback summary");
    }

    #[test]
    fn build_subrun_failure_classifies_transport_errors() {
        let failure = build_subrun_failure(&AstrError::Network("connection reset".to_string()));

        assert_eq!(failure.code, SubRunFailureCode::Transport);
        assert_eq!(failure.technical_message, "network error: connection reset");
    }

    #[test]
    fn build_background_subrun_handoff_exposes_subrun_artifact() {
        let handoff = build_background_subrun_handoff(&astrcode_core::SubRunHandle {
            sub_run_id: "subrun-1".to_string(),
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            child_session_id: Some("child-1".to_string()),
            depth: 1,
            parent_turn_id: Some("turn-1".to_string()),
            parent_agent_id: None,
            agent_profile: "plan".to_string(),
            storage_mode: SubRunStorageMode::IndependentSession,
            status: AgentStatus::Running,
        });

        assert_eq!(handoff.summary, "spawnAgent 已在后台启动。");
        assert_eq!(handoff.artifacts[0].kind, "subRun");
        assert_eq!(handoff.artifacts[0].id, "subrun-1");
        assert_eq!(handoff.artifacts[1].kind, "session");
    }

    #[test]
    fn build_execution_spec_uses_stricter_of_profile_and_request_max_steps() {
        let profile = AgentProfile {
            id: "plan".to_string(),
            name: "Plan".to_string(),
            description: "plan".to_string(),
            mode: AgentMode::All,
            system_prompt: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            max_steps: Some(8),
            token_budget: None,
            model_preference: None,
        };
        let request = AgentExecutionRequest {
            subagent_type: Some("plan".to_string()),
            description: "task".to_string(),
            prompt: "task".to_string(),
            context: None,
            max_steps: Some(3),
            context_overrides: None,
        };

        let spec = build_execution_spec(
            InvocationKind::RootExecution,
            &profile,
            &request,
            &["readFile".to_string()],
            &astrcode_runtime_config::RuntimeConfig::default(),
            None,
        )
        .expect("execution spec should build");

        assert_eq!(spec.resolved_limits.max_steps, Some(3));
    }

    #[test]
    fn execution_request_can_be_built_from_spawn_agent_params() {
        let request = AgentExecutionRequest::from_spawn_agent_params(
            &SpawnAgentParams {
                r#type: Some("reviewer".to_string()),
                description: "review patch".to_string(),
                prompt: "review the latest diff".to_string(),
                context: Some("focus on correctness".to_string()),
            },
            Some(5),
            Some(SubagentContextOverrides::default()),
        );

        assert_eq!(request.subagent_type.as_deref(), Some("reviewer"));
        assert_eq!(request.description, "review patch");
        assert_eq!(request.prompt, "review the latest diff");
        assert_eq!(request.context.as_deref(), Some("focus on correctness"));
        assert_eq!(request.max_steps, Some(5));
        assert!(request.context_overrides.is_some());
    }
}
