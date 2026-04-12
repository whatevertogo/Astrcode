//! Agent 执行装配模块。
//!
//! 负责执行前的准备工作，包括：
//! - Profile 工具集裁剪与验证
//! - 子 Agent 状态构建
//! - 执行结果构建（handoff/failure/artifacts）
//!
//! 设计原则：纯函数无状态，不持有运行时锁、也不启动后台任务。
//! 从 runtime-execution/prep.rs 迁移，去除对旧 crate 的依赖。

use std::collections::HashSet;

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentState, ArtifactRef, AstrError, ExecutionOwner,
    InvocationKind, LlmMessage, SpawnAgentParams, StorageEvent, StorageEventPayload, SubRunFailure,
    SubRunFailureCode, SubRunHandoff, SubRunStorageMode, SubagentContextOverrides, ToolContext,
    UserMessageOrigin,
};

use crate::{
    execution::{context::ResolvedContextSnapshot, policy::resolve_subagent_overrides},
    registry::CapabilityRouter,
};

// ── 数据结构 ─────────────────────────────────────────────────

/// Agent 执行规格，包含解析后的覆盖、限制和上下文快照。
#[derive(Debug, Clone)]
pub struct AgentExecutionSpec {
    pub invocation_kind: InvocationKind,
    pub resolved_overrides: astrcode_core::ResolvedSubagentContextOverrides,
    pub resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot,
    pub resolved_context_snapshot: ResolvedContextSnapshot,
}

/// Agent 执行请求。
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
    /// 上下文继承控制。
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

/// 提交提示的准备结果。
#[derive(Debug, Clone)]
pub struct PreparedPromptSubmission {
    pub text: String,
    pub user_event: StorageEvent,
    pub execution_owner: ExecutionOwner,
}

/// 中断会话的计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSessionPlan {
    pub should_cancel_session: bool,
    pub active_turn_id: Option<String>,
}

/// 根执行启动参数。
#[derive(Debug, Clone)]
pub struct RootExecutionLaunch {
    pub agent: AgentEventContext,
    pub user_event: StorageEvent,
    pub execution_owner: ExecutionOwner,
}

// ── 执行准备函数 ──────────────────────────────────────────────

/// 构建执行规格。
pub fn build_execution_spec(
    invocation_kind: InvocationKind,
    params: &AgentExecutionRequest,
    allowed_tools: &[String],
    parent_state: Option<&AgentState>,
) -> Result<AgentExecutionSpec, AstrError> {
    let resolved_overrides = resolve_subagent_overrides(params.context_overrides.as_ref())?;
    let resolved_context_snapshot = crate::execution::context::resolve_context_snapshot(
        params,
        parent_state,
        &resolved_overrides,
    );

    Ok(AgentExecutionSpec {
        invocation_kind,
        resolved_overrides,
        resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot {
            allowed_tools: allowed_tools.to_vec(),
        },
        resolved_context_snapshot,
    })
}

/// 准备提示提交。
pub fn prepare_prompt_submission(
    session_id: &str,
    turn_id: &str,
    text: String,
    _token_budget: Option<u64>,
) -> PreparedPromptSubmission {
    prepare_prompt_submission_with_origin(
        session_id,
        turn_id,
        text,
        _token_budget,
        UserMessageOrigin::User,
    )
}

/// 准备提示提交（带指定 origin）。
pub fn prepare_prompt_submission_with_origin(
    session_id: &str,
    turn_id: &str,
    text: String,
    _token_budget: Option<u64>,
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

/// 解析中断会话计划。
pub fn resolve_interrupt_session_plan(
    is_running: bool,
    active_turn_id: Option<&str>,
) -> InterruptSessionPlan {
    InterruptSessionPlan {
        should_cancel_session: is_running && active_turn_id.is_some(),
        active_turn_id: active_turn_id.map(ToOwned::to_owned),
    }
}

/// 截断执行描述用于日志。
pub fn summarize_execution_description(task: &str) -> String {
    if task.len() > 50 {
        task.chars().take(30).collect::<String>() + "..."
    } else {
        task.to_string()
    }
}

/// 构建根 Agent 的 spawn 参数。
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

/// 只有一种存储模式 IndependentSession，无需校验。
pub fn validate_root_execution_storage_mode(
    _storage_mode: SubRunStorageMode,
) -> Result<(), AstrError> {
    Ok(())
}

/// 准备根执行启动。
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

/// 校验 profile 允许作为子 Agent 运行。
pub fn ensure_subagent_mode(profile: &AgentProfile) -> Result<(), AstrError> {
    if matches!(profile.mode, AgentMode::SubAgent | AgentMode::All) {
        return Ok(());
    }
    Err(AstrError::Validation(format!(
        "agent profile '{}' is not allowed to run as a sub-agent",
        profile.id
    )))
}

/// 校验 profile 允许作为根执行运行。
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

/// 根据 profile 的 allowed/disallowed 工具列表和当前能力路由，
/// 计算出最终的工具名列表。
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

/// 构建子 Agent 的初始 AgentState。
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
        last_assistant_at: None,
    }
}

