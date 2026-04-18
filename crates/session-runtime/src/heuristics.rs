//! 运行时启发式常量。
//!
//! 这些值当前服务于 prompt 预算估算。
//! 它们不是用户配置项，但应集中管理，避免 magic number 分散。

/// 单条消息的固定估算开销。
pub(crate) const MESSAGE_BASE_TOKENS: usize = 6;

/// 单个工具调用元数据的固定估算开销。
pub(crate) const TOOL_CALL_BASE_TOKENS: usize = 12;
