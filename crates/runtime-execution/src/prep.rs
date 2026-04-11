//! Agent 执行装配模块。
//!
//! 负责执行前的准备工作，包括：
//! - Profile 工具集裁剪与验证
//! - 执行限制解析（步数、token、工具白名单）
//! - 子 Agent 状态构建
//! - 执行结果构建（handoff/failure/artifacts）
//!
//! 设计原则：纯函数无状态，让 runtime façade 专注于编排。
//! 本文件刻意不持有运行时锁、也不启动后台任务，便于持续审查
//! lock-held-await 和 unmanaged spawn 风险。

use std::{collections::HashSet, sync::Arc};

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentState, ArtifactRef, AstrError, ExecutionOwner,
    HookHandler, InvocationKind, LlmMessage, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SpawnAgentParams, StorageEvent, StorageEventPayload,
    SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunStorageMode, SubagentContextOverrides,
    UserMessageOrigin,
};
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;

use crate::{
    ResolvedContextSnapshot,
    context::{CHILD_INHERITED_COMPACT_SUMMARY_BLOCK_ID, CHILD_INHERITED_RECENT_TAIL_BLOCK_ID},
    policy::resolve_subagent_overrides,
    resolve_context_snapshot,
};

#[derive(Debug, Clone)]
pub struct AgentExecutionSpec {
    pub invocation_kind: InvocationKind,
    pub resolved_overrides: ResolvedSubagentContextOverrides,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub resolved_context_snapshot: ResolvedContextSnapshot,
}

/// Agent 执行请求。
// TODO: 未来可能需要重新添加 max_steps 参数来限制子智能体执行
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
    /// 内部使用：上下文继承控制。
    /// TODO: 未来 compact agent 将通过此字段实现 fork 上下文继承。
    pub context_overrides: Option<SubagentContextOverrides>,
}

