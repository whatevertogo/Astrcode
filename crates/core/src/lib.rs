//! # Astrcode 核心库
//!
//! 本库定义了 Astrcode 系统的核心领域模型和接口，与具体的运行时实现解耦。
//!
//! ## 主要模块
//!
//! - [`event`][]: 事件存储与回放系统（JSONL append-only 日志）
//! - [`session`][]: 会话管理与持久化
//! - [`tool`][]: Tool trait 定义（插件系统的基础抽象）
//! - [`capability`][]: 能力描述符（用于策略引擎和 UI 展示）
//! - [`policy`][]: 策略引擎 trait（审批、内容审查、上下文压缩决策）
//! - [`plugin`][]: 插件清单与注册表
//! - [`registry`][]: 能力路由器（将能力调用分派到具体的 invoker）
//! - [`runtime`][]: 运行时协调器接口
//! - [`projection`][]: Agent 状态投影（从事件流推导状态）
//! - [`action`][]: LLM 消息与工具调用相关的数据结构

mod action;
mod cancel;
pub mod capability;
pub mod env;
mod error;
pub mod event;
pub mod home;
pub mod plugin;
pub mod policy;
pub mod project;
pub mod projection;
pub mod registry;
pub mod runtime;
pub mod session;
pub mod store;
// test_support 是 pub mod（而非 #[cfg(test)]），因为其他 crate（runtime, runtime-config,
// runtime-prompt）的测试代码通过 `astrcode_core::test_support::TestEnvGuard` 导入它。
// Rust 不支持跨 crate 的 #[cfg(test)] 导出，所以只能保持 pub。tempfile 依赖也因此
// 无法移到 [dev-dependencies]，但该模块除测试外不会在生产代码路径中被调用。
pub mod test_support;
mod tool;

pub use action::{
    split_assistant_content, AssistantContentParts, LlmMessage, ReasoningContent, ToolCallRequest,
    ToolDefinition, ToolExecutionResult, ToolOutputDelta, ToolOutputStream,
};
pub use cancel::CancelToken;
pub use capability::{
    CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, DescriptorBuildError,
    PermissionHint, SideEffectLevel, StabilityLevel,
};
pub use error::{AstrError, Result, ResultExt};
pub use event::{
    generate_session_id, phase_of_storage_event, replay_records, AgentEvent, EventTranslator,
    Phase, StorageEvent, StoredEvent, StoredEventLine,
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
pub use runtime::{ManagedRuntimeComponent, RuntimeCoordinator, RuntimeHandle};
pub use session::{DeleteProjectResult, SessionEventRecord, SessionMessage, SessionMeta};
pub use store::{
    EventLogWriter, SessionManager, SessionTurnAcquireResult, SessionTurnBusy, SessionTurnLease,
    StoreError, StoreResult,
};
pub use tool::{
    SessionId, Tool, ToolCapabilityMetadata, ToolContext, ToolPromptMetadata,
    DEFAULT_MAX_OUTPUT_SIZE,
};
