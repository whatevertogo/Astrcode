//! # 插件清单
//!
//! 定义了插件的描述性元数据结构（`PluginManifest`）和类型枚举（`PluginType`）。
//!
//! 插件清单从 `Plugin.toml` 文件解析而来，描述插件的名称、版本、能力声明和启动方式。

use astrcode_protocol::capability::CapabilityDescriptor;
use serde::{Deserialize, Serialize};

use crate::AstrError;

/// 插件类型。
///
/// 一个插件可以同时声明多种类型，每种类型对应不同的运行时行为。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginType {
    /// 工具插件：提供可被 LLM 调用的能力
    Tool,
    /// 编排器插件：控制 Agent 的执行流程
    Orchestrator,
    /// 提供商插件：提供 LLM API 访问
    Provider,
    /// 钩子插件：在特定生命周期事件上执行
    Hook,
}

/// 插件清单。
///
/// 从 `Plugin.toml` 解析，描述插件的元数据和能力声明。
/// `name` 字段必须与插件目录名一致（kebab-case）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    /// 插件名称（必须与目录名一致，kebab-case）
    pub name: String,
    /// 语义化版本号
    pub version: String,
    /// 插件描述
    pub description: String,
    /// 插件类型列表
    pub plugin_type: Vec<PluginType>,
    /// 能力描述符列表（声明该插件提供的能力）
    pub capabilities: Vec<CapabilityDescriptor>,
    /// 可执行文件路径（可选，用于 sidecar 模式启动）
    pub executable: Option<String>,
    /// 启动参数
    #[serde(default)]
    pub args: Vec<String>,
    /// 工作目录（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// 仓库地址（可选）
    pub repository: Option<String>,
}

impl PluginManifest {
    /// 从 TOML 字符串解析插件清单。
    ///
    /// 解析失败时返回 `AstrError::Validation`，包含详细的错误信息。
    pub fn from_toml(s: &str) -> std::result::Result<Self, AstrError> {
        toml::from_str(s).map_err(|error| {
            AstrError::Validation(format!("failed to parse plugin manifest TOML: {error}"))
        })
    }
}
