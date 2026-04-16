//! # Astrcode 插件系统
//!
//! 本库实现了插件进程的管理和 JSON-RPC 通信，是 Astrcode 可扩展架构的核心。
//!
//! ## 架构概览
//!
//! ```text
//! Runtime / Server                    插件宿主 (本 crate)                    插件进程
//! ──────────────                     ──────────────────                    ──────────
//! CapabilityInvoker ──────────────►  Supervisor                           Worker
//!                                     ├─ PluginProcess (子进程管理)         ├─ CapabilityRouter
//!                                     ├─ Peer (JSON-RPC 通信)              ├─ StdioTransport
//!                                     └─ CapabilityRouter (反向调用)        └─ 能力处理器
//! ```
//!
//! ## 核心组件
//!
//! - **进程管理** (`process`): 启动、监控、重启插件子进程
//! - **通信** (`peer`, `transport`): 基于 stdio 的 JSON-RPC 双向通信
//! - **生命周期** (`supervisor`): 处理插件进程的握手、健康检查和优雅关闭
//! - **流式执行** (`streaming`): 支持插件的流式响应（增量事件）
//! - **能力路由** (`capability_router`): 路由能力调用并执行权限检查
//! - **插件加载** (`loader`): 发现、解析和启动插件
//! - **Worker** (`worker`): 插件进程侧的入口，用于编写插件二进制
//!
//! ## 通信协议
//!
//! 插件通过 stdio 与宿主进行 JSON-RPC 通信，消息类型包括：
//!
//! - `InitializeMessage` / `InitializeResultData` — 握手协商
//! - `InvokeMessage` / `ResultMessage` — 能力调用与结果
//! - `EventMessage` — 流式增量事件（started → delta × N → completed/failed）
//! - `CancelMessage` — 取消请求
//!
//! ## 使用方式
//!
//! ### 宿主侧
//!
//! ```ignore
//! let loader = PluginLoader { search_paths: vec!["plugins/".into()] };
//! let manifests = loader.discover()?;
//! for manifest in manifests {
//!     let supervisor = loader.start(&manifest, local_peer, None).await?;
//!     let invokers = supervisor.capability_invokers();
//!     // 注册到 runtime...
//! }
//! ```
//!
//! ### 插件侧
//!
//! ```ignore
//! let mut router = CapabilityRouter::default();
//! router.register(MyHandler)?;
//!
//! let worker = Worker::from_stdio(peer_descriptor, router, None);
//! worker.run().await?;
//! ```

mod capability_mapping;
mod capability_router;
mod invoker;
mod loader;
mod peer;
mod process;
mod streaming;
mod supervisor;
pub mod transport;
mod worker;

pub use capability_mapping::{spec_to_wire_descriptor, wire_descriptor_to_spec};
pub use capability_router::{
    AllowAllPermissionChecker, CapabilityHandler, CapabilityRouter, PermissionChecker,
};
pub use invoker::PluginCapabilityInvoker;
pub use loader::PluginLoader;
pub use peer::Peer;
pub use process::{PluginProcess, PluginProcessStatus};
pub use streaming::{EventEmitter, StreamExecution};
pub use supervisor::{
    Supervisor, SupervisorHealth, SupervisorHealthReport, default_initialize_message,
    default_profiles,
};
pub use worker::Worker;
