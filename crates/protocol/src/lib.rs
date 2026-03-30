//! # Astrcode 协议定义
//!
//! 本库定义了跨模块通信的协议格式，包括：
//!
//! - **HTTP DTO**: API 请求/响应的数据结构
//! - **插件协议**: 与插件进程通信的 JSON-RPC 消息格式
//! - **传输层**: stdio 传输的实现

pub mod http;
pub mod plugin;
pub mod transport;
