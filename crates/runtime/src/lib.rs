//! # Astrcode 运行时
//!
//! 本库实现了 Astrcode Agent 的运行时系统，负责：
//!
//! - **Agent 循环**: 运行 LLM 调用和工具执行的主循环
//! - **Prompt 组装**: 构建发送给 LLM 的提示词
//! - **审批服务**: 处理需要用户确认的能力调用
//! - **运行时服务**: HTTP 服务器的后端，管理会话状态
//! - **配置管理**: API 密钥、Profile 配置
//!
//! ## 架构
//!
//! `RuntimeService` 是门面，通过 `AgentLoop` 执行 Turn，
//! 通过 `ApprovalBroker` 处理审批，通过 `CapabilityRouter` 调用工具。

mod bootstrap;
#[cfg(test)]
mod bootstrap_tests;
mod builtin_capabilities;
mod plugin_discovery;
mod plugin_hook_adapter;
mod plugin_skill_materializer;
mod provider_factory;
mod runtime_governance;
mod runtime_surface_assembler;
mod service;
mod skill_tool;
#[cfg(test)]
mod test_support;

pub use astrcode_runtime_agent_control as agent_control;
pub use astrcode_runtime_agent_control::{AgentControl, AgentControlError};
pub use astrcode_runtime_agent_loader as agent_loader;
pub use astrcode_runtime_agent_loader::{
    AgentLoaderError, AgentProfileLoader, AgentProfileRegistry,
};
pub use astrcode_runtime_agent_loop as agent_loop;
pub use astrcode_runtime_config as config;
pub use astrcode_runtime_config::{
    ActiveSelection, AgentConfig, Config, ConfigOverlay, CurrentModelSelection, ModelConfig,
    ModelOption, ParsedEnvValue, Profile, RuntimeConfig, TestResult, config_path, env_reference,
    is_env_var_name, list_model_options, load_config, load_resolved_config, open_config_in_editor,
    parse_env_value, resolve_active_selection, resolve_agent_finalized_retain_limit,
    resolve_agent_max_concurrent, resolve_agent_max_subrun_depth,
    resolve_anthropic_messages_api_url, resolve_anthropic_models_api_url,
    resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent, resolve_continuation_min_delta_tokens,
    resolve_current_model, resolve_default_token_budget, resolve_env_value,
    resolve_max_consecutive_failures, resolve_max_continuations, resolve_max_tool_concurrency,
    resolve_recovery_truncate_bytes, resolve_tool_result_max_bytes, save_config, test_connection,
};
pub use astrcode_runtime_llm as llm;
pub use astrcode_runtime_prompt as prompt;
pub use astrcode_runtime_skill_loader as skills;
pub use bootstrap::{PluginLoadHandle, PluginLoadState, RuntimeBootstrap, bootstrap_runtime};
pub use runtime_governance::{RuntimeGovernance, RuntimeGovernanceSnapshot, RuntimeReloadResult};
pub use service::{
    AgentExecutionServiceHandle, AgentProfileSummary, ComposerOption, ComposerOptionKind,
    ComposerOptionsRequest, ComposerServiceHandle, ExecutionDiagnosticsSnapshot,
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent, SessionEventRecord,
    SessionHistorySnapshot, SessionReplay, SessionReplaySource, SessionServiceHandle,
    SubRunExecutionMetricsSnapshot, SubRunStatusSnapshot, SubRunStatusSource,
    ToolExecutionServiceHandle, ToolSummary,
};
