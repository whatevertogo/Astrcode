//! # Agent / Tool 执行服务
//!
//! 把“Agent 作为工具”和“未来独立 API”的执行入口收敛到 runtime 一层：
//! - `runAgent` 通过这里执行真实子 Agent
//! - server 未来的 `/api/v1/agents`、`/api/v1/tools` 直接复用同一份列表能力
//! - 复杂的子 Agent 策略、能力裁剪、父取消传播只在这里维护一份

use std::{
    collections::HashSet,
    sync::{Arc, RwLock as StdRwLock, Weak},
    time::Instant,
};

use astrcode_core::{
    AgentEventContext, AgentMode, AgentProfile, AgentState, AstrError, CancelToken, LlmMessage,
    Result, StorageEvent, ToolContext, UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{ChildExecutionTracker, SubAgentPolicyEngine};
use astrcode_runtime_agent_tool::{
    RunAgentParams, SubAgentExecutor, SubAgentOutcome, SubAgentResult,
};
use async_trait::async_trait;
use serde_json::json;
use uuid::Uuid;

use super::{RuntimeService, ServiceError, ServiceResult, build_agent_loop_from_parts};

/// 面向 API / Tool 的 Agent Profile 摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub max_steps: Option<u32>,
    pub token_budget: Option<u64>,
}

/// 面向 API / Tool 的工具摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
    pub streaming: bool,
}

/// Agent 执行服务句柄。
#[derive(Clone)]
pub struct AgentExecutionServiceHandle {
    runtime: Arc<RuntimeService>,
}

/// Tool 执行服务句柄。
#[derive(Clone)]
pub struct ToolExecutionServiceHandle {
    runtime: Arc<RuntimeService>,
}

/// bootstrap 阶段使用的延迟执行器桥。
///
/// builtin router 在 `RuntimeService` 创建前就要注册 `runAgent`，因此这里先占位，
/// 等 service 创建完成后再绑定真实 runtime。
#[derive(Default)]
pub(crate) struct DeferredSubAgentExecutor {
    runtime: StdRwLock<Option<Weak<RuntimeService>>>,
}

impl DeferredSubAgentExecutor {
    pub(crate) fn bind(&self, runtime: &Arc<RuntimeService>) {
        let mut guard = self
            .runtime
            .write()
            .expect("sub-agent executor binding lock should not be poisoned");
        *guard = Some(Arc::downgrade(runtime));
    }

    fn runtime(&self) -> Result<Arc<RuntimeService>> {
        let guard = self
            .runtime
            .read()
            .expect("sub-agent executor binding lock should not be poisoned");
        let Some(runtime) = guard.as_ref().and_then(Weak::upgrade) else {
            return Err(AstrError::Internal(
                "runAgent executor is not bound to runtime service yet".to_string(),
            ));
        };
        Ok(runtime)
    }
}