/// 在 durable replay 的基础上为 child session 追加一条新的恢复任务。
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

/// 推导子 Agent 的 ExecutionOwner。
pub fn derive_child_execution_owner(
    ctx: &ToolContext,
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

/// 构建 subrun 执行结果的手交信息。
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
        findings: Vec::new(),
        artifacts: {
            let mut artifacts = build_identity_artifacts(child, parent_session_id);
            artifacts.extend(build_result_artifacts(child));
            artifacts
        },
    }
}

/// 构建后台 subrun 的手交信息。
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

/// 构建子 Agent 执行失败的结果。
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

/// 构建子 Agent 结果的 artifact 列表。
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

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentLifecycleStatus, AgentMode, AgentProfile, AgentState, AstrError, InvocationKind,
        SubRunFailureCode, SubRunStorageMode, SubagentContextOverrides,
    };

    use super::{
        AgentExecutionRequest, build_background_subrun_handoff, build_child_agent_state,
        build_execution_spec, build_resumed_child_agent_state, build_root_spawn_params,
        build_subrun_failure, build_subrun_handoff, prepare_prompt_submission,
        prepare_root_execution_launch, resolve_interrupt_session_plan,
        summarize_execution_description, validate_root_execution_storage_mode,
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
                parent_turn_id: "turn-1".to_string(),
                parent_agent_id: None,
                parent_sub_run_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: None,
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
                parent_sub_run_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: None,
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
                parent_sub_run_id: None,
                agent_profile: "plan".to_string(),
                storage_mode: SubRunStorageMode::IndependentSession,
                lifecycle: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
            "session-parent-1",
        );

        assert_eq!(handoff.summary, "spawn 已在后台启动。");
        assert_eq!(handoff.artifacts[0].kind, "subRun");
        assert_eq!(handoff.artifacts[0].id, "subrun-1");
    }

    #[test]
    fn build_execution_spec_resolves_allowed_tools() {
        let _profile = AgentProfile {
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
            &request,
            &["readFile".to_string()],
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
            &astrcode_core::SpawnAgentParams {
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
        ));
    }

    #[test]
    fn build_resumed_child_agent_state_keeps_replayed_history_and_appends_resume_message() {
        let replayed = AgentState {
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
            last_assistant_at: None,
        };

        let resumed = build_resumed_child_agent_state(replayed, "继续完成剩余检查");

        assert_eq!(resumed.messages.len(), 3);
        assert_eq!(resumed.turn_count, 2);
        assert_eq!(resumed.phase, astrcode_core::Phase::Thinking);
    }

    #[test]
    fn prepare_prompt_submission_builds_root_owner_and_user_event() {
        let prepared =
            prepare_prompt_submission("session-1", "turn-1", "hello".to_string(), Some(128));

        assert_eq!(prepared.text, "hello");
        assert_eq!(
            prepared.execution_owner.root_session_id,
            astrcode_core::SessionId::from("session-1")
        );
        assert!(matches!(
            prepared.user_event.payload,
            astrcode_core::StorageEventPayload::UserMessage { .. }
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
        assert!(params.description.ends_with("..."));
        assert!(summarize_execution_description("short") == "short");
        assert!(
            validate_root_execution_storage_mode(SubRunStorageMode::IndependentSession).is_ok()
        );

        let launch = prepare_root_execution_launch(
            "session-1",
            "turn-1",
            "root-agent-1".to_string(),
            "plan".to_string(),
            "task body".to_string(),
        );
        assert_eq!(launch.agent.agent_id.as_deref(), Some("root-agent-1"));
        assert!(matches!(
            launch.user_event.payload,
            astrcode_core::StorageEventPayload::UserMessage { .. }
        ));
    }
}