impl AgentExecutionRequest {
    pub fn from_spawn_agent_params(
        params: &SpawnAgentParams,
        context_overrides: Option<SubagentContextOverrides>,
    ) -> Self {
        Self {
            subagent_type: params.r#type.clone(),
            description: params.description.clone(),
            prompt: params.prompt.clone(),
            context: params.context.clone(),
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
    pub prompt_builder: astrcode_runtime_prompt::LayeredPromptBuilder,
    pub active_profile: String,
    pub runtime_config: astrcode_runtime_config::RuntimeConfig,
}

#[derive(Debug, Clone)]
pub struct PreparedPromptSubmission {
    pub text: String,
    // TODO: 未来可能需要添加 token_budget 参数
    pub user_event: StorageEvent,
    pub execution_owner: ExecutionOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSessionPlan {
    pub should_cancel_session: bool,
    pub active_turn_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RootExecutionLaunch {
    pub agent: AgentEventContext,
    pub user_event: StorageEvent,
    pub execution_owner: ExecutionOwner,
}

fn build_execution_spec(
    invocation_kind: InvocationKind,
    _profile: &AgentProfile, // TODO: 未来可能需要使用 profile 参数
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
        astrcode_runtime_prompt::LayeredPromptBuilder,
        &str,
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
        &execution_spec.resolved_context_snapshot,
    );
    let scoped_capabilities = surface.capabilities.subset_for_tools(&final_tool_names)?;
    let loop_ = build_loop(
        scoped_capabilities,
        prompt_declarations.clone(),
        surface.skill_catalog.clone(),
        surface.hook_handlers.clone(),
        surface.prompt_builder.clone(),
        &surface.active_profile,
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
    resolved_context_snapshot: &ResolvedContextSnapshot,
) -> Vec<PromptDeclaration> {
    let mut declarations =
        if overrides.inherit_system_instructions || overrides.inherit_project_instructions {
            parent.to_vec()
        } else {
            Vec::new()
        };
    declarations.extend(build_inherited_context_prompt_declarations(
        resolved_context_snapshot,
    ));
    if let Some(system_prompt) = profile.system_prompt.as_ref() {
        declarations.push(PromptDeclaration {
            block_id: format!("subagent.profile.{}", profile.id),
            title: format!("Sub-Agent Profile: {}", profile.name),
            content: system_prompt.clone(),
            render_target: astrcode_runtime_prompt::PromptDeclarationRenderTarget::System,
            layer: astrcode_runtime_prompt::PromptLayer::SemiStable,
            kind: astrcode_runtime_prompt::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(100),
            always_include: true,
            source: astrcode_runtime_prompt::PromptDeclarationSource::Builtin,
            capability_name: Some("spawn".to_string()),
            origin: Some(format!("agent-profile:{}", profile.id)),
        });
    }
    declarations
}

fn build_inherited_context_prompt_declarations(
    resolved_context_snapshot: &ResolvedContextSnapshot,
) -> Vec<PromptDeclaration> {
    let mut declarations = Vec::new();

    if let Some(summary) = resolved_context_snapshot.inherited_compact_summary.as_ref() {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            return declarations;
        }
        declarations.push(PromptDeclaration {
            block_id: CHILD_INHERITED_COMPACT_SUMMARY_BLOCK_ID.to_string(),
            title: "Inherited Compact Summary".to_string(),
            content: trimmed.to_string(),
            render_target: astrcode_runtime_prompt::PromptDeclarationRenderTarget::System,
            layer: astrcode_runtime_prompt::PromptLayer::Inherited,
            kind: astrcode_runtime_prompt::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(540),
            always_include: true,
            source: astrcode_runtime_prompt::PromptDeclarationSource::Builtin,
            capability_name: Some("spawn".to_string()),
            origin: Some("child-context:compact-summary".to_string()),
        });
    }

    if !resolved_context_snapshot.inherited_recent_tail.is_empty() {
        let joined: String = resolved_context_snapshot
            .inherited_recent_tail
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if joined.is_empty() {
            return declarations;
        }
        declarations.push(PromptDeclaration {
            block_id: CHILD_INHERITED_RECENT_TAIL_BLOCK_ID.to_string(),
            title: "Inherited Recent Tail".to_string(),
            content: joined,
            render_target: astrcode_runtime_prompt::PromptDeclarationRenderTarget::System,
            layer: astrcode_runtime_prompt::PromptLayer::Inherited,
            kind: astrcode_runtime_prompt::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(550),
            always_include: true,
            source: astrcode_runtime_prompt::PromptDeclarationSource::Builtin,
            capability_name: Some("spawn".to_string()),
            origin: Some("child-context:recent-tail".to_string()),
        });
    }

    declarations
}

pub fn build_child_agent_state(
    session_id: &str,
    working_dir: std::path::PathBuf,
    task_payload: &str,
) -> AgentState {
    AgentState {
        session_id: session_id.to_string(),
        working_dir,
        messages: vec![LlmMessage::User {
            content: task_payload.to_string(),
            origin: UserMessageOrigin::User,
        }],
        phase: astrcode_core::Phase::Thinking,
        turn_count: 0,
    }
}

/// 在 durable replay 的基础上为 child session 追加一条新的恢复任务。
///
/// 为什么不直接从空状态重建：
/// resume 的语义是继续同一个 child session，而不是伪造一个看起来相似的新 spawn。
pub fn build_resumed_child_agent_state(
    mut replayed_state: AgentState,
    resume_message: &str,
) -> AgentState {
    replayed_state.messages.push(LlmMessage::User {
        content: resume_message.to_string(),
        origin: UserMessageOrigin::User,
    });
    replayed_state.phase = astrcode_core::Phase::Thinking;
    replayed_state
}

pub fn prepare_prompt_submission(
    session_id: &str,
    turn_id: &str,
    text: String,
    _token_budget: Option<u64>, // TODO: 未来可能需要使用 token_budget 参数
) -> PreparedPromptSubmission {
    prepare_prompt_submission_with_origin(
        session_id,
        turn_id,
        text,
        _token_budget,
        UserMessageOrigin::User,
    )
}

pub fn prepare_prompt_submission_with_origin(
    session_id: &str,
    turn_id: &str,
    text: String,
    _token_budget: Option<u64>, // TODO: 未来可能需要使用 token_budget 参数
    origin: UserMessageOrigin,
) -> PreparedPromptSubmission {
    PreparedPromptSubmission {
        user_event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::UserMessage {
                content: text.clone(),
                timestamp: chrono::Utc::now(),
                origin,
            },
        },
        execution_owner: ExecutionOwner::root(
            session_id.to_string(),
            turn_id.to_string(),
            InvocationKind::RootExecution,
        ),
        text,
    }
}

pub fn resolve_interrupt_session_plan(
    is_running: bool,
    active_turn_id: Option<&str>,
) -> InterruptSessionPlan {
    InterruptSessionPlan {
        should_cancel_session: is_running && active_turn_id.is_some(),
        active_turn_id: active_turn_id.map(ToOwned::to_owned),
    }
}

pub fn summarize_execution_description(task: &str) -> String {
    if task.len() > 50 {
        task.chars().take(30).collect::<String>() + "..."
    } else {
        task.to_string()
    }
}

pub fn build_root_spawn_params(
    agent_id: String,
    task: String,
    context: Option<String>,
) -> SpawnAgentParams {
    SpawnAgentParams {
        r#type: Some(agent_id),
        description: summarize_execution_description(&task),
        prompt: task,
        context,
    }
}

pub fn validate_root_execution_storage_mode(
    storage_mode: SubRunStorageMode,
) -> Result<(), AstrError> {
    if matches!(storage_mode, SubRunStorageMode::IndependentSession) {
        return Err(AstrError::Validation(
            "root execution already runs in its own session; \
             contextOverrides.storageMode=independentSession is not applicable"
                .to_string(),
        ));
    }
    Ok(())
}

pub fn prepare_root_execution_launch(
    session_id: &str,
    turn_id: &str,
    root_agent_id: String,
    profile_id: String,
    task_payload: String,
) -> RootExecutionLaunch {
    let agent = AgentEventContext::root_execution(root_agent_id, profile_id);
    RootExecutionLaunch {
        user_event: StorageEvent {
            turn_id: Some(turn_id.to_string()),
            agent: agent.clone(),
            payload: StorageEventPayload::UserMessage {
                content: task_payload,
                timestamp: chrono::Utc::now(),
                origin: UserMessageOrigin::User,
            },
        },
        execution_owner: ExecutionOwner::root(
            session_id.to_string(),
            turn_id.to_string(),
            InvocationKind::RootExecution,
        ),
        agent,
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
    let open_session_id = child
        .child_session_id
        .as_ref()
        .cloned()
        .unwrap_or_else(|| child.session_id.clone());

    vec![ArtifactRef {
        kind: "session".to_string(),
        id: open_session_id.clone(),
        label: if child.child_session_id.is_some() {
            "Independent child session".to_string()
        } else {
            "Shared parent session".to_string()
        },
        session_id: Some(open_session_id),
        storage_seq: None,
        uri: None,
    }]
}

fn build_identity_artifacts(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
) -> Vec<ArtifactRef> {
    let mut artifacts = vec![
        ArtifactRef {
            kind: "subRun".to_string(),
            id: child.sub_run_id.clone(),
            label: "Background sub-run".to_string(),
            session_id: None,
            storage_seq: None,
            uri: None,
        },
        ArtifactRef {
            kind: "agent".to_string(),
            id: child.agent_id.clone(),
            label: "Child agent id".to_string(),
            session_id: None,
            storage_seq: None,
            uri: None,
        },
        ArtifactRef {
            kind: "parentSession".to_string(),
            id: parent_session_id.to_string(),
            label: "Parent session".to_string(),
            session_id: Some(parent_session_id.to_string()),
            storage_seq: None,
            uri: None,
        },
    ];
    if let Some(parent_agent_id) = child.parent_agent_id.as_ref() {
        artifacts.push(ArtifactRef {
            kind: "parentAgent".to_string(),
            id: parent_agent_id.clone(),
            label: "Parent agent id".to_string(),
            session_id: None,
            storage_seq: None,
            uri: None,
        });
    }
    artifacts
}

pub fn build_subrun_handoff(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
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
        artifacts: {
            let mut artifacts = build_identity_artifacts(child, parent_session_id);
            artifacts.extend(build_result_artifacts(child));
            artifacts
        },
    }
}

pub fn build_background_subrun_handoff(
    child: &astrcode_core::SubRunHandle,
    parent_session_id: &str,
) -> SubRunHandoff {
    let mut artifacts = build_identity_artifacts(child, parent_session_id);
    artifacts.extend(build_result_artifacts(child));

    SubRunHandoff {
        summary: "spawn 已在后台启动。".to_string(),
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
        AgentMode, AgentProfile, AgentStatus, AstrError, InvocationKind,
        ResolvedSubagentContextOverrides, SpawnAgentParams, SubRunFailureCode, SubRunStorageMode,
        SubagentContextOverrides,
    };
    use astrcode_runtime_prompt::PromptLayer;

    use super::{
        AgentExecutionRequest, build_background_subrun_handoff, build_child_agent_state,
        build_child_prompt_declarations, build_execution_spec, build_resumed_child_agent_state,
        build_root_spawn_params, build_subrun_failure, build_subrun_handoff,
        prepare_prompt_submission, prepare_prompt_submission_with_origin,
        prepare_root_execution_launch, resolve_interrupt_session_plan,
        summarize_execution_description, validate_root_execution_storage_mode,
    };
    use crate::ResolvedContextSnapshot;

    #[test]
    fn build_subrun_handoff_appends_budget_limit_note() {
        let handoff = build_subrun_handoff(
            &astrcode_core::SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-1".to_string(),
                session_id: "session-1".to_string(),
                child_session_id: None,
                depth: 1,
                parent_turn_id: "turn-1".to_string(),
                parent_agent_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::SharedSession,
                status: AgentStatus::Completed,
            },
            "session-parent-1",
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
                parent_turn_id: "turn-1".to_string(),
                parent_agent_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::SharedSession,
                status: AgentStatus::Completed,
            },
            "session-parent-1",
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
        let handoff = build_background_subrun_handoff(
            &astrcode_core::SubRunHandle {
                sub_run_id: "subrun-1".to_string(),
                agent_id: "agent-1".to_string(),
                session_id: "session-1".to_string(),
                child_session_id: Some("child-1".to_string()),
                depth: 1,
                parent_turn_id: "turn-1".to_string(),
                parent_agent_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                status: AgentStatus::Running,
            },
            "session-parent-1",
        );

        assert_eq!(handoff.summary, "spawn 已在后台启动。");
        assert_eq!(handoff.artifacts[0].kind, "subRun");
        assert_eq!(handoff.artifacts[0].id, "subrun-1");
        assert_eq!(handoff.artifacts[1].kind, "agent");
        assert_eq!(handoff.artifacts[2].kind, "parentSession");
        assert_eq!(handoff.artifacts[3].kind, "session");
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
            model_preference: None,
        };
        let request = AgentExecutionRequest {
            subagent_type: Some("plan".to_string()),
            description: "task".to_string(),
            prompt: "task".to_string(),
            context: None,
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

        assert_eq!(
            spec.resolved_limits.allowed_tools,
            vec!["readFile".to_string()]
        );
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
            Some(SubagentContextOverrides::default()),
        );

        assert_eq!(request.subagent_type.as_deref(), Some("reviewer"));
        assert_eq!(request.description, "review patch");
        assert_eq!(request.prompt, "review the latest diff");
        assert_eq!(request.context.as_deref(), Some("focus on correctness"));
        assert!(request.context_overrides.is_some());
    }