#[async_trait]
impl SubAgentExecutor for DeferredSubAgentExecutor {
    async fn execute(&self, params: RunAgentParams, ctx: &ToolContext) -> Result<SubAgentResult> {
        let runtime = self.runtime()?;
        runtime
            .agent_execution_service()
            .execute_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl RuntimeService {
    pub fn agent_execution_service(self: &Arc<Self>) -> AgentExecutionServiceHandle {
        AgentExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    pub fn tool_execution_service(self: &Arc<Self>) -> ToolExecutionServiceHandle {
        ToolExecutionServiceHandle {
            runtime: Arc::clone(self),
        }
    }
}

impl AgentExecutionServiceHandle {
    pub fn list_profiles(&self) -> Vec<AgentProfileSummary> {
        let mut profiles = self
            .runtime
            .agent_profiles()
            .list()
            .into_iter()
            .map(|profile| AgentProfileSummary {
                id: profile.id.clone(),
                name: profile.name.clone(),
                description: profile.description.clone(),
                mode: profile.mode,
                allowed_tools: profile.allowed_tools.clone(),
                disallowed_tools: profile.disallowed_tools.clone(),
                max_steps: profile.max_steps,
                token_budget: profile.token_budget,
            })
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(&right.id));
        profiles
    }

    pub async fn execute_subagent(
        &self,
        params: RunAgentParams,
        ctx: &ToolContext,
    ) -> ServiceResult<SubAgentResult> {
        let parent_turn_id = ctx.turn_id().ok_or_else(|| {
            ServiceError::InvalidInput("runAgent requires a parent turn id".to_string())
        })?;
        let event_sink = ctx.event_sink().ok_or_else(|| {
            ServiceError::InvalidInput(
                "runAgent requires a tool event sink in the current runtime".to_string(),
            )
        })?;
        let runtime = &self.runtime;
        let profiles = runtime.agent_profiles();
        let profile = profiles.get(&params.name).cloned().ok_or_else(|| {
            ServiceError::InvalidInput(format!("unknown agent profile '{}'", params.name))
        })?;
        ensure_subagent_mode(&profile)?;

        let (capabilities, prompt_declarations, skill_catalog, hook_handlers, runtime_config) = {
            let surface = runtime.surface.read().await;
            let runtime_config = runtime.config.lock().await.runtime.clone();
            let final_tool_names = resolve_profile_tool_names(&surface.capabilities, &profile);
            if final_tool_names.is_empty() {
                return Err(ServiceError::InvalidInput(format!(
                    "agent profile '{}' does not expose any available tools in the current \
                     runtime surface",
                    profile.id
                )));
            }

            (
                surface.capabilities.subset_for_tools(&final_tool_names)?,
                build_child_prompt_declarations(&surface.prompt_declarations, &profile),
                Arc::clone(&surface.skill_catalog),
                surface.hook_handlers.clone(),
                runtime_config,
            )
        };

        let child = runtime
            .agent_control
            .spawn(
                &profile,
                ctx.session_id().to_string(),
                Some(parent_turn_id.to_string()),
                ctx.agent_context().agent_id.clone(),
            )
            .await
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let _ = runtime.agent_control.mark_running(&child.agent_id).await;
        let child_cancel = runtime
            .agent_control
            .cancel_token(&child.agent_id)
            .await
            .unwrap_or_else(CancelToken::new);
        let child_turn_id = format!("{}-child-{}", parent_turn_id, Uuid::new_v4());
        let child_agent = AgentEventContext::subagent(
            child.agent_id.clone(),
            parent_turn_id.to_string(),
            profile.id.clone(),
        );
        let child_state = build_child_agent_state(ctx, &params);
        let child_policy = Arc::new(SubAgentPolicyEngine::new(
            Arc::clone(&runtime.policy),
            allowed_tool_set(&capabilities),
        ));
        let child_loop = build_agent_loop_from_parts(
            capabilities,
            prompt_declarations,
            skill_catalog,
            hook_handlers,
            &runtime_config,
            child_policy,
            Arc::clone(&runtime.approval),
        );

        // 先记录子 Agent 的输入，再跑 loop，这样 UI 和历史回放能看到嵌套入口。
        event_sink.emit(StorageEvent::UserMessage {
            turn_id: Some(child_turn_id.clone()),
            agent: child_agent.clone(),
            content: compose_subagent_task(&params),
            timestamp: chrono::Utc::now(),
            origin: UserMessageOrigin::User,
        })?;

        let mut tracker = ChildExecutionTracker::new(
            profile.max_steps.or(params.max_steps),
            profile.token_budget,
        );
        let started_at = Instant::now();
        let outcome = child_loop
            .run_turn_with_agent_context(
                &child_state,
                &child_turn_id,
                &mut |event| {
                    if ctx.cancel().is_cancelled() {
                        child_cancel.cancel();
                    }
                    tracker.observe(&event, &child_cancel);
                    event_sink.emit(event)
                },
                child_cancel.clone(),
                child_agent.clone(),
            )
            .await;

        let result = match outcome {
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Completed) => {
                let _ = runtime.agent_control.mark_completed(&child.agent_id).await;
                SubAgentResult {
                    outcome: if tracker.token_limit_hit() || tracker.step_limit_hit() {
                        SubAgentOutcome::TokenExceeded
                    } else {
                        SubAgentOutcome::Completed
                    },
                    summary: summarize_child_result(
                        &tracker,
                        started_at.elapsed().as_millis() as u64,
                        "子 Agent 已完成任务。",
                    ),
                    metadata: Some(json!({
                        "agentId": child.agent_id,
                        "agentProfile": profile.id,
                    })),
                }
            },
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Cancelled) => {
                let _ = runtime.agent_control.cancel(&child.agent_id).await;
                let outcome = if tracker.token_limit_hit() || tracker.step_limit_hit() {
                    SubAgentOutcome::TokenExceeded
                } else {
                    SubAgentOutcome::Aborted
                };
                SubAgentResult {
                    outcome,
                    summary: summarize_child_result(
                        &tracker,
                        started_at.elapsed().as_millis() as u64,
                        "子 Agent 被中止。",
                    ),
                    metadata: Some(json!({
                        "agentId": child.agent_id,
                        "agentProfile": profile.id,
                    })),
                }
            },
            Ok(astrcode_runtime_agent_loop::TurnOutcome::Error { message }) => {
                let _ = runtime.agent_control.mark_failed(&child.agent_id).await;
                SubAgentResult {
                    outcome: SubAgentOutcome::Failed {
                        error: message.clone(),
                    },
                    summary: summarize_child_result(
                        &tracker,
                        started_at.elapsed().as_millis() as u64,
                        &format!("子 Agent 执行失败：{message}"),
                    ),
                    metadata: Some(json!({
                        "agentId": child.agent_id,
                        "agentProfile": profile.id,
                    })),
                }
            },
            Err(error) => {
                let _ = runtime.agent_control.mark_failed(&child.agent_id).await;
                SubAgentResult {
                    outcome: SubAgentOutcome::Failed {
                        error: error.to_string(),
                    },
                    summary: summarize_child_result(
                        &tracker,
                        started_at.elapsed().as_millis() as u64,
                        &format!("子 Agent 执行失败：{error}"),
                    ),
                    metadata: Some(json!({
                        "agentId": child.agent_id,
                        "agentProfile": profile.id,
                    })),
                }
            },
        };

        Ok(result)
    }
}

