mod agent_loop;
mod cancel;
mod config;
mod event_log;
mod events;
mod llm;
mod projection;
mod prompt;
mod provider_factory;
mod service;
#[cfg(test)]
mod test_support;
mod tool_registry;

pub use config::{
    config_path, load_config, open_config_in_editor, save_config, test_connection, Config, Profile,
    TestResult,
};
pub use event_log::{DeleteProjectResult, EventLog, SessionMeta};
pub use service::{
    AgentService, PromptAccepted, ServiceError, SessionEventRecord, SessionMessage, SessionReplay,
    SessionReplaySource,
};
pub use tool_registry::{ToolRegistry, ToolRegistryBuilder};
