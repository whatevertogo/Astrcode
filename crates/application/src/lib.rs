use std::{path::Path, sync::Arc};

use astrcode_core::{AgentProfile, ExecutionAccepted, config::Config};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use tokio::sync::broadcast;

mod app_agent;
mod app_session;

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
pub use astrcode_session_runtime::{
    SessionCatalogEvent, SessionHistorySnapshot, SessionReplay, SessionViewSnapshot,
    TurnCollaborationSummary, TurnSummary,
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
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, GovernanceSnapshot,
    OperationMetricsSnapshot, ReloadResult, ReplayMetricsSnapshot, ReplayPath,
    RuntimeObservabilityCollector, RuntimeObservabilitySnapshot, SubRunExecutionMetricsSnapshot,
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

/// 手动压缩请求的返回结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionAccepted {
    /// true 表示压缩被推迟（当前有 turn 正在执行），待 turn 结束后自动执行。
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
}
