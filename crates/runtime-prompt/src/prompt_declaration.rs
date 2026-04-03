//! Prompt 声明的 DTO 定义。
//!
//! [`PromptDeclaration`] 是插件或 MCP 服务器向 prompt 组装管线注入内容的标准化格式。
//! 与 skill 不同，prompt declaration 直接提供完整的 prompt 文本，而非按需加载。
//!
//! # 与 Skill 的区别
//!
//! - Skill：system prompt 中仅暴露索引（名称+描述），正文通过 `Skill` tool 按需加载
//! - PromptDeclaration：直接注入到 system prompt 或对话消息中，始终可见

use serde::{Deserialize, Serialize};

use crate::{BlockKind, RenderTarget};

/// Prompt 声明的来源。
///
/// 与 `astrcode_runtime_skill_loader::SkillSource` 保持独立，因为两者的来源集合可能不同
/// （如未来可能出现 `PromptDeclarationSource::System`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationSource {
    Builtin,
    #[default]
    Plugin,
    Mcp,
}

impl PromptDeclarationSource {
    pub fn as_tag(&self) -> &'static str {
        match self {
            Self::Builtin => "source:builtin",
            Self::Plugin => "source:plugin",
            Self::Mcp => "source:mcp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationKind {
    /// 工具使用指南，映射到 [`BlockKind::ToolGuide`]。
    ToolGuide,
    /// 扩展指令（插件或 MCP 注入的 prompt），默认类型。
    /// 映射到 [`BlockKind::ExtensionInstruction`]。
    #[default]
    ExtensionInstruction,
}

impl PromptDeclarationKind {
    pub fn as_block_kind(&self) -> BlockKind {
        match self {
            Self::ToolGuide => BlockKind::ToolGuide,
            Self::ExtensionInstruction => BlockKind::ExtensionInstruction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptDeclarationRenderTarget {
    /// 渲染到 system prompt（默认）。
    #[default]
    System,
    /// 插入到用户消息列表头部。
    PrependUser,
    /// 插入到助手消息列表头部。
    PrependAssistant,
    /// 追加到用户消息列表尾部。
    AppendUser,
    /// 追加到助手消息列表尾部。
    AppendAssistant,
}

impl PromptDeclarationRenderTarget {
    pub fn as_render_target(&self) -> RenderTarget {
        match self {
            Self::System => RenderTarget::System,
            Self::PrependUser => RenderTarget::PrependUser,
            Self::PrependAssistant => RenderTarget::PrependAssistant,
            Self::AppendUser => RenderTarget::AppendUser,
            Self::AppendAssistant => RenderTarget::AppendAssistant,
        }
    }
}

/// 插件或 MCP 服务器声明的 prompt 内容。
///
/// 这是一种"直接注入"式的 prompt 贡献，与 contributor 的编程式生成不同，
/// prompt declaration 通过序列化数据（通常来自插件 API）直接定义 block 内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptDeclaration {
    pub block_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub render_target: PromptDeclarationRenderTarget,
    #[serde(default)]
    pub kind: PromptDeclarationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<i32>,
    #[serde(default)]
    pub always_include: bool,
    #[serde(default)]
    pub source: PromptDeclarationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}
