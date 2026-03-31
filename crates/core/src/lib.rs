//! # Astrcode 核心库
//!
//! 本库定义了 Astrcode 系统的核心领域模型和接口，与具体的运行时实现解耦。
//!
//! ## 主要模块
//!
//! - [`event`]: 事件存储与回放系统（JSONL append-only 日志）
//! - [`session`]: 会话管理与持久化
//! - [`tool`]: Tool trait 定义（插件系统的基础抽象）
//! - [`capability`]: 能力描述符（用于策略引擎和 UI 展示）
//! - [`policy`]: 策略引擎 trait（审批、内容审查、上下文压缩决策）
//! - [`plugin`]: 插件清单与注册表
//! - [`registry`]: 能力路由器（将能力调用分派到具体的 invoker）
//! - [`runtime`]: 运行时协调器接口
//! - [`projection`]: Agent 状态投影（从事件流推导状态）
//! - [`action`]: LLM 消息与工具调用相关的数据结构

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
    generate_session_id, phase_of_storage_event, replay_records, AgentEvent, EventLog,
    EventLogIterator, EventStore, EventTranslator, Phase, StorageEvent, StoredEvent,
};
pub use plugin::{PluginHealth, PluginManifest, PluginRegistry, PluginState, PluginType};
pub use policy::{
    AllowAllPolicyEngine, ApprovalDefault, ApprovalPending, ApprovalRequest, ApprovalResolution,
    CapabilityCall, ContextPressureInput, ContextStrategyDecision, ModelRequest, PolicyContext,
    PolicyEngine, PolicyVerdict,
};
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
pub use tool::{SessionId, Tool, ToolCapabilityMetadata, ToolContext, DEFAULT_MAX_OUTPUT_SIZE};
