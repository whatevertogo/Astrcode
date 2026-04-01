//! # Astrcode 插件系统
//!
//! 本库实现了插件进程的管理和通信，包括：
//!
//! - **进程管理**: 启动、监控、重启插件进程
//! - **通信**: 与插件进程进行 JSON-RPC 通信
//! - **生命周期**: 处理插件进程的崩溃和重启
//! - **流式执行**: 支持插件的流式响应

mod capability_router;
mod invoker;
mod loader;
mod peer;
mod process;
mod streaming;
mod supervisor;
pub mod transport;
mod worker;

pub use capability_router::{
    AllowAllPermissionChecker, CapabilityHandler, CapabilityRouter, PermissionChecker,
};
pub use invoker::PluginCapabilityInvoker;
pub use loader::PluginLoader;
pub use peer::Peer;
pub use process::{PluginProcess, PluginProcessStatus};
pub use streaming::{EventEmitter, StreamExecution};
pub use supervisor::{
    default_initialize_message, default_profiles, Supervisor, SupervisorHealth,
    SupervisorHealthReport,
};
pub use worker::Worker;
