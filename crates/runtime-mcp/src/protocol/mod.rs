//! # MCP JSON-RPC 协议层
//!
//! 定义 MCP 协议的消息类型、DTO 和错误类型。
//! MCP 协议基于 JSON-RPC 2.0，本模块直接实现协议层而非依赖外部 SDK。

mod client;
mod error;
pub mod types;

pub use client::McpClient;
pub use error::McpProtocolError;
pub use types::*;