    #[test]
    fn child_prompt_declarations_add_inherited_summary_and_tail_blocks() {
        let profile = AgentProfile {
            id: "review".to_string(),
            name: "Review".to_string(),
            description: "review".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: Some("profile guidance".to_string()),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model_preference: None,
        };

        let declarations = build_child_prompt_declarations(
            &[],
            &profile,
            &ResolvedSubagentContextOverrides {
                inherit_system_instructions: true,
                inherit_project_instructions: true,
                ..ResolvedSubagentContextOverrides::default()
            },
            &ResolvedContextSnapshot {
                task_payload: "# Task\ninspect auth flow".to_string(),
                inherited_compact_summary: Some("parent summary".to_string()),
                inherited_recent_tail: vec![
                    "- user: inspect auth".to_string(),
                    "- assistant: checking".to_string(),
                ],
            },
        );

        assert_eq!(declarations.len(), 3);
        assert!(declarations.iter().any(|declaration| {
            declaration.block_id == "child.inherited.compact_summary"
                && declaration.layer == PromptLayer::Inherited
                && declaration.content == "parent summary"
        }));
        assert!(declarations.iter().any(|declaration| {
            declaration.block_id == "child.inherited.recent_tail"
                && declaration.layer == PromptLayer::Inherited
                && declaration.content.contains("- user: inspect auth")
        }));
        assert!(declarations.iter().any(|declaration| {
            declaration.block_id == "subagent.profile.review"
                && declaration.layer == PromptLayer::SemiStable
        }));
    }

