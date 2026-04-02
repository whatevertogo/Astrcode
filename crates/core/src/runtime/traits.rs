//! # 运行时接口
//!
//! 定义了运行时组件的抽象接口，用于管理 LLM 连接和生命周期。
//!
//! ## 核心接口
//!
//! - [`RuntimeHandle`][]: 运行时主句柄，提供名称、类型和关闭接口
//! - [`ManagedRuntimeComponent`][]: 可被运行时协调器管理的子组件

use async_trait::async_trait;

use crate::AstrError;

/// 运行时主句柄。
///
/// 代表一个具体的 LLM 运行时实现（如 OpenAI 兼容 API 客户端）。
/// 通过 [`RuntimeCoordinator`](crate::RuntimeCoordinator) 统一管理生命周期。
#[async_trait]
pub trait RuntimeHandle: Send + Sync {
    /// 运行时实例的名称（用于日志和错误信息）。
    fn runtime_name(&self) -> &'static str;

    /// 运行时的类型标识（如 "openai"、"anthropic"）。
    fn runtime_kind(&self) -> &'static str;

    /// 优雅关闭运行时，释放所有连接和资源。
    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError>;
}

/// 可被运行时协调器管理的子组件。
///
/// 用于管理除主运行时之外的其他需要生命周期管理的组件
/// （如 SSE 广播器、后台任务等）。
#[async_trait]
pub trait ManagedRuntimeComponent: Send + Sync {
    /// 组件名称（用于日志和错误信息）。
    fn component_name(&self) -> String;

    /// 优雅关闭组件，释放资源。
    async fn shutdown_component(&self) -> std::result::Result<(), AstrError>;
}
