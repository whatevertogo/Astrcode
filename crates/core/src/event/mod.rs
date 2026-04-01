//! # 事件存储与回放系统
//!
//! 本模块实现了 append-only 的事件日志系统，用于持久化 Agent 会话的所有事件。
//!
//! ## 核心设计
//!
//! - **JSONL 格式**: 每行一个 JSON 事件，append-only 写入
//! - **存储序号 (storage_seq)**: 每个事件携带单调递增的序号，用于 SSE 的 `id` 字段实现断点续传
//! - **子序号 (subindex)**: 一个存储事件可能产生多个领域事件，通过 `{storage_seq}.{subindex}` 唯一标识
//!
//! ## 模块说明
//!
//! - `domain`: 领域事件类型（`AgentEvent`）和会话阶段（`Phase`）
//! - `types`: 存储事件类型（`StorageEvent`）和序列化格式
//! - `store`: `EventLog` 实现（文件的创建、打开、追加、加载）
//! - `translate`: `EventTranslator` 将存储事件转换为领域事件
//! - `paths`: 会话文件路径生成和验证
//! - `query`: 事件查询功能

mod domain;
mod paths;
mod query;
mod store;
mod translate;
mod types;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use self::domain::{AgentEvent, Phase};
pub use self::paths::generate_session_id;
use self::paths::{session_path, validated_session_id};
pub use self::store::EventLogIterator;
pub use self::translate::{phase_of_storage_event, replay_records, EventTranslator};
pub use self::types::{StorageEvent, StoredEvent, StoredEventLine};

/// 会话元数据
///
/// 包含会话的基本信息和当前状态，用于会话列表展示。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    /// 会话唯一标识符
    pub session_id: String,
    /// 工作目录（项目路径）
    pub working_dir: String,
    /// 显示名称（用户友好的项目名）
    pub display_name: String,
    /// 会话标题（从最新消息推导）
    pub title: String,
    /// 会话创建时间
    pub created_at: DateTime<Utc>,
    /// 会话最后更新时间
    pub updated_at: DateTime<Utc>,
    /// 当前阶段
    pub phase: Phase,
}

/// 项目删除结果
///
/// 返回成功删除的会话数和失败的会话 ID 列表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectResult {
    /// 成功删除的会话数量
    pub success_count: usize,
    /// 删除失败的会话 ID 列表
    pub failed_session_ids: Vec<String>,
}

/// 事件日志
///
/// 负责将会话事件追加到 JSONL 文件。每个会话对应一个文件：
/// `~/.astrcode/sessions/session-{id}.jsonl`
///
/// ## 存储格式
///
/// 每行一个 JSON 对象：
/// ```json
/// {"storage_seq": 1, "event": {"SessionStart": {...}}}
/// {"storage_seq": 2, "event": {"UserMessage": {...}}}
/// ```
///
/// ## 存储 Seq 分配
///
/// - `storage_seq` 由 Writer 独占分配，保证单调递增
/// - SSE 事件 ID 使用 `{storage_seq}.{subindex}` 格式
/// - 客户端可以用 `Last-Event-ID` 实现断点续传
pub struct EventLog {
    /// 会话 ID
    session_id: String,
    /// 日志文件路径
    path: PathBuf,
    /// 缓冲写入器（提高写入性能）
    writer: BufWriter<File>,
    /// 下一个事件的存储序号
    next_storage_seq: u64,
}

/// 确保 EventLog 在 Drop 时正确刷新和同步
///
/// 这是防止数据丢失的关键：即使 panic 发生，也要尽力刷新缓冲区。
impl Drop for EventLog {
    fn drop(&mut self) {
        if let Err(error) = self.writer.flush() {
            log::warn!(
                "failed to flush event log '{}' on drop: {}",
                self.path.display(),
                error
            );
            return;
        }

        if let Err(error) = self.writer.get_ref().sync_all() {
            log::warn!(
                "failed to sync event log '{}' on drop: {}",
                self.path.display(),
                error
            );
        }
    }
}

#[cfg(test)]
mod tests;
