//! # 事件存储与回放系统
//!
//! 本模块实现了 append-only 的事件日志系统，用于持久化 Agent 会话的所有事件。
//!
//! ## 核心设计
//!
//! - **JSONL 格式**: 每行一个 JSON 事件，append-only 写入
//! - **存储序号 (storage_seq)**: 每个事件携带单调递增的序号，用于 SSE 的 `id` 字段实现断点续传
//! - **子序号 (subindex)**: 一个存储事件可能产生多个领域事件，通过 `{storage_seq}.{subindex}`
//!   唯一标识
//!
//! ## 模块说明
//!
//! - `domain`: 领域事件类型（`AgentEvent`）和会话阶段（`Phase`）
//! - `types`: 存储事件类型（`StorageEvent`）和序列化格式
//! - `translate`: `EventTranslator` 将存储事件转换为领域事件

mod domain;
mod phase;
mod translate;
mod types;

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use self::{
    domain::{AgentEvent, Phase},
    phase::{PhaseTracker, normalize_recovered_phase, target_phase as phase_of_storage_event},
    translate::{EventTranslator, replay_records},
    types::{
        CompactAppliedMeta, CompactMode, CompactTrigger, PromptMetricsPayload, StorageEvent,
        StorageEventPayload, StoredEvent, TurnTerminalKind,
    },
};

/// 生成全局唯一的会话 ID，格式为 `YYYY-MM-DDTHH-MM-SS-xxxxxxxx`。
///
/// 时间戳部分使用 `-` 而非 `:` 分隔（如 `T10-00-00` 而非 `T10:00:00`），
/// 因为冒号在 Windows 文件名中非法，而 session ID 直接用于 `.jsonl` 文件名。
/// 末尾 8 位 hex 取自 UUID v4，保证同一秒内生成的 ID 也不重复。
pub fn generate_session_id() -> String {
    // session_id 直接出现在用户可见的目录名里，用本地时间更符合排障直觉；
    // 内部排序仍然依赖 storage_seq，而不是文件名中的时间前缀。
    let dt = Local::now().format("%Y-%m-%dT%H-%M-%S");
    let uuid = Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    format!("{dt}-{short}")
}

/// 生成全局唯一的 turn ID。
///
/// turn 会在极短时间内连续创建，单纯依赖毫秒时间戳会撞 ID，
/// 从而让后续提交错误复用前一个 turn 的终态快照。
pub fn generate_turn_id() -> String {
    let uuid = Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    format!("turn-{}-{short}", Utc::now().timestamp_millis())
}

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
    #[serde(with = "crate::local_rfc3339")]
    pub created_at: DateTime<Utc>,
    /// 会话最后更新时间
    #[serde(with = "crate::local_rfc3339")]
    pub updated_at: DateTime<Utc>,
    /// 分叉来源 session。根会话为 None。
    pub parent_session_id: Option<String>,
    /// 分叉点在父 session 中的最后一个稳定 storage_seq。
    pub parent_storage_seq: Option<u64>,
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
