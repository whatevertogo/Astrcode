//! # Session Catalog 事件
//!
//! 定义会话目录变更通知事件，用于向前端和其他订阅者广播
//! session 的创建、删除、分支等生命周期变化。

use serde::{Deserialize, Serialize};

/// Session catalog 变更事件，用于通知外部订阅者 session 列表变化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum SessionCatalogEvent {
    SessionCreated {
        session_id: String,
    },
    SessionDeleted {
        session_id: String,
    },
    ProjectDeleted {
        working_dir: String,
    },
    SessionBranched {
        session_id: String,
        source_session_id: String,
    },
}
