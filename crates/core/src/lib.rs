mod action;
mod cancel;
pub mod capability;
mod error;
pub mod event;
pub mod plugin;
pub mod policy;
pub mod projection;
pub mod registry;
pub mod runtime;
pub mod session;
#[cfg(test)]
mod test_support;
mod tool;

pub use action::{
    split_assistant_content, AssistantContentParts, LlmMessage, LlmResponse, ReasoningContent,
    ToolCallRequest, ToolDefinition, ToolExecutionResult,
};
pub use cancel::CancelToken;
pub use capability::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, CapabilityNamespace,
    DescriptorBuildError, PermissionHint, SideEffectLevel, StabilityLevel,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    generate_session_id, phase_of_storage_event, replay_records, AgentEvent, EventLog, EventStore,
    EventTranslator, Phase, StorageEvent, StoredEvent,
};
pub use plugin::{PluginHealth, PluginManifest, PluginRegistry, PluginState, PluginType};
pub use policy::{AllowAllPolicyEngine, PolicyDecision, PolicyEngine};
pub use projection::{project, AgentState, AgentStateProjector};
pub use registry::{
    CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilityRouter,
    CapabilityRouterBuilder,
};
pub use registry::{ToolCapabilityInvoker, ToolRegistry, ToolRegistryBuilder};
pub use runtime::{
    KernelApi, ManagedRuntimeComponent, Orchestrator, RuntimeCoordinator, RuntimeHandle,
    TurnContext, TurnOutcome,
};
pub use session::{
    DeleteProjectResult, FileSystemSessionRepository, SessionEventRecord, SessionManager,
    SessionMessage, SessionMeta, SessionWriter,
};
pub use tool::{SessionId, Tool, ToolContext, DEFAULT_MAX_OUTPUT_SIZE};
