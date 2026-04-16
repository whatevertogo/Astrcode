//! 子代理执行入口。
//!
//! 实现 `launch_subagent`：spawn 参数解析 → profile 校验 → child session 创建 → turn 启动。
//! 子代理执行结果通过 parent delivery 机制回流到父级。

use std::sync::Arc;

use astrcode_core::{
    AgentLifecycleStatus, AgentMode, AgentProfile, ExecutionAccepted, ForkMode, LlmMessage,
    ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
    RuntimeMetricsRecorder, SessionId, SpawnCapabilityGrant, UserMessageOrigin,
    normalize_non_empty_unique_string_list, project,
};
use astrcode_kernel::AgentControlError;
use astrcode_session_runtime::AgentPromptSubmission;

use crate::{
    AgentKernelPort, AgentSessionPort,
    agent::{
        build_delegation_metadata, build_fresh_child_contract, persist_delegation_for_handle,
        persist_resolved_limits_for_handle, subrun_event_context,
    },
    errors::ApplicationError,
    execution::{ensure_profile_mode, merge_task_with_context},
};

/// 子代理执行请求。
pub struct SubagentExecutionRequest {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub parent_turn_id: String,
    pub working_dir: String,
    pub profile: AgentProfile,
    pub description: String,
    pub task: String,
    pub context: Option<String>,
    pub parent_allowed_tools: Vec<String>,
    pub capability_grant: Option<SpawnCapabilityGrant>,
    pub source_tool_call_id: Option<String>,
}

/// 启动子代理执行。
///
/// 完整流程：
/// 1. 参数校验
/// 2. 校验 profile mode
/// 3. 创建独立 child session
/// 4. 在控制树中注册子 agent
/// 5. 异步提交 prompt
pub async fn launch_subagent(
    kernel: &dyn AgentKernelPort,
    session_runtime: &dyn AgentSessionPort,
    request: SubagentExecutionRequest,
    runtime_config: ResolvedRuntimeConfig,
    metrics: &Arc<dyn RuntimeMetricsRecorder>,
) -> Result<ExecutionAccepted, ApplicationError> {
    validate_subagent_request(&request)?;
    ensure_subagent_profile_mode(&request.profile)?;
    let resolved_limits = resolve_child_execution_limits(
        &request.parent_allowed_tools,
        request.capability_grant.as_ref(),
        runtime_config.max_steps as u32,
    )?;
    let scoped_gateway = kernel
        .gateway()
        .subset_for_tools_checked(&resolved_limits.allowed_tools)
        .map_err(|error| ApplicationError::InvalidArgument(error.to_string()))?;
    let scoped_router = scoped_gateway.capabilities().clone();
    let resolved_overrides = ResolvedSubagentContextOverrides::default();
    let inherited_messages = resolve_inherited_parent_messages(
        session_runtime,
        &request.parent_session_id,
        &resolved_overrides,
    )
    .await?;
    let delegation = build_delegation_metadata(
        request.description.as_str(),
        request.task.as_str(),
        &resolved_limits,
        request.capability_grant.is_some(),
    );
    let contract = build_fresh_child_contract(&delegation);

    let child_session = session_runtime
        .create_child_session(&request.working_dir, &request.parent_session_id)
        .await
        .map_err(ApplicationError::from)?;

    let handle = kernel
        .spawn_independent_child(
            &request.profile,
            request.parent_session_id.clone(),
            child_session.session_id.clone(),
            request.parent_turn_id,
            request.parent_agent_id,
        )
        .await
        .map_err(map_spawn_error)?;
    if kernel
        .set_lifecycle(&handle.agent_id, AgentLifecycleStatus::Running)
        .await
        .is_none()
    {
        return Err(ApplicationError::Internal(format!(
            "failed to mark child agent '{}' as running because the control handle disappeared \
             immediately after spawn",
            handle.agent_id
        )));
    }
    let handle = persist_resolved_limits_for_handle(kernel, handle, resolved_limits.clone())
        .await
        .map_err(ApplicationError::Internal)?;
    let handle = persist_delegation_for_handle(kernel, handle, delegation)
        .await
        .map_err(ApplicationError::Internal)?;

    let merged_task = merge_task_with_context(&request.task, request.context.as_deref());

    let mut accepted = session_runtime
        .submit_prompt_for_agent_with_submission(
            &child_session.session_id,
            merged_task,
            runtime_config,
            AgentPromptSubmission {
                agent: subrun_event_context(&handle),
                capability_router: Some(scoped_router),
                prompt_declarations: vec![contract],
                resolved_limits: Some(resolved_limits),
                resolved_overrides: Some(resolved_overrides),
                injected_messages: inherited_messages,
                source_tool_call_id: request.source_tool_call_id,
            },
        )
        .await
        .map_err(ApplicationError::from)?;
    metrics.record_child_spawned();
    accepted.agent_id = Some(handle.agent_id);
    Ok(accepted)
}

