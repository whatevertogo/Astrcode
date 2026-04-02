//! 会话目录事件 DTO
//!
//! 定义会话生命周期中的目录级事件（创建、删除、分支），
//! 通过 SSE 广播通知前端更新会话列表视图。
//! 与 `event.rs` 中的 Agent 事件不同，这些事件关注会话管理而非 Agent 执行。

use serde::{Deserialize, Serialize};

use crate::http::PROTOCOL_VERSION;

/// 会话目录事件载荷的 tagged enum。
///
/// 采用 `#[serde(tag = "event", content = "data")]` 序列化策略。
/// 这些事件由 server 在会话生命周期变更时广播，前端据此更新会话列表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum SessionCatalogEventPayload {
    /// 新会话创建事件。
    SessionCreated { session_id: String },
    /// 会话删除事件。
    SessionDeleted { session_id: String },
    /// 整个项目（工作目录）删除事件。
    ///
    /// 删除项目会级联删除其下所有会话。
    ProjectDeleted { working_dir: String },
    /// 会话分支事件。
    ///
    /// 当从一个现有会话分支出新会话时触发。
    SessionBranched {
        session_id: String,
        source_session_id: String,
    },
}

/// 会话目录事件信封。
///
/// 为事件载荷添加协议版本号，确保前端可以验证兼容性。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCatalogEventEnvelope {
    /// 协议版本号
    pub protocol_version: u32,
    /// 事件载荷，序列化后扁平化到信封层级
    #[serde(flatten)]
    pub event: SessionCatalogEventPayload,
}

impl SessionCatalogEventEnvelope {
    /// 创建新的事件信封，自动设置协议版本。
    pub fn new(event: SessionCatalogEventPayload) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            event,
        }
    }
}
