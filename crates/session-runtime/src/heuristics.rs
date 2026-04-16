//! 运行时启发式常量。
//!
//! 这些值当前服务于 prompt 预算估算和只读观察摘要。
//! 它们不是用户配置项，但应集中管理，避免 magic number 分散。

/// 单条消息的固定估算开销。
pub(crate) const MESSAGE_BASE_TOKENS: usize = 6;

/// 单个工具调用元数据的固定估算开销。
pub(crate) const TOOL_CALL_BASE_TOKENS: usize = 12;

/// agent observe 返回的最近 mailbox 消息条数。
pub(crate) const MAX_RECENT_MAILBOX_MESSAGES: usize = 3;

/// task/mailbox 摘要的最大字符数。
pub(crate) const MAX_TASK_SUMMARY_CHARS: usize = 120;