fn resolve_child_execution_limits(
    parent_allowed_tools: &[String],
    capability_grant: Option<&SpawnCapabilityGrant>,
    max_steps: u32,
) -> Result<ResolvedExecutionLimitsSnapshot, ApplicationError> {
    let parent_allowed_tools =
        normalize_non_empty_unique_string_list(parent_allowed_tools, "parentAllowedTools")
            .map_err(ApplicationError::from)?;
    let allowed_tools = match capability_grant {
        Some(grant) => {
            let requested_tools = grant
                .normalized_allowed_tools()
                .map_err(ApplicationError::from)?;
            let allowed_tools = requested_tools
                .into_iter()
                .filter(|tool| parent_allowed_tools.iter().any(|parent| parent == tool))
                .collect::<Vec<_>>();
            if allowed_tools.is_empty() {
                return Err(ApplicationError::InvalidArgument(
                    "capability grant 与父级当前可继承工具无交集".to_string(),
                ));
            }
            allowed_tools
        },
        None => parent_allowed_tools,
    };

    Ok(ResolvedExecutionLimitsSnapshot {
        allowed_tools,
        max_steps: Some(max_steps),
    })
}

fn validate_subagent_request(request: &SubagentExecutionRequest) -> Result<(), ApplicationError> {
    if request.parent_session_id.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'parentSessionId' must not be empty".to_string(),
        ));
    }
    if request.parent_agent_id.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'parentAgentId' must not be empty".to_string(),
        ));
    }
    if request.working_dir.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'workingDir' must not be empty".to_string(),
        ));
    }
    if request.task.trim().is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "field 'task' must not be empty".to_string(),
        ));
    }
    Ok(())
}

async fn resolve_inherited_parent_messages(
    session_runtime: &dyn AgentSessionPort,
    parent_session_id: &str,
    overrides: &ResolvedSubagentContextOverrides,
) -> Result<Vec<LlmMessage>, ApplicationError> {
    let parent_events = session_runtime
        .session_stored_events(&SessionId::from(parent_session_id.to_string()))
        .await
        .map_err(ApplicationError::from)?;
    let projected = project(
        &parent_events
            .iter()
            .map(|stored| stored.event.clone())
            .collect::<Vec<_>>(),
    );

    Ok(build_inherited_messages(&projected.messages, overrides))
}

fn build_inherited_messages(
    parent_messages: &[LlmMessage],
    overrides: &ResolvedSubagentContextOverrides,
) -> Vec<LlmMessage> {
    let mut inherited = Vec::new();

    if overrides.include_compact_summary {
        if let Some(summary) = parent_messages.iter().find(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::CompactSummary,
                    ..
                }
            )
        }) {
            inherited.push(summary.clone());
        }
    }

    if overrides.include_recent_tail {
        inherited.extend(select_inherited_recent_tail(
            parent_messages,
            overrides.fork_mode.as_ref(),
        ));
    }

    inherited
}

