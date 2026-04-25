//! 统一 plugin 宿主骨架。
//!
//! 后续这里将承接旧 `crates/plugin` 的进程管理、descriptor 校验、
//! snapshot 激活和资源发现。

pub mod backend;
pub mod builtin_hooks;
pub mod descriptor;
pub mod hooks;
pub mod host;
pub mod loader;
pub mod manifest;
pub mod modes;
pub mod protocol;
pub mod providers;
pub mod registry;
pub mod resource_provider;
pub mod resources;
pub mod snapshot;
pub mod tools;
pub mod transport;

pub use builtin_hooks::{
    BuiltinHookExecutor, BuiltinHookRegistry, HookBinding, HookContext, HookExecutorRef,
    HookHostView,
};
pub use descriptor::{
    CommandDescriptor, HookDescriptor, PluginDescriptor, PluginSourceKind, PromptDescriptor,
    ProviderDescriptor, ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
};
pub use hooks::{
    ExternalHookDispatchRequest, ExternalHookDispatcher, HookDispatchMode, HookFailurePolicy,
    HookStage, SUPPORTED_HOOK_EVENTS, dispatch_hooks, dispatch_hooks_with_external,
    hook_effect_from_wire, hook_effects_from_wire,
};
pub use host::{
    ActivePluginRuntimeCatalog, ActivePluginRuntimeEntry, BuiltinCapabilityExecutor,
    BuiltinCapabilityExecutorRegistry, ExternalBackendHealthCatalog, PluginCapabilityBinding,
    PluginCapabilityDispatchKind, PluginCapabilityDispatchOutcome,
    PluginCapabilityDispatchReadiness, PluginCapabilityDispatchTicket,
    PluginCapabilityDispatcherSet, PluginCapabilityHttpDispatch, PluginCapabilityHttpDispatcher,
    PluginCapabilityHttpDispatcherRegistry, PluginCapabilityInvocationPlan,
    PluginCapabilityInvocationTarget, PluginCapabilityProtocolDispatch,
    PluginCapabilityProtocolDispatcher, PluginCapabilityProtocolDispatcherRegistry,
    PluginCapabilityProtocolExecution, PluginCapabilityProtocolTransport, PluginHost,
    PluginHostReload, PluginRuntimeHandleRef, PluginRuntimeHandleSnapshot,
    TransportBackedProtocolDispatcher,
};
pub use loader::PluginLoader;
pub use manifest::{
    CommandManifestEntry, PluginManifest, PluginType, PromptManifestEntry, ProviderManifestEntry,
    ResourceManifestEntry, SkillManifestEntry, ThemeManifestEntry,
};
pub use modes::builtin_modes_descriptor;
pub use protocol::{
    PluginInitializeState, RemotePluginHandshakeSummary, default_initialize_message,
    default_local_peer_descriptor, default_profiles,
};
pub use providers::{
    OPENAI_API_KIND, OPENAI_PROVIDER_ID, ProviderContributionCatalog,
    builtin_openai_provider_descriptor,
};
pub use registry::{PluginEntry, PluginHealth, PluginRegistry, PluginState};
pub use resource_provider::{ResourceProvider, ResourceReadResult, ResourceRequestContext};
pub use resources::{
    ResourceCatalog, ResourceDiscoverReport, SkillCatalogBaseBuild, build_skill_catalog_base,
    resources_discover,
};
pub use snapshot::PluginActiveSnapshot;
pub use tools::{
    ToolContributionCatalog, builtin_collaboration_tools_descriptor, builtin_tools_descriptor,
    tool_contribution_catalog,
};
pub use transport::PluginStdioTransport;
