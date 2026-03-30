mod agent_loop;
mod approval_service;
mod bootstrap;
#[cfg(test)]
mod bootstrap_tests;
mod builtin_capabilities;
mod cancel;
mod config;
mod llm;
mod plugin_discovery;
mod prompt;
mod provider_factory;
mod runtime_governance;
mod runtime_surface_assembler;
mod service;
#[cfg(test)]
mod test_support;

pub use approval_service::{ApprovalBroker, DefaultApprovalBroker};
pub use bootstrap::{bootstrap_runtime, RuntimeBootstrap};
pub use config::{
    config_path, load_config, open_config_in_editor, save_config, test_connection, Config, Profile,
    TestResult,
};
pub use runtime_governance::{RuntimeGovernance, RuntimeGovernanceSnapshot, RuntimeReloadResult};
pub use service::{
    OperationMetricsSnapshot, PromptAccepted, ReplayMetricsSnapshot, ReplayPath,
    RuntimeObservabilitySnapshot, RuntimeService, ServiceError, ServiceResult, SessionEventRecord,
    SessionMessage, SessionReplay, SessionReplaySource,
};