fn select_inherited_recent_tail(
    parent_messages: &[LlmMessage],
    fork_mode: Option<&ForkMode>,
) -> Vec<LlmMessage> {
    let non_summary_messages = parent_messages
        .iter()
        .filter(|message| {
            !matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::CompactSummary,
                    ..
                }
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    match fork_mode {
        Some(ForkMode::LastNTurns(turns)) => {
            tail_messages_for_last_n_turns(&non_summary_messages, *turns)
        },
        Some(ForkMode::FullHistory) | None => non_summary_messages,
    }
}

fn tail_messages_for_last_n_turns(messages: &[LlmMessage], turns: usize) -> Vec<LlmMessage> {
    if turns == 0 || messages.is_empty() {
        return Vec::new();
    }

    let mut remaining_turns = turns;
    let mut start_index = 0usize;
    for (index, message) in messages.iter().enumerate().rev() {
        if matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            remaining_turns = remaining_turns.saturating_sub(1);
            start_index = index;
            if remaining_turns == 0 {
                break;
            }
        }
    }

    messages[start_index..].to_vec()
}

fn ensure_subagent_profile_mode(profile: &AgentProfile) -> Result<(), ApplicationError> {
    ensure_profile_mode(profile, &[AgentMode::SubAgent, AgentMode::All], "subagent")
}