#[async_trait]
impl SubAgentExecutor for AgentExecutionServiceHandle {
    async fn execute(&self, params: RunAgentParams, ctx: &ToolContext) -> Result<SubAgentResult> {
        self.execute_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}

impl ToolExecutionServiceHandle {
    pub async fn list_tools(&self) -> Vec<ToolSummary> {
        let surface = self.runtime.surface.read().await;
        let mut tools = surface
            .capabilities
            .descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind.is_tool())
            .map(|descriptor| ToolSummary {
                name: descriptor.name,
                description: descriptor.description,
                profiles: descriptor.profiles,
                streaming: descriptor.streaming,
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools
    }
}

fn build_child_prompt_declarations(
    parent: &[astrcode_runtime_prompt::PromptDeclaration],
    profile: &AgentProfile,
) -> Vec<astrcode_runtime_prompt::PromptDeclaration> {
    let mut declarations = parent.to_vec();
    if let Some(system_prompt) = profile.system_prompt.as_ref() {
        declarations.push(astrcode_runtime_prompt::PromptDeclaration {
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

fn build_child_agent_state(ctx: &ToolContext, params: &RunAgentParams) -> AgentState {
    AgentState {
        session_id: ctx.session_id().to_string(),
        working_dir: ctx.working_dir().to_path_buf(),
        messages: vec![LlmMessage::User {
            content: compose_subagent_task(params),
            origin: UserMessageOrigin::User,
        }],
        phase: astrcode_core::Phase::Thinking,
        turn_count: 0,
    }
}

fn compose_subagent_task(params: &RunAgentParams) -> String {
    match params.context.as_deref() {
        Some(context) if !context.trim().is_empty() => {
            format!(
                "# Task\n{}\n\n# Context\n{}",
                params.task.trim(),
                context.trim()
            )
        },
        _ => params.task.trim().to_string(),
    }
}

fn ensure_subagent_mode(profile: &AgentProfile) -> ServiceResult<()> {
    if matches!(profile.mode, AgentMode::SubAgent | AgentMode::All) {
        return Ok(());
    }
    Err(ServiceError::InvalidInput(format!(
        "agent profile '{}' is not allowed to run as a sub-agent",
        profile.id
    )))
}

fn resolve_profile_tool_names(
    capabilities: &astrcode_core::CapabilityRouter,
    profile: &AgentProfile,
) -> Vec<String> {
    let available = capabilities
        .tool_names()
        .into_iter()
        .collect::<HashSet<_>>();
    let requested = if profile.allowed_tools.is_empty() {
        available.clone()
    } else {
        profile
            .allowed_tools
            .iter()
            .filter_map(|tool| normalize_profile_tool_name(tool, &available))
            .collect::<HashSet<_>>()
    };
    let denied = profile
        .disallowed_tools
        .iter()
        .filter_map(|tool| normalize_profile_tool_name(tool, &available))
        .collect::<HashSet<_>>();

    let mut final_tools = requested
        .into_iter()
        .filter(|tool| !denied.contains(tool))
        .collect::<Vec<_>>();
    final_tools.sort();
    final_tools
}

fn normalize_profile_tool_name(tool: &str, available: &HashSet<String>) -> Option<String> {
    if available.contains(tool) {
        return Some(tool.to_string());
    }

    let alias = match tool.to_ascii_lowercase().as_str() {
        "read" => "readFile",
        "write" => "writeFile",
        "edit" => "editFile",
        "bash" => "shell",
        "grep" => "grep",
        "glob" => "findFiles",
        "ls" => "listDir",
        _ => return None,
    };

    available.contains(alias).then(|| alias.to_string())
}

fn allowed_tool_set(capabilities: &astrcode_core::CapabilityRouter) -> HashSet<String> {
    capabilities.tool_names().into_iter().collect()
}

fn summarize_child_result(
    tracker: &ChildExecutionTracker,
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

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use astrcode_core::{
        CancelToken, StorageEvent, Tool, ToolContext, ToolEventSink, ToolRegistry,
        test_support::TestEnvGuard,
    };
    use astrcode_runtime_agent_tool::{RunAgentParams, RunAgentTool, SubAgentExecutor};
    use serde_json::json;

    use super::{DeferredSubAgentExecutor, RuntimeService, normalize_profile_tool_name};
    use crate::test_support::capabilities_from_tools;

    struct RecordingEventSink {
        events: Mutex<Vec<StorageEvent>>,
    }

    impl ToolEventSink for RecordingEventSink {
        fn emit(&self, event: StorageEvent) -> astrcode_core::Result<()> {
            self.events.lock().expect("events lock").push(event);
            Ok(())
        }
    }

    #[test]
    fn normalize_profile_tool_name_accepts_claude_aliases() {
        let available = ["readFile".to_string(), "shell".to_string()]
            .into_iter()
            .collect::<HashSet<_>>();
        assert_eq!(
            normalize_profile_tool_name("Read", &available).as_deref(),
            Some("readFile")
        );
        assert_eq!(
            normalize_profile_tool_name("Bash", &available).as_deref(),
            Some("shell")
        );
    }

    #[tokio::test]
    async fn deferred_executor_fails_before_runtime_binding() {
        let executor = DeferredSubAgentExecutor::default();
        let context = ToolContext::new(
            "session-1".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        );

        let error = executor
            .execute(
                RunAgentParams {
                    name: "review".to_string(),
                    task: "check".to_string(),
                    context: None,
                    max_steps: None,
                },
                &context,
            )
            .await
            .expect_err("unbound executor should fail");

        assert!(error.to_string().contains("not bound"));
    }

    #[tokio::test]
    async fn run_agent_tool_emits_child_events_with_agent_context() {
        let _guard = TestEnvGuard::new();
        let service = Arc::new(
            RuntimeService::from_capabilities(capabilities_from_tools(
                ToolRegistry::builder()
                    .register(Box::new(
                        astrcode_runtime_tool_loader::builtin_tools::read_file::ReadFileTool,
                    ))
                    .register(Box::new(
                        astrcode_runtime_tool_loader::builtin_tools::grep::GrepTool,
                    ))
                    .build(),
            ))
            .expect("runtime service should build"),
        );
        let executor = Arc::new(DeferredSubAgentExecutor::default());
        executor.bind(&service);
        let tool = RunAgentTool::new(executor);
        let sink = Arc::new(RecordingEventSink {
            events: Mutex::new(Vec::new()),
        });

        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let context = ToolContext::new(
            "session-1".to_string(),
            temp_dir.path().to_path_buf(),
            CancelToken::new(),
        )
        .with_turn_id("turn-parent")
        .with_event_sink(sink.clone());

        let result = tool
            .execute(
                "call-1".to_string(),
                json!({
                    "name": "plan",
                    "task": "summarize the repository layout",
                    "maxSteps": 1
                }),
                &context,
            )
            .await
            .expect("tool execution should always return a tool result");

        assert!(
            result.metadata.is_some(),
            "runAgent should always return child metadata"
        );
        let events = sink.events.lock().expect("events lock");
        assert!(!events.is_empty());
        assert!(events.iter().all(|event| {
            event
                .agent_context()
                .is_some_and(|agent| agent.parent_turn_id.as_deref() == Some("turn-parent"))
        }));
    }
}
