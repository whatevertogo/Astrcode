//! # 会话类型定义
//!
//! 定义会话事件记录和会话元数据的数据结构。
//!
//! ## 与存储事件的区别
//!
//! - `SessionEventRecord`: SSE 推送和会话回放的领域事件记录
//! - `StorageEvent`: 面向持久化的 JSONL 事件格式

pub use event::{DeleteProjectResult, SessionMeta};

use crate::{AgentEvent, event};

/// 会话事件记录
///
/// 包含事件 ID 和实际的领域事件。
/// 用于 SSE 推送和会话回放。
#[derive(Clone, Debug)]
pub struct SessionEventRecord {
    /// 事件 ID，格式为 `{storage_seq}.{subindex}`
    pub event_id: String,
    /// 实际的领域事件
    pub event: AgentEvent,
}