    #[test]
    fn build_child_agent_state_keeps_only_task_payload_in_messages() {
        let child_state = build_child_agent_state(
            "session-child",
            std::env::temp_dir(),
            "# Task\ninspect auth module\n\n# Context\nfocus on cache misses",
        );

        assert_eq!(child_state.messages.len(), 1);
        assert!(matches!(
            &child_state.messages[0],
            astrcode_core::LlmMessage::User { content, .. }
                if content.contains("# Task\ninspect auth module")
                    && !content.contains("Parent Compact Summary")
                    && !content.contains("Recent Tail")
        ));
    }

    #[test]
    fn build_resumed_child_agent_state_keeps_replayed_history_and_appends_resume_message() {
        let replayed = astrcode_core::AgentState {
            session_id: "session-child".to_string(),
            working_dir: std::env::temp_dir(),
            messages: vec![
                astrcode_core::LlmMessage::User {
                    content: "旧任务".to_string(),
                    origin: astrcode_core::UserMessageOrigin::User,
                },
                astrcode_core::LlmMessage::Assistant {
                    content: "已有分析".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            phase: astrcode_core::Phase::Idle,
            turn_count: 2,
        };

        let resumed = build_resumed_child_agent_state(replayed, "继续完成剩余检查");

        assert_eq!(resumed.messages.len(), 3);
        assert_eq!(resumed.turn_count, 2);
        assert_eq!(resumed.phase, astrcode_core::Phase::Thinking);
        assert!(matches!(
            &resumed.messages[1],
            astrcode_core::LlmMessage::Assistant { content, .. } if content == "已有分析"
        ));
        assert!(matches!(
            &resumed.messages[2],
            astrcode_core::LlmMessage::User { content, .. } if content == "继续完成剩余检查"
        ));
    }

    #[test]
    fn prepare_prompt_submission_builds_root_owner_and_user_event() {
        let prepared =
            prepare_prompt_submission("session-1", "turn-1", "hello".to_string(), Some(128));

        assert_eq!(prepared.text, "hello");
        // TODO: 未来可能需要验证 token_budget
        assert_eq!(prepared.execution_owner.root_session_id, "session-1");
        assert_eq!(prepared.execution_owner.root_turn_id, "turn-1");
        assert!(matches!(
            prepared.user_event.payload,
            astrcode_core::StorageEventPayload::UserMessage { .. }
        ));
    }

    #[test]
    fn prepare_prompt_submission_with_origin_keeps_internal_origin() {
        let prepared = prepare_prompt_submission_with_origin(
            "session-1",
            "turn-2",
            "internal".to_string(),
            None,
            astrcode_core::UserMessageOrigin::ReactivationPrompt,
        );

        assert!(matches!(
            prepared.user_event.payload,
            astrcode_core::StorageEventPayload::UserMessage {
                origin: astrcode_core::UserMessageOrigin::ReactivationPrompt,
                ..
            }
        ));
    }

    #[test]
    fn resolve_interrupt_session_plan_requires_running_turn() {
        assert_eq!(
            resolve_interrupt_session_plan(false, Some("turn-1")),
            super::InterruptSessionPlan {
                should_cancel_session: false,
                active_turn_id: Some("turn-1".to_string()),
            }
        );
        assert_eq!(
            resolve_interrupt_session_plan(true, None),
            super::InterruptSessionPlan {
                should_cancel_session: false,
                active_turn_id: None,
            }
        );
        assert!(resolve_interrupt_session_plan(true, Some("turn-1")).should_cancel_session);
    }

    #[test]
    fn root_execution_helpers_build_consistent_launch_shapes() {
        let params = build_root_spawn_params(
            "plan".to_string(),
            "review the repository layout and write down findings".to_string(),
            Some("focus on boundaries".to_string()),
        );
        assert_eq!(params.r#type.as_deref(), Some("plan"));
        assert_eq!(params.context.as_deref(), Some("focus on boundaries"));
        assert!(params.description.ends_with("..."));
        assert!(summarize_execution_description("short") == "short");
        assert!(validate_root_execution_storage_mode(SubRunStorageMode::SharedSession).is_ok());
        assert!(
            validate_root_execution_storage_mode(SubRunStorageMode::IndependentSession).is_err()
        );

        let launch = prepare_root_execution_launch(
            "session-1",
            "turn-1",
            "root-agent-1".to_string(),
            "plan".to_string(),
            "task body".to_string(),
        );
        assert_eq!(launch.agent.agent_id.as_deref(), Some("root-agent-1"));
        assert_eq!(launch.execution_owner.root_session_id, "session-1");
        assert!(matches!(
            launch.user_event.payload,
            astrcode_core::StorageEventPayload::UserMessage { .. }
        ));
    }
}
