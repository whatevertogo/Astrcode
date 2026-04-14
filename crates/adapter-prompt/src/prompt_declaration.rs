//! Prompt 声明的 DTO 定义。
//!
//! [`PromptDeclaration`] 是插件或 MCP 服务器向 prompt 组装管线注入内容的标准化格式。
//! 与 skill 不同，prompt declaration 直接提供完整的 prompt 文本，而非按需加载。
//!
//! # 与 Skill 的区别
//!
//! - Skill：system prompt 中仅暴露索引（名称+描述），正文通过 `Skill` tool 按需加载
//! - PromptDeclaration：直接注入到 system prompt 或对话消息中，始终可见

pub use astrcode_core::{
    PromptDeclarationKind, PromptDeclarationRenderTarget, PromptDeclarationSource,
    SystemPromptLayer as PromptLayer,
};
use serde::{Deserialize, Serialize};

use crate::{BlockKind, RenderTarget};

pub(crate) fn prompt_declaration_block_kind(kind: &PromptDeclarationKind) -> BlockKind {
    match kind {
        PromptDeclarationKind::ToolGuide => BlockKind::ToolGuide,
        PromptDeclarationKind::ExtensionInstruction => BlockKind::ExtensionInstruction,
    }
}

pub(crate) fn prompt_declaration_category(kind: &PromptDeclarationKind) -> &'static str {
    match kind {
        PromptDeclarationKind::ToolGuide => "capabilities",
        PromptDeclarationKind::ExtensionInstruction => "extensions",
    }
}

pub(crate) fn prompt_declaration_source_tag(source: &PromptDeclarationSource) -> &'static str {
    match source {
        PromptDeclarationSource::Builtin => "source:builtin",
        PromptDeclarationSource::Plugin => "source:plugin",
        PromptDeclarationSource::Mcp => "source:mcp",
    }
}

pub(crate) fn prompt_declaration_render_target(
    render_target: &PromptDeclarationRenderTarget,
) -> RenderTarget {
    match render_target {
        PromptDeclarationRenderTarget::System => RenderTarget::System,
        PromptDeclarationRenderTarget::PrependUser => RenderTarget::PrependUser,
        PromptDeclarationRenderTarget::PrependAssistant => RenderTarget::PrependAssistant,
        PromptDeclarationRenderTarget::AppendUser => RenderTarget::AppendUser,
        PromptDeclarationRenderTarget::AppendAssistant => RenderTarget::AppendAssistant,
    }
}

impl From<astrcode_core::PromptDeclaration> for PromptDeclaration {
    fn from(value: astrcode_core::PromptDeclaration) -> Self {
        Self {
            block_id: value.block_id,
            title: value.title,
            content: value.content,
            render_target: value.render_target,
            layer: value.layer,
            kind: value.kind,
            priority_hint: value.priority_hint,
            always_include: value.always_include,
            source: value.source,
            capability_name: value.capability_name,
            origin: value.origin,
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
    #[serde(default, skip_serializing_if = "is_unspecified_prompt_layer")]
    pub layer: PromptLayer,
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

impl From<PromptDeclaration> for astrcode_core::PromptDeclaration {
    fn from(value: PromptDeclaration) -> Self {
        Self {
            block_id: value.block_id,
            title: value.title,
            content: value.content,
            render_target: value.render_target,
            layer: value.layer,
            kind: value.kind,
            priority_hint: value.priority_hint,
            always_include: value.always_include,
            source: value.source,
            capability_name: value.capability_name,
            origin: value.origin,
        }
    }
}

fn is_unspecified_prompt_layer(layer: &PromptLayer) -> bool {
    matches!(layer, PromptLayer::Unspecified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_declaration_round_trip_matches_core_shape() {
        let declaration = PromptDeclaration {
            block_id: "tool.shell".to_string(),
            title: "Shell Tool".to_string(),
            content: "use shell carefully".to_string(),
            render_target: PromptDeclarationRenderTarget::PrependAssistant,
            layer: PromptLayer::Inherited,
            kind: PromptDeclarationKind::ToolGuide,
            priority_hint: Some(42),
            always_include: true,
            source: PromptDeclarationSource::Builtin,
            capability_name: Some("shell".to_string()),
            origin: Some("builtin:test".to_string()),
        };

        let core: astrcode_core::PromptDeclaration = declaration.clone().into();
        let round_trip: PromptDeclaration = core.into();

        assert_eq!(round_trip, declaration);
    }
}
