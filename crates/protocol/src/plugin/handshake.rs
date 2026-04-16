//! 握手协议
//!
//! 定义 host 与插件之间握手阶段的消息格式。
//!
//! ## 握手流程
//!
//! 1. Host 发送 `InitializeMessage`，携带自身 peer 信息、支持的能力列表、处理器和 profile
//! 2. 插件回复 `InitializeResultData`（通过 `ResultMessage` 包装），携带自身信息
//! 3. 双方验证协议版本兼容性，确认能力注册
//!
//! 握手完成后，双方进入正常的调用/事件流阶段。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    CapabilityWireDescriptor, HandlerDescriptor, PeerDescriptor, ProfileDescriptor, SkillDescriptor,
};

/// 插件协议版本号。
///
/// 当前版本为 "5"，与 capability wire shape 的 `invocationMode` 收口一致。
pub const PROTOCOL_VERSION: &str = "5";

/// 握手初始化消息，由 host 发送给插件。
///
/// 携带 host 的 peer 信息、支持的能力列表、事件处理器和 profile 定义。
/// `supported_protocol_versions` 允许 host 声明兼容的多个协议版本，
/// 插件可据此选择双方都支持的版本进行通信。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeMessage {
    /// 消息唯一标识
    pub id: String,
    /// 当前使用的协议版本
    pub protocol_version: String,
    /// host 兼容的协议版本列表，插件可据此协商版本
    #[serde(default)]
    pub supported_protocol_versions: Vec<String>,
    /// host 的 peer 描述
    pub peer: PeerDescriptor,
    /// host 暴露的能力列表
    #[serde(default)]
    pub capabilities: Vec<CapabilityWireDescriptor>,
    /// host 注册的事件处理器列表
    #[serde(default)]
    pub handlers: Vec<HandlerDescriptor>,
    /// host 支持的 profile 列表
    #[serde(default)]
    pub profiles: Vec<ProfileDescriptor>,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 握手初始化结果，由插件回复给 host。
///
/// 结构与 `InitializeMessage` 类似，但不包含 `id` 和 `supported_protocol_versions`，
/// 因为插件不需要发起新的握手流程。
///
/// ## Skill 声明
///
/// 插件可以通过 `skills` 字段声明自己提供的 skill。Host 将这些声明解析为
/// `SkillSpec`（来源标记为 `Plugin`），并统一纳入 `SkillCatalog` 管理。
/// Skill 资产文件会在初始化时被物化到 runtime 缓存目录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResultData {
    /// 插件确认使用的协议版本
    pub protocol_version: String,
    /// 插件的 peer 描述
    pub peer: PeerDescriptor,
    /// 插件注册的能力列表
    #[serde(default)]
    pub capabilities: Vec<CapabilityWireDescriptor>,
    /// 插件注册的事件处理器列表
    #[serde(default)]
    pub handlers: Vec<HandlerDescriptor>,
    /// 插件支持的 profile 列表
    #[serde(default)]
    pub profiles: Vec<ProfileDescriptor>,
    /// 插件声明的 skill 列表。
    ///
    /// 这些 skill 会被 host 解析为 `SkillSpec`，来源标记为 `Plugin`。
    /// Skill 资产文件会被物化到 runtime 缓存目录供运行时访问。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillDescriptor>,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}