fn map_spawn_error(error: AgentControlError) -> ApplicationError {
    match error {
        AgentControlError::MaxDepthExceeded { current, max } => {
            ApplicationError::InvalidArgument(format!(
                "subagent depth limit reached at depth {current} (configured max: {max}); reuse \
                 an existing child with send/observe/close, or finish the work in the current \
                 agent"
            ))
        },
        AgentControlError::MaxConcurrentExceeded { current, max } => {
            ApplicationError::Conflict(format!(
                "too many active agents ({current}/{max}); wait for an existing child to go idle \
                 or close one before spawning more"
            ))
        },
        AgentControlError::ParentAgentNotFound { agent_id } => {
            ApplicationError::NotFound(format!("parent agent '{agent_id}' not found"))
        },
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentMode, AgentProfile, ForkMode, LlmMessage, UserMessageOrigin};

    use super::*;

    fn test_profile() -> AgentProfile {
        AgentProfile {
            id: "explore".to_string(),
            name: "Explore".to_string(),
            description: "探索代码".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            model_preference: None,
        }
    }

    fn valid_request() -> SubagentExecutionRequest {
        SubagentExecutionRequest {
            parent_session_id: "session-1".to_string(),
            parent_agent_id: "root-agent".to_string(),
            parent_turn_id: "turn-1".to_string(),
            working_dir: "/tmp/project".to_string(),
            profile: test_profile(),
            description: "探索代码".to_string(),
            task: "explore the code".to_string(),
            context: None,
            parent_allowed_tools: vec!["read_file".to_string(), "grep".to_string()],
            capability_grant: None,
            source_tool_call_id: None,
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_subagent_request(&valid_request()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_parent_session_id() {
        let mut req = valid_request();
        req.parent_session_id = "  ".to_string();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("parentSessionId"),
            "should mention parentSessionId: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_parent_agent_id() {
        let mut req = valid_request();
        req.parent_agent_id = String::new();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("parentAgentId"),
            "should mention parentAgentId: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_working_dir() {
        let mut req = valid_request();
        req.working_dir.clear();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(err.to_string().contains("workingDir"));
    }

    #[test]
    fn validate_rejects_empty_task() {
        let mut req = valid_request();
        req.task = "   ".to_string();
        let err = validate_subagent_request(&req).unwrap_err();
        assert!(
            err.to_string().contains("task"),
            "should mention task: {err}"
        );
    }

    #[test]
    fn validate_does_not_check_profile_fields() {
        let mut req = valid_request();
        req.profile.name = String::new();
        assert!(validate_subagent_request(&req).is_ok());
    }

    #[test]
    fn ensure_subagent_profile_mode_rejects_primary_only_profile() {
        let err = ensure_subagent_profile_mode(&AgentProfile {
            mode: AgentMode::Primary,
            ..test_profile()
        })
        .expect_err("primary-only profile should be rejected");

        assert!(err.to_string().contains("subagent"));
    }

    #[test]
    fn map_spawn_error_turns_depth_limit_into_actionable_invalid_argument() {
        let err = map_spawn_error(AgentControlError::MaxDepthExceeded { current: 4, max: 3 });

        assert!(matches!(err, ApplicationError::InvalidArgument(_)));
        assert!(err.to_string().contains("configured max: 3"));
        assert!(err.to_string().contains("send/observe/close"));
    }

    #[test]
    fn map_spawn_error_turns_concurrency_limit_into_conflict() {
        let err = map_spawn_error(AgentControlError::MaxConcurrentExceeded { current: 8, max: 4 });

        assert!(matches!(err, ApplicationError::Conflict(_)));
        assert!(err.to_string().contains("too many active agents"));
    }

    #[test]
    fn resolve_child_execution_limits_inherits_parent_tools_when_no_grant() {
        let limits = resolve_child_execution_limits(
            &["read_file".to_string(), "grep".to_string()],
            None,
            12,
        )
        .expect("limits should resolve");

        assert_eq!(limits.allowed_tools, vec!["read_file", "grep"]);
        assert_eq!(limits.max_steps, Some(12));
    }

    #[test]
    fn resolve_child_execution_limits_intersects_parent_and_grant() {
        let limits = resolve_child_execution_limits(
            &["read_file".to_string(), "grep".to_string()],
            Some(&SpawnCapabilityGrant {
                allowed_tools: vec!["grep".to_string(), "write_file".to_string()],
            }),
            8,
        )
        .expect("limits should resolve");

        assert_eq!(limits.allowed_tools, vec!["grep"]);
        assert_eq!(limits.max_steps, Some(8));
    }

    #[test]
    fn resolve_child_execution_limits_rejects_disjoint_grant() {
        let error = resolve_child_execution_limits(
            &["read_file".to_string(), "grep".to_string()],
            Some(&SpawnCapabilityGrant {
                allowed_tools: vec!["write_file".to_string()],
            }),
            8,
        )
        .expect_err("disjoint grants should fail fast");

        assert!(error.to_string().contains("无交集"));
    }

    #[test]
    fn build_inherited_messages_uses_compact_summary_and_last_n_turn_tail() {
        let inherited = build_inherited_messages(
            &[
                LlmMessage::User {
                    content: "<summary>older summary</summary>".to_string(),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: "turn-1".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "answer-1".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
                LlmMessage::User {
                    content: "turn-2".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "answer-2".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            &ResolvedSubagentContextOverrides {
                include_compact_summary: true,
                include_recent_tail: true,
                fork_mode: Some(ForkMode::LastNTurns(1)),
                ..ResolvedSubagentContextOverrides::default()
            },
        );

        assert_eq!(inherited.len(), 3);
        assert!(matches!(
            &inherited[0],
            LlmMessage::User {
                origin: UserMessageOrigin::CompactSummary,
                ..
            }
        ));
        assert!(matches!(
            &inherited[1],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::User,
            } if content == "turn-2"
        ));
        assert!(matches!(
            &inherited[2],
            LlmMessage::Assistant { content, .. } if content == "answer-2"
        ));
    }

    #[test]
    fn select_inherited_recent_tail_uses_full_history_by_default() {
        let inherited = select_inherited_recent_tail(
            &[
                LlmMessage::User {
                    content: "<summary>older summary</summary>".to_string(),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: "turn-1".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "answer-1".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            None,
        );

        assert_eq!(inherited.len(), 2);
        assert!(matches!(
            &inherited[0],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::User,
            } if content == "turn-1"
        ));
        assert!(matches!(
            &inherited[1],
            LlmMessage::Assistant { content, .. } if content == "answer-1"
        ));
    }
}
