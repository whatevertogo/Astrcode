//! # Astrcode MCP Server 接入支持
//!
//! 本 crate 实现 MCP (Model Context Protocol) 服务器的连接管理和工具桥接，
//! 将外部 MCP 服务器提供的工具、prompt 和资源注册到 Astrcode 能力路由中。
//!
//! ## 架构定位
//!
//! `runtime-mcp` 与 `plugin` crate 并行，位于同一架构层。
//! 仅依赖 `core` 和 `protocol`，不依赖 `runtime`（宪法编译隔离约束）。
//!
//! ## 模块组织
//!
//! - `protocol`: MCP JSON-RPC 协议层（消息类型、客户端、DTO）
//! - `transport`: MCP 传输层抽象（stdio、HTTP、SSE）
//! - `config`: MCP 配置管理（加载、去重、审批、策略）
//! - `manager`: 连接生命周期管理（连接管理器、状态机、重连、热加载）
//! - `bridge`: 工具/prompt/资源/skill 桥接层（Phase 3+ 声明）

pub mod bridge;
pub mod config;
pub mod manager;
pub mod protocol;
pub mod transport;
