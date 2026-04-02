//! # 会话类型定义
//!
//! 定义会话消息和事件记录的数据结构。
//!
//! ## 与存储事件的区别
//!
//! - `SessionMessage`: 面向前端展示的历史消息格式
//! - `SessionEventRecord`: SSE 推送和会话回放的领域事件记录
//! - `StorageEvent`: 面向持久化的 JSONL 事件格式

use crate::{event, AgentEvent};

pub use event::{DeleteProjectResult, SessionMeta};

/// 会话消息
///
/// 表示会话历史中的一条消息，用于前端展示。
/// 与 `StorageEvent` 不同，这是面向展示的格式。
#[derive(Clone, Debug)]
pub enum SessionMessage {
    /// 用户消息
    User {
        turn_id: Option<String>,
        content: String,
        timestamp: String,
    },
    /// 助手消息
    Assistant {
        turn_id: Option<String>,
        content: String,
        timestamp: String,
        reasoning_content: Option<String>,
    },
    /// 工具调用消息
    ToolCall {
        turn_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        output: Option<String>,
        error: Option<String>,
        metadata: Option<serde_json::Value>,
        ok: Option<bool>,
        duration_ms: Option<u64>,
    },
}

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
