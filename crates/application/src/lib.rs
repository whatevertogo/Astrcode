use std::{path::Path, sync::Arc};

use astrcode_core::{
    AgentEventContext, AgentProfile, DeleteProjectResult, ExecutionAccepted, SessionMeta,
    config::Config,
};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use tokio::sync::broadcast;

pub mod agent;
pub mod composer;
pub mod config;
pub mod errors;
pub mod execution;
pub mod lifecycle;
pub mod mcp;
pub mod observability;
pub mod watch;

pub use agent::AgentOrchestrationService;
use agent::{
    IMPLICIT_ROOT_PROFILE_ID, implicit_session_root_agent_id, root_execution_event_context,
};
pub use astrcode_session_runtime::{
    SessionCatalogEvent, SessionHistorySnapshot, SessionReplay, SessionViewSnapshot, TurnSummary,
};
pub use composer::{ComposerOption, ComposerOptionKind, ComposerOptionsRequest};
pub use config::{
    // 常量与解析函数
    ALL_ASTRCODE_ENV_VARS,
    ANTHROPIC_API_KEY_ENV,
    ANTHROPIC_MESSAGES_API_URL,
    ANTHROPIC_MODELS_API_URL,
    ANTHROPIC_VERSION,
    ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV,
    ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_TOOL_INLINE_LIMIT_PREFIX,
    ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV,
    BUILD_ENV_VARS,
    CURRENT_CONFIG_VERSION,
    ConfigService,
    DEEPSEEK_API_KEY_ENV,
    DEFAULT_API_SESSION_TTL_HOURS,
    DEFAULT_AUTO_COMPACT_ENABLED,
    DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT,
    DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT,
    DEFAULT_INBOX_CAPACITY,
    DEFAULT_LLM_CONNECT_TIMEOUT_SECS,
    DEFAULT_LLM_MAX_RETRIES,
    DEFAULT_LLM_READ_TIMEOUT_SECS,
    DEFAULT_LLM_RETRY_BASE_DELAY_MS,
    DEFAULT_MAX_CONCURRENT_AGENTS,
    DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH,
    DEFAULT_MAX_CONSECUTIVE_FAILURES,
    DEFAULT_MAX_GREP_LINES,
    DEFAULT_MAX_IMAGE_SIZE,
    DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS,
    DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS,
    DEFAULT_MAX_RECOVERED_FILES,
    DEFAULT_MAX_STEPS,
    DEFAULT_MAX_SUBRUN_DEPTH,
    DEFAULT_MAX_TOOL_CONCURRENCY,
    DEFAULT_MAX_TRACKED_FILES,
    DEFAULT_OPENAI_CONTEXT_LIMIT,
    DEFAULT_PARENT_DELIVERY_CAPACITY,
    DEFAULT_RECOVERY_TOKEN_BUDGET,
    DEFAULT_RECOVERY_TRUNCATE_BYTES,
    DEFAULT_SESSION_BROADCAST_CAPACITY,
    DEFAULT_SESSION_RECENT_RECORD_LIMIT,
    DEFAULT_SUMMARY_RESERVE_TOKENS,
    DEFAULT_TOOL_RESULT_INLINE_LIMIT,
    DEFAULT_TOOL_RESULT_MAX_BYTES,
    DEFAULT_TOOL_RESULT_PREVIEW_LIMIT,
    ENV_REFERENCE_PREFIX,
    HOME_ENV_VARS,
    LITERAL_VALUE_PREFIX,
    McpConfigFileScope,
    PLUGIN_ENV_VARS,
    PROVIDER_API_KEY_ENV_VARS,
    PROVIDER_KIND_ANTHROPIC,
    PROVIDER_KIND_OPENAI,
    RUNTIME_ENV_VARS,
    ResolvedAgentConfig,
    ResolvedRuntimeConfig,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    TestConnectionResult,
    is_env_var_name,
    list_model_options,
    max_tool_concurrency,
    resolve_active_selection,
    resolve_agent_config,
    resolve_anthropic_messages_api_url,
    resolve_anthropic_models_api_url,
    resolve_current_model,
    resolve_openai_chat_completions_api_url,
    resolve_runtime_config,
};
pub use errors::ApplicationError;
pub use execution::{ExecutionControl, ProfileResolutionService, RootExecutionRequest};
pub use lifecycle::governance::{
    AppGovernance, ObservabilitySnapshotProvider, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeReloader, SessionInfoProvider,
};
pub use mcp::{McpConfigScope, McpPort, McpServerStatusView, McpService, RegisterMcpServerInput};
pub use observability::{
    ExecutionDiagnosticsSnapshot, GovernanceSnapshot, OperationMetricsSnapshot, ReloadResult,
    ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilityCollector, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use watch::{WatchEvent, WatchPort, WatchService, WatchSource};

/// 唯一业务用例入口。
pub struct App {
    kernel: Arc<Kernel>,
    session_runtime: Arc<SessionRuntime>,
    profiles: Arc<ProfileResolutionService>,
    config_service: Arc<ConfigService>,
    composer_service: Arc<composer::ComposerService>,
    mcp_service: Arc<mcp::McpService>,
    agent_service: Arc<AgentOrchestrationService>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionAccepted {
    pub deferred: bool,
}

impl App {
    pub fn new(
        kernel: Arc<Kernel>,
        session_runtime: Arc<SessionRuntime>,
        profiles: Arc<ProfileResolutionService>,
        config_service: Arc<ConfigService>,
        mcp_service: Arc<mcp::McpService>,
        agent_service: Arc<AgentOrchestrationService>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            profiles,
            config_service,
            composer_service: Arc::new(composer::ComposerService::new()),
            mcp_service,
            agent_service,
        }
    }

    pub fn kernel(&self) -> &Arc<Kernel> {
        &self.kernel
    }

    pub fn session_runtime(&self) -> &Arc<SessionRuntime> {
        &self.session_runtime
    }

    pub fn config(&self) -> &Arc<ConfigService> {
        &self.config_service
    }

    pub fn profiles(&self) -> &Arc<ProfileResolutionService> {
        &self.profiles
    }

    pub fn mcp(&self) -> &Arc<mcp::McpService> {
        &self.mcp_service
    }

    pub fn composer(&self) -> &Arc<composer::ComposerService> {
        &self.composer_service
    }

    pub fn agent(&self) -> &Arc<AgentOrchestrationService> {
        &self.agent_service
    }

    pub fn subscribe_catalog(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.session_runtime.subscribe_catalog_events()
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>, ApplicationError> {
        self.session_runtime
            .list_session_metas()
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<String>,
    ) -> Result<SessionMeta, ApplicationError> {
        let working_dir = working_dir.into();
        self.validate_non_empty("workingDir", &working_dir)?;
        self.session_runtime
            .create_session(working_dir)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn execute_root_agent(
        &self,
        request: RootExecutionRequest,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&request.working_dir)))?;
        execution::execute_root_agent(
            &self.kernel,
            &self.session_runtime,
            &self.profiles,
            request,
            runtime,
        )
        .await
    }

    pub fn list_global_agent_profiles(&self) -> Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.resolve_global()?.as_ref().clone())
    }

    pub fn list_agent_profiles_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.resolve(working_dir)?.as_ref().clone())
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .delete_session(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn delete_project(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult, ApplicationError> {
        self.session_runtime
            .delete_project(working_dir)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        self.submit_prompt_with_control(session_id, text, None)
            .await
    }

    pub async fn submit_prompt_with_control(
        &self,
        session_id: &str,
        text: String,
        control: Option<ExecutionControl>,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        self.validate_non_empty("prompt", &text)?;
        if let Some(control) = &control {
            control.validate()?;
        }
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
        let mut runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&working_dir)))?;
        if let Some(control) = control {
            if control.manual_compact.is_some() {
                return Err(ApplicationError::InvalidArgument(
                    "manualCompact is not valid for prompt submission".to_string(),
                ));
            }
            if let Some(max_steps) = control.max_steps {
                runtime.max_steps = max_steps as usize;
            }
        }
        let root_agent = self.ensure_session_root_agent_context(session_id).await?;
        self.session_runtime
            .submit_prompt_for_agent(session_id, text, runtime, root_agent)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn interrupt_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .interrupt_session(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn compact_session(
        &self,
        session_id: &str,
    ) -> Result<CompactSessionAccepted, ApplicationError> {
        self.compact_session_with_control(session_id, None).await
    }

    pub async fn compact_session_with_control(
        &self,
        session_id: &str,
        control: Option<ExecutionControl>,
    ) -> Result<CompactSessionAccepted, ApplicationError> {
        if let Some(control) = &control {
            control.validate()?;
            if control.max_steps.is_some() {
                return Err(ApplicationError::InvalidArgument(
                    "maxSteps is not valid for manual compact".to_string(),
                ));
            }
        }
        let deferred = self
            .session_runtime
            .compact_session(session_id)
            .await
            .map_err(ApplicationError::from)?;
        Ok(CompactSessionAccepted { deferred })
    }

    pub async fn session_history(
        &self,
        session_id: &str,
    ) -> Result<SessionHistorySnapshot, ApplicationError> {
        self.session_runtime
            .session_history(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn session_view(
        &self,
        session_id: &str,
    ) -> Result<SessionViewSnapshot, ApplicationError> {
        self.session_runtime
            .session_view(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<SessionReplay, ApplicationError> {
        self.session_runtime
            .session_replay(session_id, last_event_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn list_composer_options(
        &self,
        request: ComposerOptionsRequest,
    ) -> Vec<ComposerOption> {
        let gateway = self.kernel.gateway();
        self.composer_service.list_options(request, Some(gateway))
    }

    pub async fn get_config(&self) -> Config {
        self.config_service.get_config().await
    }

    pub fn validate_non_empty(
        &self,
        field: &'static str,
        value: &str,
    ) -> Result<(), ApplicationError> {
        if value.trim().is_empty() {
            return Err(ApplicationError::InvalidArgument(format!(
                "field '{}' must not be empty",
                field
            )));
        }
        Ok(())
    }

    pub fn require_permission(
        &self,
        allowed: bool,
        reason: impl Into<String>,
    ) -> Result<(), ApplicationError> {
        if allowed {
            return Ok(());
        }
        Err(ApplicationError::PermissionDenied(reason.into()))
    }

    // ── Agent 控制用例（通过 kernel 稳定控制合同） ──────────

    /// 查询子运行状态。
    pub async fn get_subrun_status(
        &self,
        agent_id: &str,
    ) -> Result<Option<astrcode_kernel::SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("agentId", agent_id)?;
        Ok(self.kernel.query_subrun_status(agent_id).await)
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn get_root_agent_status(
        &self,
        session_id: &str,
    ) -> Result<Option<astrcode_kernel::SubRunStatusView>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        Ok(self.kernel.query_root_agent_status(session_id).await)
    }

    /// 列出所有 agent 状态。
    pub async fn list_agent_statuses(&self) -> Vec<astrcode_kernel::SubRunStatusView> {
        self.kernel.list_all_agent_statuses().await
    }

    /// 关闭 agent 及其子树。
    pub async fn close_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<astrcode_kernel::CloseSubtreeResult, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        self.validate_non_empty("agentId", agent_id)?;
        let Some(handle) = self.kernel.get_agent_handle(agent_id).await else {
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found",
                agent_id
            )));
        };
        if handle.session_id != session_id {
            // 显式校验归属，避免仅凭 agent_id 跨 session 关闭不相关子树。
            return Err(ApplicationError::NotFound(format!(
                "agent '{}' not found in session '{}'",
                agent_id, session_id
            )));
        }
        self.kernel
            .close_agent_subtree(agent_id)
            .await
            .map_err(|e| ApplicationError::Internal(e.to_string()))
    }

    async fn ensure_session_root_agent_context(
        &self,
        session_id: &str,
    ) -> Result<AgentEventContext, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let normalized_session_id = astrcode_session_runtime::normalize_session_id(session_id);

        if let Some(handle) = self
            .kernel
            .agent_control()
            .find_root_agent_for_session(&normalized_session_id)
            .await
        {
            return Ok(root_execution_event_context(
                handle.agent_id,
                handle.agent_profile,
            ));
        }

        let handle = self
            .kernel
            .agent_control()
            .register_root_agent(
                implicit_session_root_agent_id(&normalized_session_id),
                normalized_session_id,
                IMPLICIT_ROOT_PROFILE_ID.to_string(),
            )
            .await
            .map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to register implicit root agent for session prompt: {error}"
                ))
            })?;
        Ok(root_execution_event_context(
            handle.agent_id,
            handle.agent_profile,
        ))
    }
}
