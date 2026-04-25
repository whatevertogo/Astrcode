//! server-owned runtime bridge 层。
//!
//! 这里集中放 server 到 owner/runtime/adapter 的正式接线面，避免组合根直接暴露底层实现。

pub(crate) mod agent_control;
pub(crate) mod agent_control_registry;
pub(crate) mod agent_runtime;
pub(crate) mod capability_router;
pub(crate) mod config_service;
pub(crate) mod governance_service;
pub(crate) mod hook_dispatcher;
pub(crate) mod mcp_service;
pub(crate) mod mode_catalog;
pub(crate) mod ports;
pub(crate) mod profile_service;
pub(crate) mod runtime_owner;
pub(crate) mod session_owner;
pub(crate) mod session_port;
pub(crate) mod tool_capability;
pub(crate) mod watch_service;
