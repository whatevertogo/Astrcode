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

mod agent_loop;
mod approval_service;
mod bootstrap;
#[cfg(test)]
mod bootstrap_tests;
mod builtin_capabilities;
mod builtin_skills;
mod plugin_discovery;
mod provider_factory;
mod runtime_governance;
mod runtime_surface_assembler;
mod service;
mod skill_tool;
#[cfg(test)]
mod test_support;

pub use astrcode_runtime_config as config;
pub use astrcode_runtime_config::{
    config_path, env_reference, is_env_var_name, list_model_options, load_config,
    load_resolved_config, open_config_in_editor, parse_env_value, resolve_active_selection,
    resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent, resolve_continuation_min_delta_tokens,
    resolve_current_model, resolve_default_token_budget, resolve_env_value,
    resolve_max_continuations, resolve_max_tool_concurrency, resolve_tool_result_max_bytes,
    save_config, test_connection, ActiveSelection, Config, ConfigOverlay, CurrentModelSelection,
    ModelOption, ParsedEnvValue, Profile, RuntimeConfig, TestResult,
};
pub use astrcode_runtime_llm as llm;
pub use astrcode_runtime_prompt as prompt;
pub use bootstrap::{bootstrap_runtime, RuntimeBootstrap};
pub use runtime_governance::{RuntimeGovernance, RuntimeGovernanceSnapshot, RuntimeReloadResult};
pub use service::{
    OperationMetricsSnapshot, PromptAccepted, ReplayMetricsSnapshot, ReplayPath,
    RuntimeObservabilitySnapshot, RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent,
    SessionEventRecord, SessionMessage, SessionReplay, SessionReplaySource,
};
