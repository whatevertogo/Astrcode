//! 会话目录事件 DTO。
//!
//! 目录事件载荷已经是共享语义，协议层直接复用 `core`；
//! 外层信封仍由 protocol 拥有。

pub use astrcode_core::SessionCatalogEvent as SessionCatalogEventPayload;
use serde::{Deserialize, Serialize};

use crate::http::PROTOCOL_VERSION;

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
