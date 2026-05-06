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
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, SystemPromptLayer as PromptLayer,
};

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
