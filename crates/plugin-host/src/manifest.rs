//! # 插件清单
//!
//! 插件清单从 `Plugin.toml` 文件解析而来，描述插件的名称、版本、能力声明和启动方式。

use astrcode_core::CapabilitySpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResourceManifestEntry {
    pub id: String,
    pub kind: String,
    pub locator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CommandManifestEntry {
    pub id: String,
    pub entry_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ThemeManifestEntry {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProviderManifestEntry {
    pub id: String,
    pub api_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PromptManifestEntry {
    pub id: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillManifestEntry {
    pub id: String,
    pub entry_ref: String,
}

/// 插件类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PluginType {
    Tool,
    Orchestrator,
    Provider,
    Hook,
}

/// 插件清单。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub plugin_type: Vec<PluginType>,
    pub capabilities: Vec<CapabilitySpec>,
    pub executable: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub repository: Option<String>,
    #[serde(default)]
    pub resources: Vec<ResourceManifestEntry>,
    #[serde(default)]
    pub commands: Vec<CommandManifestEntry>,
    #[serde(default)]
    pub themes: Vec<ThemeManifestEntry>,
    #[serde(default)]
    pub providers: Vec<ProviderManifestEntry>,
    #[serde(default)]
    pub prompts: Vec<PromptManifestEntry>,
    #[serde(default)]
    pub skills: Vec<SkillManifestEntry>,
}
