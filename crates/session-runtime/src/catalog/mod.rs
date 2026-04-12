//! Session catalog 事件与生命周期协调。
//!
//! 从 `runtime/service/session/` 迁入核心 catalog 事件定义。
//! 实际的磁盘 create/load/delete 由 adapter-storage 实现，
//! session-runtime 只编排生命周期和广播。

use serde::{Deserialize, Serialize};

/// Session catalog 变更事件，用于通知外部订阅者 session 列表变化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SessionCatalogEvent {
    SessionCreated { session_id: String },
    SessionDeleted { session_id: String },
    ProjectDeleted { working_dir: String },
}
