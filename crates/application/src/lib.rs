use std::sync::Arc;

use astrcode_core::{DeleteProjectResult, ExecutionAccepted, SessionMeta, config::Config};
use astrcode_kernel::Kernel;
use astrcode_session_runtime::SessionRuntime;
use tokio::sync::broadcast;

pub mod composer;
pub mod config;
pub mod errors;
pub mod lifecycle;
pub mod mcp;
pub mod observability;
pub mod watch;

pub use astrcode_session_runtime::{
    SessionCatalogEvent, SessionHistorySnapshot, SessionReplay, SessionViewSnapshot,
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
    DEFAULT_CONTINUATION_MIN_DELTA_TOKENS,
    DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT,
    DEFAULT_INBOX_CAPACITY,
    DEFAULT_LLM_CONNECT_TIMEOUT_SECS,
    DEFAULT_LLM_MAX_RETRIES,
    DEFAULT_LLM_READ_TIMEOUT_SECS,
    DEFAULT_LLM_RETRY_BASE_DELAY_MS,
    DEFAULT_MAX_AGENT_DEPTH,
    DEFAULT_MAX_CONCURRENT_AGENTS,
    DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH,
    DEFAULT_MAX_CONSECUTIVE_FAILURES,
    DEFAULT_MAX_CONTINUATIONS,
    DEFAULT_MAX_GREP_LINES,
    DEFAULT_MAX_IMAGE_SIZE,
    DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS,
    DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS,
    DEFAULT_MAX_RECOVERED_FILES,
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
    DEFAULT_TOKEN_BUDGET,
    DEFAULT_TOOL_RESULT_INLINE_LIMIT,
    DEFAULT_TOOL_RESULT_MAX_BYTES,
    DEFAULT_TOOL_RESULT_PREVIEW_LIMIT,
    ENV_REFERENCE_PREFIX,
    HOME_ENV_VARS,
    LITERAL_VALUE_PREFIX,
    PLUGIN_ENV_VARS,
    PROVIDER_API_KEY_ENV_VARS,
    PROVIDER_KIND_ANTHROPIC,
    PROVIDER_KIND_OPENAI,
    RUNTIME_ENV_VARS,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    TestConnectionResult,
    is_env_var_name,
    list_model_options,
    max_tool_concurrency,
    resolve_active_selection,
    resolve_agent_finalized_retain_limit,
    resolve_agent_inbox_capacity,
    resolve_agent_max_concurrent,
    resolve_agent_max_subrun_depth,
    resolve_agent_parent_delivery_capacity,
    resolve_aggregate_result_bytes_budget,
    resolve_anthropic_messages_api_url,
    resolve_anthropic_models_api_url,
    resolve_api_session_ttl_hours,
    resolve_auto_compact_enabled,
    resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent,
    resolve_continuation_min_delta_tokens,
    resolve_current_model,
    resolve_default_token_budget,
    resolve_llm_connect_timeout_secs,
    resolve_llm_max_retries,
    resolve_llm_read_timeout_secs,
    resolve_max_concurrent_branch_depth,
    resolve_max_consecutive_failures,
    resolve_max_continuations,
    resolve_max_grep_lines,
    resolve_max_image_size,
    resolve_max_output_continuation_attempts,
    resolve_max_reactive_compact_attempts,
    resolve_max_recovered_files,
    resolve_max_tool_concurrency,
    resolve_max_tracked_files,
    resolve_micro_compact_gap_threshold_secs,
    resolve_micro_compact_keep_recent_results,
    resolve_openai_chat_completions_api_url,
    resolve_recovery_token_budget,
    resolve_recovery_truncate_bytes,
    resolve_session_broadcast_capacity,
    resolve_session_recent_record_limit,
    resolve_summary_reserve_tokens,
    resolve_tool_result_inline_limit,
    resolve_tool_result_max_bytes,
    resolve_tool_result_preview_limit,
};
pub use errors::ApplicationError;
pub use lifecycle::governance::{
    AppGovernance, ObservabilitySnapshotProvider, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeReloader, SessionInfoProvider,
};
pub use mcp::{McpConfigScope, McpPort, McpServerStatusView, McpService, RegisterMcpServerInput};
pub use observability::{
    ExecutionDiagnosticsSnapshot, GovernanceSnapshot, OperationMetricsSnapshot, ReloadResult,
    ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use watch::{WatchEvent, WatchPort, WatchService, WatchSource};

/// 唯一业务用例入口。
pub struct App {
    kernel: Arc<Kernel>,
    session_runtime: Arc<SessionRuntime>,
    config_service: Arc<ConfigService>,
    composer_service: Arc<composer::ComposerService>,
    mcp_service: Arc<mcp::McpService>,
}

impl App {
    pub fn new(
        kernel: Arc<Kernel>,
        session_runtime: Arc<SessionRuntime>,
        config_service: Arc<ConfigService>,
        mcp_service: Arc<mcp::McpService>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            config_service,
            composer_service: Arc::new(composer::ComposerService::new()),
            mcp_service,
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

    pub fn mcp(&self) -> &Arc<mcp::McpService> {
        &self.mcp_service
    }

    pub fn composer(&self) -> &Arc<composer::ComposerService> {
        &self.composer_service
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
        self.validate_non_empty("prompt", &text)?;
        let runtime = self.config_service.get_config().await.runtime;
        self.session_runtime
            .submit_prompt(session_id, text, runtime)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn interrupt_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .interrupt_session(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn compact_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .compact_session(session_id)
            .await
            .map_err(ApplicationError::from)
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
}
