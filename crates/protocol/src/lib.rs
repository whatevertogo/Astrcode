//! # Astrcode 协议定义
//!
//! 本库定义了跨模块通信的协议格式，是整个后端架构的**纯数据契约层**。
//!
//! ## 架构定位
//!
//! `protocol` crate 处于依赖图的最底层，不得依赖 `core`/`runtime` 等上层 crate。
//! 所有跨边界数据交换都通过本库定义的显式 DTO + mapper 进行转换，确保模块间编译隔离。
//!
//! ## 核心功能模块
//!
//! - **HTTP DTO** (`http`): API 请求/响应的数据结构，用于 server 与前端之间的序列化通信
//! - **插件协议** (`plugin`): 基于 JSON-RPC 的插件进程通信消息格式，包括握手、能力描述、调用/事件流
//! - **传输层** (`transport`): 抽象传输 trait，定义 send/recv 接口
//!
//! ## 设计原则
//!
//! - 所有 DTO 都是纯数据，不包含业务逻辑
//! - 使用 serde 进行序列化/反序列化
//! - 版本控制通过 `PROTOCOL_VERSION` 常量管理

pub mod http;
pub mod plugin;
pub mod transport;
