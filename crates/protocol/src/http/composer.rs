//! 输入候选（composer options）相关 DTO。
//!
//! 单个候选项已经是跨层共享的 canonical 读模型，协议层直接复用 `core`；
//! 外层响应壳仍由 protocol 拥有。

pub use astrcode_core::{
    ComposerOption as ComposerOptionDto, ComposerOptionActionKind as ComposerOptionActionKindDto,
    ComposerOptionKind as ComposerOptionKindDto,
};
use serde::{Deserialize, Serialize};

/// 输入候选列表响应。
///
/// 预留响应外层对象而非直接返回数组，是为了后续在不破坏协议的前提下
/// 增加服务端元数据（例如 query 规范化结果或分页信息）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerOptionsResponseDto {
    pub items: Vec<ComposerOptionDto>,
}
