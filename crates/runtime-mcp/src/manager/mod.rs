//! # MCP 连接管理
//!
//! 负责所有 MCP 服务器的连接生命周期管理。

pub mod connection;

pub use connection::{McpConnection, McpConnectionState};
